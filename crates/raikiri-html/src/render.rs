//! Resource-aware layout pipeline shared by document layout.

use raikiri_dom::{
    FontFaceLoader, InitialPageContextError, InitialPageProbeResources, PageContentInsets,
    PageLayoutControl, PageMargins, PageSlice, first_page_name,
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control,
    layout_pages_with_resolver_and_base_url_and_control, page_content_insets_for_page,
    page_margins_for_page, resolve_initial_page_context, validate_layout_depth,
};
use raikiri_style::FontFaceRegistry;
use raikiri_traits::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyValue, IntrinsicBox,
    LayoutConfig, LayoutError, PageBox, PageDefaults, PaintRect, PolicyViolation, RenderError,
    RenderWarning, ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError,
    ResolverRequest, ResourceKind, ResourcePolicy, ViolationType, WarningKind,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use raikiri_style::{
    Atom, CascadeOptions, CascadeResult, ConsumerPropertyGrammar, ConsumerPropertyRegistration,
    MediaContext, PageCascadeResult, PageContextQuery, PageInheritance, RuleTree,
    cascade_page_with_media_context, cascade_with_options,
};

use crate::HtmlDocument;
use crate::cascade::build_rule_tree_with_consumer_properties;
#[cfg(doc)]
use crate::parse_html_with_resources;
use crate::resources::{
    NetworkFontFaceLoader, RenderResources, SharedRenderWarnings, push_resource_warning,
    redacted_url, sanitize_policy_violation,
};

mod continuation;
pub(crate) use continuation::Continuation;

struct RenderExecutionResources<'a> {
    font_faces: &'a FontFaceRegistry,
    font_face_loader: &'a dyn FontFaceLoader,
    effective_base_url: Option<&'a url::Url>,
    warnings: SharedRenderWarnings,
}

struct FallbackRecordingResolver<'a> {
    inner: &'a (dyn ReplacedResolver + Send + Sync),
    policy: Option<&'a dyn ResourcePolicy>,
    image_pixel_source: Option<&'a (dyn raikiri_traits::ImagePixelSource + Send + Sync)>,
    warnings: SharedRenderWarnings,
    seen: Mutex<HashSet<(url::Url, String)>>,
}

impl ReplacedResolver for FallbackRecordingResolver<'_> {
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
            && let Some(actual) = source.decoded_byte_len(&url)
            && actual > limit
        {
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

fn parse_consumer_keyword(
    raw: &str,
    keywords: &'static [&'static str],
) -> Option<ConsumerPropertyValue> {
    let mut input = cssparser::ParserInput::new(raw);
    let mut parser = cssparser::Parser::new(&mut input);
    parser
        .parse_entirely(
            |parser| -> Result<ConsumerPropertyValue, cssparser::ParseError<'_, ()>> {
                let ident = parser.expect_ident().cloned()?;
                keywords
                    .iter()
                    .find(|keyword| keyword.eq_ignore_ascii_case(&ident))
                    .map(|keyword| ConsumerPropertyValue::Keyword((*keyword).to_owned()))
                    .ok_or_else(|| parser.new_custom_error(()))
            },
        )
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
    let mut pending = vec![node_id];
    let mut raw = String::new();
    while let Some(node_id) = pending.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: DOM child indices are validated by the document arena
        };
        if !node.is_in_document() {
            continue; // cov:ignore: detached nodes are excluded before traversal
        }
        if node.is_non_rendered_html_element() {
            continue;
        }
        if let Some(text) = node.text_content() {
            raw.push_str(text);
            continue;
        }
        pending.extend(node.children.iter().rev().copied());
    }

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

/// Document-order state read by resolved-text consumer values.
///
/// `counter()` / `counters()` read the counters in effect at the element, after
/// its own `counter-reset` / `counter-increment` / `counter-set`, exactly as
/// the element's `::before` content starts from. `string()` reads the named
/// string's latest `string-set` assignment at or before the element in
/// document order; the page-relative keywords (`first`, `start`, `last`,
/// `first-except`) have no page to select within here and are ignored. A name
/// that has not been assigned yet resolves to the empty string (CSS GCPM 3
/// §1.1.2).
struct ConsumerTextContext<'a> {
    cascade: &'a raikiri_style::CascadeResult,
    counters: Option<Option<Vec<raikiri_dom::CounterSnapshot>>>,
    strings: std::collections::HashMap<smol_str::SmolStr, String>,
}

