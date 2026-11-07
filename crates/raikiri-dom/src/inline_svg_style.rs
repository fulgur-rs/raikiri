//! Host-cascaded styles of an inline SVG, shared by the page's vector export
//! and the built-in painter so that both draw the same document.
//!
//! The SVG source only holds authored attributes and `style` attributes. The
//! declarations a page stylesheet gives to its descendants, and the host's
//! root font, reach the SVG renderer through these helpers.

use crate::Document;
use raikiri_style::cascade::SvgStyleProperty;
use raikiri_style::property::{FontFamilyKind, PropertyKey};
use raikiri_style::{CascadeResult, ComputedValues, StyleNodeId};
use raikiri_traits::NodeKind;
use rustc_hash::FxHashMap;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fmt::Write as _;

const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
const XLINK_NAMESPACE: &str = "http://www.w3.org/1999/xlink";

/// Upper bound on the declaration text the descendants of one SVG may add.
const DECLARATION_BYTE_LIMIT: usize = 32 * 1024 * 1024;

/// Resolved declarations for one element of an inline SVG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineSvgElementDeclarations {
    /// Zero-based element preorder index within the SVG subtree, counting the
    /// root as index zero. The root is styled separately, so it is listed only
    /// for the painting properties (`fill`, `stroke`, the stroke shape and dash
    /// properties, their opacities and the fill and clip rules) that its root
    /// style does not carry.
    pub element_index: usize,
    /// CSS declarations, each marked `!important`.
    pub declarations: String,
}

/// The declarations the cascade resolves for the elements of the inline SVG
/// rooted at `root`, in document order.
///
/// Only elements whose cascade holds an explicit SVG property are listed, and
/// the root only by the painting properties among them.
/// Fails when the declarations would exceed the size bound, so that a few
/// rules matching a very large SVG cannot allocate without limit.
pub fn inline_svg_element_declarations(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
) -> Result<Vec<InlineSvgElementDeclarations>, String> {
    let mut elements = Vec::new();
    walk_styled_elements(document, cascade, root, |element, work| {
        let mut declarations = String::new();
        element.write_to(&mut declarations, work)?;
        elements.push(InlineSvgElementDeclarations {
            element_index: element.element_index,
            declarations,
        });
        Ok(true)
    })?;
    Ok(elements)
}

/// Whether [`inline_svg_element_declarations`] would return `expected` now,
/// found without keeping what it spells out.
///
/// Every declaration is written as the collection writes it and checked
/// against `expected` as it goes, so a match costs no allocation, and a
/// mismatch stops at its first difference. A collection that fails, which
/// reads as no declarations to a caller that draws the SVG unstyled, matches
/// an empty `expected`.
pub fn inline_svg_element_declarations_match(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    expected: &[InlineSvgElementDeclarations],
) -> bool {
    let mut next = 0;
    let walked = walk_styled_elements(document, cascade, root, |element, work| {
        let Some(entry) = expected
            .get(next)
            .filter(|entry| entry.element_index == element.element_index)
        else {
            return Ok(false);
        };
        next += 1;
        let mut declarations = Expected(&entry.declarations);
        Ok(element.write_to(&mut declarations, work)? && declarations.0.is_empty())
    });
    if matches!(walked, Ok(true)) {
        return next == expected.len();
    }
    // Not shown to be the same. A collection that fails reads as no
    // declarations, so only an empty `expected` can still match one: the walk
    // may have ended at its first difference before the failure further on.
    expected.is_empty() && inline_svg_element_declarations(document, cascade, root).is_err()
}

/// An element of an inline SVG that holds declarations.
struct StyledElement<'a> {
    /// Zero-based element preorder index, counting the root as zero.
    element_index: usize,
    style: &'a ComputedValues,
    properties: &'a [SvgStyleProperty],
    relative_attributes: &'a [SvgStyleProperty],
    placement: Placement,
}

/// Where an element sits among the subtrees that `use` elements instantiate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Placement {
    /// Not in such a subtree.
    Tree,
    /// In such a subtree, which each instance reads with its own font size.
    Instance,
    /// In such a subtree, and nothing in the SVG source sets a stroke length
    /// for the element, so that a stroke length it inherits can be left to
    /// the renderer.
    InstanceInheritingByItself,
}

