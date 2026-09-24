use super::*;
/// Document arena を DFS で walk し、最初の `<body>` element の arena index を返す。
///
/// iterative `Vec` stack で実装 (cascade §deep_nesting の pattern と一貫、
/// deep DOM で stack overflow を回避)。fragment parse (no `<body>`) では
/// `None`、caller が `LayoutError::Internal` に昇格させる。
///
/// `!is_in_document()` の subtree
/// (`<template>` descendants など) を skip する。inert subtree 内に `<body>`
/// tag があってもそれを本物の body として選ばないため — 例えば
/// `<template><body>ghost</body></template>` の後に real `<body>` が来る HTML
/// で ghost body を選んでしまうと後続の layout / paint が inert subtree に対して
/// 実行されてしまう。
pub(crate) fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root];
    while let Some(node_idx) = stack.pop() {
        let node = &doc.nodes[node_idx];
        if !node.is_in_document() {
            // inert subtree — 本 subtree の中に body があっても選ばない。
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some("body") {
            return Some(node_idx);
        }
        // children を reverse push すると document order で pop される
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// `<body>` の taffy::Style.size を PageBox の width / height (CSS px) に強制する。
///
/// CSS Paged Media の initial containing block = @page size。現行実装は @page 非対応
/// のため body.style.size に直接注入する妥協。将来 @page cascade + per-page
/// PageBox を導入する際に `<html>` root style に site を昇格予定。
#[allow(dead_code)]
pub(crate) fn apply_page_box_to_body(doc: &mut Document, body_id: usize, page_box: PageBox) {
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(page_box.width),
        height: Dimension::length(page_box.height),
    };
}

pub(crate) fn apply_page_content_box_to_body(
    doc: &mut Document,
    body_id: usize,
    page_box: PageBox,
    margins: PageMargins,
    insets: PageContentInsets,
) {
    // Page border/padding shift the painted page origin, but they do not
    // establish a narrower inline containing block for document flow.  Keep
    // the initial containing-block width at the margin content width; the
    // paint walk applies the horizontal inset when positioning the flow.
    doc.nodes[body_id].style.size = Size {
        width: Dimension::length(margins.content_width(page_box).max(0.0)),
        height: Dimension::length(
            (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0),
        ),
    };
}

/// Resolve the width of a direct-body absolutely positioned box whose
/// horizontal insets and preferred width are all `auto`.
///
/// Taffy's absolute-position fallback leaves such a box at the containing
/// block width even when a resolved horizontal margin consumes part of that
/// width. CSS 2.1 §10.3.7 solves the horizontal constraint instead: the
/// border box plus both margins must fit the containing block. Do this before
/// the root layout so text descendants receive the corrected width while
/// shaping/reflowing; changing only `unrounded_layout` after the compute would
/// leave their line breaks stale.
///
/// The first implementation is deliberately limited to direct `<body>`
/// children in the static containing block. Nested containing blocks and
/// viewport-fixed boxes need their own containing-block geometry and remain
/// on Taffy's normal path until that geometry is available.
pub(crate) fn used_style_length_percentage(value: LengthPercentage, basis: f32) -> Option<f32> {
    let raw = value.into_raw();
    let resolved = match raw.tag() {
        CompactLength::LENGTH_TAG => raw.value(),
        CompactLength::PERCENT_TAG => raw.value() * basis,
        _ => return None,
    };
    resolved.is_finite().then_some(resolved)
}

pub(crate) fn used_style_length_percentage_auto(
    value: LengthPercentageAuto,
    basis: f32,
) -> Option<f32> {
    let raw = value.into_raw();
    let resolved = match raw.tag() {
        CompactLength::LENGTH_TAG => raw.value(),
        CompactLength::PERCENT_TAG => raw.value() * basis,
        _ => return None,
    };
    resolved.is_finite().then_some(resolved)
}

pub(crate) fn used_computed_length_percentage_or_auto(
    value: ComputedLengthPercentageOrAuto,
    basis: f32,
) -> Option<f32> {
    let resolved = match value {
        ComputedLengthPercentageOrAuto::Px(px) => px,
        ComputedLengthPercentageOrAuto::Percent(percent) => basis * percent / 100.0,
        ComputedLengthPercentageOrAuto::Calc(value) => value.px + basis * value.percent / 100.0,
        ComputedLengthPercentageOrAuto::Auto => return None,
    };
    resolved.is_finite().then_some(resolved)
}

pub(crate) fn resolve_direct_absolute_auto_widths(
    document: &mut Document,
    cascade: &CascadeResult,
    body_id: usize,
    containing_width: f32,
) {
    if !containing_width.is_finite() || containing_width <= 0.0 {
        return;
    }
    let children = document.nodes[body_id].children.clone();
    for child_id in children {
        let computed = &cascade.computed[child_id];
        if !matches!(computed.position, PositionValue::Absolute)
            || !matches!(computed.width, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(computed.left, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(computed.right, ComputedLengthPercentageOrAuto::Auto)
        {
            continue;
        }

        let style = &document.nodes[child_id].style;
        let margin_left = used_style_length_percentage_auto(style.margin.left, containing_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.left, containing_width)
            });
        let margin_right = used_style_length_percentage_auto(style.margin.right, containing_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.right, containing_width)
            });
        if margin_left.is_none() && margin_right.is_none() {
            continue;
        }
        let margin_left = margin_left.unwrap_or(0.0);
        let margin_right = margin_right.unwrap_or(0.0);
        let padding_left = used_style_length_percentage(style.padding.left, containing_width);
        let padding_right = used_style_length_percentage(style.padding.right, containing_width);
        let Some(padding_left) = padding_left else {
            continue;
        };
        let Some(padding_right) = padding_right else {
            continue;
        };
        let border_left = computed.border.left.width().px();
        let border_right = computed.border.right.width().px();
        if !border_left.is_finite() || !border_right.is_finite() {
            continue;
        }

        let available_border_box = containing_width - margin_left - margin_right;
        let width = match computed.box_sizing {
            StyleBoxSizing::BorderBox => available_border_box,
            StyleBoxSizing::ContentBox => {
                available_border_box - border_left - border_right - padding_left - padding_right
            }
            _ => available_border_box - border_left - border_right - padding_left - padding_right,
        };
        if width.is_finite() {
            document.nodes[child_id].style.size.width = Dimension::length(width.max(0.0));
        }
    }
}

