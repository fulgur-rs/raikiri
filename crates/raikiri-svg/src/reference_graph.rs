//! Bounds the references between `mask`, `clipPath`, `pattern` and `marker`
//! resources before `usvg` follows them.
//!
//! `usvg` converts a resource by converting the elements inside it, and
//! converts the resource those elements refer to in the same way. A resource
//! is cached only once it has been converted, and nothing in that recursion
//! counts its depth, so a document of a few hundred bytes overflows the stack
//! of the thread that parses it:
//!
//! - a cycle, such as three masks that use one another through their `mask`
//!   property, a mask and a pattern that use each other, or a pattern whose
//!   content inherits a `fill` that refers to the pattern. `usvg` breaks only
//!   the `mask`, `clip-path`, `fill` and `stroke` that a resource or an element
//!   in it sets when it refers to that resource itself, or when two resources
//!   of the same kind use each other through one such property;
//! - a chain of resources that is not a cycle but is long enough to run out of
//!   stack by its length alone.
//!
//! The XML limits do not stop either, because the references are between
//! sibling elements and add no nesting. A graph of the references is built from
//! the attributes, the `style` attributes and the matching style rules, and
//! the document is refused when it holds a cycle `usvg` does not break, or a
//! chain deeper than [`MAX_REFERENCE_LEVELS`].
//!
//! `usvg` does not draw a marker inside itself, so a cycle that goes through a
//! marker ends by itself and only costs the depth of the laps it makes.
//!
//! The graph over-approximates what `usvg` follows: an ancestor's `fill` that a
//! nearer ancestor overrides still counts as inherited, a shape counts as
//! inheriting the paint that it only sets in a style rule, every ancestor's
//! `mask` counts as the one an element takes with `inherit`, resources that
//! share an `id` all count as the one `usvg` picks, and a `use` is always
//! followed. A document refused for that reason refers to a resource
//! from inside itself in a way no browser draws either.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use roxmltree::{Node, NodeId};

use crate::{
    InitialParseSelectorBudget, MAX_INITIAL_PARSE_EXPANDED_NODES, MAX_INITIAL_PARSE_XML_NODES,
    SVG_NAMESPACE, SvgError, XLINK_NAMESPACE, budgeted_selector_matches_with_budget,
    initial_parse_selector_limit_error, svg_style_has_css_type,
};

/// What `usvg` spends on following one reference, in levels of element nesting:
/// the frames that convert the resource, on top of those of the elements in it.
const REFERENCE_HOP_LEVELS: usize = 4;

/// Deepest chain of references, in levels of element nesting. A level costs
/// about a kilobyte of stack, so this fits in a thread of the smallest stack
/// with room left for the parser around it.
const MAX_REFERENCE_LEVELS: usize = 512;

/// What keeping one edge of the graph costs, in visits. An edge takes memory
/// that a visit does not, and a reference to an `id` that many resources share
/// makes an edge to each of them, so a few thousand of these would otherwise
/// take hundreds of megabytes.
const EDGE_VISITS: usize = 8;

/// Checks the references between the resources of `xml`.
///
/// `usvg` has not read the document when this is called, and the use
/// expansion of the document has already been bounded.
pub(crate) fn check_resource_references(xml: &roxmltree::Document<'_>) -> Result<(), SvgError> {
    check_with_visit_limit(xml, MAX_INITIAL_PARSE_EXPANDED_NODES)
}

/// Checks the references between the resources of `source`, a document that
/// was prepared from one that has been checked.
///
/// Preparing adds the declarations a host has resolved for the elements, and
/// those may refer to resources the document itself does not.
pub(crate) fn check_source(source: &str) -> Result<(), SvgError> {
    let xml = roxmltree::Document::parse_with_options(
        source,
        roxmltree::ParsingOptions {
            nodes_limit: MAX_INITIAL_PARSE_XML_NODES,
            ..Default::default()
        },
    )
    .map_err(|error| SvgError::InvalidDocument(error.to_string()))?;
    check_resource_references(&xml)
}

fn check_with_visit_limit(
    xml: &roxmltree::Document<'_>,
    visit_limit: usize,
) -> Result<(), SvgError> {
    let Some(mut graph) = GraphBuilder::new(xml, visit_limit) else {
        return Ok(());
    };
    let mut edges = Vec::with_capacity(graph.resources.len());
    for index in 0..graph.resources.len() {
        edges.push(graph.own_edges(index)?);
    }
    let edges = graph.with_pattern_content(&edges)?;
    let kinds: Vec<Kind> = graph
        .resources
        .iter()
        .map(|resource| resource.kind)
        .collect();
    check_graph(&kinds, &edges)
}

