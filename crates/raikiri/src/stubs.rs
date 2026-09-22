//! Entry points that are not yet backed by the full planning state machine.
//!
//! `plan` remains an explicit unavailable API. `render_streaming` is the
//! neutral page-output bridge: it uses the merged `raikiri-dom` pagination
//! projection and keeps renderer-specific scene and drawable code out of the
//! sink contract.

use parley::FontContext;
use raikiri_dom::{
    FontFaceLoader, PageSlice, apply_font_faces, first_page_name,
    layout_pages_with_page_geometry_and_resolver_and_base_url,
    layout_pages_with_resolver_and_base_url, page_fragment_events_from_pages,
    page_fragments_from_slices_with_page_geometry, resolve_page_fragment_geometry,
};
use raikiri_style::FontFaceRegistry;
use raikiri_traits::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyValue, DocumentPlan,
    IntrinsicBox, PageBox, PageDefaults, PageEventObserver, PageFragment, PageFragmentPageGeometry,
    PlanConfig, PolicyViolation, RenderError, RenderSink, RenderStatus, RenderStatus::Aborted,
    RenderStatus::Completed, RenderSummary, RenderWarning, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest, ResourceKind, ResourcePolicy,
    StreamingConfig, ViolationType, WarningKind,
};
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::resources::{
    NetworkFontFaceLoader, RenderResources, SharedRenderWarnings, parse_html_with_resources,
    push_resource_warning, redacted_url, sanitize_policy_violation,
};
use crate::{
    Atom, ConsumerPropertyGrammar, ConsumerPropertyRegistration, HtmlDocument, MediaContext,
    PageContextQuery, build_cascaded_with_media_context_for_page_and_consumer_properties,
};

struct RenderExecutionResources<'a> {
    font_context: FontContext,
    font_faces: Option<&'a FontFaceRegistry>,
    font_face_loader: Option<&'a dyn FontFaceLoader>,
    effective_base_url: Option<&'a url::Url>,
    warnings: SharedRenderWarnings,
}

impl RenderExecutionResources<'_> {
    fn legacy() -> Self {
        Self {
            font_context: FontContext::new(),
            font_faces: None,
            font_face_loader: None,
            effective_base_url: None,
            warnings: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

struct FallbackRecordingResolver<'a> {
    inner: &'a dyn ReplacedResolver,
    policy: Option<&'a dyn ResourcePolicy>,
    image_pixel_source: Option<&'a dyn crate::ImagePixelSource>,
    warnings: SharedRenderWarnings,
    seen: Mutex<HashSet<(url::Url, String)>>,
}

impl ReplacedResolver for FallbackRecordingResolver<'_> {
    #[allow(clippy::result_large_err)]
    fn resolve(&self, request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        let url = request.url().clone();
        let mut resolved = if let Some(policy) = self.policy {
            let violation_type = if !policy.is_scheme_allowed(url.scheme(), ResourceKind::Image) {
                Some(ViolationType::SchemeNotAllowed)
            } else if !policy.is_host_allowed(url.host_str().unwrap_or(""), ResourceKind::Image) {
                Some(ViolationType::HostNotAllowed)
            } else {
                None
            };
            if let Some(violation_type) = violation_type {
                let violation = PolicyViolation {
                    kind: ResourceKind::Image,
                    url: url.clone(),
                    violation_type,
                    details: "replaced resource URL denied by policy".to_owned(),
                };
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind: WarningKind::PolicyWarning {
                            violation: sanitize_policy_violation(violation),
                        },
                        node_id: None,
                        details: "replaced resource URL denied by policy".to_owned(),
                    },
                );
                Ok(ResolvedIntrinsic {
                    intrinsic: IntrinsicBox::default(),
                    disposition: ResolveDisposition::Fallback {
                        reason: "resource policy denied the replaced resource".to_owned(),
                    },
                })
            } else {
                self.inner.resolve(request)
            }
        } else {
            self.inner.resolve(request)
        }?;
        if matches!(&resolved.disposition, ResolveDisposition::Ok)
            && let (Some(source), Some(limit)) = (
                self.image_pixel_source,
                self.policy
                    .and_then(|policy| policy.max_decoded_bytes(ResourceKind::Image)),
            )
            && let Some(image) = source.get_decoded(&url)
        {
            let actual = image.rgba.len() as u64;
            if actual > limit {
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind: WarningKind::ResourceLimitExceeded {
                            kind: ResourceKind::Image,
                            limit,
                            actual,
                        },
                        node_id: None,
                        details: "decoded image exceeded its configured byte limit".to_owned(),
                    },
                );
                resolved.intrinsic = IntrinsicBox::default();
                resolved.disposition = ResolveDisposition::Fallback {
                    reason: "decoded image exceeded its configured byte limit".to_owned(),
                };
            }
        }
        if let ResolveDisposition::Fallback { reason } = &resolved.disposition {
            let mut seen = self
                .seen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if seen.insert((url.clone(), reason.clone())) {
                push_resource_warning(
                    &self.warnings,
                    RenderWarning {
                        kind: WarningKind::ResourceFallback {
                            kind: ResourceKind::Image,
                            url: Some(redacted_url(&url)),
                        },
                        node_id: None,
                        details: reason.clone(),
                    },
                );
            }
        }
        Ok(resolved)
    }
}