/// Used page margins resolved from the page-context cascade.
///
/// The four values are paper-relative CSS pixels.  Unlike ordinary element
/// margins, page percentages use the corresponding page-box axis: top and
/// bottom use the page height, while left and right use the page width.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PageMargins {
    /// Used top margin in CSS pixels.
    pub top: f32,
    /// Used right margin in CSS pixels.
    pub right: f32,
    /// Used bottom margin in CSS pixels.
    pub bottom: f32,
    /// Used left margin in CSS pixels.
    pub left: f32,
}

impl PageMargins {
    /// Width available to the page's content area.
    pub fn content_width(self, page_box: PageBox) -> f32 {
        (page_box.width - self.left - self.right).max(0.0)
    }

    /// Height available to the page's content area.
    pub fn content_height(self, page_box: PageBox) -> f32 {
        (page_box.height - self.top - self.bottom).max(0.0)
    }

    /// Whether all four used margins are zero.
    pub fn is_zero(self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

fn page_length_to_px(length: Length, basis: f32) -> f32 {
    let value = match length {
        Length::Px(value) => value,
        Length::Pt(value) => value * 96.0 / 72.0,
        Length::Cm(value) => value * 96.0 / 2.54,
        Length::Mm(value) => value * 96.0 / 25.4,
        Length::Q(value) => value * 96.0 / 101.6,
        Length::In(value) => value * 96.0,
        Length::Pc(value) => value * 96.0 / 6.0,
        // Page-context absolutization normally resolves these before this
        // consumer sees them.  Keep a deterministic initial-font fallback for
        // values introduced through direct/internal cascade construction.
        Length::Em(value)
        | Length::Rem(value)
        | Length::Ex(value)
        | Length::Rex(value)
        | Length::Ch(value)
        | Length::Rch(value)
        | Length::Ic(value)
        | Length::Ric(value)
        | Length::Lh(value)
        | Length::Rlh(value) => value * 16.0,
        Length::Percent(value) => basis * value / 100.0,
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

fn page_margin_length(value: LengthOrAuto, basis: f32) -> f32 {
    let value = match value {
        LengthOrAuto::Auto => 0.0,
        LengthOrAuto::Length(length) => page_length_to_px(length, basis),
        LengthOrAuto::Calc(CalcLengthPercentage { percent, px }) => {
            let percent = if percent.is_finite() { percent } else { 0.0 };
            let px = if px.is_finite() { px } else { 0.0 };
            px + basis * percent / 100.0
        }
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

/// Used border and padding inset inside the page margin area.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PageContentInsets {
    /// Top border plus padding.
    pub top: f32,
    /// Right border plus padding.
    pub right: f32,
    /// Bottom border plus padding.
    pub bottom: f32,
    /// Left border plus padding.
    pub left: f32,
}

fn page_box_side(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    padding: PropertyKey,
    border: PropertyKey,
    basis: f32,
) -> f32 {
    let value = match declarations.get(&padding) {
        Some(PropertyValue::PaddingTop(value))
        | Some(PropertyValue::PaddingRight(value))
        | Some(PropertyValue::PaddingBottom(value))
        | Some(PropertyValue::PaddingLeft(value)) => Some(*value),
        _ => None,
    };
    let padding = value.map_or(0.0, |length| page_length_to_px(length, basis));
    // A page border with an explicit margin encloses the page content box;
    // preserve the existing overlay behavior for a border-only page with no
    // margin (for example the page-box border smoke test).  Padding already
    // establishes an explicit content box and therefore always includes the
    // border.
    let has_page_margin = declarations.keys().any(|key| {
        matches!(
            key,
            PropertyKey::Margin
                | PropertyKey::MarginTop
                | PropertyKey::MarginRight
                | PropertyKey::MarginBottom
                | PropertyKey::MarginLeft
        )
    });
    let border = if padding > 0.0 || has_page_margin {
        match declarations.get(&border) {
            Some(PropertyValue::BorderTopWidth(value))
            | Some(PropertyValue::BorderRightWidth(value))
            | Some(PropertyValue::BorderBottomWidth(value))
            | Some(PropertyValue::BorderLeftWidth(value)) => page_length_to_px(*value, basis),
            _ => 0.0,
        }
    } else {
        0.0
    };
    (padding + border).max(0.0)
}

/// Resolve the border and padding inset of the page content box.
///
/// Page margins remain separate because margin boxes occupy the margin strips.
/// The inset shifts the physical flow origin and reduces the block-axis
/// fragmentainer extent; the inline containing-block width remains the page's
/// margin content width so page decorations do not force text rewrapping.
pub fn page_content_insets(cascade: &CascadeResult, page_box: PageBox) -> PageContentInsets {
    let declarations = cascade.page.declarations();
    PageContentInsets {
        top: page_box_side(
            declarations,
            PropertyKey::PaddingTop,
            PropertyKey::BorderTopWidth,
            page_box.height,
        ),
        right: page_box_side(
            declarations,
            PropertyKey::PaddingRight,
            PropertyKey::BorderRightWidth,
            page_box.width,
        ),
        bottom: page_box_side(
            declarations,
            PropertyKey::PaddingBottom,
            PropertyKey::BorderBottomWidth,
            page_box.height,
        ),
        left: page_box_side(
            declarations,
            PropertyKey::PaddingLeft,
            PropertyKey::BorderLeftWidth,
            page_box.width,
        ),
    }
}

fn page_margin_value(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    side: PropertyKey,
    shorthand: Option<LengthOrAuto>,
) -> Option<LengthOrAuto> {
    match (side, declarations.get(&side)) {
        (PropertyKey::MarginTop, Some(PropertyValue::MarginTop(value)))
        | (PropertyKey::MarginRight, Some(PropertyValue::MarginRight(value)))
        | (PropertyKey::MarginBottom, Some(PropertyValue::MarginBottom(value)))
        | (PropertyKey::MarginLeft, Some(PropertyValue::MarginLeft(value))) => Some(*value),
        _ => shorthand,
    }
}

fn page_dimension(
    declarations: &std::collections::HashMap<PropertyKey, PropertyValue>,
    key: PropertyKey,
    basis: f32,
) -> Option<f32> {
    let value = declarations.get(&key)?;
    let value = match (key, value) {
        (PropertyKey::Width, PropertyValue::Width(LengthOrAuto::Length(value)))
        | (PropertyKey::Height, PropertyValue::Height(LengthOrAuto::Length(value))) => {
            page_length_to_px(*value, basis)
        }
        (PropertyKey::Width, PropertyValue::Width(LengthOrAuto::Calc(value)))
        | (PropertyKey::Height, PropertyValue::Height(LengthOrAuto::Calc(value))) => {
            value.px + basis * value.percent / 100.0
        }
        _ => return None,
    };
    (value.is_finite() && value >= 0.0).then_some(value)
}

/// Resolve the used page margins from a page cascade and paper size.
///
/// `cascade_page` deliberately keeps page percentages symbolic because the
/// page box is a downstream used-value basis.  This is the single geometry
/// entry point used by layout, scene extraction, and paint so all three agree
/// on the paper/content split.
pub fn page_margins(cascade: &CascadeResult, page_box: PageBox) -> PageMargins {
    let declarations = cascade.page.declarations();
    // The legacy page `width`/`height` descriptors size the page area inside
    // the page box.  When they are combined with percentage margins, those
    // percentages resolve against the pre-descriptor page size, not the
    // already reduced outer box.  Keep that basis stable so e.g. `size: 500px;
    // margin: 10%; width: 40%; height: 60%` uses 50px margins and a 200x300
    // content area after the outer box is reduced to 300x400.
    let margin_basis = if (declarations.contains_key(&PropertyKey::Width)
        || declarations.contains_key(&PropertyKey::Height))
        && cascade.page.size().is_some()
    {
        PageBox::from_page_size(cascade.page.size())
    } else {
        page_box
    };
    let shorthand = match declarations.get(&PropertyKey::Margin) {
        Some(PropertyValue::Margin(sides)) => Some(*sides),
        _ => None,
    };
    let top_value = page_margin_value(
        declarations,
        PropertyKey::MarginTop,
        shorthand.map(|s| s.top),
    );
    let right_value = page_margin_value(
        declarations,
        PropertyKey::MarginRight,
        shorthand.map(|s| s.right),
    );
    let bottom_value = page_margin_value(
        declarations,
        PropertyKey::MarginBottom,
        shorthand.map(|s| s.bottom),
    );
    let left_value = page_margin_value(
        declarations,
        PropertyKey::MarginLeft,
        shorthand.map(|s| s.left),
    );
    let mut margins = PageMargins {
        top: top_value.map_or(0.0, |value| page_margin_length(value, margin_basis.height)),
        right: right_value.map_or(0.0, |value| page_margin_length(value, margin_basis.width)),
        bottom: bottom_value.map_or(0.0, |value| page_margin_length(value, margin_basis.height)),
        left: left_value.map_or(0.0, |value| page_margin_length(value, margin_basis.width)),
    };

    // In a page context, legacy `width`/`height` describe the page area.  If
    // that area is smaller than the physical page and a margin is `auto`, the
    // remaining space is distributed between the auto sides of that axis.
    // Without those descriptors the existing zero fallback is retained.
    let page_area_width = page_dimension(declarations, PropertyKey::Width, page_box.width);
    let page_area_height = page_dimension(declarations, PropertyKey::Height, page_box.height);
    if let Some(area_width) = page_area_width {
        let auto_left = matches!(left_value, Some(LengthOrAuto::Auto));
        let auto_right = matches!(right_value, Some(LengthOrAuto::Auto));
        let auto_count = usize::from(auto_left) + usize::from(auto_right);
        if auto_count > 0 {
            // Auto page margins absorb the full remainder, including a
            // negative one when the requested page area is larger than the
            // physical page box.
            let remaining = page_box.width - area_width - margins.left - margins.right;
            let auto_margin = remaining / auto_count as f32;
            if auto_left {
                margins.left = auto_margin;
            }
            if auto_right {
                margins.right = auto_margin;
            }
        }
    }
    if let Some(area_height) = page_area_height {
        let auto_top = matches!(top_value, Some(LengthOrAuto::Auto));
        let auto_bottom = matches!(bottom_value, Some(LengthOrAuto::Auto));
        let auto_count = usize::from(auto_top) + usize::from(auto_bottom);
        if auto_count > 0 {
            let remaining = page_box.height - area_height - margins.top - margins.bottom;
            let auto_margin = remaining / auto_count as f32;
            if auto_top {
                margins.top = auto_margin;
            }
            if auto_bottom {
                margins.bottom = auto_margin;
            }
        }
    }
    margins
}

/// Resolve the first page-context scan's child order from cascaded styles.
/// This runs before the first Taffy layout, so it can use order-modified source
/// order for flex items and grid items with auto row start/end and column
/// placement at auto or line 1, but not Taffy's resolved grid row details.
fn initial_page_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    let children = document.nodes[parent_id].children.clone();
    let parent_style = &cascade.computed[parent_id];
    let is_flex = matches!(
        parent_style.display,
        DisplayValue::Flex | DisplayValue::InlineFlex
    );
    let is_auto_row_grid = matches!(
        parent_style.display,
        DisplayValue::Grid | DisplayValue::InlineGrid
    ) && children
        .iter()
        .copied()
        .filter(|&child_id| {
            !matches!(
                cascade.computed[child_id].position,
                PositionValue::Absolute | PositionValue::Fixed
            ) && cascade.computed[child_id].display != DisplayValue::None
        })
        .all(|child_id| {
            matches!(
                &cascade.computed[child_id].grid_row_start,
                GridLineValue::Auto
            ) && matches!(
                &cascade.computed[child_id].grid_row_end,
                GridLineValue::Auto
            ) && matches!(
                &cascade.computed[child_id].grid_column_start,
                GridLineValue::Auto | GridLineValue::Line(1)
            )
        });
    if !is_flex && !is_auto_row_grid {
        return children;
    }

    let is_in_flow = |child_id: usize| {
        cascade.computed[child_id].display != DisplayValue::None
            && !matches!(
                cascade.computed[child_id].position,
                PositionValue::Absolute | PositionValue::Fixed
            )
    };
    let mut ordered_items: Vec<_> = children
        .iter()
        .copied()
        .filter(|&child_id| is_in_flow(child_id))
        .collect();
    ordered_items.sort_by_key(|&child_id| cascade.computed[child_id].order);
    if is_flex
        && matches!(
            parent_style.flex_direction,
            FlexDirectionValue::ColumnReverse
        )
    {
        ordered_items.reverse();
    }

    let mut ordered_items = ordered_items.into_iter();
    let mut ordered_children = children;
    for child_id in &mut ordered_children {
        if is_in_flow(*child_id) {
            *child_id = ordered_items
                .next()
                .expect("every in-flow child has one initial-page order entry");
        }
    }
    ordered_children
}

/// Cached child order and Grid rows are usable only for the exact cascade run
/// that produced the layout, while the tree is clean and stored `order` values
/// still match the cascade the caller wants to inspect.
fn document_has_resolved_pagination_order(document: &Document, cascade: &CascadeResult) -> bool {
    let has_resolved_order = document.nodes.iter().any(|node| {
        node.grid_column_count > 0
            || !node.grid_item_row_starts.is_empty()
            || !node.order_modified_children.is_empty()
    });
    let cascade_order_matches_layout = document.nodes.iter().enumerate().all(|(node_id, node)| {
        cascade
            .computed
            .get(node_id)
            .is_some_and(|computed| node.order == computed.order)
    });
    has_resolved_order
        && !document.layout_dirty
        && document.layout_cascade_generation == Some(cascade.generation())
        && cascade_order_matches_layout
}

/// Find the named page requested by the first rendered in-flow class-A box.
///
/// A named page on the first in-flow box selects the initial page context, so
/// callers must resolve its `@page` size and margins before the first layout
/// pass. Before layout, recursive scans use cascaded flex/grid order where
/// placement is known; after layout, they use order projections and resolved
/// grid rows. Later named descendants belong to a pagination transition and
/// must not change the initial layout geometry. The scan therefore stops after
/// the first rendered in-flow body child.
pub fn first_page_name(document: &Document, cascade: &CascadeResult) -> Option<String> {
    let body_id = (0..document.nodes.len())
        .find(|&node_id| document.nodes[node_id].tag_name() == Some("body"))?;
    let resolved_layout = document_has_resolved_pagination_order(document, cascade);
    if let Some(raikiri_style::property::PageValue::Named(name)) = cascade.page_values.get(body_id)
    {
        return Some(name.to_string());
    }
    let child_order = if resolved_layout {
        pagination_child_order(document, cascade, body_id)
    } else {
        initial_page_child_order(document, cascade, body_id)
    };
    for child_id in child_order {
        let node = document.get_node(child_id)?;
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || (node.kind() == NodeKind::Element
                && cascade.computed[child_id].display == DisplayValue::None)
        {
            continue;
        }
        match node.kind() {
            NodeKind::Text => {
                if let crate::node::NodeData::Text(text) = &node.data
                    && !text.text_content.trim().is_empty()
                {
                    return None;
                }
            }
            NodeKind::Element => {
                let computed = &cascade.computed[child_id];
                if !matches!(
                    computed.position,
                    raikiri_style::property::PositionValue::Static
                        | raikiri_style::property::PositionValue::Relative
                        | raikiri_style::property::PositionValue::Sticky
                ) || !matches!(computed.float, FloatValue::None)
                {
                    continue;
                }
                if let Some(raikiri_style::property::PageValue::Named(name)) =
                    cascade.page_values.get(child_id)
                {
                    return Some(name.to_string());
                }
                // A first in-flow descendant can carry the page value through
                // an anonymous block. Resolve that propagation before falling
                // back to the anonymous initial page; later siblings must not
                // change the initial page context.
                let (_, propagated) = propagated_start_page_name_with_order(
                    document,
                    cascade,
                    child_id,
                    None,
                    resolved_layout,
                );
                return propagated;
            }
            _ => {}
        }
    }
    None
}

/// Cascaded state selected for the first page after verifying resolved Grid placement.
///
/// `page_name` is the query name used to build `cascade`; `page_box` and
/// `font_context` are the matching layout inputs. Callers rebuild these values
/// in `recascade` when the resolved first in-flow box selects a different page.
pub struct InitialPageContext {
    /// Named page used by the current first-page cascade, if any.
    pub page_name: Option<String>,
    /// Cascade for the current first-page query.
    pub cascade: CascadeResult,
    /// Physical page dimensions derived from that cascade or caller defaults.
    pub page_box: PageBox,
    /// Font context used when probing the first-page layout.
    pub font_context: FontContext,
}

/// Failure while resolving the page context selected by the first placed Grid item.
#[derive(Debug)]
pub enum InitialPageContextError {
    /// The single-page placement probe failed.
    Layout(LayoutError),
    /// Named-page size changes kept changing the first resolved Grid item.
    PageGeometryDidNotConverge {
        /// Number of placement passes attempted.
        iterations: u32,
    },
}

impl std::fmt::Display for InitialPageContextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Layout(error) => write!(f, "initial page placement failed: {error}"),
            Self::PageGeometryDidNotConverge { iterations } => write!(
                f,
                "initial page context did not converge after {iterations} placement passes"
            ),
        }
    }
}

impl std::error::Error for InitialPageContextError {}

/// Optional resources used by the initial-page layout probe.
///
/// Provide the same resolver and base URL as the final layout pass so image
/// intrinsic sizes produce matching Flex/Grid placement.
#[derive(Clone, Copy, Default)]
pub struct InitialPageProbeResources<'a> {
    resolver: Option<&'a dyn ReplacedResolver>,
    base_url: Option<&'a url::Url>,
}