fn circular_error() -> SvgError {
    SvgError::InvalidDocument(
        "mask, clip-path, pattern and marker references form a cycle".to_owned(),
    )
}

fn too_deep_error() -> SvgError {
    SvgError::InvalidDocument(
        "mask, clip-path, pattern and marker references are chained too deeply".to_owned(),
    )
}

/// The elements that hold other elements for `usvg` to draw when something
/// refers to them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Mask,
    ClipPath,
    Pattern,
    Marker,
}

/// The properties through which an element refers to a resource.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Property {
    Mask,
    ClipPath,
    Fill,
    Stroke,
    Marker,
}

impl Property {
    fn from_name(name: &str) -> Option<Self> {
        [
            ("mask", Self::Mask),
            ("clip-path", Self::ClipPath),
            ("fill", Self::Fill),
            ("stroke", Self::Stroke),
            ("marker", Self::Marker),
            ("marker-start", Self::Marker),
            ("marker-mid", Self::Marker),
            ("marker-end", Self::Marker),
        ]
        .into_iter()
        .find_map(|(known, property)| name.eq_ignore_ascii_case(known).then_some(property))
    }

    /// The kind of resource `usvg` follows this property to. A reference to
    /// any other element is dropped, and so is no edge here.
    fn resource(self) -> Kind {
        match self {
            Self::Mask => Kind::Mask,
            Self::ClipPath => Kind::ClipPath,
            Self::Fill | Self::Stroke => Kind::Pattern,
            Self::Marker => Kind::Marker,
        }
    }
}

/// The references that an element can take from its ancestors.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Inheritance {
    /// `fill` and `stroke`, which every element inherits.
    Paint,
    /// `mask` and `clip-path`, which an element takes from its parent only when
    /// it sets them to `inherit`.
    Cutout,
}

impl Inheritance {
    fn keeps(self, property: Property) -> bool {
        match self {
            Self::Paint => matches!(property, Property::Fill | Property::Stroke),
            Self::Cutout => matches!(property, Property::Mask | Property::ClipPath),
        }
    }
}

struct Resource<'a, 'input> {
    node: Node<'a, 'input>,
    kind: Kind,
}

/// A reference from one resource to another.
#[derive(Clone, Copy)]
struct Edge {
    to: usize,
    property: Property,
    /// Whether the element that refers is one the resource holds, which is
    /// where `usvg` looks for a reference to break.
    direct: bool,
    /// What following the reference adds to the depth of a chain.
    cost: usize,
}

/// An edge by what tells it apart, with its greatest cost.
type EdgeSet = HashMap<(usize, Property, bool), usize>;

/// A style rule that sets a property to a reference, or a `mask` or a
/// `clip-path` to `inherit`.
struct ReferenceRule<'a> {
    selector: simplecss::Selector<'a>,
    references: Vec<(Property, &'a str)>,
    inherits: Vec<Property>,
}

/// The references of the ancestors of an element that it takes from them.
type Inherited<'a> = Rc<Vec<(Property, &'a str)>>;

/// The paint properties that an element or one of its ancestors in a resource
/// gives a value of its own, so that the elements below do not inherit one.
#[derive(Clone, Copy, Default)]
struct Declared {
    fill: bool,
    stroke: bool,
}

impl Declared {
    fn with(self, node: Node<'_, '_>) -> Self {
        Self {
            fill: self.fill || declares(node, Property::Fill),
            stroke: self.stroke || declares(node, Property::Stroke),
        }
    }

    /// Whether `property`, which is `fill` or `stroke`, has such a value.
    fn has(self, property: Property) -> bool {
        if property == Property::Fill {
            self.fill
        } else {
            self.stroke
        }
    }
}

enum Frame<'a, 'input> {
    Visit {
        node: Node<'a, 'input>,
        levels: usize,
        direct: bool,
        declared: Declared,
        cutouts: Inherited<'a>,
    },
    /// The expansion of a `use` of this element is over.
    Leave(NodeId),
}

