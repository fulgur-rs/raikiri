use super::box_tree::BoxTree;
use super::*;
use crate::node::FormattingContext;
use raikiri_layout_style::values::Position;
use std::borrow::Cow;

#[cfg(test)]
thread_local! {
    static PAGE_NAME_WORK: std::cell::Cell<[usize; 6]> = const { std::cell::Cell::new([0; 6]) };
}

#[cfg(test)]
pub(super) fn record_page_name_work(kind: usize) {
    PAGE_NAME_WORK.with(|work| {
        let mut counts = work.get();
        counts[kind] += 1;
        work.set(counts);
    });
}

pub(crate) fn has_named_pages(cascade: &CascadeResult) -> bool {
    cascade
        .page_values
        .iter()
        .any(|value| matches!(value, raikiri_style::property::PageValue::Named(_)))
}

/// Whether `id` can be the body element. HTML defines the body element as
/// the first `body` or `frameset` child of the root `html` element; a
/// frameset document has no `body`. A `frameset` elsewhere (one a script
/// inserted into the head, say) or in another namespace is not the body.
pub(crate) fn is_body_element(doc: &Document, id: usize) -> bool {
    const HTML: Option<&str> = Some("http://www.w3.org/1999/xhtml");
    let node = &doc.nodes[id];
    match node.tag_name() {
        Some("body") => true,
        Some("frameset") => node.parent.is_some_and(|parent| {
            doc.nodes[parent].tag_name() == Some("html")
                && doc.nodes[parent].parent == Some(doc.root)
                && doc.element_namespace_uri(id) == HTML
                && doc.element_namespace_uri(parent) == HTML
        }),
        _ => false,
    }
}