impl<'a> InitialPageProbeResources<'a> {
    /// Create probe resources from the caller's optional image resolver and base URL.
    pub const fn new(
        resolver: Option<&'a dyn ReplacedResolver>,
        base_url: Option<&'a url::Url>,
    ) -> Self {
        Self { resolver, base_url }
    }
}

/// Resolve and rebuild the initial page context using the first placed Grid box.
///
/// Flex/Grid order and Grid placement may change which named-page descendant
/// starts a document. When a document has both a named page and a Flex/Grid
/// container, this helper probes placement, asks [`first_page_name`] for the
/// resolved first box, and invokes `recascade` when that name differs from the
/// current page query. Every probe uses the caller's replaced-resource resolver
/// and base URL so intrinsic sizes match the final layout. The operation is
/// bounded so self-referential page-size changes fail explicitly. Documents
/// without both features return their input state without a probe.
#[allow(clippy::result_large_err)]
pub fn resolve_initial_page_context(
    document: &Document,
    page_name: Option<String>,
    cascade: CascadeResult,
    page_box: PageBox,
    font_context: FontContext,
    resources: InitialPageProbeResources<'_>,
    mut recascade: impl FnMut(Option<&str>) -> (CascadeResult, PageBox, FontContext),
) -> Result<InitialPageContext, InitialPageContextError> {
    let mut context = InitialPageContext {
        page_name,
        cascade,
        page_box,
        font_context,
    };
    let has_named_page = context
        .cascade
        .page_values
        .iter()
        .any(|page_value| matches!(page_value, raikiri_style::property::PageValue::Named(_)));
    let has_flex_or_grid = context.cascade.computed.iter().any(|computed| {
        matches!(
            computed.display,
            DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
        )
    });
    if !has_named_page || !has_flex_or_grid {
        return Ok(context);
    }

    const MAX_INITIAL_PAGE_CONTEXT_PASSES: u32 = 3;
    for _ in 0..MAX_INITIAL_PAGE_CONTEXT_PASSES {
        let mut probe_document = document.clone();
        let probe_layout = if let Some(resolver) = resources.resolver {
            layout_single_page_with_resolver_and_base_url(
                &mut probe_document,
                &context.cascade,
                context.page_box,
                context.font_context.clone(),
                resolver,
                resources.base_url,
            )
        } else {
            layout_single_page(
                &mut probe_document,
                &context.cascade,
                context.page_box,
                context.font_context.clone(),
            )
        };
        probe_layout.map_err(InitialPageContextError::Layout)?;
        let resolved_name = first_page_name(&probe_document, &context.cascade);
        if resolved_name == context.page_name {
            return Ok(context);
        }

        context.page_name = resolved_name;
        let (cascade, page_box, font_context) = recascade(context.page_name.as_deref());
        context.cascade = cascade;
        context.page_box = page_box;
        context.font_context = font_context;
    }
    // cov:ignore: an oscillating named-grid page query needs a self-referential page-size fixture; callers expose a structured terminal error.
    Err(InitialPageContextError::PageGeometryDidNotConverge {
        iterations: MAX_INITIAL_PAGE_CONTEXT_PASSES,
    })
}