impl<'a> ConsumerTextContext<'a> {
    fn new(cascade: &'a raikiri_style::CascadeResult) -> Self {
        Self {
            cascade,
            counters: None,
            strings: std::collections::HashMap::new(),
        }
    }

    /// The counters at `node_id`, or `None` when the snapshots would exceed
    /// their memory budget.
    fn counters_at(
        &mut self,
        document: &raikiri_dom::Document,
        node_id: usize,
    ) -> Option<&raikiri_dom::CounterSnapshot> {
        self.counters
            .get_or_insert_with(|| raikiri_dom::counter_snapshots(document, self.cascade).ok())
            .as_ref()?
            .get(node_id)
    }

    /// Record the element's `string-set` assignments in declaration order.
    fn apply_string_set(&mut self, document: &raikiri_dom::Document, node_id: usize) {
        let Some(computed) = self.cascade.computed.get(node_id) else {
            return; // cov:ignore: cascade output has one computed value per arena node
        };
        let assignments = computed.string_set.clone();
        for (name, components) in assignments.iter() {
            if let Some(value) = self.resolve(document, node_id, components, true) {
                self.strings.insert(name.clone(), value);
            }
        }
    }

    /// The text of `components` at `node_id`, or `None` when a component has
    /// no text approximation here. `string_set` selects the named-string
    /// pipeline's element text, which collapses only CSS document white space.
    fn resolve(
        &mut self,
        document: &raikiri_dom::Document,
        node_id: usize,
        components: &[raikiri_style::property::ContentComponent],
        string_set: bool,
    ) -> Option<String> {
        use raikiri_dom::generated_content::{format_counter_component, format_counters_component};
        use raikiri_style::property::{ContentComponent, ContentTextKeyword};

        let node = document.get_node(node_id)?;
        let attribute = |name: &str| {
            node.attribute(name).or_else(|| {
                let normalized = name.to_ascii_lowercase();
                (normalized != name)
                    .then(|| node.attribute(&normalized))
                    .flatten()
            })
        };
        let mut output = String::new();
        for component in components {
            match component {
                ContentComponent::Literal(value) => output.push_str(value),
                ContentComponent::Attr { name } => {
                    output.push_str(attribute(name).unwrap_or_default());
                }
                ContentComponent::AttrFallback { name, fallback } => {
                    if let Some(value) = attribute(name) {
                        output.push_str(value);
                    } else if let Some(fallback) = fallback {
                        output.push_str(fallback);
                    }
                }
                ContentComponent::Content {
                    keyword: ContentTextKeyword::Text,
                } if string_set => {
                    output.push_str(&raikiri_dom::element_string_value(document, node_id));
                }
                ContentComponent::Content {
                    keyword: ContentTextKeyword::Text,
                } => output.push_str(&element_text_value(document, node_id)),
                ContentComponent::Counter { name, style } => {
                    let registry = &self.cascade.counter_styles;
                    let counters = self.counters_at(document, node_id)?;
                    output.push_str(&format_counter_component(counters, name, style, registry));
                }
                ContentComponent::Counters {
                    name,
                    separator,
                    style,
                } => {
                    let registry = &self.cascade.counter_styles;
                    let counters = self.counters_at(document, node_id)?;
                    output.push_str(&format_counters_component(
                        counters, name, separator, style, registry,
                    ));
                }
                ContentComponent::String { name, .. } => {
                    if let Some(value) = self.strings.get(name) {
                        output.push_str(value);
                    }
                }
                _ => return None,
            }
        }
        Some(output)
    }
}