/// Walk the Document arena with DFS and return the arena index of the first
/// `<body>` (or `<frameset>`, see [`is_body_element`]) element.
///
/// Use an iterative `Vec` stack (as in cascade §deep_nesting) to avoid stack
/// overflows in deeply nested DOMs. Return `None` for a fragment parse (no
/// `<body>`); layout promotes this to `LayoutError::Internal`, the depth
/// check has nothing to measure, and paint has no root.
///
/// The walk follows `children` from the document root, so it never enters a
/// `<template>` element's contents: those live under a detached fragment
/// root, not among the element's children. A `<body>` there is inert and must
/// not be selected; for example, if the real `<body>` follows
/// `<template><body>ghost</body></template>`, choosing the ghost would run
/// layout and paint on an inert subtree. Reachability decides this rather
/// than the `IS_IN_DOCUMENT` bit, so the answer does not depend on whether
/// [`Document::mark_in_document_flags`] ran after the last tree mutation:
/// the depth check runs before layout refreshes the bits.
pub(crate) fn find_body(doc: &Document) -> Option<usize> {
    let mut stack: Vec<usize> = vec![doc.root];
    while let Some(node_idx) = stack.pop() {
        let node = &doc.nodes[node_idx];
        if node.kind() == NodeKind::Element && is_body_element(doc, node_idx) {
            return Some(node_idx);
        }
        // Reverse-pushing children makes the stack pop them in document order.
        for &child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

/// CSS 2.1 §10.3.7 note: absolutely positioned non-replaced boxes with
/// `width:auto` and `left:auto` / `right:auto` use shrink-to-fit sizing
/// (`min(max(preferred minimum, available), preferred)`), not the containing
/// block width. Taffy already performs this shrink-to-fit for both direct-body
/// and nested absolute boxes (verified: an empty direct-body absolute with a
/// 20px margin and 20px borders lays out to a 20px border box, and a 100px
/// block child yields a 100px box). A previous pre-layout fill override forced
/// direct-body boxes to the containing width and broke this case (WPT
/// line-break-anywhere 70k-pixel mismatches); it was removed so taffy's
/// shrink-to-fit path applies. Keep float (§10.3.5) and line-break handling
/// out of this module.
pub(crate) fn used_style_length_percentage(value: LengthPercentage, basis: f32) -> Option<f32> {
    super::resolve_length_percentage(value, Some(basis)).filter(|value| value.is_finite())
}

pub(crate) fn used_style_length_percentage_auto(
    value: LengthPercentageAuto,
    basis: f32,
) -> Option<f32> {
    super::resolve_length_percentage_auto(value, Some(basis)).filter(|value| value.is_finite())
}

pub(crate) fn used_computed_length_percentage_or_auto(
    value: &ComputedLengthPercentageOrAuto,
    basis: f32,
) -> Option<f32> {
    let resolved = match value {
        ComputedLengthPercentageOrAuto::Px(px) => *px,
        ComputedLengthPercentageOrAuto::Percent(percent) => basis * percent / 100.0,
        ComputedLengthPercentageOrAuto::Math(value) => value.resolve(basis),
        ComputedLengthPercentageOrAuto::Calc(value) => value.px + basis * value.percent / 100.0,
        ComputedLengthPercentageOrAuto::Auto
        | ComputedLengthPercentageOrAuto::MinContent
        | ComputedLengthPercentageOrAuto::MaxContent
        | ComputedLengthPercentageOrAuto::FitContent
        | ComputedLengthPercentageOrAuto::FitContentOf(_)
        | ComputedLengthPercentageOrAuto::Stretch => {
            return None;
        }
    };
    resolved.is_finite().then_some(resolved)
}

/// Used page margins resolved from the page-context cascade.
///
/// The four values are paper-relative CSS pixels.  Unlike ordinary element
/// margins, page percentages use the corresponding page-box axis: top and
/// bottom use the page height, while left and right use the page width.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[doc(hidden)]
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
    let value = page_length_raw(length, basis);
    if value.is_finite() { value } else { 0.0 }
}

// Comparisons must see raw leaf values; range checks belong to the whole expression.
fn page_length_raw(length: Length, basis: f32) -> f32 {
    match length {
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
    }
}

fn page_margin_length(value: &LengthOrAuto, basis: f32) -> f32 {
    let value = match value {
        LengthOrAuto::Auto => 0.0,
        LengthOrAuto::Length(length) => page_length_to_px(*length, basis),
        LengthOrAuto::Calc(CalcLengthPercentage { percent, px }) => {
            let percent = if percent.is_finite() { *percent } else { 0.0 };
            let px = if px.is_finite() { *px } else { 0.0 };
            px + basis * percent / 100.0
        }
        _ => 0.0,
    };
    if value.is_finite() { value } else { 0.0 }
}

/// Used border and padding inset inside the page margin area.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[doc(hidden)]
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

impl PageContentInsets {
    /// Width of the page area: the content area of a page box whose margins
    /// are `margins`, inside these border and padding insets.
    ///
    /// CSS Paged Media 3 §3: "The content area of a page box is called the
    /// page area." It is the inline size of the initial containing block, so
    /// document flow wraps to it.
    pub fn page_area_width(self, margins: PageMargins, page_box: PageBox) -> f32 {
        (margins.content_width(page_box) - self.left - self.right).max(0.0)
    }

    /// Height of the page area of a page box whose margins are `margins`.
    /// See [`Self::page_area_width`].
    pub fn page_area_height(self, margins: PageMargins, page_box: PageBox) -> f32 {
        (margins.content_height(page_box) - self.top - self.bottom).max(0.0)
    }
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
        | Some(PropertyValue::PaddingLeft(value)) => Some(value),
        _ => None,
    };
    let padding = value.map_or(0.0, |value| match value {
        LengthPercentageOrCalc::Length(length) => page_length_to_px(*length, basis),
        // Page-context absolutization folds font-relative terms, leaving a
        // percentage with an absolute offset.
        LengthPercentageOrCalc::Calc(calc) => {
            ComputedLengthPercentageOrCalc::Calc(*calc).resolve(basis)
        }
        _ => 0.0,
    });
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
#[doc(hidden)]
pub fn page_content_insets(cascade: &CascadeResult, page_box: PageBox) -> PageContentInsets {
    page_content_insets_for_page(&cascade.page, page_box)
}

/// Resolve the border and padding inset of the page content box from an
/// `@page` cascade alone.
///
/// This is [`page_content_insets`] for a caller that cascaded only the page
/// context, for example one that reruns the `@page` cascade per page query
/// against a single element cascade.
#[doc(hidden)]
pub fn page_content_insets_for_page(
    page: &raikiri_style::PageCascadeResult,
    page_box: PageBox,
) -> PageContentInsets {
    let declarations = page.declarations();
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
) -> Option<&LengthOrAuto> {
    match (side, declarations.get(&side)) {
        (PropertyKey::MarginTop, Some(PropertyValue::MarginTop(value)))
        | (PropertyKey::MarginRight, Some(PropertyValue::MarginRight(value)))
        | (PropertyKey::MarginBottom, Some(PropertyValue::MarginBottom(value)))
        | (PropertyKey::MarginLeft, Some(PropertyValue::MarginLeft(value))) => Some(value),
        _ => None,
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
        (PropertyKey::Width, PropertyValue::Width(LengthOrAuto::Math(value)))
        | (PropertyKey::Height, PropertyValue::Height(LengthOrAuto::Math(value))) => {
            let value = value.resolve_with(|length| page_length_raw(length, basis));
            if !value.is_finite() {
                return None;
            }
            value.max(0.0)
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
#[doc(hidden)]
pub fn page_margins(cascade: &CascadeResult, page_box: PageBox) -> PageMargins {
    page_margins_for_page(&cascade.page, page_box)
}

/// Resolve the used page margins from an `@page` cascade alone.
///
/// This is [`page_margins`] for a caller that cascaded only the page context,
/// for example one that reruns the `@page` cascade per page query against a
/// single element cascade.
#[doc(hidden)]
pub fn page_margins_for_page(
    page: &raikiri_style::PageCascadeResult,
    page_box: PageBox,
) -> PageMargins {
    let declarations = page.declarations();
    // The legacy page `width`/`height` descriptors size the page area inside
    // the page box.  When they are combined with percentage margins, those
    // percentages resolve against the pre-descriptor page size, not the
    // already reduced outer box.  Keep that basis stable so e.g. `size: 500px;
    // margin: 10%; width: 40%; height: 60%` uses 50px margins and a 200x300
    // content area after the outer box is reduced to 300x400.
    let margin_basis = if (declarations.contains_key(&PropertyKey::Width)
        || declarations.contains_key(&PropertyKey::Height))
        && page.size().is_some()
    {
        PageBox::from_page_size(page.size())
    } else {
        page_box
    };
    // Page rules expand the `margin` shorthand into its longhands when
    // parsed, so only the longhands are read here.
    let top_value = page_margin_value(declarations, PropertyKey::MarginTop);
    let right_value = page_margin_value(declarations, PropertyKey::MarginRight);
    let bottom_value = page_margin_value(declarations, PropertyKey::MarginBottom);
    let left_value = page_margin_value(declarations, PropertyKey::MarginLeft);
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
    // The descriptors size the content area, so the page border and padding
    // come out of the auto margins too.
    let insets = page_content_insets_for_page(page, page_box);
    if let Some(area_width) = page_area_width {
        let auto_left = matches!(left_value, Some(LengthOrAuto::Auto));
        let auto_right = matches!(right_value, Some(LengthOrAuto::Auto));
        let auto_count = usize::from(auto_left) + usize::from(auto_right);
        if auto_count > 0 {
            // Auto page margins absorb the full remainder, including a
            // negative one when the requested page area is larger than the
            // physical page box.
            let remaining = page_box.width
                - area_width
                - insets.left
                - insets.right
                - margins.left
                - margins.right;
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
            let remaining = page_box.height
                - area_height
                - insets.top
                - insets.bottom
                - margins.top
                - margins.bottom;
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
/// This runs before the first Taffy layout, so it can use table section order,
/// order-modified source order for flex items, and grid items with auto row start/end and column
/// placement at auto or line 1, but not Taffy's resolved grid row details.
fn initial_page_child_order<'doc>(
    document: &'doc Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Cow<'doc, [usize]> {
    let boxes = BoxTree::for_cascade(document, cascade);
    let children = &document.nodes[parent_id].children;
    let is_flex = boxes.identity(parent_id).display().is_flex_container();
    if is_flex {
        return Cow::Owned(flex_pagination_child_order(
            document, cascade, parent_id, boxes,
        ));
    }
    if matches!(
        boxes.identity(parent_id).display(),
        DisplayValue::Table | DisplayValue::InlineTable
    ) {
        return Cow::Owned(table_pagination_child_order(
            document, cascade, parent_id, boxes,
        ));
    }
    let is_auto_row_grid = boxes.identity(parent_id).display().is_grid_container()
        && children
            .iter()
            .copied()
            .filter(|&child_id| {
                !matches!(
                    boxes.identity(child_id).position(),
                    PositionValue::Absolute | PositionValue::Fixed
                ) && boxes.identity(child_id).display() != DisplayValue::None
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
    if !is_auto_row_grid {
        return Cow::Borrowed(children);
    }

    let is_in_flow = |child_id: usize| {
        boxes.identity(child_id).display() != DisplayValue::None
            && !matches!(
                boxes.identity(child_id).position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
    };
    let mut ordered_items: Vec<_> = children
        .iter()
        .copied()
        .filter(|&child_id| is_in_flow(child_id))
        .collect();
    ordered_items.sort_by_key(|&child_id| cascade.computed[child_id].order);
    let mut ordered_items = ordered_items.into_iter();
    let mut ordered_children = children.clone();
    for child_id in &mut ordered_children {
        if is_in_flow(*child_id) {
            *child_id = ordered_items
                .next()
                .expect("every in-flow child has one initial-page order entry");
        }
    }
    Cow::Owned(ordered_children)
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
#[doc(hidden)]
pub fn first_page_name(document: &Document, cascade: &CascadeResult) -> Option<String> {
    first_page_name_with_transposed_axes(document, cascade, false)
}

pub(crate) fn first_page_name_with_transposed_axes(
    document: &Document,
    cascade: &CascadeResult,
    pagination_on_transposed_axes: bool,
) -> Option<String> {
    if !has_named_pages(cascade) {
        return None;
    }
    let body_id = find_body(document)?;
    let resolved_layout = document_has_resolved_pagination_order(document, cascade);
    let boxes = if resolved_layout {
        BoxTree::for_layout(document)
    } else {
        BoxTree::for_cascade(document, cascade)
    };
    if let Some(raikiri_style::property::PageValue::Named(name)) = cascade.page_values.get(body_id)
    {
        return page_name_to_owned(Some(name.0.as_str()));
    }
    // In-flow generated content before the first box takes the body's page,
    // like text, and so does not leave the first box to name the first page.
    if crate::generated_content::is_page_flow_generated_text(
        cascade,
        body_id,
        raikiri_style::PseudoElem::Before,
    ) {
        return None;
    }
    let child_order = if resolved_layout {
        pagination_child_order_borrowed(document, cascade, body_id)
    } else {
        initial_page_child_order(document, cascade, body_id)
    };
    for &child_id in child_order.iter() {
        let node = document.get_node(child_id)?;
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || (node.kind() == NodeKind::Element
                && boxes.identity(child_id).display() == DisplayValue::None)
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
                // An absolutely positioned box left at its static position
                // keeps the page names of its in-flow descendants, so a named
                // first descendant selects the initial page as it does in the
                // page pipeline. Its own `page` value is ignored.
                if static_absolute_with_boxes(cascade, child_id, boxes) {
                    let child_order = if resolved_layout {
                        pagination_child_order_borrowed(document, cascade, child_id)
                    } else {
                        initial_page_child_order(document, cascade, child_id)
                    };
                    let first_child = child_order.iter().copied().find(|&grandchild| {
                        !matches!(
                            boxes.identity(grandchild).position(),
                            PositionValue::Absolute | PositionValue::Fixed
                        ) && !floating_box_with_boxes(cascade, grandchild, boxes)
                    });
                    if let Some(first_child) = first_child
                        && let (_, Some(name)) = propagated_start_page_name_with_order(
                            document,
                            cascade,
                            first_child,
                            None,
                            resolved_layout,
                            pagination_on_transposed_axes,
                        )
                    {
                        return page_name_to_owned(Some(name));
                    }
                    continue;
                }
                if !matches!(
                    boxes.identity(child_id).position(),
                    raikiri_style::property::PositionValue::Static
                        | raikiri_style::property::PositionValue::Relative
                        | raikiri_style::property::PositionValue::Sticky
                ) || floating_box_with_boxes(cascade, child_id, boxes)
                {
                    continue;
                }
                if page_applies_with_boxes(boxes, child_id)
                    && let Some(raikiri_style::property::PageValue::Named(name)) =
                        cascade.page_values.get(child_id)
                {
                    return page_name_to_owned(Some(name.0.as_str()));
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
                    pagination_on_transposed_axes,
                );
                return page_name_to_owned(propagated);
            }
            _ => {}
        }
    }
    None
}

/// Cascaded state selected for the first page after verifying resolved Grid placement.
///
/// `page_name` is the query name used to build `cascade`; `page_box` is the
/// matching layout input. Callers rebuild these values
/// in `recascade` when the resolved first in-flow box selects a different page.
#[doc(hidden)]
pub struct InitialPageContext {
    /// Named page used by the current first-page cascade, if any.
    pub page_name: Option<String>,
    /// Cascade for the current first-page query.
    pub cascade: CascadeResult,
    /// Physical page dimensions derived from that cascade or caller defaults.
    pub page_box: PageBox,
    /// Paragraphs the last placement probe built, if one ran.
    paragraphs: Option<crate::layout::ifc::root::RetainedParagraphs>,
}

impl InitialPageContext {
    /// Give `document` the paragraphs the placement probe built, so that its
    /// next layout breaks their lines instead of shaping them again.
    /// `document` must be the document the context was resolved for; the
    /// paragraphs are used only with an element cascade from the same run.
    #[doc(hidden)]
    pub fn give_paragraphs_to(&mut self, document: &mut Document) {
        if let (Some(paragraphs), Some(state)) = (self.paragraphs.take(), document.ifc.as_mut()) {
            state.retained = Some(paragraphs);
        }
    }
}

/// Failure while resolving the page context selected by the first placed Grid item.
#[derive(Debug)]
#[doc(hidden)]
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
#[doc(hidden)]
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
/// current page query. `recascade` receives the new page name and the current
/// cascade, and returns the `@page` cascade and page box for that name; the
/// element cascade does not depend on the page query, so only the page context
/// is replaced with [`CascadeResult::replace_page`]. Every probe lays out a
/// clone of `document`, with its fonts, and uses the caller's replaced-resource
/// resolver and base URL so intrinsic sizes match the final layout. The
/// operation is bounded so self-referential page-size changes fail
/// explicitly. Documents without both features return their input state
/// without a probe.
#[doc(hidden)]
pub fn resolve_initial_page_context(
    document: &Document,
    page_name: Option<String>,
    cascade: CascadeResult,
    page_box: PageBox,
    resources: InitialPageProbeResources<'_>,
    mut recascade: impl FnMut(
        Option<&str>,
        &CascadeResult,
    ) -> (raikiri_style::PageCascadeResult, PageBox),
) -> Result<InitialPageContext, InitialPageContextError> {
    super::validate_layout_depth(document).map_err(InitialPageContextError::Layout)?;
    let mut context = InitialPageContext {
        page_name,
        cascade,
        page_box,
        paragraphs: None,
    };
    let has_named_page = context
        .cascade
        .page_values
        .iter()
        .any(|page_value| matches!(page_value, raikiri_style::property::PageValue::Named(_)));
    let has_flex_or_grid = context
        .cascade
        .computed
        .iter()
        .any(|computed| computed.display.is_flex_or_grid_container());
    if !has_named_page || !has_flex_or_grid {
        return Ok(context);
    }

    const MAX_INITIAL_PAGE_CONTEXT_PASSES: u32 = 3;
    for _ in 0..MAX_INITIAL_PAGE_CONTEXT_PASSES {
        let mut probe_document = document.clone();
        // A page context changes no paragraph: every probe, and the caller's
        // first layout, breaks the lines of the first probe's paragraphs.
        probe_document.set_ifc_retain_paragraphs(true);
        context.give_paragraphs_to(&mut probe_document);
        let probe_layout = if let Some(resolver) = resources.resolver {
            layout_single_page_with_resolver_and_base_url(
                &mut probe_document,
                &context.cascade,
                context.page_box,
                resolver,
                resources.base_url,
            )
        } else {
            layout_single_page(&mut probe_document, &context.cascade, context.page_box)
        };
        probe_layout.map_err(InitialPageContextError::Layout)?;
        context.paragraphs = probe_document
            .ifc
            .as_mut()
            .and_then(|state| state.retained.take());
        let resolved_name = first_page_name(&probe_document, &context.cascade);
        if resolved_name == context.page_name {
            return Ok(context);
        }

        context.page_name = resolved_name;
        let (page, page_box) = recascade(context.page_name.as_deref(), &context.cascade);
        context.cascade.replace_page(page);
        context.page_box = page_box;
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
        let parent = &doc.nodes[parent_idx];
        // The items of a flex or grid container include the children of its
        // `display: contents` children (see `TaffyChildIter`), so `order`
        // sorts that flattened sequence. Taffy takes no `order` input, so the
        // sorted items replace the DOM children here.
        let mut children = Vec::new();
        if parent.formatting_context.is_flex_or_grid() {
            let mut pending: Vec<usize> = parent.children.iter().rev().copied().collect();
            while let Some(child) = pending.pop() {
                let node = &doc.nodes[child];
                if node.kind() == NodeKind::Element
                    && node.display() == DisplayValue::Contents
                    && !node.is_ifc_root()
                {
                    pending.extend(node.children.iter().rev().copied());
                } else {
                    children.push(child);
                }
            }
        }
        let should_sort = children.iter().any(|&child| {
            doc.nodes[child].layout_position() != Position::Absolute && doc.nodes[child].order != 0
        });
        let new_projection = if should_sort {
            let mut ordered_items: Vec<usize> = children
                .iter()
                .copied()
                .filter(|&child| doc.nodes[child].layout_position() != Position::Absolute)
                .collect();
            // `sort_by_key` is stable, so equal `order` values retain DOM order.
            ordered_items.sort_by_key(|&child| doc.nodes[child].order);
            let mut ordered_items = ordered_items.into_iter();
            let mut ordered_children = children;
            for child in &mut ordered_children {
                if doc.nodes[*child].layout_position() != Position::Absolute {
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
        for node in doc.nodes.layout_nodes_mut() {
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

/// Whether `child` is a text node after the first of the run of text that
/// makes one anonymous flex or grid item: the paragraph of that first node
/// lays it out, so it is not an item of its own.
fn is_later_text_of_item(child: &crate::Node) -> bool {
    child.kind() == NodeKind::Text && child.in_ifc_subtree()
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
        && !child.is_display_none()
        && !is_later_text_of_item(child)
        && child.layout_position() != Position::Absolute
        && !matches!(
            &child.data,
            NodeData::Text(text)
                if text.text_content.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}'))
        )
        && document.nodes[parent_id].formatting_context == FormattingContext::Flex
}

/// Return effective flex items in CSS order-modified order, reversed for
/// top-to-bottom visual traversal of `column-reverse`.
fn flex_pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
    boxes: BoxTree<'_>,
) -> Vec<usize> {
    // Contents wrappers do not delimit order-modified flex item sequences.
    // Flatten the complete effective sequence before stable sorting so items
    // in different wrappers can interleave without changing DOM box layout.
    let mut pending: Vec<_> = document.nodes[parent_id]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    let mut children = Vec::new();
    while let Some(child) = pending.pop() {
        if boxes.identity(child).display() == DisplayValue::Contents {
            pending.extend(document.nodes[child].children.iter().rev().copied());
        } else {
            children.push(child);
        }
    }
    let is_in_flow = |child_id: usize| {
        let child = &document.nodes[child_id];
        child.is_in_document()
            && !is_later_text_of_item(child)
            && boxes.identity(child_id).display() != DisplayValue::None
            && !matches!(
                boxes.identity(child_id).position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
            && !matches!(&child.data, NodeData::Text(text) if text.text_content.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{000c}')))
    };
    let mut ordered_items: Vec<_> = children
        .iter()
        .copied()
        .filter(|&id| is_in_flow(id))
        .collect();
    ordered_items.sort_by_key(|&id| cascade.computed[id].order);
    if matches!(
        cascade.computed[parent_id].flex_direction,
        FlexDirectionValue::ColumnReverse
    ) {
        ordered_items.reverse();
    }

    let mut ordered_items = ordered_items.into_iter();
    for child_id in &mut children {
        if is_in_flow(*child_id) {
            *child_id = ordered_items
                .next()
                .expect("each in-flow flex child has one ordered entry");
        }
    }
    children
}

/// Whether a float declaration creates a floating box in pagination.
/// Ordinary floats on flex items remain in normal flow. Footnotes retain
/// their existing page-local placement; Contents elements generate no box.
pub(crate) fn is_floating_box_for_pagination(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    floating_box_with_boxes(cascade, node_id, BoxTree::for_layout(document))
}

fn floating_box_with_boxes(cascade: &CascadeResult, node_id: usize, boxes: BoxTree<'_>) -> bool {
    let computed = &cascade.computed[node_id];
    // Layout floats only the physical sides; a logical `inline-start` or
    // `inline-end` box stays in normal flow there.
    matches!(
        computed.float,
        FloatValue::Left | FloatValue::Right | FloatValue::Footnote
    ) && !matches!(
        boxes.identity(node_id).display(),
        DisplayValue::None | DisplayValue::Contents
    ) && (matches!(computed.float, FloatValue::Footnote)
        || !boxes
            .parent(node_id, BoxParentScope::Document)
            .is_some_and(|parent| {
                // Flex and grid items are not floated (CSS Flexbox 1 §3, CSS
                // Grid 1 §6.1).
                boxes.identity(parent).display().is_flex_or_grid_container()
            }))
}

/// Last in-flow effective item in the flex container's visual page order.
pub(crate) fn trailing_flex_child_for_pagination(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Option<usize> {
    // The effective flex sequence already flattens all Contents wrappers.
    pagination_child_order(document, cascade, parent_id)
        .into_iter()
        .rev()
        .find(|&child| is_in_flow_flex_child_for_pagination(document, parent_id, child))
}

fn is_in_flow_grid_item_for_pagination(document: &Document, child_id: usize) -> bool {
    match document.nodes[child_id].kind() {
        NodeKind::Element => {
            document.nodes[child_id].display() != DisplayValue::None
                && matches!(
                    document.nodes[child_id].position(),
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

/// A grid container's order-modified children with each `display: contents`
/// child that is not a paragraph root replaced by its own children,
/// recursively: those children are the grid items Taffy places (see
/// `TaffyChildIter`), so their resolved rows and break candidates belong to
/// the grid rather than to the wrapper.
fn grid_pagination_children(document: &Document, parent_id: usize) -> Vec<usize> {
    let mut pending: Vec<_> = document.nodes[parent_id]
        .layout_children()
        .iter()
        .rev()
        .copied()
        .collect();
    let mut children = Vec::new();
    while let Some(child) = pending.pop() {
        let node = &document.nodes[child];
        if node.kind() == NodeKind::Element
            && node.display() == DisplayValue::Contents
            && !node.is_ifc_root()
        {
            pending.extend(node.layout_children().iter().rev().copied());
        } else {
            children.push(child);
        }
    }
    children
}

/// Child order shared by page-candidate collection and named-page propagation,
/// once layout has classified each box's formatting context (before that,
/// [`initial_page_child_order`] reads the cascade instead).
pub(crate) fn pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Vec<usize> {
    pagination_child_order_borrowed(document, cascade, parent_id).into_owned()
}

fn pagination_child_order_borrowed<'doc>(
    document: &'doc Document,
    cascade: &CascadeResult,
    parent_id: usize,
) -> Cow<'doc, [usize]> {
    match document.nodes[parent_id].formatting_context {
        FormattingContext::Flex => {
            return Cow::Owned(flex_pagination_child_order(
                document,
                cascade,
                parent_id,
                BoxTree::for_layout(document),
            ));
        }
        FormattingContext::Table => {
            return Cow::Owned(table_pagination_child_order(
                document,
                cascade,
                parent_id,
                BoxTree::for_layout(document),
            ));
        }
        // Multi-column pagination is not implemented, but named-page
        // propagation and candidate traversal must retain Grid's
        // order-modified child sequence.
        FormattingContext::Grid if document.nodes[parent_id].grid_column_count != 1 => {
            return Cow::Owned(grid_pagination_children(document, parent_id));
        }
        FormattingContext::Grid => {}
        _ => return Cow::Borrowed(&document.nodes[parent_id].children),
    }

    let mut children = grid_pagination_children(document, parent_id);
    let mut in_flow_items: Vec<_> = document.nodes[parent_id]
        .grid_item_row_starts
        .iter()
        .copied()
        .filter(|(child_id, _)| is_in_flow_grid_item_for_pagination(document, *child_id))
        .collect();
    let pageable_count = children
        .iter()
        .filter(|&&child_id| is_in_flow_grid_item_for_pagination(document, child_id))
        .count();
    if in_flow_items.len() == pageable_count {
        // Stable sorting keeps order-modified order within a resolved row.
        in_flow_items.sort_by_key(|&(_, row_start)| row_start);
        let mut ordered_items = in_flow_items.into_iter().map(|(child_id, _)| child_id);
        for child_id in &mut children {
            if is_in_flow_grid_item_for_pagination(document, *child_id) {
                *child_id = ordered_items
                    .next()
                    .expect("Taffy row details cover each in-flow grid item");
            }
        }
    }
    Cow::Owned(children)
}

/// A table's children in the order the table lays them out: a caption above
/// the grid, the first header group, the other groups and rows in source
/// order, the first footer group, then a caption below the grid (CSS 2.1
/// §17.4 and §17.5). Other captions, columns and column groups are left out.
fn table_pagination_child_order(
    document: &Document,
    cascade: &CascadeResult,
    table: usize,
    boxes: BoxTree<'_>,
) -> Vec<usize> {
    // Table layout lays out the children of `display: contents` wrappers as
    // the table's own children before it picks the header and footer.
    let mut pending: Vec<_> = document.nodes[table]
        .children
        .iter()
        .rev()
        .copied()
        .collect();
    let mut children = Vec::new();
    while let Some(child) = pending.pop() {
        let node = &document.nodes[child];
        // Table layout drops these without a box, as it does for any
        // non-rendered element whatever its `display`.
        if !node.is_in_document()
            || node.is_non_rendered_html_element()
            || boxes.identity(child).display() == DisplayValue::None
            || !matches!(node.kind(), NodeKind::Element | NodeKind::Text)
        {
            continue;
        }
        if boxes.identity(child).display() == DisplayValue::Contents {
            pending.extend(node.children.iter().rev().copied());
        } else {
            children.push(child);
        }
    }
    let first = |display: DisplayValue| {
        children
            .iter()
            .copied()
            .find(|&id| boxes.identity(id).display() == display)
    };
    let header = first(DisplayValue::TableHeaderGroup);
    let footer = first(DisplayValue::TableFooterGroup);
    // Table layout renders only the first caption among the table's own
    // children, those of `display: contents` wrappers included, so other
    // captions have no box to paginate or to take a page name from.
    let caption = children
        .iter()
        .copied()
        .find(|&id| boxes.identity(id).display() == DisplayValue::TableCaption);
    // Columns and column groups only size the grid and generate no box.
    children.retain(|&id| match boxes.identity(id).display() {
        DisplayValue::TableCaption => Some(id) == caption,
        DisplayValue::TableColumn | DisplayValue::TableColumnGroup => false,
        _ => true,
    });
    children.sort_by_key(|&id| {
        let computed = &cascade.computed[id];
        if boxes.identity(id).display() == DisplayValue::TableCaption {
            if computed.caption_side == raikiri_style::property::CaptionSideValue::Top {
                0u8
            } else {
                4
            }
        } else if Some(id) == header {
            1
        } else if Some(id) == footer {
            3
        } else {
            2
        }
    });
    children
}

/// Whether the `page` property applies to the box of `node_id`.
///
/// CSS Page 3 §4.1 applies `page` to boxes that create class A break points,
/// which are block-level. An inline-level box takes the page type of its
/// containing block instead. A flex or grid item and an absolutely
/// positioned box are blockified whatever their computed `display`.
pub(crate) fn page_applies_to_box(document: &Document, node_id: usize) -> bool {
    page_applies_with_boxes(BoxTree::for_layout(document), node_id)
}

fn page_applies_with_boxes(boxes: BoxTree<'_>, node_id: usize) -> bool {
    let blockified = boxes
        .parent(node_id, BoxParentScope::Document)
        .is_some_and(|parent| {
            boxes
                .display(parent)
                .is_some_and(DisplayValue::is_flex_or_grid_container)
        });
    blockified
        || matches!(
            boxes.identity(node_id).position(),
            PositionValue::Absolute | PositionValue::Fixed
        )
        || !boxes.identity(node_id).display().is_inline_level()
}

/// Whether `node_id` is an absolutely positioned box whose block-start and
/// block-end insets are both `auto`, which leaves it at its static position
/// in the page flow.
pub(crate) fn is_static_position_absolute(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    static_absolute_with_boxes(cascade, node_id, BoxTree::for_layout(document))
}

/// Insets come straight from the computed values: nothing overrides them, and
/// the cascade handed to a transposed pagination carries them on its axes, so
/// `top` and `bottom` are the sides the flow starts and ends at whichever way
/// the document is paginated.
fn static_absolute_with_boxes(cascade: &CascadeResult, node_id: usize, boxes: BoxTree<'_>) -> bool {
    let computed = &cascade.computed[node_id];
    matches!(boxes.identity(node_id).position(), PositionValue::Absolute)
        && inset_is_auto(&computed.top)
        && inset_is_auto(&computed.bottom)
}

/// Whether an inset is `auto` once layout has read it: only lengths,
/// percentages and `calc()` constrain a box. This is the mapping of
/// `LayoutStyleView::inset`; the sizing keywords cannot be authored for an
/// inset and read as `auto` there too.
pub(crate) fn inset_is_auto(inset: &ComputedLengthPercentageOrAuto) -> bool {
    !matches!(
        inset,
        ComputedLengthPercentageOrAuto::Px(_)
            | ComputedLengthPercentageOrAuto::Percent(_)
            | ComputedLengthPercentageOrAuto::Calc(_)
    )
}

/// Whether pagination treats the box of `node_id` as monolithic: a replaced
/// element, an atomic inline-level box or an orthogonal flow, and, while a
/// vertical document is paginated on transposed axes, a box laid out on
/// horizontal axes. Its content offers no class A break points, and its
/// descendants' page names neither open pages nor propagate to the box.
pub(crate) fn is_monolithic_for_pagination_with_transposed_axes(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    pagination_on_transposed_axes: bool,
) -> bool {
    monolithic_with_boxes(
        document,
        cascade,
        node_id,
        BoxTree::for_layout(document),
        pagination_on_transposed_axes,
    )
}

/// Whether `node_id` is a replaced element or an inline SVG root: content that
/// no page break goes through.
pub(crate) fn is_replaced_for_pagination(document: &Document, node_id: usize) -> bool {
    use crate::layout::ifc::projection::{ATOMIC_TAGS, REPLACED_BOX_TAGS};
    let node = &document.nodes[node_id];
    node.flags
        .contains(crate::node::NodeFlags::IS_INLINE_SVG_ROOT)
        || node.tag_name().is_some_and(|tag| {
            // Programmatically created DOM nodes can retain mixed-case tags.
            ATOMIC_TAGS
                .iter()
                .chain(REPLACED_BOX_TAGS)
                .any(|replaced_tag| tag.eq_ignore_ascii_case(replaced_tag))
        })
}

fn monolithic_with_boxes(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    boxes: BoxTree<'_>,
    pagination_on_transposed_axes: bool,
) -> bool {
    let node = &document.nodes[node_id];
    is_replaced_for_pagination(document, node_id)
        || (!page_applies_with_boxes(boxes, node_id)
            && boxes.identity(node_id).display().is_atomic_inline_level())
        || orthogonal_flow_with_boxes(document, cascade, node_id, boxes)
        || (pagination_on_transposed_axes
            && node.kind() == NodeKind::Element
            && !crate::layout::bridge::uses_vertical_axes(cascade, node_id))
}

/// Whether `node_id` establishes an orthogonal flow: its writing mode is
/// vertical while its parent's is horizontal, or the reverse. Layout
/// normalizes writing modes to horizontal, so the comparison uses the
/// specified keyword kept for CSSOM. Pagination treats such a box as
/// monolithic. A `display: contents` element generates no box, so it never
/// establishes a flow, and the comparison uses the box parent.
pub(crate) fn establishes_orthogonal_flow(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
) -> bool {
    orthogonal_flow_with_boxes(document, cascade, node_id, BoxTree::for_layout(document))
}

fn orthogonal_flow_with_boxes(
    document: &Document,
    cascade: &CascadeResult,
    node_id: usize,
    boxes: BoxTree<'_>,
) -> bool {
    let vertical = |id: usize| {
        cascade
            .computed
            .get(id)
            .is_some_and(|computed| computed.cssom_writing_mode.is_vertical())
    };
    if boxes
        .display(node_id)
        .is_none_or(|display| display == DisplayValue::Contents)
    {
        return false;
    }
    boxes
        .parent(node_id, BoxParentScope::Document)
        .filter(|&parent| document.nodes[parent].kind() == NodeKind::Element)
        .is_some_and(|parent| vertical(parent) != vertical(node_id))
}

/// Return the start page value propagated from the first in-flow child box.
///
/// CSS Page 3 derives a box's start page value from its first child when
/// that child participates in a class-A break point. Nested named boxes
/// therefore do not each open a page; the deepest first in-flow box owns
/// the value compared at the boundary.
#[cfg(test)]
pub(crate) fn propagated_start_page_name<'name>(
    document: &Document,
    cascade: &'name CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&'name str>,
) -> (bool, Option<&'name str>) {
    propagated_start_page_name_with_transposed_axes(
        document,
        cascade,
        node_id,
        inherited_page_name,
        false,
    )
}

pub(crate) fn propagated_start_page_name_with_transposed_axes<'name>(
    document: &Document,
    cascade: &'name CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&'name str>,
    pagination_on_transposed_axes: bool,
) -> (bool, Option<&'name str>) {
    propagated_start_page_name_with_order(
        document,
        cascade,
        node_id,
        inherited_page_name,
        true,
        pagination_on_transposed_axes,
    )
}

fn propagated_start_page_name_with_order<'name>(
    document: &Document,
    cascade: &'name CascadeResult,
    node_id: usize,
    inherited_page_name: Option<&'name str>,
    resolved_layout: bool,
    pagination_on_transposed_axes: bool,
) -> (bool, Option<&'name str>) {
    let boxes = if resolved_layout {
        BoxTree::for_layout(document)
    } else {
        BoxTree::for_cascade(document, cascade)
    };
    enum Task<'doc, 'name> {
        Visit(usize, Option<&'name str>),
        ContinueChildren {
            children: Cow<'doc, [usize]>,
            next: usize,
            page_name: Option<&'name str>,
        },
    }

    let mut pending = vec![Task::Visit(node_id, inherited_page_name)];
    while let Some(task) = pending.pop() {
        let (node_id, inherited_page_name) = match task {
            Task::Visit(node_id, inherited_page_name) => {
                #[cfg(test)]
                record_page_name_work(0);
                (node_id, inherited_page_name)
            }
            Task::ContinueChildren {
                children,
                next,
                page_name,
            } => {
                let found = children[next..].iter().position(|&child_id| {
                    cascade.computed.get(child_id).is_some_and(|_| {
                        !matches!(
                            boxes.identity(child_id).position(),
                            PositionValue::Absolute | PositionValue::Fixed
                        ) && !floating_box_with_boxes(cascade, child_id, boxes)
                    })
                });
                if let Some(offset) = found {
                    let next_index = next + offset;
                    let child_id = children[next_index];
                    pending.push(Task::ContinueChildren {
                        children,
                        next: next_index + 1,
                        page_name,
                    });
                    pending.push(Task::Visit(child_id, page_name));
                    continue;
                }
                return (true, page_name);
            }
        };
        let Some(node) = document.get_node(node_id) else {
            continue;
        };
        if !node.is_in_document() || boxes.identity(node_id).display() == DisplayValue::None {
            continue;
        }
        if matches!(node.kind(), NodeKind::Text) {
            let has_text = matches!(
                &node.data,
                crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
            );
            if has_text {
                return (true, inherited_page_name);
            }
            continue;
        }

        let page_applies = page_applies_with_boxes(boxes, node_id);
        let explicit_page_name = if page_applies {
            selected_page_name(cascade, node_id)
        } else {
            None
        };
        let used_page_name = explicit_page_name.or(inherited_page_name);
        // Generated content before the children is the first content of the
        // box, and takes its page value.
        if crate::generated_content::is_page_flow_generated_text(
            cascade,
            node_id,
            raikiri_style::PseudoElem::Before,
        ) {
            return (true, used_page_name);
        }
        // A monolithic box does not pass a descendant's page value up.
        if monolithic_with_boxes(
            document,
            cascade,
            node_id,
            boxes,
            pagination_on_transposed_axes,
        ) {
            return (true, used_page_name);
        }
        let child_order = if resolved_layout {
            pagination_child_order_borrowed(document, cascade, node_id)
        } else {
            initial_page_child_order(document, cascade, node_id)
        };
        #[cfg(test)]
        if matches!(child_order, Cow::Owned(_)) {
            record_page_name_work(1);
        }
        let next = child_order.iter().position(|&child_id| {
            cascade.computed.get(child_id).is_some_and(|_| {
                !matches!(
                    boxes.identity(child_id).position(),
                    PositionValue::Absolute | PositionValue::Fixed
                ) && !floating_box_with_boxes(cascade, child_id, boxes)
            })
        });
        if let Some(index) = next {
            let child_id = child_order[index];
            pending.push(Task::ContinueChildren {
                children: child_order,
                next: index + 1,
                page_name: used_page_name,
            });
            pending.push(Task::Visit(child_id, used_page_name));
        } else {
            return (true, used_page_name);
        }
    }
    (false, None)
}

#[cfg(test)]
mod tests;