/// Rebuild the derived child-order view after every computed-style bridge.
///
/// CSS Flexbox and Grid consume flex/grid items in order-modified document
/// order, while the DOM child vector must remain in source order for traversal,
/// selectors, counters, and serialization. Taffy has no CSS `order` input in
/// `Style`, so retain a stable sorted projection only for containers with a
/// non-zero ordered child. `TaffyChildIter` applies its existing flat-tree and
/// whitespace filtering to this view.
pub(crate) fn refresh_order_modified_children(doc: &mut Document) {
    let mut projection_changed = false;
    let grid_details_stale = doc.layout_dirty;
    for parent_idx in 0..doc.nodes.len() {
        let should_sort = {
            let parent = &doc.nodes[parent_idx];
            matches!(parent.style.display, Display::Flex | Display::Grid)
                && parent.children.iter().any(|&child| {
                    doc.nodes[child].style.position != TaffyPosition::Absolute
                        && doc.nodes[child].order != 0
                })
        };
        let new_projection = if should_sort {
            let children = doc.nodes[parent_idx].children.clone();
            let mut ordered_items: Vec<usize> = children
                .iter()
                .copied()
                .filter(|&child| doc.nodes[child].style.position != TaffyPosition::Absolute)
                .collect();
            // `sort_by_key` is stable, so equal `order` values retain DOM order.
            ordered_items.sort_by_key(|&child| doc.nodes[child].order);
            let mut ordered_items = ordered_items.into_iter();
            let mut ordered_children = children;
            for child in &mut ordered_children {
                if doc.nodes[*child].style.position != TaffyPosition::Absolute {
                    *child = ordered_items
                        .next()
                        .expect("every in-flow child has one ordered entry");
                }
            }
            ordered_children.into_boxed_slice()
        } else {
            Box::new([])
        };

        if doc.nodes[parent_idx].order_modified_children.as_ref() != new_projection.as_ref() {
            projection_changed = true;
            doc.nodes[parent_idx].order_modified_children = new_projection;
        }
    }

    if projection_changed {
        // Reordering changes Taffy's child traversal. Clear every cache before
        // layout starts: a root/ancestor cache hit would otherwise skip the
        // normal lazy invalidation hook and preserve stale item positions.
        for node in &mut doc.nodes {
            node.cache.clear();
            node.grid_item_row_starts = Box::new([]);
            node.grid_column_count = 0;
        }
    } else if grid_details_stale {
        // A DOM mutation will invalidate layout caches in Taffy's normal path;
        // discard placement metadata until the next grid layout repopulates it.
        for node in &mut doc.nodes {
            node.grid_item_row_starts = Box::new([]);
            node.grid_column_count = 0;
        }
    }
}