impl StyledElement<'_> {
    /// Writes the declarations of the element to `out`. Says `false` when
    /// `out` stopped taking them.
    fn write_to(&self, out: &mut impl Declarations, work: &mut Work) -> Result<bool, String> {
        write_node_declarations(
            self.style,
            self.properties,
            self.relative_attributes,
            self.placement,
            &mut work.remaining_bytes,
            &mut work.value,
            out,
        )
    }
}

/// What the elements of one walk share while their declarations are written.
struct Work {
    /// The bytes left of the size bound.
    remaining_bytes: usize,
    /// Room for the text of a value that no declaration holds, kept from one
    /// element to the next so that each does not allocate its own.
    value: String,
}

/// Calls `visit` for each element of the inline SVG rooted at `root` that
/// holds declarations, in document order. The walk ends when `visit` says
/// `false` or fails, and says which.
fn walk_styled_elements(
    document: &Document,
    cascade: &CascadeResult,
    root: usize,
    mut visit: impl FnMut(StyledElement<'_>, &mut Work) -> Result<bool, String>,
) -> Result<bool, String> {
    let Some(root_node) = document.get_node(root) else {
        return Ok(true);
    };
    let mut work = Work {
        remaining_bytes: DECLARATION_BYTE_LIMIT,
        value: String::new(),
    };
    let SourceScan {
        use_targets,
        stroke_style,
    } = scan_source(document, root);
    // Whether an inherited value comes from inside an instance is asked per
    // property of each element that an instance holds, so the elements above
    // the one at hand are kept to answer that without climbing to the
    // instance. Nothing asks without a `use` element.
    let mut ancestors = if use_targets.is_empty() {
        Ancestors::default()
    } else {
        Ancestors::above(document, root)
    };
    // Each entry carries the nearest element above it, or itself, that a `use`
    // element instantiates, which stands for the subtree a renderer clones
    // under the `use` element. One list per element has to serve every
    // instance, so an element inside two nested targets is judged by the inner
    // one: what lies between the targets is the ancestry of a copy of the outer
    // target only, and a copy of the inner one inherits from its own `use`
    // element there.
    let mut stack = vec![(root, root_node, None, 0)];
    let mut element_index = 0;
    while let Some((node_id, node, mut instance_root, depth)) = stack.pop() {
        if node.kind() == NodeKind::Element {
            if !use_targets.is_empty() {
                ancestors.enter(node_id, depth);
            }
            if !use_targets.is_empty()
                && node
                    .attribute("id")
                    .is_some_and(|id| use_targets.contains(id))
            {
                instance_root = Some(node_id);
            }
            let style_node = StyleNodeId(node_id as u64);
            let properties = cascade.svg_style_properties(style_node);
            let relative_attributes = cascade.svg_relative_presentation_attributes(style_node);
            // The root's color, font, visibility and opacity reach the renderer
            // through the root style, so only its painting properties are
            // listed. Descendants inherit them from the root there.
            let root_painting: Vec<SvgStyleProperty>;
            let properties = if element_index == 0 {
                root_painting = properties
                    .iter()
                    .filter(|property| property.property.is_svg_presentation())
                    .cloned()
                    .collect();
                &root_painting[..]
            } else {
                properties
            };
            // An HTML element inside the SVG (under `foreignObject`) lists the
            // painting properties its SVG descendants inherit, which the
            // renderer has no use for.
            if node.namespace_uri() == Some(SVG_NAMESPACE)
                && (!properties.is_empty() || !relative_attributes.is_empty())
                && let Some(style) = cascade.computed.get(node_id)
            {
                let properties = resolve_geometry_inheritance(
                    document,
                    cascade,
                    node_id,
                    instance_root,
                    properties,
                );
                let properties = match instance_root {
                    Some(instance_root) => inherit_from_instantiating_use(
                        document,
                        &ancestors,
                        &properties,
                        instance_root,
                    ),
                    None => Cow::Borrowed(&properties[..]),
                };
                let element = StyledElement {
                    element_index,
                    style,
                    properties: &properties,
                    relative_attributes,
                    placement: match instance_root {
                        None => Placement::Tree,
                        // A stylesheet of the SVG itself, or the source of the
                        // element, may set a stroke length that the cascade
                        // gave another winner.
                        Some(_) if stroke_style || sets_stroke_length(node) => Placement::Instance,
                        Some(_) => Placement::InstanceInheritingByItself,
                    },
                };
                if !visit(element, &mut work)? {
                    return Ok(false);
                }
            }
            element_index += 1;
        }
        // Only elements make a level of the path.
        let child_depth = depth + usize::from(node.kind() == NodeKind::Element);
        stack.extend(node.children.iter().rev().filter_map(|&child| {
            document
                .get_node(child)
                .map(|node| (child, node, instance_root, child_depth))
        }));
    }
    Ok(true)
}

/// What the source of an inline SVG tells before its elements are walked.
struct SourceScan<'a> {
    /// The ids that the `use` elements reference.
    use_targets: HashSet<&'a str>,
    /// Whether a `style` element of the SVG may set a stroke property.
    stroke_style: bool,
}