struct GraphBuilder<'a, 'input: 'a> {
    resources: Vec<Resource<'a, 'input>>,
    resources_by_id: HashMap<&'a str, Vec<usize>>,
    /// The elements a `use` or an `href` may copy, by their `id`.
    copyable_by_id: HashMap<&'a str, Vec<Node<'a, 'input>>>,
    rules: Vec<ReferenceRule<'a>>,
    /// Whether a style rule sets `fill` or `stroke`, to a value that may be
    /// `inherit`. The rules are not matched against the elements that set a
    /// paint of their own, so no element is then taken to set one.
    rules_set_paint: bool,
    /// The references of an element and of its ancestors that an element takes
    /// from them.
    inherited: HashMap<(NodeId, Inheritance), Inherited<'a>>,
    budget: InitialParseSelectorBudget,
    visits_left: usize,
}

impl<'a, 'input: 'a> GraphBuilder<'a, 'input> {
    /// `None` when the document has no resource.
    fn new(xml: &'a roxmltree::Document<'input>, visit_limit: usize) -> Option<Self> {
        let resources: Vec<_> = xml
            .descendants()
            .filter_map(|node| {
                Some(Resource {
                    node,
                    kind: resource_kind(node)?,
                })
            })
            .collect();
        if resources.is_empty() {
            return None;
        }
        let index: HashMap<NodeId, usize> = resources
            .iter()
            .enumerate()
            .map(|(index, resource)| (resource.node.id(), index))
            .collect();
        let copied: HashSet<&str> = xml
            .descendants()
            .filter(|node| is_svg_element(*node))
            .filter(|node| node.tag_name().name() == "use")
            .filter_map(href_id)
            .collect();
        let mut resources_by_id: HashMap<&str, Vec<usize>> = HashMap::new();
        let mut copyable_by_id: HashMap<&str, Vec<Node<'_, '_>>> = HashMap::new();
        for node in xml.descendants().filter(|node| node.is_element()) {
            let Some(id) = node.attribute("id") else {
                continue;
            };
            if let Some(&resource) = index.get(&node.id()) {
                resources_by_id.entry(id).or_default().push(resource);
            }
            if copied.contains(id) {
                copyable_by_id.entry(id).or_default().push(node);
            }
        }
        let mut rules_set_paint = false;
        let rules = xml
            .descendants()
            .filter(|node| node.has_tag_name("style") && svg_style_has_css_type(*node))
            .filter_map(|node| node.text())
            .flat_map(|text| simplecss::StyleSheet::parse(text).rules)
            .filter_map(|rule| {
                rules_set_paint |= rule.declarations.iter().any(|declaration| {
                    matches!(
                        Property::from_name(declaration.name),
                        Some(Property::Fill | Property::Stroke)
                    )
                });
                let declarations = || {
                    rule.declarations
                        .iter()
                        .map(|declaration| (declaration.name, declaration.value))
                };
                let references = declaration_references(declarations());
                let inherits = declaration_inherits(declarations());
                (!references.is_empty() || !inherits.is_empty()).then_some(ReferenceRule {
                    selector: rule.selector,
                    references,
                    inherits,
                })
            })
            .collect();
        Some(Self {
            resources,
            resources_by_id,
            copyable_by_id,
            rules,
            rules_set_paint,
            inherited: HashMap::new(),
            budget: InitialParseSelectorBudget::new(),
            visits_left: visit_limit,
        })
    }

    /// The references an element makes through the properties `only` keeps, or
    /// through every property without one.
    fn references_of(
        &mut self,
        node: Node<'a, 'input>,
        only: Option<Inheritance>,
    ) -> Result<Vec<(Property, &'a str)>, SvgError> {
        let keep = |property: Property| only.is_none_or(|kind| kind.keeps(property));
        self.budget.charge_work(1)?;
        let mut references = declaration_references(
            node.attributes()
                .filter(|attribute| attribute.namespace().is_none())
                .map(|attribute| (attribute.name(), attribute.value())),
        );
        if let Some(style) = node.attribute("style") {
            references.extend(declaration_references(
                simplecss::DeclarationTokenizer::from(style)
                    .map(|declaration| (declaration.name, declaration.value)),
            ));
        }
        for rule in &self.rules {
            if !rule.references.iter().any(|&(property, _)| keep(property)) {
                continue;
            }
            if budgeted_selector_matches_with_budget(
                &rule.selector,
                node,
                &mut self.budget.remaining_work,
                initial_parse_selector_limit_error,
            )? {
                references.extend_from_slice(&rule.references);
            }
        }
        references.retain(|&(property, _)| keep(property));
        Ok(references)
    }

    /// The references of `node` and of its ancestors that `kind` keeps, which
    /// the elements in `node` take from them.
    fn inherited(
        &mut self,
        node: Node<'a, 'input>,
        kind: Inheritance,
    ) -> Result<Inherited<'a>, SvgError> {
        let mut missing = Vec::new();
        let mut inherited = Rc::default();
        let mut current = Some(node);
        while let Some(element) = current {
            if let Some(known) = self.inherited.get(&(element.id(), kind)) {
                inherited = Rc::clone(known);
                break;
            }
            missing.push(element);
            current = element.parent_element();
        }
        for element in missing.into_iter().rev() {
            let own = self.references_of(element, Some(kind))?;
            if !own.is_empty() {
                let mut merged = Vec::clone(&inherited);
                merged.extend(own);
                inherited = Rc::new(merged);
            }
            self.inherited
                .insert((element.id(), kind), Rc::clone(&inherited));
        }
        Ok(inherited)
    }

    /// The `mask` and `clip-path` properties that an element sets to `inherit`
    /// in an attribute, in its `style` attribute or in a style rule.
    fn inherits_of(&mut self, node: Node<'a, 'input>) -> Result<Vec<Property>, SvgError> {
        let mut inherits = declaration_inherits(
            node.attributes()
                .filter(|attribute| attribute.namespace().is_none())
                .map(|attribute| (attribute.name(), attribute.value())),
        );
        if let Some(style) = node.attribute("style") {
            inherits.extend(declaration_inherits(
                simplecss::DeclarationTokenizer::from(style)
                    .map(|declaration| (declaration.name, declaration.value)),
            ));
        }
        for rule in &self.rules {
            if rule.inherits.is_empty() {
                continue;
            }
            if budgeted_selector_matches_with_budget(
                &rule.selector,
                node,
                &mut self.budget.remaining_work,
                initial_parse_selector_limit_error,
            )? {
                inherits.extend_from_slice(&rule.inherits);
            }
        }
        Ok(inherits)
    }

    /// The references of resource `index` to others, made by the resource,
    /// the elements in it and the paint they inherit.
    fn own_edges(&mut self, index: usize) -> Result<Vec<Edge>, SvgError> {
        let root = self.resources[index].node;
        let mut edges = EdgeSet::new();
        // The paint above the resource, which a shape or a text in it inherits
        // unless that or an element between sets its own. What the resource
        // itself sets is reached below, where `usvg` removes it when it refers
        // back.
        let parent = root.parent_element();
        let above = parent
            .map(|parent| self.inherited(parent, Inheritance::Paint))
            .transpose()?
            .unwrap_or_default();
        let cutouts = parent
            .map(|parent| self.inherited(parent, Inheritance::Cutout))
            .transpose()?
            .unwrap_or_default();
        let mut active = HashSet::new();
        // `usvg` looks for a reference to break in the resource and in every
        // element below it.
        let mut pending = vec![Frame::Visit {
            node: root,
            levels: 0,
            direct: true,
            declared: Declared::default(),
            cutouts,
        }];
        while let Some(frame) = pending.pop() {
            let (node, levels, direct, declared, cutouts) = match frame {
                Frame::Leave(copied) => {
                    active.remove(&copied);
                    continue;
                }
                Frame::Visit {
                    node,
                    levels,
                    direct,
                    declared,
                    cutouts,
                } => (node, levels, direct, declared, cutouts),
            };
            self.charge_visits(1)?;
            let references = self.references_of(node, None)?;
            for &(property, id) in &references {
                self.add_edges(&mut edges, property, id, direct, levels)?;
            }
            // An element that sets `mask` or `clip-path` to `inherit` takes the
            // reference of its parent. Every ancestor counts as the parent,
            // for the parent of a copy is the `use` and not where it was found.
            for property in self.inherits_of(node)? {
                for &(from, id) in cutouts.iter().filter(|(from, _)| *from == property) {
                    self.add_edges(&mut edges, from, id, false, levels)?;
                }
            }
            let own_cutouts: Vec<_> = references
                .into_iter()
                .filter(|&(property, _)| Inheritance::Cutout.keeps(property))
                .collect();
            let cutouts = if own_cutouts.is_empty() {
                cutouts
            } else {
                let mut all = Vec::clone(&cutouts);
                all.extend(own_cutouts);
                Rc::new(all)
            };
            let declared = if self.rules_set_paint {
                declared
            } else {
                declared.with(node)
            };
            if paints(node) {
                for &(property, id) in above
                    .iter()
                    .filter(|(property, _)| !declared.has(*property))
                {
                    self.add_edges(&mut edges, property, id, false, levels)?;
                }
            }
            if node.tag_name().name() == "use" {
                for target in href_id(node)
                    .and_then(|id| self.copyable_by_id.get(id))
                    .into_iter()
                    .flatten()
                    .copied()
                {
                    if is_drawn(target) && active.insert(target.id()) {
                        pending.push(Frame::Leave(target.id()));
                        pending.push(Frame::Visit {
                            node: target,
                            levels: levels + 1,
                            direct,
                            declared,
                            cutouts: Rc::clone(&cutouts),
                        });
                    }
                }
                continue;
            }
            pending.extend(
                node.children()
                    .filter(|child| is_drawn(*child))
                    .map(|child| Frame::Visit {
                        node: child,
                        levels: levels + 1,
                        direct,
                        declared,
                        cutouts: Rc::clone(&cutouts),
                    }),
            );
        }
        Ok(edges
            .into_iter()
            .map(|((to, property, direct), cost)| Edge {
                to,
                property,
                direct,
                cost,
            })
            .collect())
    }

    fn add_edges(
        &mut self,
        edges: &mut EdgeSet,
        property: Property,
        id: &str,
        direct: bool,
        levels: usize,
    ) -> Result<(), SvgError> {
        let cost = REFERENCE_HOP_LEVELS + levels;
        let targets: Vec<usize> = self
            .resources_by_id
            .get(id)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&to| self.resources[to].kind == property.resource())
            .collect();
        // Charged before the edges are kept, so resources that share an `id`
        // cannot make more of them than the check may spend.
        self.charge_visits(EDGE_VISITS * targets.len())?;
        for to in targets {
            let known = edges.entry((to, property, direct)).or_default();
            *known = (*known).max(cost);
        }
        Ok(())
    }

    /// Takes `amount` visits from the work this check may do.
    fn charge_visits(&mut self, amount: usize) -> Result<(), SvgError> {
        self.visits_left = self
            .visits_left
            .checked_sub(amount)
            .ok_or_else(initial_parse_selector_limit_error)?;
        Ok(())
    }

    /// `own` with the references of the pattern content that a `pattern`
    /// without content takes from the pattern its `href` names.
    fn with_pattern_content(&mut self, own: &[Vec<Edge>]) -> Result<Vec<Vec<Edge>>, SvgError> {
        let mut all = own.to_vec();
        for (index, edges) in all.iter_mut().enumerate() {
            if self.resources[index].kind != Kind::Pattern {
                continue;
            }
            let mut seen = HashSet::from([index]);
            let mut pending = vec![index];
            while let Some(current) = pending.pop() {
                // `usvg` takes the content of the first pattern in the chain
                // that has some, and does not add the content of the rest.
                if self.resources[current].node.children().any(is_drawn) {
                    continue;
                }
                let targets: Vec<usize> = href_id(self.resources[current].node)
                    .and_then(|id| self.resources_by_id.get(id))
                    .into_iter()
                    .flatten()
                    .copied()
                    .filter(|&target| self.resources[target].kind == Kind::Pattern)
                    .collect();
                for target in targets {
                    if seen.insert(target) {
                        // Every edge copied is work, so a chain of patterns
                        // that each hold many references cannot multiply them.
                        self.charge_visits(1 + own[target].len())?;
                        edges.extend(own[target].iter().map(|edge| Edge {
                            direct: false,
                            ..*edge
                        }));
                        pending.push(target);
                    }
                }
            }
        }
        Ok(all)
    }
}