/// Whether `child_id` participates in its flex parent's ordered item sequence.
/// Match Taffy's flat-tree, box-generation, out-of-flow, and whitespace-text
/// filters so reversing the sequence changes only actual flex items.
pub(crate) fn is_in_flow_flex_child_for_pagination(
    document: &Document,
    parent_id: usize,
    child_id: usize,
) -> bool {
    let child = &document.nodes[child_id];
    child.is_in_document()
        && child.style.display != Display::None
        && child.style.position != TaffyPosition::Absolute
        && !matches!(
            &child.data,
            NodeData::Text(text)
                if text.text_content.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        )
        && document.nodes[parent_id].style.display == Display::Flex
}

/// Return a flex parent's page-traversal order, which follows top-to-bottom
/// visual order for `column-reverse` and Taffy's order-modified order otherwise.
fn flex_pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    let mut children = document.nodes[parent_id].layout_children().to_vec();
    if !matches!(
        cascade.computed[parent_id].flex_direction,
        FlexDirectionValue::ColumnReverse
    ) {
        return children;
    }

    let mut ordered_items: Vec<_> = children
        .iter()
        .copied()
        .filter(|&child_id| is_in_flow_flex_child_for_pagination(document, parent_id, child_id))
        .collect();
    ordered_items.reverse();
    let mut ordered_items = ordered_items.into_iter();
    for child_id in &mut children {
        if is_in_flow_flex_child_for_pagination(document, parent_id, *child_id) {
            *child_id = ordered_items
                .next()
                .expect("each in-flow flex child has one reversed entry");
        }
    }
    children
}