/// Reads the source of the SVG rooted at `root` once.
fn scan_source(document: &Document, root: usize) -> SourceScan<'_> {
    let mut scan = SourceScan {
        use_targets: HashSet::new(),
        stroke_style: false,
    };
    let mut stack: Vec<_> = document.get_node(root).into_iter().collect();
    while let Some(node) = stack.pop() {
        if node.kind() == NodeKind::Element
            && node.namespace_uri() == Some(SVG_NAMESPACE)
            && node.tag_name() == Some("use")
            && let Some(id) = node
                .attribute("href")
                .or_else(|| node.attribute_ns(XLINK_NAMESPACE, "href"))
                .and_then(iri_fragment)
        {
            scan.use_targets.insert(id);
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("style") {
            scan.stroke_style |= node.children.iter().any(|&child| {
                document
                    .get_node(child)
                    .and_then(|child| child.text_content())
                    .is_none_or(names_a_stroke_property)
            });
        }
        stack.extend(
            node.children
                .iter()
                .filter_map(|&child| document.get_node(child)),
        );
    }
    scan
}

/// Whether the source of `node` sets a stroke length by itself: by one of the
/// presentation attributes, or by a `style` attribute that may name a stroke
/// property.
fn sets_stroke_length(node: &crate::Node) -> bool {
    ["stroke-width", "stroke-dasharray", "stroke-dashoffset"]
        .into_iter()
        .any(|name| node.attribute(name).is_some())
        || node.attribute("style").is_some_and(names_a_stroke_property)
}

/// Whether the declarations or rules in `css` may name a stroke property. A
/// name can be spelled with an escape or split by a comment, so any text with
/// a backslash or a slash is taken to.
fn names_a_stroke_property(css: &str) -> bool {
    css.contains(['\\', '/'])
        || css
            .as_bytes()
            .windows("stroke".len())
            .any(|window| window.eq_ignore_ascii_case(b"stroke"))
}

/// The id of a local reference such as `#shape`, read as the renderer reads
/// it: leading spaces are skipped, the id ends at the next space, and
/// nothing but spaces may follow.
fn iri_fragment(href: &str) -> Option<&str> {
    const SPACES: [char; 4] = [' ', '\t', '\n', '\r'];
    let rest = href.trim_start_matches(SPACES).strip_prefix('#')?;
    let (id, tail) = rest.split_once(' ').unwrap_or((rest, ""));
    (!id.is_empty() && tail.trim_matches(SPACES).is_empty()).then_some(id)
}

/// `properties` with every geometry `inherit` replaced by the value that the
/// parent has for it. The cascade keeps no computed value for those
/// properties, so the parent is asked here, where its attributes are known.
/// An `inherit` that cannot be answered is left as it is, and nothing is
/// written for it.
fn resolve_geometry_inheritance<'a>(
    document: &Document,
    cascade: &CascadeResult,
    node: usize,
    instance_root: Option<usize>,
    properties: &'a [SvgStyleProperty],
) -> Cow<'a, [SvgStyleProperty]> {
    let inherits =
        |property: &SvgStyleProperty| property.inherited && property.property.is_svg_geometry();
    if !properties.iter().any(inherits) {
        return Cow::Borrowed(properties);
    }
    Cow::Owned(
        properties
            .iter()
            .map(|property| {
                let mut property = property.clone();
                if inherits(&property)
                    && let Some(value) = inherited_geometry(
                        document,
                        cascade,
                        node,
                        instance_root,
                        property.property,
                    )
                {
                    property.inherited = false;
                    property.expression = Some(value);
                }
                property
            })
            .collect(),
    )
}