fn is_svg_element(node: Node<'_, '_>) -> bool {
    node.is_element()
        && node
            .tag_name()
            .namespace()
            .is_none_or(|namespace| namespace == SVG_NAMESPACE)
}

fn resource_kind(node: Node<'_, '_>) -> Option<Kind> {
    if !is_svg_element(node) {
        return None;
    }
    match node.tag_name().name() {
        "mask" => Some(Kind::Mask),
        "clipPath" => Some(Kind::ClipPath),
        "pattern" => Some(Kind::Pattern),
        "marker" => Some(Kind::Marker),
        _ => None,
    }
}

/// Whether `usvg` draws `node` where it is found inside a resource or a
/// group, or when a `use` copies it. Definitions and the other resources are
/// left out, so the references inside them are theirs.
fn is_drawn(node: Node<'_, '_>) -> bool {
    is_svg_element(node)
        && matches!(
            node.tag_name().name(),
            "a" | "circle"
                | "ellipse"
                | "g"
                | "image"
                | "line"
                | "path"
                | "polygon"
                | "polyline"
                | "rect"
                | "svg"
                | "switch"
                | "symbol"
                | "text"
                | "textPath"
                | "tspan"
                | "use"
        )
}

/// Whether `usvg` converts the paint of `node`, which is what follows the
/// `fill` and `stroke` an element inherits to a pattern.
fn paints(node: Node<'_, '_>) -> bool {
    is_svg_element(node)
        && matches!(
            node.tag_name().name(),
            "circle"
                | "ellipse"
                | "line"
                | "path"
                | "polygon"
                | "polyline"
                | "rect"
                | "text"
                | "textPath"
                | "tspan"
        )
}