fn is_in_flow_grid_item_for_pagination(
    document: &Document,
    cascade: &CascadeResult,
    child_id: usize,
) -> bool {
    match document.nodes[child_id].kind() {
        NodeKind::Element => {
            cascade.computed[child_id].display != DisplayValue::None
                && matches!(
                    cascade.computed[child_id].position,
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                )
        }
        NodeKind::Text => matches!(
            &document.nodes[child_id].data,
            NodeData::Text(text) if !text.text_content.trim().is_empty()
        ),
        _ => false,
    }
}

/// Child order shared by page-candidate collection and named-page propagation.
pub(crate) fn pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    let computed = &cascade.computed[parent_id];
    if matches!(
        computed.display,
        DisplayValue::Flex | DisplayValue::InlineFlex
    ) {
        return flex_pagination_child_order(document, cascade, parent_id);
    }
    let is_single_column_grid = matches!(
        computed.display,
        DisplayValue::Grid | DisplayValue::InlineGrid
    ) && document.nodes[parent_id].grid_column_count == 1;
    if !is_single_column_grid {
        if matches!(
            computed.display,
            DisplayValue::Grid | DisplayValue::InlineGrid
        ) {
            // Multi-column pagination is not implemented, but named-page
            // propagation and candidate traversal must retain Grid's
            // order-modified child sequence.
            return document.nodes[parent_id].layout_children().to_vec();
        }
        return document.nodes[parent_id].children.clone();
    }

    let mut children = document.nodes[parent_id].layout_children().to_vec();
    let mut in_flow_items: Vec<_> = document.nodes[parent_id]
        .grid_item_row_starts
        .iter()
        .copied()
        .filter(|(child_id, _)| is_in_flow_grid_item_for_pagination(document, cascade, *child_id))
        .collect();
    let pageable_count = children
        .iter()
        .filter(|&&child_id| is_in_flow_grid_item_for_pagination(document, cascade, child_id))
        .count();
    if in_flow_items.len() == pageable_count {
        // Stable sorting keeps order-modified order within a resolved row.
        in_flow_items.sort_by_key(|&(_, row_start)| row_start);
        let mut ordered_items = in_flow_items.into_iter().map(|(child_id, _)| child_id);
        for child_id in &mut children {
            if is_in_flow_grid_item_for_pagination(document, cascade, *child_id) {
                *child_id = ordered_items
                    .next()
                    .expect("Taffy row details cover each in-flow grid item");
            }
        }
    }
    children
}