/// The value that the geometry property `key` takes for `node` from its
/// parent, which is what the parent declares, or the attribute of a nested
/// `svg` for its position, or else the initial value. A parent that inherits
/// the property asks its own parent. `None` when the answer differs between
/// the instances of a `use` element: a target of one has another parent in
/// each.
fn inherited_geometry(
    document: &Document,
    cascade: &CascadeResult,
    mut node: usize,
    instance_root: Option<usize>,
    key: PropertyKey,
) -> Option<String> {
    loop {
        if instance_root == Some(node) {
            return None;
        }
        let parent = document.parent_of(node)?;
        let parent_style = cascade.svg_style_properties(StyleNodeId(parent as u64));
        match parent_style
            .iter()
            .find(|declared| declared.property == key)
        {
            Some(declared) if declared.inherited => node = parent,
            Some(declared) => {
                return at_font_size_of(cascade, parent, key, declared.expression.clone()?);
            }
            None => {
                let name = key.svg_geometry_name()?;
                let element = document.get_node(parent)?;
                let attribute = (element.namespace_uri() == Some(SVG_NAMESPACE)
                    && element.tag_name() == Some("svg")
                    && matches!(name, "x" | "y"))
                .then(|| element.attribute(name))
                .flatten();
                let text = match attribute {
                    // An attribute takes a number where CSS takes a length.
                    Some(text) if text.trim().parse::<f32>().is_ok() => {
                        format!("{}px", text.trim())
                    }
                    Some(text) => text.trim().to_owned(),
                    None => key.svg_geometry_initial()?.to_owned(),
                };
                return at_font_size_of(cascade, parent, key, text);
            }
        }
    }
}

/// `text`, which the element `declarer` gave to the geometry property `key`,
/// with a length in `em` or `ex` taken at the font size of `declarer`. What is
/// inherited is the computed length, which is not the one that the font size of
/// the inheriting shape would make. `None` when the length cannot be read.
fn at_font_size_of(
    cascade: &CascadeResult,
    declarer: usize,
    key: PropertyKey,
    text: String,
) -> Option<String> {
    if key == PropertyKey::SvgD || !is_font_relative(&text) {
        return Some(text);
    }
    let font_size = cascade.computed.get(declarer)?.font_size.0;
    resolve_relative_svg_length(&text, font_size).map(|(length, _)| length)
}

/// `properties` of an element in the subtree that a `use` element
/// instantiates at `instance_root`, with every `inherit` that the cascade
/// resolved against an ancestor outside that subtree left as `inherit`.
///
/// The cascade resolves `inherit` against the source tree, but the instance
/// inherits from the `use` element and its ancestors, which the renderer
/// reaches by itself.
fn inherit_from_instantiating_use<'a>(
    document: &Document,
    ancestors: &Ancestors,
    properties: &'a [SvgStyleProperty],
    instance_root: usize,
) -> Cow<'a, [SvgStyleProperty]> {
    let outside = |property: &SvgStyleProperty| {
        property
            .inherited_from
            .is_some_and(|source| !ancestors.is_within(document, source.0 as usize, instance_root))
    };
    let Some(first) = properties.iter().position(outside) else {
        return Cow::Borrowed(properties);
    };
    Cow::Owned(
        properties
            .iter()
            .enumerate()
            .map(|(at, property)| {
                let mut property = property.clone();
                // None of the properties before `first` comes from outside.
                if at >= first && outside(&property) {
                    property.inherited = true;
                    property.inherited_from = None;
                }
                property
            })
            .collect(),
    )
}

/// The elements above the one a walk is at, from the top of the document down
/// to that element, which tell whether another element is within a subtree
/// that one of them roots without climbing to its root.
#[derive(Default)]
struct Ancestors {
    /// The elements above the root of the walk come first, then the elements
    /// from the root of the walk to the current one.
    path: Vec<usize>,
    /// How many of `path` lie above the root of the walk.
    above: usize,
    /// Where each element of `path` sits in it.
    position: FxHashMap<usize, usize>,
}

