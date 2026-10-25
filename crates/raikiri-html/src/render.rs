//! Resource-aware layout pipeline shared by document layout.

use raikiri_dom::{
    FontFaceLoader, InitialPageContextError, PageLayoutControl, PageSlice, validate_layout_depth,
};
use raikiri_dom::{ResolvedPageGeometry, resolve_page_geometry};
use raikiri_style::FontFaceRegistry;
use raikiri_traits::{
    ConsumerPropertyEvent, ConsumerPropertyObserver, ConsumerPropertyValue, IntrinsicBox,
    LayoutConfig, PageBox, PageDefaults, PolicyViolation, RenderError, RenderWarning,
    ReplacedResolver, ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
    ResourceKind, ResourcePolicy, WarningKind,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use raikiri_style::{
    Atom, CascadeResult, ConsumerPropertyGrammar, ConsumerPropertyRegistration, MediaContext,
    PageCascadeResult, PageContextQuery, RuleTree,
};

use crate::HtmlDocument;
use crate::cascade::build_rule_tree_with_limits;
use crate::fetch::{redacted_url, sanitize_policy_violation, url_policy_violation};
#[cfg(doc)]
use crate::parse_html_with_resources;
use crate::resources::{
    NetworkFontFaceLoader, RenderResources, SharedRenderWarnings, push_resource_warning,
};

pub(crate) mod continuation;
pub(crate) use continuation::Continuation;

mod execute;
mod first_page;
mod page_layout;
mod stop;

/// What every stage of one layout run reads.
struct Run<'r, 'a> {
    defaults: &'r PageDefaults,
    config: &'r LayoutConfig,
    doc: &'r HtmlDocument,
    resources: &'r RenderResources<'a>,
    warnings: &'r SharedRenderWarnings,
    resolver: &'r FallbackRecordingResolver<'r>,
    font_loader: &'r dyn FontFaceLoader,
    /// The `@font-face` rules of the run's environment.
    font_faces: &'r FontFaceRegistry,
    base_url: Option<&'r url::Url>,
    tree: &'r RuleTree,
    signal: Option<&'r raikiri_traits::AbortSignal>,
    is_aborted: &'r dyn Fn() -> bool,
    /// The control of a pagination pass that confirms the final page count.
    page_control: &'r PageLayoutControl<'r>,
    /// The control of a provisional pass that discovers the page geometry.
    discovery_control: &'r PageLayoutControl<'r>,
}

/// The images a run has fetched, which later fetches of the run count with:
/// the URLs tried and how many tries were made.
#[derive(Default)]
struct ImagePreloads {
    seen: HashSet<url::Url>,
    attempts: usize,
}

struct FallbackRecordingResolver<'a> {
    inner: &'a (dyn ReplacedResolver + Send + Sync),
    policy: Option<&'a dyn ResourcePolicy>,
    image_pixel_source: Option<&'a (dyn raikiri_traits::ImagePixelSource + Send + Sync)>,
    warnings: SharedRenderWarnings,
    seen: Mutex<HashSet<(url::Url, String)>>,
    /// Every URL resolved, with the warnings resolving it reported.
    outcomes: Mutex<ReplacedOutcomes>,
}

/// The replaced resources one layout resolved, in the order first resolved,
/// each with the warnings resolving it reported: none when it loaded.
#[derive(Default)]
struct ReplacedOutcomes {
    resources: Vec<(url::Url, Vec<RenderWarning>)>,
    index: HashMap<url::Url, usize>,
}

impl ReplacedOutcomes {
    /// Record that resolving `url` reported `warnings`.
    fn record(&mut self, url: &url::Url, warnings: Vec<RenderWarning>) {
        let index = match self.index.get(url) {
            Some(&index) => index,
            None => {
                self.resources.push((url.clone(), Vec::new()));
                self.index.insert(url.clone(), self.resources.len() - 1);
                self.resources.len() - 1
            }
        };
        let held = &mut self.resources[index].1;
        for warning in warnings {
            let key = format!("{warning:?}");
            if !held.iter().any(|kept| format!("{kept:?}") == key) {
                held.push(warning);
            }
        }
    }
}

impl FallbackRecordingResolver<'_> {
    /// The replaced resources resolved so far, which a later call leaves out.
    fn take_outcomes(&self) -> Vec<(url::Url, Vec<RenderWarning>)> {
        std::mem::take(
            &mut *self
                .outcomes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .resources
    }

    /// Report `warning` and note it among the outcomes of resolving `url`.
    fn report(&self, url: &url::Url, warning: RenderWarning) {
        self.outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(url, vec![warning.clone()]);
        push_resource_warning(&self.warnings, warning);
    }
}