/// Return the start page value propagated from the first in-flow child box.
///
/// CSS Page 3 derives a box's start page value from its first child when
/// that child participates in a class-A break point. Nested named boxes
/// therefore do not each open a page; the deepest first in-flow box owns
/// the value compared at the boundary.
pub(crate) fn propagated_start_page_name(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&str>,
) -> (bool, Option<String>) {
    propagated_start_page_name_with_order(document, cascade, node_id, inherited_page_name, true)
}

fn propagated_start_page_name_with_order(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&str>,
    resolved_layout: bool,
) -> (bool, Option<String>) {
    let Some(node) = document.get_node(node_id) else {
        return (false, None);
    };
    if !node.is_in_document() || node.is_display_none() {
        return (false, None);
    }
    if matches!(node.kind(), NodeKind::Text) {
        let has_text = matches!(
            &node.data,
            crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
        );
        if !has_text {
            return (false, None);
        }
        return (true, inherited_page_name.map(ToOwned::to_owned));
    }
    let display = cascade.computed[node_id].display;
    let page_applies = !matches!(
        display,
        DisplayValue::Inline | DisplayValue::InlineFlex | DisplayValue::InlineGrid
    );
    let explicit_page_name = if page_applies {
        selected_page_name(cascade, node_id)
    } else {
        None
    };
    let used_page_name = explicit_page_name
        .clone()
        .or_else(|| inherited_page_name.map(ToOwned::to_owned));
    let child_order = if resolved_layout {
        pagination_child_order(document, cascade, node_id)
    } else {
        initial_page_child_order(document, cascade, node_id)
    };
    for child_id in child_order {
        if child_id >= cascade.computed.len() {
            continue;
        }
        let child_computed = &cascade.computed[child_id];
        if matches!(
            child_computed.position,
            PositionValue::Absolute | PositionValue::Fixed
        ) || !matches!(child_computed.float, FloatValue::None)
        {
            continue;
        }
        let (has_box, child_start) = propagated_start_page_name_with_order(
            document,
            cascade,
            child_id,
            used_page_name.as_deref(),
            resolved_layout,
        );
        if has_box {
            return (true, child_start);
        }
    }
    (true, used_page_name)
}

#[cfg(test)]
mod tests;