/// Whether `node` gives `property`, `fill` or `stroke`, a value of its own with
/// an attribute or its `style` attribute, so that the elements below it do not
/// inherit one. `inherit` is not a value of its own.
///
/// A declaration in a style rule is not looked at, so an element that sets its
/// paint that way still counts as inheriting it.
fn declares(node: Node<'_, '_>, property: Property) -> bool {
    let name = if property == Property::Fill {
        "fill"
    } else {
        "stroke"
    };
    let own = |value: &str| {
        let value = value.trim();
        !value.is_empty() && !value.eq_ignore_ascii_case("inherit")
    };
    // The `style` attribute wins over the presentation attribute, and the last
    // declaration over the ones before it unless an earlier one is important.
    let mut from_style: Option<(bool, &str)> = None;
    if let Some(style) = node.attribute("style") {
        for declaration in simplecss::DeclarationTokenizer::from(style) {
            if declaration.name.eq_ignore_ascii_case(name)
                && from_style.is_none_or(|(important, _)| declaration.important || !important)
            {
                from_style = Some((declaration.important, declaration.value));
            }
        }
    }
    match from_style {
        Some((_, value)) => own(value),
        None => node.attribute(name).is_some_and(own),
    }
}

/// The id a `use` or a pattern refers to in the same document.
fn href_id<'a>(node: Node<'a, '_>) -> Option<&'a str> {
    let value = node
        .attributes()
        .find(|attribute| attribute.name() == "href" && attribute.namespace().is_none())
        .or_else(|| {
            node.attributes().find(|attribute| {
                attribute.name() == "href" && attribute.namespace() == Some(XLINK_NAMESPACE)
            })
        })?
        .value();
    svgtypes::IRI::from_str(value).ok().map(|iri| iri.0)
}