fn consumer_property_value(
    document: &raikiri_dom::Document,
    node_id: usize,
    registration: &ConsumerPropertyRegistration,
    raw: &str,
    text: &mut ConsumerTextContext<'_>,
) -> Option<ConsumerPropertyValue> {
    match registration.grammar() {
        ConsumerPropertyGrammar::Integer => {
            parse_consumer_integer(raw).map(ConsumerPropertyValue::Integer)
        }
        ConsumerPropertyGrammar::IntegerOrNone => parse_consumer_integer_or_none(raw),
        ConsumerPropertyGrammar::Text => {
            let components = raikiri_style::property::parse_consumer_text_value(raw)?;
            text.resolve(document, node_id, &components, false)
                .map(ConsumerPropertyValue::Text)
        }
        ConsumerPropertyGrammar::Keyword(keywords) => parse_consumer_keyword(raw, keywords),
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
    // Elements that generate no box assign no named strings (CSS GCPM 3
    // §1.1.1 assigns them when the element's box is created).
    let mut stack: Vec<(usize, Option<usize>, bool)> = root_children
        .into_iter()
        .rev()
        .map(|child| (child, Some(root), false))
        .collect();
    let mut source_order = 0_u32;
    let mut text = ConsumerTextContext::new(cascade);
    let tracks_strings = registrations
        .iter()
        .any(|registration| registration.grammar() == ConsumerPropertyGrammar::Text);

    while let Some((node_id, parent_id, mut hidden)) = stack.pop() {
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
            hidden |= computed.display == raikiri_style::property::DisplayValue::None;
            // `display: contents` generates no box of its own, but its
            // children still do.
            let generates_box =
                !hidden && computed.display != raikiri_style::property::DisplayValue::Contents;
            if tracks_strings && generates_box && !computed.string_set.is_empty() {
                text.apply_string_set(document, node_id);
            }
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
                let Some(value) =
                    consumer_property_value(document, node_id, registration, &raw, &mut text)
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
            stack.push((child, Some(node_id), hidden));
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

fn page_box_for_page(page: &PageCascadeResult, defaults: &PageDefaults) -> PageBox {
    PageBox::from_page_size_or(page.size(), defaults.page_box)
}

/// Reruns only the `@page` cascade for page queries of one pipeline run.
///
/// The element cascade does not depend on the page query, and the rule tree
/// and media context are fixed for the run, so per-page geometry and styles
/// only need the page-context cascade inheriting from `base`'s root element.
struct PageCascader<'a> {
    tree: &'a RuleTree,
    base: &'a CascadeResult,
    media_context: &'a MediaContext,
}

impl PageCascader<'_> {
    fn page(&self, query: &PageContextQuery) -> PageCascadeResult {
        cascade_page_for_query(self.tree, self.base, self.media_context, query)
    }
}

/// The `@page` cascade for `query`, inheriting from `base`'s root element.
fn cascade_page_for_query(
    tree: &RuleTree,
    base: &CascadeResult,
    media_context: &MediaContext,
    query: &PageContextQuery,
) -> PageCascadeResult {
    cascade_page_with_media_context(
        tree,
        query,
        PageInheritance::FromRoot(base.root_element_computed()),
        media_context,
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ResolvedPageGeometry {
    pub(crate) page_box: PageBox,
    pub(crate) margins: PageMargins,
    pub(crate) content_insets: PageContentInsets,
    pub(crate) content_box: PaintRect,
}

pub(crate) fn resolve_page_geometry(
    page: &PageCascadeResult,
    page_box: PageBox,
) -> ResolvedPageGeometry {
    let margins = page_margins_for_page(page, page_box);
    let content_insets = page_content_insets_for_page(page, page_box);
    let content_box = PaintRect::new(
        margins.left + content_insets.left,
        margins.top + content_insets.top,
        content_insets.page_area_width(margins, page_box),
        content_insets.page_area_height(margins, page_box),
    );
    ResolvedPageGeometry {
        page_box,
        margins,
        content_insets,
        content_box,
    }
}

fn resolve_page_geometries(
    cascader: &PageCascader<'_>,
    defaults: &PageDefaults,
    slices: &[PageSlice],
) -> (Vec<ResolvedPageGeometry>, Vec<PageCascadeResult>) {
    let mut geometries = Vec::with_capacity(slices.len());
    let mut styles = Vec::with_capacity(slices.len());
    for slice in slices {
        let page = cascader.page(&page_query_for_slice(slice));
        let page_box = page_box_for_page(&page, defaults);
        geometries.push(resolve_page_geometry(&page, page_box));
        styles.push(page);
    }
    (geometries, styles)
}

#[allow(clippy::too_many_arguments)]
fn preload_page_background_images(
    cascader: &PageCascader<'_>,
    slices: &[PageSlice],
    resources: &RenderResources<'_>,
    base_url: Option<&url::Url>,
    warnings: &SharedRenderWarnings,
    signal: Option<&raikiri_traits::AbortSignal>,
    seen: &mut HashSet<url::Url>,
    attempts: &mut usize,
) {
    // Element backgrounds are page-independent: scan them once, before the
    // page-context backgrounds of the first page. A per-slice rescan would
    // only revisit URLs already recorded in `seen`, or ones skipped before
    // reaching it, so attempt order, deduplication, and limits are unchanged.
    if slices.is_empty() {
        return; // cov:ignore: a successful document with a body always emits a page slice.
    }
    resources.preload_element_background_images(
        &cascader.base.computed,
        base_url,
        warnings,
        seen,
        attempts,
        signal,
    );
    for slice in slices {
        let page = cascader.page(&page_query_for_slice(slice));
        resources.preload_page_context_background_images(
            &page, base_url, warnings, seen, attempts, signal,
        );
    }
}

/// The size of the page area of a page with the context `page`: the initial
/// containing block when `page` is the first page.
///
/// CSS Values 4 §6.1.2 makes the viewport-percentage lengths "relative to the
/// size of the initial containing block", and CSS Paged Media 3 §3: "The
/// edges of the page area on the first page establish the rectangle that is
/// the initial containing block of the document." The page area is the
/// content area of the page box, inside its border and padding.
fn page_area_size(page: &PageCascadeResult, defaults: &PageDefaults) -> (f32, f32) {
    let content_box = resolve_page_geometry(page, page_box_for_page(page, defaults)).content_box;
    (content_box.width, content_box.height)
}

fn content_width_for_geometry(geometry: ResolvedPageGeometry) -> f32 {
    geometry.content_box.width
}

#[derive(Debug, Clone, PartialEq)]
struct PageGeometrySchedule {
    page_steps: Vec<f32>,
    page_widths: Vec<f32>,
    page_names: Vec<Option<String>>,
}

fn page_geometry_schedule(
    page_geometries: &[ResolvedPageGeometry],
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

fn geometry_differs(left: ResolvedPageGeometry, right: ResolvedPageGeometry) -> bool {
    left.page_box != right.page_box
        || left.margins != right.margins
        || left.content_insets != right.content_insets
        || left.content_box != right.content_box
}

fn map_initial_page_context_error(error: InitialPageContextError) -> RenderError {
    match error {
        InitialPageContextError::Layout(error) => RenderError::from(error),
        InitialPageContextError::PageGeometryDidNotConverge { iterations } => {
            RenderError::PageGeometryDidNotConverge { iterations }
        }
    }
}

/// Give `dom` the fonts of `resources` for the inline engine.
///
/// Faces from `@font-face` go into a document layer over the shared font
/// layer under their authored family names, so computed font families need
/// no rewriting. The faces are fetched through `font_loader`. Returns which
/// faces were registered and which had no usable source.
fn enable_inline_engine(
    dom: &mut raikiri_dom::Document,
    resources: &RenderResources<'_>,
    font_faces: &FontFaceRegistry,
    font_loader: &dyn FontFaceLoader,
) -> raikiri_dom::FontFaceApplyReport {
    let shared = resources.inline_engine_fonts();
    let (fonts, report) = if font_faces.is_empty() {
        (shared, raikiri_dom::FontFaceApplyReport::default())
    } else {
        raikiri_dom::build_inline_document_fonts(&shared, font_faces, font_loader)
    };
    dom.set_font_collection(fonts);
    // The layer of the installed fonts loads a face the first time a lookup
    // selects it, so threads would race to decide which face a fallback lands
    // on: only a font set of bundled fonts builds paragraphs in parallel.
    dom.set_ifc_parallel_build(resources.inline_engine_parallel_build());
    report
}

pub(crate) struct PipelineOutput {
    pub(crate) document: raikiri_dom::Document,
    pub(crate) cascade: raikiri_style::CascadeResult,
    /// Later pages laid out again at their own content width, in page order.
    pub(crate) continuations: Vec<Continuation>,
    pub(crate) slices: Vec<PageSlice>,
    pub(crate) geometries: Vec<ResolvedPageGeometry>,
    pub(crate) page_styles: Vec<raikiri_style::PageCascadeResult>,
    pub(crate) warnings: Vec<RenderWarning>,
    pub(crate) base_url: Option<url::Url>,
}
pub(crate) enum PipelineRun {
    Completed(Box<PipelineOutput>),
    Aborted,
}
pub(crate) struct PipelineInputs<'r, 'a> {
    pub(crate) resources: Option<&'r RenderResources<'a>>,
    pub(crate) consumer_properties: &'r [ConsumerPropertyRegistration],
    pub(crate) property_observer: Option<&'r mut dyn ConsumerPropertyObserver>,
    pub(crate) preload_background_images: bool,
}

pub(crate) fn run_pipeline(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    config: &LayoutConfig,
    inputs: PipelineInputs<'_, '_>,
) -> Result<PipelineRun, RenderError> {
    let consumer_properties = inputs.consumer_properties;
    let property_observer = inputs.property_observer;
    let media_context = &config.media_context;
    let default_resources;
    let resources = match inputs.resources {
        Some(resources) => resources,
        None => {
            default_resources = RenderResources::new();
            &default_resources
        }
    };
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let network = resources.network_adapter();
    let network_ref = network
        .as_ref()
        .map(|provider| provider as &dyn raikiri_traits::NetworkProvider);
    let effective_base_url =
        crate::effective_document_base_url(&doc.uncascaded, resources.fallback_base_url());
    let font_loader = NetworkFontFaceLoader::new(
        network_ref,
        effective_base_url.as_ref(),
        Arc::clone(&warnings),
    );
    let noop_resolver = NoopReplacedResolver;
    let inner_resolver = resources.resolver().unwrap_or(&noop_resolver);
    let resolver = FallbackRecordingResolver {
        inner: inner_resolver,
        policy: resources.policy(),
        image_pixel_source: resources.raw_image_pixel_source(),
        warnings: Arc::clone(&warnings),
        seen: Mutex::new(HashSet::new()),
    };
    let signal = config.signal.clone();
    let is_aborted = || signal.as_ref().is_some_and(|signal| signal.is_aborted());
    let page_control =
        PageLayoutControl::new(config.limits.max_document_pages).with_abort_check(&is_aborted);
    let discovery_control =
        PageLayoutControl::for_geometry_discovery(config.limits.max_document_pages)
            .with_abort_check(&is_aborted);
    if is_aborted() {
        return Ok(PipelineRun::Aborted);
    }
    if config.limits.max_document_pages == Some(0) {
        return Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 0,
            actual: 1,
        });
    }
    validate_layout_depth(&doc.uncascaded.dom)?;

    // Every cascade of this run reads the same parsed document, stylesheets,
    // consumer registrations, and media context, so the rule tree is built once.
    let tree = build_rule_tree_with_consumer_properties(&doc.uncascaded, consumer_properties);

    // The parse-time font registry is cached for the default environment.
    // Other layout environments must select faces using their own dimensions,
    // just like the element and page cascades below.
    let layout_font_faces =
        (*media_context != MediaContext::default()).then(|| tree.font_faces_for(media_context));
    let runtime = RenderExecutionResources {
        font_faces: layout_font_faces.as_ref().unwrap_or(&doc.font_faces),
        font_face_loader: &font_loader,
        effective_base_url: effective_base_url.as_ref(),
        warnings: Arc::clone(&warnings),
    };

    // Resolve the first page context before layout so `:first` and the first
    // resolved `@page size` participate in the initial fragmentainer.
    let mut first_query = PageContextQuery::default();
    first_query.is_first = true;
    first_query.is_right = true;
    let mut cascade_options = CascadeOptions::default();
    cascade_options.limits = config.limits.cascade_limits();
    // The viewport-percentage lengths need the first page's page area before
    // the element cascade, whose root the page context inherits from. Take it
    // from the page context inheriting initial values; the root's styles only
    // matter to font-relative page lengths, which are checked again below.
    let provisional_page = cascade_page_with_media_context(
        &tree,
        &first_query,
        PageInheritance::LegacyInitialValues,
        media_context,
    );
    cascade_options.viewport = Some(page_area_size(&provisional_page, &defaults));
    let mut first_cascade = cascade_with_options(
        &doc.uncascaded.dom,
        &tree,
        media_context,
        &first_query,
        &cascade_options,
    )?;
    // The first class-A box can select a named page. Resolve that name before
    // the initial layout so a named `:first` page is not flattened to the
    // anonymous page geometry. Only the page context depends on the name.
    if let Some(name) = first_page_name(&doc.uncascaded.dom, &first_cascade) {
        first_query.page_name = Some(Atom::from(name.as_str()));
        let page = cascade_page_for_query(&tree, &first_cascade, media_context, &first_query);
        first_cascade.replace_page(page);
    }
    let page_box = page_box_for_page(&first_cascade.page, &defaults);

    // The fonts are given to the document before the first-page probe, which
    // lays out a clone of this document, so the probe and the layout below
    // use the same fonts.
    let mut dom = doc.uncascaded.dom.clone();
    let font_report = enable_inline_engine(
        &mut dom,
        resources,
        runtime.font_faces,
        runtime.font_face_loader,
    );
    let mut marker_image_seen = HashSet::new();
    let mut marker_image_attempts = 0;
    resources.preload_list_marker_images(
        &first_cascade,
        runtime.effective_base_url,
        &runtime.warnings,
        &mut marker_image_seen,
        &mut marker_image_attempts,
        signal.as_ref(),
    );
    if let Some(source) = resources.image_pixel_source_ref() {
        dom.prepare_list_marker_images(&first_cascade, source, runtime.effective_base_url);
    }
    let resolved_initial_context = resolve_initial_page_context(
        &dom,
        first_query.page_name.as_ref().map(ToString::to_string),
        first_cascade,
        page_box,
        InitialPageProbeResources::new(Some(&resolver), runtime.effective_base_url),
        |page_name, cascade| {
            first_query.page_name = page_name.map(Atom::from);
            let page = cascade_page_for_query(&tree, cascade, media_context, &first_query);
            let page_box = page_box_for_page(&page, &defaults);
            (page, page_box)
        },
    )
    .map_err(map_initial_page_context_error)?;
    let mut first_cascade = resolved_initial_context.cascade;
    let page_box = resolved_initial_context.page_box;
    // A named first page, or page lengths relative to the root's font, can
    // give the first page another page area than the provisional one. The
    // element cascade is then run again in that viewport; the page context
    // it already resolved is kept.
    let page_area = page_area_size(&first_cascade.page, &defaults);
    if cascade_options.viewport != Some(page_area) {
        cascade_options.viewport = Some(page_area);
        let page = first_cascade.page.clone();
        first_cascade = cascade_with_options(
            &doc.uncascaded.dom,
            &tree,
            media_context,
            &first_query,
            &cascade_options,
        )?;
        first_cascade.replace_page(page);
        // Marker images without full intrinsic dimensions are sized from
        // the marker's font, which can be viewport-relative.
        resources.preload_list_marker_images(
            &first_cascade,
            runtime.effective_base_url,
            &runtime.warnings,
            &mut marker_image_seen,
            &mut marker_image_attempts,
            signal.as_ref(),
        );
        if let Some(source) = resources.image_pixel_source_ref() {
            dom.prepare_list_marker_images(&first_cascade, source, runtime.effective_base_url);
        }
    }
    let first_cascade = first_cascade;
    // Only `page` in a cascade result depends on the page query, so the
    // first-page cascade serves as the element cascade of every later page.
    let page_cascader = PageCascader {
        tree: &tree,
        base: &first_cascade,
        media_context,
    };

    for family in font_report.skipped {
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
    // Pages whose content width differs from the first page's are laid out
    // again from a copy of the document as it is before any layout.
    let pristine = dom.clone();
    let mut document = dom;
    let mut slices = match layout_pages_with_resolver_and_base_url_and_control(
        &mut document,
        &first_cascade,
        page_box,
        &resolver,
        runtime.effective_base_url,
        &discovery_control,
    ) {
        Ok(slices) => slices,
        Err(LayoutError::Aborted) => return Ok(PipelineRun::Aborted),
        Err(error) => return Err(RenderError::from(error)),
    };
    let mut pagination_truncated = discovery_control.page_limit_reached();
    if is_aborted() {
        return Ok(PipelineRun::Aborted); // cov:ignore: closes the race after the paginator's final cancellation poll.
    }
    const MAX_PAGE_GEOMETRY_PASSES: u32 = 3;
    let mut page_geometries = resolve_page_geometries(&page_cascader, &defaults, &slices).0;
    let mut geometry_converged = true;
    let mut schedule_changed = false;
    for pass in 0..MAX_PAGE_GEOMETRY_PASSES {
        let Some(first_geometry) = page_geometries.first().copied() else {
            break; // cov:ignore: a successful document with a body always emits a page slice.
        };
        let geometry_varies = page_geometries
            .iter()
            .any(|geometry| geometry_differs(*geometry, first_geometry));
        let schedule_must_be_applied = std::mem::take(&mut schedule_changed);
        if !geometry_varies && !pagination_truncated && !schedule_must_be_applied {
            break;
        }

        let schedule = page_geometry_schedule(&page_geometries, &slices);
        // The first pagination pass establishes page count, names, and source
        // coordinates. If page selectors resolve different used geometry, rerun
        // the existing scheduled paginator with producer-owned page steps/widths.
        // Provisional passes stop at the configured page count and return only
        // the prefix needed to discover page geometry. Once the schedule is
        // stable, a strict pass confirms the final count.
        let scheduled_control = if geometry_varies {
            &discovery_control
        } else {
            &page_control
        };
        slices = match layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
            &mut document,
            &first_cascade,
            page_box,
            &schedule.page_steps,
            &schedule.page_widths,
            &resolver,
            runtime.effective_base_url,
            scheduled_control,
        ) {
            Ok(slices) => slices,
            Err(LayoutError::Aborted) => return Ok(PipelineRun::Aborted),
            Err(error) => return Err(RenderError::from(error)),
        };
        if is_aborted() {
            return Ok(PipelineRun::Aborted); // cov:ignore: closes the race after the paginator's final cancellation poll.
        }
        pagination_truncated = geometry_varies && discovery_control.page_limit_reached();
        // A scheduled pass can change both page count and page selectors. Re-
        // resolve before the next iteration so the following schedule is
        // derived from the slices it will actually replace.
        page_geometries = resolve_page_geometries(&page_cascader, &defaults, &slices).0;
        let mut refreshed_schedule = page_geometry_schedule(&page_geometries, &slices);
        if refreshed_schedule == schedule && pagination_truncated {
            // The soft pass confirmed the geometry schedule only for the
            // configured prefix. Re-run it strictly before treating that
            // prefix as a complete document.
            slices = match layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
                &mut document,
                &first_cascade,
                page_box,
                &schedule.page_steps,
                &schedule.page_widths,
                &resolver,
                runtime.effective_base_url,
                &page_control,
            ) {
                Ok(slices) => slices,
                Err(LayoutError::Aborted) => return Ok(PipelineRun::Aborted),
                Err(error) => return Err(RenderError::from(error)),
            };
            // A cancellation arriving after the paginator's final poll is a race-only case.
            // cov:ignore: closes the cancellation race after strict confirmation completes.
            if is_aborted() {
                return Ok(PipelineRun::Aborted);
            }
            pagination_truncated = false;
            page_geometries = resolve_page_geometries(&page_cascader, &defaults, &slices).0;
            refreshed_schedule = page_geometry_schedule(&page_geometries, &slices);
        }
        if refreshed_schedule == schedule && !pagination_truncated {
            break;
        }
        // A refreshed schedule may differ due to page names or page count even
        // when every page currently has the same geometry. Apply it before converging.
        schedule_changed = refreshed_schedule != schedule;
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

    let total_pages = u64::try_from(slices.len()).unwrap_or(u64::MAX);
    if let Some(limit) = config
        .limits
        .max_document_pages
        .filter(|limit| total_pages > u64::from(*limit))
    // cov:ignore: the strict paginator rejects excess pages before returning its slice vector.
    {
        return Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: u64::from(limit),
            actual: u64::from(limit) + 1,
        });
    }

    // Resolve once more after the final bounded schedule pass so metadata and
    // page names always describe the slices that will actually be emitted.
    let (final_geometries, page_styles) =
        resolve_page_geometries(&page_cascader, &defaults, &slices);
    page_geometries = final_geometries;

    let geometries: Vec<_> = page_geometries
        .iter()
        .map(|geometry| (geometry.page_box, geometry.margins, geometry.content_insets))
        .collect();
    match document.project_pages_with_control(
        &first_cascade,
        page_box,
        &slices,
        &geometries,
        &page_control,
    ) {
        Ok(()) => {}
        Err(LayoutError::Aborted) => return Ok(PipelineRun::Aborted),
        Err(error) => return Err(RenderError::from(error)),
    }

    let continuations = match continuation::continue_at_page_widths(
        &continuation::ContinuationInputs {
            pristine: &pristine,
            source: &doc.uncascaded.dom,
            tree: &tree,
            media_context,
            cascade_options: &cascade_options,
            cascader: &page_cascader,
            defaults: &defaults,
            resolver: &resolver,
            base_url: runtime.effective_base_url,
            max_pages: config.limits.max_document_pages,
            abort_check: &is_aborted,
        },
        &document,
        &first_cascade,
        &mut slices,
        &mut page_geometries,
    ) {
        Ok(continuations) => continuations,
        Err(LayoutError::Aborted) => return Ok(PipelineRun::Aborted), // cov:ignore: needs a cancellation landing during a relayout
        Err(error) => return Err(RenderError::from(error)),
    };
    drop(pristine);
    let page_styles = if let Some(first) = continuations.first() {
        // The first layout keeps only the pages before the first continuation.
        let own = first.first_page as usize;
        match document.project_pages_with_control(
            &first_cascade,
            page_box,
            &slices[..own],
            &geometries[..own],
            &page_control,
        ) {
            Ok(()) => {}
            // These pages were projected once already with the same limits.
            Err(LayoutError::Aborted) => return Ok(PipelineRun::Aborted), // cov:ignore: needs a cancellation landing during this projection
            Err(error) => return Err(RenderError::from(error)), // cov:ignore: the same projection succeeded above
        }
        resolve_page_geometries(&page_cascader, &defaults, &slices).1
    } else {
        page_styles
    };

    // Fetch CSS background sources only after the final page schedule is known.
    // Paint remains read-only and consumes the cache through ImagePixelSource.
    if inputs.preload_background_images {
        preload_page_background_images(
            &page_cascader,
            &slices,
            resources,
            runtime.effective_base_url,
            &runtime.warnings,
            signal.as_ref(),
            &mut marker_image_seen,
            &mut marker_image_attempts,
        );
        // A fetch above may have observed cancellation; do not hand out
        // pages whose backgrounds were left unloaded.
        if is_aborted() {
            return Ok(PipelineRun::Aborted);
        }
    }

    if let Some(property_observer) = property_observer {
        if is_aborted() {
            return Ok(PipelineRun::Aborted); // cov:ignore: cancellation can race after layout and before observer delivery
        }
        for event in
            resolved_consumer_property_events(&document, &first_cascade, consumer_properties)
        {
            property_observer
                .observe_event(event)
                .map_err(RenderError::Observer)?;
        }
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
    drop(runtime);
    drop(resolver);
    Ok(PipelineRun::Completed(Box::new(PipelineOutput {
        document,
        cascade: first_cascade,
        continuations,
        slices,
        geometries: page_geometries,
        page_styles,
        warnings,
        base_url: effective_base_url,
    })))
}

impl PipelineOutput {
    /// The laid-out document and cascade that page `page_index` is taken from.
    pub(crate) fn layout_for_page(
        &self,
        page_index: u32,
    ) -> (&raikiri_dom::Document, &raikiri_style::CascadeResult) {
        self.continuations
            .iter()
            .rev()
            .find(|continuation| continuation.first_page <= page_index)
            .map_or((&self.document, &self.cascade), |continuation| {
                (&continuation.document, &continuation.cascade)
            })
    }
}

#[cfg(test)]
mod tests;