impl Ancestors {
    /// The ancestors of `root`, where a walk of its subtree starts.
    fn above(document: &Document, root: usize) -> Self {
        let mut path: Vec<usize> = document.ancestors(root).collect();
        path.reverse();
        Self {
            above: path.len(),
            position: path.iter().enumerate().map(|(at, &id)| (id, at)).collect(),
            path,
        }
    }

    /// Make `node`, `depth` levels below the root of the walk, the current
    /// element.
    fn enter(&mut self, node: usize, depth: usize) {
        while self.path.len() > self.above + depth {
            if let Some(left) = self.path.pop() {
                self.position.remove(&left);
            }
        }
        self.position.insert(node, self.path.len());
        self.path.push(node);
    }

    /// Whether `node` is `ancestor` or one of its descendants, `ancestor` being
    /// the current element or one above it. When `node` is on the path too, it
    /// is one exactly when it comes at or after `ancestor` there; any other
    /// element is searched for in the tree.
    fn is_within(&self, document: &Document, node: usize, ancestor: usize) -> bool {
        match (self.position.get(&node), self.position.get(&ancestor)) {
            (Some(node), Some(ancestor)) => node >= ancestor,
            _ => is_inclusive_descendant(document, node, ancestor),
        }
    }
}

/// Whether `node` is `ancestor` or one of its descendants.
fn is_inclusive_descendant(document: &Document, node: usize, ancestor: usize) -> bool {
    let steps = document.inclusive_ancestors(node);
    #[cfg(test)]
    let steps = steps.inspect(|_| {
        crate::layout::test_support::INLINE_SVG_PARENT_STEPS
            .with(|count| count.set(count.get() + 1))
    });
    steps.into_iter().any(|id| id == ancestor)
}

/// The CSS `font-family` list of `style`, with every non-generic name quoted.
pub fn svg_font_family(style: &ComputedValues) -> String {
    style
        .font_family
        .iter()
        .map(|family| {
            if family.1 == FontFamilyKind::Generic {
                family.as_str().to_owned()
            } else {
                // Writing to a `String` cannot fail.
                let mut quoted = String::new();
                let _ = cssparser::serialize_string(family.as_str(), &mut quoted);
                quoted
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The CSS name of an SVG painting property.
fn svg_presentation_name(key: PropertyKey) -> &'static str {
    key.svg_presentation_name().unwrap_or_default()
}

/// Where the declarations of an element are written.
trait Declarations {
    /// Takes `text` as the next part of the declarations. Says `false` when
    /// the declarations cannot go on.
    fn push(&mut self, text: &str) -> bool;
}

impl Declarations for String {
    fn push(&mut self, text: &str) -> bool {
        self.push_str(text);
        true
    }
}

/// Declarations that are checked against the text they must spell out
/// instead of being kept. What is left of that text is the field.
struct Expected<'a>(&'a str);

impl Declarations for Expected<'_> {
    fn push(&mut self, text: &str) -> bool {
        match self.0.strip_prefix(text) {
            Some(rest) => {
                self.0 = rest;
                true
            }
            None => false,
        }
    }
}

/// Writes the declaration `name: value !important`.
fn push_declaration(out: &mut impl Declarations, name: &str, value: &str) -> bool {
    out.push(name) && out.push(":") && out.push(value) && out.push("!important;")
}

/// The name and value that `property` declares for an element with `style`,
/// or `None` when it declares nothing the renderer needs. A value that is
/// computed is written to `scratch`.
fn declared<'a>(
    style: &'a ComputedValues,
    property: &'a SvgStyleProperty,
    scratch: &'a mut String,
) -> Option<(&'static str, &'a str)> {
    let name = match property.property {
        PropertyKey::Color => "color",
        PropertyKey::Display => "display",
        PropertyKey::Opacity => "opacity",
        PropertyKey::Visibility => "visibility",
        PropertyKey::FontSize => "font-size",
        PropertyKey::FontFamily => "font-family",
        PropertyKey::FontStyle => "font-style",
        PropertyKey::FontWeight => "font-weight",
        key if key.is_svg_presentation() => svg_presentation_name(key),
        // The geometry of a shape is written only when a declaration gives it
        // a value: it is not inherited, and nothing computed stands for it.
        PropertyKey::Width => "width",
        PropertyKey::Height => "height",
        key if key.is_svg_geometry() => key.svg_geometry_name().unwrap_or_default(),
        _ => return None,
    };
    let geometry = property.property.is_svg_geometry() || property.property.is_svg_shape_size();
    if geometry && property.inherited {
        return None;
    }
    if property.inherited {
        // SVG renderers such as usvg copy an ancestor's raw `font-size`
        // for `inherit` and apply it again, compounding a relative size.
        // `100%` has the same computed value without that copy.
        let value = if property.property == PropertyKey::FontSize {
            "100%"
        } else {
            "inherit"
        };
        return Some((name, value));
    }
    // The declared text wins over the computed value.
    if let Some(expression) = &property.expression {
        return Some((name, expression));
    }
    scratch.clear();
    // Writing to a `String` cannot fail.
    let _ = match property.property {
        PropertyKey::Color => {
            let color = style.color;
            write!(
                scratch,
                "rgba({},{},{},{:.6})",
                color.r,
                color.g,
                color.b,
                f32::from(color.a) / 255.0
            )
        }
        PropertyKey::Display => return Some((name, style.display.as_css_str())),
        PropertyKey::Opacity => write!(scratch, "{}", style.opacity),
        PropertyKey::Visibility => return Some((name, style.visibility.as_css_str())),
        PropertyKey::FontSize => write!(scratch, "{}px", style.font_size.0),
        PropertyKey::FontFamily => scratch.write_str(&svg_font_family(style)),
        PropertyKey::FontStyle => return Some((name, style.font_style.as_css_str())),
        PropertyKey::FontWeight => write!(scratch, "{}", style.font_weight),
        // Held by the declaration alone: no computed field carries them.
        _ => return None,
    };
    Some((name, scratch))
}