struct NoopReplacedResolver;

impl ReplacedResolver for NoopReplacedResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::default(),
            disposition: ResolveDisposition::Fallback {
                reason: "no replaced-resource resolver was configured".to_owned(),
            },
        })
    }
}

/// Stream pages from a parsed document using one shared consumer resource handoff.
///
/// Font faces are registered into a clone of the configured font context before
/// layout. That prepared context is cloned for each bounded page-geometry pass;
/// paint consumes the shaped runs stored by layout. The same replaced-resource
/// provider is used by the layout resolver and remains available as the
/// configured image pixel source for downstream painting.
#[allow(clippy::result_large_err)]
pub fn render_streaming_with_resources(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resources: &RenderResources<'_>,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let network = resources.network_adapter();
    let network_ref = network
        .as_ref()
        .map(|provider| provider as &dyn raikiri_traits::NetworkProvider);
    let effective_base_url =
        raikiri_html::effective_document_base_url(&doc.uncascaded, resources.fallback_base_url());
    let font_loader = NetworkFontFaceLoader::new(
        network_ref,
        effective_base_url.as_ref(),
        Arc::clone(&warnings),
    );
    let runtime = RenderExecutionResources {
        font_context: resources.clone_font_context(),
        font_faces: Some(&doc.font_faces),
        font_face_loader: Some(&font_loader),
        effective_base_url: effective_base_url.as_ref(),
        warnings: Arc::clone(&warnings),
    };
    let noop_resolver = NoopReplacedResolver;
    let inner_resolver = resources.resolver().unwrap_or(&noop_resolver);
    let resolver = FallbackRecordingResolver {
        inner: inner_resolver,
        policy: resources.policy(),
        image_pixel_source: resources.raw_image_pixel_source(),
        warnings,
        seen: Mutex::new(HashSet::new()),
    };
    render_streaming_inner(
        doc,
        defaults,
        &resolver,
        config,
        sink,
        None,
        None,
        &[],
        runtime,
    )
}

/// Parse and stream HTML with one resource configuration shared by both phases.
#[allow(clippy::result_large_err)]
pub fn render_html_streaming_with_resources<R: std::io::Read>(
    input: R,
    defaults: PageDefaults,
    resources: &RenderResources<'_>,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    let doc = parse_html_with_resources(input, resources)?;
    render_streaming_with_resources(&doc, defaults, resources, config, sink)
}