impl ReplacedResolver for FallbackRecordingResolver<'_> {
    fn resolve(&self, request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        let url = request.url().clone();
        self.outcomes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record(&url, Vec::new());
        let mut resolved = if let Some(policy) = self.policy {
            if let Some(violation_type) = url_policy_violation(policy, &url, ResourceKind::Image) {
                let violation = PolicyViolation::new(
                    ResourceKind::Image,
                    url.clone(),
                    violation_type,
                    "replaced resource URL denied by policy".to_owned(),
                );
                self.report(
                    &url,
                    RenderWarning::new(
                        WarningKind::PolicyWarning {
                            violation: sanitize_policy_violation(violation),
                        },
                        "replaced resource URL denied by policy".to_owned(),
                    ),
                );
                Ok(ResolvedIntrinsic::new(
                    IntrinsicBox::default(),
                    ResolveDisposition::Fallback {
                        reason: "resource policy denied the replaced resource".to_owned(),
                    },
                ))
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
            self.report(
                &url,
                RenderWarning::new(
                    WarningKind::ResourceLimitExceeded {
                        kind: ResourceKind::Image,
                        limit,
                        actual,
                    },
                    "decoded image exceeded its configured byte limit".to_owned(),
                ),
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
                self.report(
                    &url,
                    RenderWarning::new(
                        WarningKind::ResourceFallback {
                            kind: ResourceKind::Image,
                            url: Some(redacted_url(&url)),
                        },
                        reason.clone(),
                    ),
                );
            }
        }
        Ok(resolved)
    }
}

struct NoopReplacedResolver;

impl ReplacedResolver for NoopReplacedResolver {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::default(),
            ResolveDisposition::Fallback {
                reason: "no replaced-resource resolver was configured".to_owned(),
            },
        ))
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
    counters: Option<Option<std::sync::Arc<raikiri_dom::CounterSnapshots>>>,
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
            .get_or_insert_with(|| document.shared_counter_snapshots(self.cascade).ok())
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
/// Pages share few distinct queries (a name, whether the page is the first,
/// and its side), so each query's result is computed once and reused. The
/// results are grouped by page name, and a cached result is reused only for
/// an equal query.
struct PageCascader<'a> {
    tree: &'a RuleTree,
    base: &'a CascadeResult,
    media_context: &'a MediaContext,
    cache: std::cell::RefCell<PageCascadeCache>,
}

/// `@page` cascade results by page name, each with the query it answers.
type PageCascadeCache =
    rustc_hash::FxHashMap<Option<Atom>, Vec<(PageContextQuery, PageCascadeResult)>>;

impl<'a> PageCascader<'a> {
    fn new(tree: &'a RuleTree, base: &'a CascadeResult, media_context: &'a MediaContext) -> Self {
        Self {
            tree,
            base,
            media_context,
            cache: std::cell::RefCell::default(),
        }
    }

    fn page(&self, query: &PageContextQuery) -> PageCascadeResult {
        if let Some((_, page)) = self
            .cache
            .borrow()
            .get(&query.page_name)
            .and_then(|cached| cached.iter().find(|(cached, _)| cached == query))
        {
            return page.clone();
        }
        let page = cascade_page_for_query(self.tree, self.base, self.media_context, query);
        self.cache
            .borrow_mut()
            .entry(query.page_name.clone())
            .or_default()
            .push((query.clone(), page.clone()));
        page
    }
}