/// The declarations of one element, kept.
#[cfg(test)]
fn node_declarations(
    style: &ComputedValues,
    properties: &[SvgStyleProperty],
    relative_attributes: &[SvgStyleProperty],
    in_instance: bool,
    remaining_bytes: &mut usize,
) -> Result<String, String> {
    let placement = if in_instance {
        Placement::Instance
    } else {
        Placement::Tree
    };
    node_declarations_at(
        style,
        properties,
        relative_attributes,
        placement,
        remaining_bytes,
    )
}

/// [`node_declarations`] for an element at `placement`.
#[cfg(test)]
fn node_declarations_at(
    style: &ComputedValues,
    properties: &[SvgStyleProperty],
    relative_attributes: &[SvgStyleProperty],
    placement: Placement,
    remaining_bytes: &mut usize,
) -> Result<String, String> {
    let mut css = String::new();
    write_node_declarations(
        style,
        properties,
        relative_attributes,
        placement,
        remaining_bytes,
        &mut String::new(),
        &mut css,
    )?;
    Ok(css)
}

/// Writes the declarations of one element to `out`, and says `false` when
/// `out` stopped taking them. `placement` tells whether the element is part of
/// a subtree a `use` element instantiates, and `value` is room for computed
/// values.
///
/// A font-relative stroke length that an element outside such a subtree
/// declares is written in px at that element's font size. A renderer copies
/// the text of an `inherit` and resolves it at the inheriting element, which
/// has another font size when it is instantiated below the `use` element.
/// Inside an instantiated subtree the length is left as written, so that each
/// instance resolves it against its own font size, and so is a presentation
/// attribute, which is written as the cascade normalized it.
///
/// What the element inherits there cannot be written in px either, for the
/// font size of the declaring ancestor is another one in each instance. When
/// the placement says that no other text of the source sets the length, the
/// declaration is left out: a renderer that finds no value resolves the one an
/// ancestor declares at that ancestor, as inheritance does, where it copies
/// the text of an `inherit` to the element.
fn write_node_declarations(
    style: &ComputedValues,
    properties: &[SvgStyleProperty],
    relative_attributes: &[SvgStyleProperty],
    placement: Placement,
    remaining_bytes: &mut usize,
    value: &mut String,
    out: &mut impl Declarations,
) -> Result<bool, String> {
    let in_instance = placement != Placement::Tree;
    // Bound metadata before allocating repeated declarations from the host CSS.
    let mut consume = |bytes: usize| {
        *remaining_bytes = remaining_bytes
            .checked_sub(bytes)
            .ok_or_else(|| "SVG descendant style source limit exceeded".to_owned())?;
        Ok::<(), String>(())
    };
    consume(128)?;
    for property in properties {
        let size = if property.property == PropertyKey::FontFamily && !property.inherited {
            style.font_family.iter().fold(128usize, |size, name| {
                size.saturating_add(name.as_str().len().saturating_mul(8))
            })
        } else if (property.property.is_svg_presentation()
            || property.property.is_svg_geometry()
            || property.property.is_svg_shape_size())
            && !property.inherited
        {
            // The declared text is cloned and then copied into the
            // declaration, so a long `url(#...)` paint reaching many elements
            // counts twice per element.
            property.expression.as_ref().map_or(128, |text| {
                128usize.saturating_add(text.len().saturating_mul(2))
            })
        } else {
            128
        };
        consume(size)?;
        if placement == Placement::InstanceInheritingByItself && inherits_stroke_length(property) {
            continue;
        }
        let Some((name, text)) = declared(style, property, value) else {
            continue;
        };
        let in_px = if in_instance || property.inherited {
            None
        } else {
            resolve_relative_svg_presentation_value(property.property, text, style.font_size.0)
        };
        if let Some(in_px) = &in_px {
            // The declaration and the text it was converted from are counted
            // above; the new text is built once and copied once.
            consume(in_px.len().saturating_mul(2))?;
        }
        let text = in_px.as_deref().unwrap_or(text);
        if !push_declaration(out, name, text) {
            return Ok(false);
        }
    }
    for attribute in relative_attributes {
        let Some(expression) = &attribute.expression else {
            continue;
        };
        // An instantiated element keeps the length as written, normalized, so
        // that each instance resolves it at its own font size. The source
        // text is not left to the renderer, which reads fewer spellings.
        let in_px;
        let text = if in_instance {
            expression.as_str()
        } else {
            let Some(value) = resolve_relative_svg_presentation_value(
                attribute.property,
                expression,
                style.font_size.0,
            ) else {
                continue;
            };
            in_px = value;
            in_px.as_str()
        };
        consume(
            128usize
                .saturating_add(expression.len())
                .saturating_add(text.len().saturating_mul(2)),
        )?;
        if !push_declaration(out, svg_presentation_name(attribute.property), text) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Whether `property` takes a stroke length from an ancestor: it inherits it,
/// or the cascade wrote the length of the ancestor in its place.
fn inherits_stroke_length(property: &SvgStyleProperty) -> bool {
    matches!(
        property.property,
        PropertyKey::StrokeWidth | PropertyKey::StrokeDasharray | PropertyKey::StrokeDashoffset
    ) && (property.inherited || property.inherited_from.is_some())
}

fn resolve_relative_svg_presentation_value(
    property: PropertyKey,
    expression: &str,
    font_size: f32,
) -> Option<String> {
    if !font_size.is_finite() {
        return None;
    }
    // Most lengths are absolute and need no new text.
    match property {
        PropertyKey::StrokeWidth | PropertyKey::StrokeDashoffset => {
            if !is_font_relative(expression) {
                return None;
            }
            let (value, changed) = resolve_relative_svg_length(expression, font_size)?;
            changed.then_some(value)
        }
        PropertyKey::StrokeDasharray => {
            if !expression.split_ascii_whitespace().any(is_font_relative) {
                return None;
            }
            let mut changed = false;
            let values = expression
                .split_ascii_whitespace()
                .map(|item| {
                    let (value, item_changed) = resolve_relative_svg_length(item, font_size)?;
                    changed |= item_changed;
                    Some(value)
                })
                .collect::<Option<Vec<_>>>()?;
            changed.then(|| values.join(" "))
        }
        _ => None,
    }
}

/// Whether the length `value` is in `em` or `ex`.
fn is_font_relative(value: &str) -> bool {
    value.ends_with("em") || value.ends_with("ex")
}

fn resolve_relative_svg_length(value: &str, font_size: f32) -> Option<(String, bool)> {
    let (number, multiplier) = if let Some(number) = value.strip_suffix("em") {
        (number, 1.0)
    } else if let Some(number) = value.strip_suffix("ex") {
        (number, 0.5)
    } else {
        return Some((value.to_owned(), false));
    };
    let number = number.parse::<f32>().ok()?;
    let pixels = number * font_size * multiplier;
    pixels.is_finite().then(|| (format!("{pixels}px"), true))
}

#[cfg(test)]
mod tests;