/// Parse and collect all emitted pages using the shared resource handoff.
///
/// This is the batch-collection sibling of [`render_html_streaming_with_resources`].
/// It uses the same parse, font registration, policy, resolver, and warning
/// paths, but retains every page in memory before returning.
#[allow(clippy::result_large_err)]
pub fn render_html_pages_with_resources<R: std::io::Read>(
    input: R,
    defaults: PageDefaults,
    resources: &RenderResources<'_>,
    config: StreamingConfig,
) -> Result<(Vec<PageFragment>, RenderStatus), RenderError> {
    let mut sink = PageCollector::default();
    let status =
        render_html_streaming_with_resources(input, defaults, resources, config, &mut sink)?;
    Ok((sink.pages, status))
}

#[derive(Default)]
struct PageCollector {
    pages: Vec<PageFragment>,
}

impl RenderSink for PageCollector {
    fn accept_page(&mut self, page: PageFragment) -> std::io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn finish_render(&mut self, _summary: RenderSummary) -> std::io::Result<()> {
        Ok(())
    }
}

/// Build the page-context query used by the neutral page stream.
///
/// The first page is treated as recto (`:right`) by default. Blank-page state
/// is not inferred here because the current `PageSlice` contract does not
/// expose blank-page insertion; that remains an explicit pagination follow-up.
fn parse_consumer_integer(raw: &str) -> Option<i32> {
    let mut input = cssparser::ParserInput::new(raw);
    let mut parser = cssparser::Parser::new(&mut input);
    parser
        .parse_entirely(|parser| {
            let value = parser
                .expect_integer()
                .map_err(|_| parser.new_custom_error(()))?;
            Ok::<_, cssparser::ParseError<'_, ()>>(value)
        })
        .ok()
}

fn parse_consumer_integer_or_none(raw: &str) -> Option<ConsumerPropertyValue> {
    let mut input = cssparser::ParserInput::new(raw);
    let mut parser = cssparser::Parser::new(&mut input);
    parser
        .parse_entirely(
            |parser| -> Result<ConsumerPropertyValue, cssparser::ParseError<'_, ()>> {
                if parser
                    .try_parse(|parser| parser.expect_ident_matching("none"))
                    .is_ok()
                {
                    Ok(ConsumerPropertyValue::None)
                } else {
                    parser
                        .expect_integer()
                        .map(ConsumerPropertyValue::Integer)
                        .map_err(|_| parser.new_custom_error(()))
                }
            },
        )
        .ok()
}

fn element_text_value(document: &raikiri_dom::Document, node_id: usize) -> String {
    fn append_text(document: &raikiri_dom::Document, node_id: usize, output: &mut String) {
        let Some(node) = document.get_node(node_id) else {
            return; // cov:ignore: DOM child indices are validated by the document arena
        };
        if !node.is_in_document() {
            return; // cov:ignore: detached nodes are excluded before traversal
        }
        if node.is_non_rendered_html_element() {
            return;
        }
        if let Some(text) = node.text_content() {
            output.push_str(text);
            return;
        }
        for &child in &node.children {
            append_text(document, child, output);
        }
    }

    let mut raw = String::new();
    append_text(document, node_id, &mut raw);
    let mut output = String::new();
    let mut pending_space = false;
    for character in raw.chars() {
        if character.is_ascii_whitespace() {
            if !output.is_empty() {
                pending_space = true;
            }
        } else {
            if pending_space {
                output.push(' ');
                pending_space = false;
            }
            output.push(character);
        }
    }
    output
}

#[derive(Debug)]
enum ConsumerTextPart {
    Literal(String),
    Attribute {
        name: String,
        fallback: Option<String>,
    },
    ElementText,
}