/// The references that the declarations `(name, value)` make.
fn declaration_references<'a>(
    declarations: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<(Property, &'a str)> {
    let mut references = Vec::new();
    for (name, value) in declarations {
        if let Some(property) = Property::from_name(name) {
            references.extend(url_fragments(value).map(|id| (property, id)));
        }
    }
    references
}

/// The `mask` and `clip-path` properties that the declarations `(name, value)`
/// set to `inherit`.
fn declaration_inherits<'a>(
    declarations: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<Property> {
    declarations
        .filter(|(_, value)| value.trim().eq_ignore_ascii_case("inherit"))
        .filter_map(|(name, _)| Property::from_name(name))
        .filter(|&property| Inheritance::Cutout.keeps(property))
        .collect()
}

/// The ids that the `url(#id)` functions in `value` name.
fn url_fragments(value: &str) -> impl Iterator<Item = &str> {
    let mut rest = value;
    std::iter::from_fn(move || {
        loop {
            let start = rest
                .as_bytes()
                .windows(4)
                .position(|window| window.eq_ignore_ascii_case(b"url("))?;
            let after = &rest[start + 4..];
            let (inside, next) = after.split_once(')').unwrap_or((after, ""));
            rest = next;
            let inside = inside.trim().trim_matches(['"', '\'']).trim();
            if let Some(id) = inside.strip_prefix('#') {
                return Some(id);
            }
        }
    })
}