/// The `@page` cascade for `query`, inheriting from `base`'s root element.
fn cascade_page_for_query(
    tree: &RuleTree,
    base: &CascadeResult,
    media_context: &MediaContext,
    query: &PageContextQuery,
) -> PageCascadeResult {
    base.cascade_page_from_root(tree, query, media_context)
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
            .map(|geometry| geometry.content_box.width)
            .collect(),
        page_names: slices.iter().map(|slice| slice.page_name.clone()).collect(),
    }
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
/// faces were registered and which had no usable source. The cascades of
/// `dom` then measure `line-height: normal` in those fonts.
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
    /// The replaced resources this layout resolved, with the warnings each
    /// reported. They are among `warnings` too.
    pub(crate) replaced_resources: Vec<(url::Url, Vec<RenderWarning>)>,
    pub(crate) base_url: Option<url::Url>,
    /// What the layout was asked for about SVG pictures. A pipeline does not
    /// draw, so it leaves the default for the layout driver to set.
    pub(crate) svg: crate::svg_rendering::SvgSettings,
}
pub(crate) enum PipelineRun {
    Completed(Box<PipelineOutput>),
    Aborted,
    /// The layout could not resume at the [`ResumePoint`] it was given.
    NotResumed,
}
pub(crate) struct PipelineInputs<'r, 'a> {
    pub(crate) resources: Option<&'r RenderResources<'a>>,
    pub(crate) consumer_properties: &'r [ConsumerPropertyRegistration],
    pub(crate) property_observer: Option<&'r mut dyn ConsumerPropertyObserver>,
    pub(crate) preload_background_images: bool,
    pub(crate) resume: Option<&'r ResumePoint>,
}

/// Where a layout begins when an earlier layout of the same document's
/// start already produced the pages before it.
///
/// The layout lays out only the pages from `first_page` on, resuming at
/// `start` as a continuation does. The pages before `first_page` keep their
/// page context (name, side, geometry and margin boxes) but no content.
#[derive(Clone, Debug)]
pub(crate) struct ResumePoint {
    /// Index of the first page laid out.
    pub(crate) first_page: u32,
    /// Where that page begins, from the earlier layout.
    pub(crate) start: raikiri_dom::PageStart,
    /// Names of the pages `0..=first_page`, from the earlier layout.
    pub(crate) page_names: Vec<Option<String>>,
    /// Elements wholly before `start`, laid out as `display: none`.
    pub(crate) skipped: Vec<usize>,
    /// The boxes crossing `start` in the earlier layout.
    pub(crate) earlier_fragments: Vec<raikiri_dom::CrossingBox>,
}

pub(crate) fn run_pipeline(
    doc: &HtmlDocument,
    defaults: PageDefaults,
    config: &LayoutConfig,
    inputs: PipelineInputs<'_, '_>,
) -> Result<PipelineRun, RenderError> {
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
    // A fallback base in the layout resources replaces the parse-time one;
    // without it the base found at parse time resolves the resources.
    let effective_base_url =
        crate::effective_document_base_url(&doc.uncascaded, resources.fallback_base_url())
            .or_else(|| doc.effective_base_url().cloned());
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
        outcomes: Mutex::new(ReplacedOutcomes::default()),
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
    let tree = build_rule_tree_with_limits(
        &doc.uncascaded,
        inputs.consumer_properties,
        &config.limits.rule_tree_limits(),
    )?;

    // The parse-time font registry is cached for the default environment.
    // Other layout environments must select faces using their own dimensions,
    // just like the element and page cascades below.
    let layout_font_faces =
        (*media_context != MediaContext::default()).then(|| tree.font_faces_for(media_context));
    let font_faces = layout_font_faces.as_ref().unwrap_or(&doc.font_faces);
    let base_url = effective_base_url.as_ref();

    let run = Run {
        defaults: &defaults,
        config,
        doc,
        resources,
        warnings: &warnings,
        resolver: &resolver,
        font_loader: &font_loader,
        font_faces,
        base_url,
        tree: &tree,
        signal: signal.as_ref(),
        is_aborted: &is_aborted,
        page_control: &page_control,
        discovery_control: &discovery_control,
    };
    match run.execute(inputs) {
        Ok(output) => Ok(PipelineRun::Completed(Box::new(output))),
        Err(stop) => stop.into_run(),
    }
}

impl PipelineOutput {
    /// The laid-out document and cascade that page `page_index` is taken from.
    pub(crate) fn layout_for_page(
        &self,
        page_index: u32,
    ) -> (&raikiri_dom::Document, &raikiri_style::CascadeResult) {
        match self.layout_slot_for_page(page_index) {
            0 => (&self.document, &self.cascade),
            slot => {
                let continuation = &self.continuations[slot - 1];
                (&continuation.document, &continuation.cascade)
            }
        }
    }

    /// Which laid-out document holds `page_index`: `0` for the original
    /// layout, `k + 1` for continuation `k`. The continuations are in page
    /// order, so the last one starting at or before the page is found by
    /// binary search.
    pub(crate) fn layout_slot_for_page(&self, page_index: u32) -> usize {
        self.continuations
            .partition_point(|continuation| continuation.first_page <= page_index)
    }
}

#[cfg(test)]
mod tests;