fn parse_consumer_text_parts(raw: &str) -> Option<Vec<ConsumerTextPart>> {
    let mut input = cssparser::ParserInput::new(raw);
    let mut parser = cssparser::Parser::new(&mut input);
    parser
        .parse_entirely(
            |parser| -> Result<Vec<ConsumerTextPart>, cssparser::ParseError<'_, ()>> {
                let mut parts = Vec::new();
                loop {
                    let token = match parser.next() {
                        Ok(token) => token.clone(),
                        Err(cssparser::BasicParseError {
                            kind: cssparser::BasicParseErrorKind::EndOfInput,
                            ..
                        }) => break,
                        // cov:ignore: declarations are validated by the style parser before this reparse
                        Err(error) => return Err(error.into()),
                    };
                    match token {
                        cssparser::Token::QuotedString(value) => {
                            parts.push(ConsumerTextPart::Literal(value.to_string()));
                        }
                        cssparser::Token::Function(name) => {
                            let function_name = name.to_ascii_lowercase();
                            let part = parser.parse_nested_block(|nested| {
                                if function_name == "attr" {
                                    let attribute = nested.expect_ident()?.to_string();
                                    let fallback =
                                        if nested.try_parse(|parser| parser.expect_comma()).is_ok()
                                        {
                                            Some(nested.expect_string()?.to_string())
                                        } else {
                                            None
                                        };
                                    nested.expect_exhausted()?;
                                    Ok(ConsumerTextPart::Attribute {
                                        name: attribute,
                                        fallback,
                                    })
                                } else if function_name == "content" {
                                    if nested
                                        .try_parse(|parser| parser.expect_ident_matching("text"))
                                        .is_err()
                                    {
                                        // The generic text grammar accepts the
                                        // omitted argument as the element text;
                                        // any other keyword is not approximated.
                                        // cov:ignore: the style grammar rejects unsupported content arguments first
                                        if !nested.is_exhausted() {
                                            return Err(nested.new_custom_error(()));
                                        }
                                    }
                                    nested.expect_exhausted()?;
                                    Ok(ConsumerTextPart::ElementText)
                                } else {
                                    Err(nested.new_custom_error(())) // cov:ignore: unknown functions are rejected by the style grammar
                                }
                            })?;
                            parts.push(part);
                        }
                        _ => return Err(parser.new_custom_error(())), // cov:ignore: non-text tokens are rejected by the style grammar
                    }
                }
                if parts.is_empty() {
                    return Err(parser.new_custom_error(())); // cov:ignore: empty content lists are rejected by the style grammar
                }
                Ok(parts)
            },
        )
        .ok()
}

fn resolve_consumer_text(
    document: &raikiri_dom::Document,
    node_id: usize,
    raw: &str,
) -> Option<String> {
    let parts = parse_consumer_text_parts(raw)?;
    let node = document.get_node(node_id)?;
    let mut output = String::new();
    for part in parts {
        match part {
            ConsumerTextPart::Literal(value) => output.push_str(&value),
            ConsumerTextPart::Attribute { name, fallback } => {
                let value = node.attribute(&name).or_else(|| {
                    let normalized = name.to_ascii_lowercase();
                    (normalized != name)
                        .then(|| node.attribute(&normalized))
                        .flatten()
                });
                if let Some(value) = value {
                    output.push_str(value);
                } else if let Some(fallback) = fallback {
                    output.push_str(&fallback);
                }
            }
            ConsumerTextPart::ElementText => {
                output.push_str(&element_text_value(document, node_id))
            }
        }
    }
    Some(output)
}

fn consumer_property_value(
    document: &raikiri_dom::Document,
    node_id: usize,
    registration: &ConsumerPropertyRegistration,
    raw: &str,
) -> Option<ConsumerPropertyValue> {
    match registration.grammar() {
        ConsumerPropertyGrammar::Integer => {
            parse_consumer_integer(raw).map(ConsumerPropertyValue::Integer)
        }
        ConsumerPropertyGrammar::IntegerOrNone => parse_consumer_integer_or_none(raw),
        ConsumerPropertyGrammar::Text => {
            resolve_consumer_text(document, node_id, raw).map(ConsumerPropertyValue::Text)
        }
        _ => None, // cov:ignore: non-exhaustive grammar variants are future-only
    }
}