/// Refuses a cycle that `usvg` does not break, and a chain that is too deep.
fn check_graph(kinds: &[Kind], edges: &[Vec<Edge>]) -> Result<(), SvgError> {
    // A cycle through a marker ends by itself, so only the cycles among the
    // other resources can recurse without end.
    let unguarded: Vec<Vec<Edge>> = edges
        .iter()
        .enumerate()
        .map(|(from, out)| match kinds[from] {
            Kind::Marker => Vec::new(),
            _ => out
                .iter()
                .filter(|edge| kinds[edge.to] != Kind::Marker)
                .copied()
                .collect(),
        })
        .collect();
    for members in strongly_connected_components(&unguarded) {
        if members.len() > 1
            || unguarded[members[0]]
                .iter()
                .any(|edge| edge.to == members[0])
        {
            check_cycle(&members, &unguarded)?;
        }
    }

    let components = strongly_connected_components(edges);
    let mut component_of = vec![0; kinds.len()];
    for (component, members) in components.iter().enumerate() {
        for &member in members {
            component_of[member] = component;
        }
    }
    // The components come last-reached first, so the depth of every component
    // a reference leads to is known by the time it is needed.
    let mut depth = vec![0; components.len()];
    for (component, members) in components.iter().enumerate() {
        let mut leaving = 0;
        let mut one_lap = 0;
        for &member in members {
            let mut inside = 0;
            for edge in &edges[member] {
                if component_of[edge.to] == component {
                    inside = inside.max(edge.cost);
                } else {
                    leaving = leaving.max(edge.cost + depth[component_of[edge.to]]);
                }
            }
            one_lap += inside;
        }
        // The cycles that are left are gone round twice at most: `usvg` removes
        // the reference of a cycle it breaks, and stops at a marker it is in.
        depth[component] = leaving + 2 * one_lap;
        if depth[component] > MAX_REFERENCE_LEVELS {
            return Err(too_deep_error());
        }
    }
    Ok(())
}

/// Accepts a resource that refers to itself, and two resources that refer to
/// each other, through elements they hold and through one property, which is
/// the one `usvg` looks at. A reference is only an edge to a resource of the
/// kind its property leads to, so the resources of such a cycle are of one kind.
/// `usvg` removes the reference.
fn check_cycle(members: &[usize], edges: &[Vec<Edge>]) -> Result<(), SvgError> {
    if members.len() > 2 {
        return Err(circular_error());
    }
    let mut through = None;
    for &member in members {
        for edge in edges[member]
            .iter()
            .filter(|edge| members.contains(&edge.to))
        {
            if !edge.direct || through.is_some_and(|property| property != edge.property) {
                return Err(circular_error());
            }
            through = Some(edge.property);
        }
    }
    Ok(())
}

/// The strongly connected components of the graph, each after every component
/// it has a reference into.
fn strongly_connected_components(edges: &[Vec<Edge>]) -> Vec<Vec<usize>> {
    const UNVISITED: usize = usize::MAX;
    let count = edges.len();
    let mut order = vec![UNVISITED; count];
    let mut lowest = vec![0; count];
    let mut on_stack = vec![false; count];
    let mut stack = Vec::new();
    let mut components = Vec::new();
    let mut next_order = 0;
    for start in 0..count {
        if order[start] != UNVISITED {
            continue;
        }
        let mut calls = vec![(start, 0)];
        order[start] = next_order;
        lowest[start] = next_order;
        next_order += 1;
        stack.push(start);
        on_stack[start] = true;
        while let Some(&(node, position)) = calls.last() {
            if let Some(edge) = edges[node].get(position) {
                calls.last_mut().expect("a call is pending").1 += 1;
                let to = edge.to;
                if order[to] == UNVISITED {
                    order[to] = next_order;
                    lowest[to] = next_order;
                    next_order += 1;
                    stack.push(to);
                    on_stack[to] = true;
                    calls.push((to, 0));
                } else if on_stack[to] {
                    lowest[node] = lowest[node].min(order[to]);
                }
                continue;
            }
            calls.pop();
            if let Some(&(parent, _)) = calls.last() {
                lowest[parent] = lowest[parent].min(lowest[node]);
            }
            if lowest[node] == order[node] {
                let mut members = Vec::new();
                while let Some(member) = stack.pop() {
                    on_stack[member] = false;
                    members.push(member);
                    if member == node {
                        break;
                    }
                }
                components.push(members);
            }
        }
    }
    components
}

#[cfg(test)]
mod tests;