fn resolved_consumer_property_events(
    document: &raikiri_dom::Document,
    cascade: &raikiri_style::CascadeResult,
    registrations: &[ConsumerPropertyRegistration],
) -> Vec<ConsumerPropertyEvent> {
    if registrations.is_empty() {
        return Vec::new();
    }

    let mut events = Vec::new();
    let root = document.root_index();
    let root_children = document
        .get_node(root)
        .map(|node| node.children.clone())
        .unwrap_or_default();
    let mut stack: Vec<(usize, Option<usize>)> = root_children
        .into_iter()
        .rev()
        .map(|child| (child, Some(root)))
        .collect();
    let mut source_order = 0_u32;

    while let Some((node_id, parent_id)) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: the document traversal stack contains arena-owned indices
        };
        if !node.is_in_document() {
            continue; // cov:ignore: detached nodes are excluded before traversal
        }
        let current_order = source_order;
        source_order = source_order.saturating_add(1);

        if node.kind() == raikiri_traits::NodeKind::Element
            && let Some(computed) = cascade.computed.get(node_id)
        {
            for registration in registrations {
                let storage_name = format!("--{}", registration.name());
                let raw = if registration.inherits() {
                    computed.resolved_custom_property(storage_name.as_str())
                } else {
                    computed.local_resolved_custom_property(storage_name.as_str())
                };
                let Some(raw) = raw else {
                    continue;
                };
                let Some(value) = consumer_property_value(document, node_id, registration, &raw)
                else {
                    continue;
                };
                events.push(ConsumerPropertyEvent::new(
                    raikiri_traits::NodeId::new(node_id as u64),
                    parent_id.map(|parent| raikiri_traits::NodeId::new(parent as u64)),
                    current_order,
                    registration.name().to_owned(),
                    value,
                ));
            }
        }

        let mut children = node.children.clone();
        children.reverse();
        for child in children {
            stack.push((child, Some(node_id)));
        }
    }
    events
}

fn page_query_for_slice(slice: &PageSlice) -> PageContextQuery {
    let mut query = PageContextQuery::default();
    query.page_name = slice.page_name.as_deref().map(Atom::from);
    query.is_first = slice.page_index == 0;
    query.is_left = slice.page_index % 2 == 1;
    query.is_right = !query.is_left;
    query
}

fn page_box_for_cascade(
    cascade: &raikiri_style::CascadeResult,
    defaults: &PageDefaults,
) -> PageBox {
    cascade
        .page
        .size()
        .map(|size| PageBox::from_page_size(Some(size)))
        .unwrap_or(defaults.page_box)
}

fn resolve_page_geometries(
    doc: &HtmlDocument,
    defaults: &PageDefaults,
    slices: &[PageSlice],
    consumer_properties: &[ConsumerPropertyRegistration],
) -> Vec<PageFragmentPageGeometry> {
    slices
        .iter()
        .map(|slice| {
            let query = page_query_for_slice(slice);
            let cascade = build_cascaded_with_media_context_for_page_and_consumer_properties(
                &doc.uncascaded,
                &MediaContext::default(),
                &query,
                consumer_properties,
            );
            let page_box = page_box_for_cascade(&cascade, defaults);
            resolve_page_fragment_geometry(&cascade, page_box, slice.page_index)
        })
        .collect()
}

fn content_width_for_geometry(geometry: PageFragmentPageGeometry) -> f32 {
    (geometry.page_box.width - geometry.margins.left - geometry.margins.right).max(0.0)
}

#[derive(Debug, Clone, PartialEq)]
struct PageGeometrySchedule {
    page_steps: Vec<f32>,
    page_widths: Vec<f32>,
    page_names: Vec<Option<String>>,
}

fn page_geometry_schedule(
    page_geometries: &[PageFragmentPageGeometry],
    slices: &[PageSlice],
) -> PageGeometrySchedule {
    PageGeometrySchedule {
        page_steps: page_geometries
            .iter()
            .map(|geometry| geometry.content_box.height)
            .collect(),
        page_widths: page_geometries
            .iter()
            .map(|geometry| content_width_for_geometry(*geometry))
            .collect(),
        page_names: slices.iter().map(|slice| slice.page_name.clone()).collect(),
    }
}

fn geometry_differs(left: PageFragmentPageGeometry, right: PageFragmentPageGeometry) -> bool {
    left.page_box != right.page_box
        || left.margins != right.margins
        || left.content_insets != right.content_insets
        || left.content_box != right.content_box
        || left.orientation != right.orientation
}

/// Plan mode (dry-run: parse+cascade+layout planning のみ、PaintedBox 構築なし)。
///
/// **Unavailable implementation**: 常に `Err(RenderError::Unimplemented { feature: "plan", .. })` を
/// 返す。本実装は pagination 完了後。
///
/// spec §L1075 の signature 準拠。
#[allow(clippy::result_large_err)]
pub fn plan(
    _doc: &HtmlDocument,
    _defaults: PageDefaults,
    _resolver: &dyn ReplacedResolver,
    _config: PlanConfig,
) -> Result<DocumentPlan, RenderError> {
    Err(RenderError::Unimplemented {
        feature: "plan",
        migration_hint: "non-goal for now; populated once pagination is implemented",
    })
}

/// Stream neutral page snapshots to a consumer sink.
///
/// The current driver lays out the document into neutral snapshots before
/// emitting them. This keeps the public contract renderer-neutral while the
/// pagination state machine grows: no Taffy, Parley, style, scene, drawable,
/// or PDF value crosses the sink boundary. The input document is cloned for
/// the mutating layout pass, so the existing shared `&HtmlDocument` API stays
/// source-compatible.
///
/// A configured [`AbortSignal`](raikiri_traits::AbortSignal) is checked before
/// layout, before every page, and before completion. Aborted renders return
/// without calling `RenderSink::finish_render`. If the bounded page-geometry
/// schedule does not converge, [`RenderError::PageGeometryDidNotConverge`]
/// is returned before any page is emitted.
///
/// The compatibility path does not register consumer properties and therefore
/// retains the existing parse, cascade, and traversal behavior.
#[allow(clippy::result_large_err)]
pub fn render_streaming(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
) -> Result<RenderStatus, RenderError> {
    render_streaming_inner(
        doc,
        defaults,
        resolver,
        config,
        sink,
        None,
        None,
        &[],
        RenderExecutionResources::legacy(),
    )
}

/// Stream neutral page snapshots and page-local link/annotation events.
///
/// This is the opt-in observer variant of [`render_streaming`]. The page
/// stream remains renderer-neutral, and the observer receives only opaque
/// [`NodeId`](raikiri_traits::NodeId), page indices, CSS-pixel rectangles, and
/// link values. Events are delivered after the corresponding
/// [`RenderSink::accept_page`] call. Observer I/O failures are returned as
/// [`RenderError::Sink`]; the page may already have been accepted and
/// `finish_render` is skipped. Successful renders still call
/// [`RenderSink::finish_render`] exactly once.
#[allow(clippy::result_large_err)]
pub fn render_streaming_with_observer(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
    observer: &mut dyn PageEventObserver,
) -> Result<RenderStatus, RenderError> {
    render_streaming_inner(
        doc,
        defaults,
        resolver,
        config,
        sink,
        Some(observer),
        None,
        &[],
        RenderExecutionResources::legacy(),
    )
}

/// Stream pages and resolved registered consumer properties.
///
/// Consumer-property events are delivered before the first page is accepted.
/// They contain no page geometry; join them to the later neutral page-fragment
/// events with their opaque `NodeId`. A property observer error is returned as
/// [`RenderError::Sink`] and prevents page emission and completion.
#[allow(clippy::result_large_err)]
pub fn render_streaming_with_consumer_properties(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    consumer_properties: &[ConsumerPropertyRegistration],
    sink: &mut dyn RenderSink,
    observer: &mut dyn ConsumerPropertyObserver,
) -> Result<RenderStatus, RenderError> {
    render_streaming_inner(
        doc,
        defaults,
        resolver,
        config,
        sink,
        None,
        Some(observer),
        consumer_properties,
        RenderExecutionResources::legacy(),
    )
}

#[allow(clippy::result_large_err, clippy::too_many_arguments)]
fn render_streaming_inner(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    resolver: &dyn ReplacedResolver,
    config: StreamingConfig,
    sink: &mut dyn RenderSink,
    mut page_observer: Option<&mut dyn PageEventObserver>,
    property_observer: Option<&mut dyn ConsumerPropertyObserver>,
    consumer_properties: &[ConsumerPropertyRegistration],
    runtime: RenderExecutionResources<'_>,
) -> Result<RenderStatus, RenderError> {
    let signal = config.signal.clone();
    let is_aborted = || signal.as_ref().is_some_and(|signal| signal.is_aborted());
    if is_aborted() {
        return Ok(Aborted { partial_pages: 0 });
    }

    // Resolve the first page context before layout so `:first` and the first
    // resolved `@page size` participate in the initial fragmentainer.
    let mut first_query = PageContextQuery::default();
    first_query.is_first = true;
    first_query.is_right = true;
    let mut first_cascade = build_cascaded_with_media_context_for_page_and_consumer_properties(
        &doc.uncascaded,
        &MediaContext::default(),
        &first_query,
        consumer_properties,
    );
    // The first class-A box can select a named page. Resolve that name before
    // the initial layout so a named `:first` page is not flattened to the
    // anonymous page geometry.
    if let Some(name) = first_page_name(&doc.uncascaded.dom, &first_cascade) {
        first_query.page_name = Some(Atom::from(name.as_str()));
        first_cascade = build_cascaded_with_media_context_for_page_and_consumer_properties(
            &doc.uncascaded,
            &MediaContext::default(),
            &first_query,
            consumer_properties,
        );
    }
    let mut font_context = runtime.font_context.clone();
    if let (Some(font_faces), Some(font_loader)) = (runtime.font_faces, runtime.font_face_loader) {
        let report = apply_font_faces(
            &mut font_context,
            &mut first_cascade.computed,
            font_faces,
            font_loader,
        );
        for family in report.skipped {
            push_resource_warning(
                &runtime.warnings,
                RenderWarning {
                    kind: WarningKind::ResourceFallback {
                        kind: ResourceKind::Font,
                        url: None,
                    },
                    node_id: None,
                    details: format!(
                        "@font-face family {family:?} had no usable source; fallback fonts will be used"
                    ),
                },
            );
        }
    }
    let page_box = page_box_for_cascade(&first_cascade, &defaults);
    let mut document = doc.uncascaded.dom.clone();
    let mut slices = layout_pages_with_resolver_and_base_url(
        &mut document,
        &first_cascade,
        page_box,
        font_context.clone(),
        resolver,
        runtime.effective_base_url,
    )
    .map_err(RenderError::from)?;
    const MAX_PAGE_GEOMETRY_PASSES: u32 = 3;
    let mut page_geometries = resolve_page_geometries(doc, &defaults, &slices, consumer_properties);
    let mut geometry_converged = true;
    for pass in 0..MAX_PAGE_GEOMETRY_PASSES {
        let Some(first_geometry) = page_geometries.first().copied() else {
            break; // cov:ignore: a successful document with a body always emits a page slice.
        };
        let geometry_varies = page_geometries
            .iter()
            .any(|geometry| geometry_differs(*geometry, first_geometry));
        if !geometry_varies {
            break;
        }

        let schedule = page_geometry_schedule(&page_geometries, &slices);
        // The first pagination pass establishes page count, names, and source
        // coordinates. If page selectors resolve different used geometry, rerun
        // the existing scheduled paginator with producer-owned page steps/widths.
        // This remains a bounded batch layout pass; a changed page count or page
        // name can trigger another schedule pass.
        slices = layout_pages_with_page_geometry_and_resolver_and_base_url(
            &mut document,
            &first_cascade,
            page_box,
            font_context.clone(),
            &schedule.page_steps,
            &schedule.page_widths,
            resolver,
            runtime.effective_base_url,
        )
        .map_err(RenderError::from)?;
        // A scheduled pass can change both page count and page selectors. Re-
        // resolve before the next iteration so the following schedule is
        // derived from the slices it will actually replace.
        page_geometries = resolve_page_geometries(doc, &defaults, &slices, consumer_properties);
        let refreshed_schedule = page_geometry_schedule(&page_geometries, &slices);
        if refreshed_schedule == schedule {
            break;
        }
        // cov:ignore: no current public fixture can keep a static page-rule schedule
        // changing through all three bounded passes; the terminal status has a
        // direct contract test in raikiri-traits.
        if pass + 1 == MAX_PAGE_GEOMETRY_PASSES {
            geometry_converged = false;
        }
    }
    // cov:ignore: see the bounded non-convergence branch above; no inconsistent
    // pages are emitted, and the structured error variant is contract-tested.
    if !geometry_converged {
        return Err(RenderError::PageGeometryDidNotConverge {
            iterations: MAX_PAGE_GEOMETRY_PASSES,
        });
    }

    // Resolve once more after the final bounded schedule pass so metadata and
    // page names always describe the slices that will actually be emitted.
    page_geometries = resolve_page_geometries(doc, &defaults, &slices, consumer_properties);

    let pages = page_fragments_from_slices_with_page_geometry(
        &document,
        &first_cascade,
        page_box,
        &slices,
        &page_geometries,
    );

    let total_pages = u32::try_from(pages.len()).unwrap_or(u32::MAX);
    if config
        .limits
        .max_document_pages
        .is_some_and(|limit| total_pages > limit)
    {
        return Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: config.limits.max_document_pages.unwrap_or(u32::MAX) as u64,
            actual: total_pages as u64,
        });
    }

    if let Some(property_observer) = property_observer {
        if is_aborted() {
            return Ok(Aborted { partial_pages: 0 }); // cov:ignore: cancellation can race after layout and before observer delivery
        }
        for event in
            resolved_consumer_property_events(&document, &first_cascade, consumer_properties)
        {
            property_observer
                .observe_event(event)
                .map_err(RenderError::Sink)?;
        }
    }

    let mut events_by_page = BTreeMap::new();
    if page_observer.is_some() {
        for event in page_fragment_events_from_pages(&document, &pages) {
            let page_index = match &event {
                raikiri_traits::PageFragmentEvent::Link(event) => event.page_index,
                _ => continue, // cov:ignore: future non-exhaustive event variant cannot be constructed here
            };
            events_by_page
                .entry(page_index)
                .or_insert_with(Vec::new)
                .push(event);
        }
    }

    let mut emitted_pages = 0_u32;
    for page in pages {
        if is_aborted() {
            return Ok(Aborted {
                partial_pages: emitted_pages,
            });
        }
        let page_index = page.page_index;
        sink.accept_page(page).map_err(RenderError::Sink)?;
        if let (Some(observer), Some(events)) = (
            page_observer.as_deref_mut(),
            events_by_page.remove(&page_index),
        ) {
            for event in events {
                observer.observe_event(event).map_err(RenderError::Sink)?;
            }
        }
        emitted_pages = emitted_pages.saturating_add(1);
    }
    if is_aborted() {
        return Ok(Aborted {
            partial_pages: emitted_pages,
        });
    }

    let mut warnings = doc.uncascaded.warnings.clone();
    warnings.extend(
        runtime
            .warnings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .cloned(),
    );
    let summary = RenderSummary {
        total_pages: emitted_pages,
        target_registry: config.initial_registry.unwrap_or_default(),
        unresolved_targets: Vec::new(),
        emitted_target_slots: Vec::new(),
        target_discrepancies: Vec::new(),
        warnings,
    };
    sink.finish_render(summary.clone())
        .map_err(RenderError::Sink)?;
    Ok(Completed(summary))
}
