//! The visit of a node: where its box lies on the page, what it paints of
//! itself, and the frames that paint its children.

use super::frames::{
    AnonymousParagraphFrame, DeferralLayer, LayerClose, LinesFrame, PaintFrame, VisitFrame,
};
use super::page_geometry::PageGeometry;
use super::paint_walk::PaintWalk;
use super::{
    BackgroundEnv, border_hides_background_edge, box_intersects_page, color_only_background_layer,
    element_transform, first_text_descendant, fixed_background_viewport, fixed_position_px,
    generated_flow_height, generated_pseudo_text_advance, generated_pseudo_text_height,
    ifc_paint_extent, img_src_url, infer_url_color, paint_canvas, paint_element_background,
    paint_element_border_image, paint_element_border_rounded, paint_element_border_with_top,
    paint_element_box_shadows, paint_element_outline, paint_generated_pseudo, paint_image,
    paint_inline_svg, paint_list_marker_with_snapshots, paintable_border_radius,
    push_overflow_clip, snap_column_clip_outward, transform_translation, translation_only,
    used_padding_for_paint, vertical_align_shift_px,
};
use crate::text;
use anyrender::PaintScene;
use kurbo::{Affine, BezPath, Rect};
use peniko::Mix;
use raikiri_dom::FragmentRect;
use raikiri_style::ComputedLengthPercentageOrAuto;
use raikiri_style::property::{
    BackgroundImage, BorderColor, BorderStyle, ColumnCountValue, DisplayValue, PositionValue,
    ZIndexValue,
};
use raikiri_traits::{NodeId, NodeKind, PaintInsets, PaintRect};

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Visit the node of `visit`: paint it and push the frames of its
    /// children, or skip it and its subtree.
    pub(super) fn visit(&mut self, visit: VisitFrame) {
        let (document, cascade, counter_snapshots, page_box, active_page_name) = (
            self.document,
            self.cascade,
            self.counter_snapshots,
            self.page_box,
            self.active_page_name,
        );
        let (image_cache, pixel_source, custom_highlights) =
            (self.image_cache, self.pixel_source, self.custom_highlights);
        let (content_origin_y, root_transform, root_id, canvas_owner, layout_fragments) = (
            self.content_origin_y,
            self.root_transform,
            self.root_id,
            self.canvas_owner,
            self.layout_fragments,
        );
        let geometry = self.geometry;
        let PageGeometry {
            margins,
            insets,
            content_width: _,
            content_height,
            page_area,
            fixed_rtl,
            fixed_content_width,
            pages_follow_x_axis: _,
            page_top,
            page_bottom,
            page_offset_x,
            page_offset_y,
            fixed_flow_shift_x,
            fixed_origin_y,
            page_clip_transform,
            page_content_clip,
            flow_band,
            page_lines,
        } = geometry;
        let named_page_matches = |node_id: usize| {
            raikiri_dom::paint_rules::named_page_matches(
                document,
                cascade,
                node_id,
                active_page_name,
            )
        };
        let text_page_clip =
            |inside_fixed: bool| geometry.text_page_clip(cascade, page_box, inside_fixed);
        let scene = &mut *self.scene;
        let trace = &mut self.trace;
        let deferral_scopes = &mut self.deferral_scopes;
        let stack = &mut self.stack;
        let warnings = &mut *self.warnings;
        let generated_flow_heights = &mut self.generated_flow_heights;
        let fragments_by_parent_and_node = &self.fragments_by_parent_and_node;
        let fragmented_nodes = &self.fragmented_nodes;
        let VisitFrame {
            node_id,
            parent_abs_x,
            mut parent_abs_y,
            parent_font_size,
            shift_y,
            transform_x,
            transform_y,
            paint_offset_y,
            fragment_clip_height,
            column_clip_height,
            fragment_id,
            fragment_origin_x,
            fragment_origin_y,
            fragmentainer,
            fragment_rect,
            fragment_clip,
            is_fragment_visit,
            inside_fixed,
            inside_fixed_containing_block,
            decorations,
        } = visit;
        if let Some(shift) = document.repeated_table_header_root_shift(node_id, content_origin_y) {
            let Some(shift) = shift else {
                return;
            };
            parent_abs_y += shift;
        }
        let Some(node) = document.get_node(node_id) else {
            return;
        };
        if let Some(fragment_ids) = fragments_by_parent_and_node.get(&(fragment_id, node_id)) {
            for &current_fragment_id in fragment_ids.iter().rev() {
                let fragment = layout_fragments[current_fragment_id];
                let (node_abs_x, node_abs_y) = fragment.paint_origin(
                    (parent_abs_x, parent_abs_y),
                    (
                        node.unrounded_layout.location.x,
                        node.unrounded_layout.location.y,
                    ),
                    (fragment_origin_x, fragment_origin_y),
                );
                let fragment_clip = fragment.fragmentainer_clip.map(|clip| FragmentRect {
                    x: clip.x - fragment.rect.x,
                    y: clip.y - fragment.rect.y,
                    width: clip.width,
                    height: clip.height,
                });
                stack.push(PaintFrame::Visit(VisitFrame {
                    node_id,
                    parent_abs_x: node_abs_x - node.unrounded_layout.location.x,
                    parent_abs_y: node_abs_y - node.unrounded_layout.location.y,
                    parent_font_size,
                    shift_y,
                    transform_x,
                    transform_y,
                    paint_offset_y,
                    fragment_clip_height,
                    column_clip_height,
                    fragment_id: Some(current_fragment_id),
                    fragment_origin_x: node_abs_x,
                    fragment_origin_y: node_abs_y,
                    fragmentainer: Some(fragment.fragmentainer),
                    fragment_rect: Some(fragment.rect),
                    fragment_clip,
                    is_fragment_visit: true,
                    inside_fixed,
                    inside_fixed_containing_block,
                    decorations: decorations.clone(),
                }));
            }
            return;
        }
        if fragmented_nodes.contains(&node_id) && !is_fragment_visit {
            return;
        }
        // Skip template descendants and future inert subtrees consistently.
        // Use an explicit gate independent of any UA CSS display:none rule.
        if !node.is_in_document() {
            return;
        }
        let mut node_layout = node.unrounded_layout;
        if let Some(rect) = fragment_rect {
            node_layout.size.width = rect.width;
            node_layout.size.height = rect.height;
        }
        // Do not paint subtrees of HTML hidden elements (metadata, raw text,
        // or fallback parentheses for ruby).
        // Treat `Node::is_non_rendered_html_element` match arms as the
        // single source of truth for the current set of tags.
        // Author / user CSS can override UA CSS `display: none`, so painting
        // fails closed with a cascade-independent defense-in-depth gate
        // (HTML LS §15.3.1 "Hidden elements":
        // https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements).
        // Namespace checks exclude same-named SVG / MathML elements.
        // <template> is doubly gated by is_in_document().
        if node.is_non_rendered_html_element() {
            return;
        }
        match node.kind() {
            NodeKind::Element => {
                // An atomic inline an ellipsis removed from its line is not
                // painted (CSS Overflow 3 §5.1).
                if node.is_display_none() || node.is_hidden_by_text_overflow() {
                    return;
                }
                let layout = node_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                let cv = &cascade.computed[node_id];
                let generates_box =
                    node_id == root_id || raikiri_dom::paint_rules::generates_box(cv);
                let paint_padding = used_padding_for_paint(cv, &layout);
                let has_border = [
                    &cv.border.top,
                    &cv.border.right,
                    &cv.border.bottom,
                    &cv.border.left,
                ]
                .iter()
                .any(|side| side.width().px() > 0.0 && side.style() != BorderStyle::None);
                let paint_border_radius = paintable_border_radius(
                    &cv.border_radius,
                    (has_border || cv.background_color.a > 0 || cv.has_background_image())
                        && !raikiri_dom::has_applied_transform(document, node_id, cv)
                        && cv.filter.is_empty(),
                );
                // A box laid out on the lines of an inline engine paragraph
                // already sits at its `vertical-align` position; shifting its
                // content again would move it twice.
                let placed_by_inline_engine = document
                    .layout_parent_of(node_id)
                    .and_then(|parent| document.get_node(parent))
                    .is_some_and(|parent| parent.is_ifc_root())
                    && !node.in_ifc_subtree();
                // A flex or grid item is blockified (CSS Display 3 section
                // 2.7), and `vertical-align` does not apply to it even though
                // the cascade keeps its `inline` display.
                let blockified_item = document.layout_parent_of(node_id).is_some_and(|parent| {
                    cascade.computed[parent].display.is_flex_or_grid_container()
                });
                let own_shift = if node_id == root_id || placed_by_inline_engine || blockified_item
                {
                    0.0
                } else {
                    vertical_align_shift_px(cv.vertical_align, cv.display, parent_font_size)
                };
                let child_shift_y = shift_y + own_shift;
                // Taffy and the IFC block/atomic placement paths already apply
                // relative insets. Floats, explicit multicol fragments and
                // manually placed table parts still use paint-side insets.
                let needs_paint_inset = raikiri_dom::placed_without_relative_offset(
                    document,
                    cascade,
                    Some(root_id),
                    node_id,
                    cv,
                ) || (is_fragment_visit && !cv.display.is_inline_level());
                let (pos_dx, pos_dy) = if needs_paint_inset {
                    raikiri_dom::used_relative_offset(document, cascade, cv, node_id)
                } else if node.needs_relative_block_paint_offset() {
                    // The foundational column post-pass replaces only Y;
                    // the existing X location already includes its inset.
                    (
                        0.0,
                        raikiri_dom::used_relative_offset(document, cascade, cv, node_id).1,
                    )
                } else {
                    (0.0, 0.0)
                };
                // The caller already applies the root matrix to the whole document.
                let transformable =
                    node_id != root_id && raikiri_dom::is_transformable(document, node_id, cv);
                let (own_transform_x, own_transform_y) = if transformable {
                    transform_translation(cv, layout.size.width, layout.size.height)
                } else {
                    (0.0, 0.0)
                };
                let child_transform_x = transform_x + own_transform_x;
                let child_transform_y = transform_y + own_transform_y;
                let is_fixed = matches!(cv.position, PositionValue::Fixed);
                // A fixed box below a containing block for fixed boxes keeps
                // its local geometry, as layout places it; only viewport-fixed
                // boxes repeat per page.
                let fixed_in_viewport = is_fixed && !inside_fixed_containing_block;
                let establishes_fixed_containing_block =
                    raikiri_dom::establishes_fixed_containing_block(document, node_id, cv);
                let (fixed_dx, fixed_dy) = if fixed_in_viewport {
                    fixed_position_px(
                        cv,
                        &layout,
                        (abs_x, abs_y),
                        fixed_rtl,
                        page_box.width,
                        fixed_origin_y,
                        fixed_content_width,
                        content_height,
                    )
                    .map(|(x, y)| (x - abs_x - fixed_flow_shift_x, y - abs_y))
                    .unwrap_or((0.0, 0.0))
                } else {
                    (0.0, 0.0)
                };
                let paint_x = abs_x + page_offset_x + pos_dx + fixed_dx + child_transform_x;
                let mut paint_y =
                    abs_y + page_offset_y + child_shift_y + pos_dy + fixed_dy + child_transform_y;
                if transformable && !translation_only(cv) {
                    let matrix = element_transform(
                        cv,
                        paint_x,
                        paint_y,
                        layout.size.width,
                        layout.size.height,
                    );
                    // spec: https://www.w3.org/TR/css-transforms-1/#transform-function-lists
                    // A singular matrix hides the element and its subtree.
                    if matrix.determinant() == 0.0
                        || !matrix.as_coeffs().iter().all(|value| value.is_finite())
                    {
                        return;
                    }
                    stack.push(PaintFrame::RestoreTransform(scene.transform));
                    scene.transform *= matrix;
                }
                let mut paint_height = layout.size.height;
                let mut paint_background_height = layout.size.height;
                let mut multicol_clip_pushed = false;
                let mut fragmentainer_clip_pushed = false;
                if let Some(clip_height) =
                    fragment_clip_height.filter(|height| height.is_finite() && *height >= 0.0)
                {
                    paint_y = paint_y.floor();
                    paint_height = paint_height.min(clip_height);
                    paint_background_height = paint_background_height.min(clip_height);
                    let clip_origin_y = paint_y.floor();
                    let clip = snap_column_clip_outward(
                        Rect::new(
                            paint_x as f64,
                            clip_origin_y as f64,
                            (paint_x + layout.size.width) as f64,
                            (clip_origin_y + clip_height) as f64,
                        ),
                        image_cache.device_grid(scene.transform),
                    );
                    scene.push_clip_layer(Affine::IDENTITY, &clip);
                    multicol_clip_pushed = true;
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushFragmentainerClip(node_id));
                    }
                }
                if let Some(clip) = fragment_clip {
                    let clip = snap_column_clip_outward(
                        Rect::new(
                            (paint_x + clip.x) as f64,
                            (paint_y + clip.y) as f64,
                            (paint_x + clip.x + clip.width) as f64,
                            (paint_y + clip.y + clip.height) as f64,
                        ),
                        image_cache.device_grid(scene.transform),
                    );
                    scene.push_clip_layer(Affine::IDENTITY, &clip);
                    fragmentainer_clip_pushed = true;
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushFragmentainerClip(node_id));
                    }
                }
                // A collapsed-border table cell paints the borders that won
                // conflict resolution with its rows, neighbours and table.
                let painted_border = document
                    .get_node(node_id)
                    .and_then(|node| node.collapsed_border())
                    .unwrap_or(&cv.border);
                // A hidden table must not paint its own collapsed border.
                let visibility_hidden_table =
                    raikiri_dom::paint_rules::is_visibility_hidden_table(cv);
                let mut paints_on_page = !visibility_hidden_table
                    && (node_id == root_id
                        || ((fixed_in_viewport || named_page_matches(node_id))
                            && (fixed_in_viewport
                                || inside_fixed
                                || (box_intersects_page(
                                    abs_y,
                                    layout.size.height,
                                    page_top,
                                    page_bottom,
                                ) && flow_band.is_none_or(|band| {
                                    // A table paints its own box around its
                                    // grid, without its captions.
                                    let own_box = node.table_grid_box().map_or(
                                        (paint_x, paint_y, layout.size.width, layout.size.height),
                                        |grid| {
                                            (
                                                paint_x + grid.x,
                                                paint_y + grid.y,
                                                grid.width,
                                                grid.height,
                                            )
                                        },
                                    );
                                    band.holds_box(scene.transform, own_box)
                                })))));
                // An absolutely positioned containing block can own a child
                // fragment on a later page even when its first layout box ends
                // before that page.  Preserve the parent's border/background
                // continuation for this narrow OOF shape.  Full OOF
                // fragmentation remains outside this painter; the gate avoids
                // changing ordinary flow, fixed-position, transformed, or
                // borderless absolute boxes.
                let is_absolute_fragment_candidate = matches!(cv.position, PositionValue::Absolute)
                    && cv.transform.is_empty()
                    && (cv.border.top.width().px() > 0.0
                        || cv.border.right.width().px() > 0.0
                        || cv.border.bottom.width().px() > 0.0
                        || cv.border.left.width().px() > 0.0)
                    && cv.background_color.a != 0;
                let descendant_end_on_page = if is_absolute_fragment_candidate {
                    node.children
                        .iter()
                        .filter_map(|child_id| {
                            let child = document.get_node(*child_id)?;
                            let child_cv = cascade.computed.get(*child_id)?;
                            if !matches!(
                                child_cv.position,
                                PositionValue::Static
                                    | PositionValue::Relative
                                    | PositionValue::Sticky
                            ) {
                                return None;
                            }
                            let child_y = abs_y + child.unrounded_layout.location.y;
                            box_intersects_page(
                                child_y,
                                child.unrounded_layout.size.height,
                                page_top,
                                page_bottom,
                            )
                            .then_some(child_y + child.unrounded_layout.size.height)
                        })
                        .fold(None, |maximum, value| {
                            Some(maximum.map_or(value, |current: f32| current.max(value)))
                        })
                } else {
                    None
                };
                let mut paints_as_absolute_continuation = false;
                // Where the box's border box starts on this page's coordinates
                // before it is cut into a continuation fragment.
                let mut unfragmented_top = paint_y;
                // Continuations cut boxes along y; a page that follows the x
                // axis holds only the boxes across its page area.
                if !paints_on_page
                    && flow_band.is_none()
                    && is_absolute_fragment_candidate
                    && let Some(descendant_end) = descendant_end_on_page
                {
                    let fragment_top = page_top - margins.top - insets.top;
                    let continuation_height =
                        (descendant_end - fragment_top + cv.border.bottom.width().px()).max(0.0);
                    if continuation_height > 0.0 {
                        unfragmented_top = paint_y;
                        paint_y = fragment_top + margins.top + insets.top - page_top
                            + pos_dy
                            + child_transform_y;
                        paint_height = continuation_height;
                        // The local reftest oracle carries only the
                        // continuation border; the ancestor background is
                        // not repeated behind the child fragment.
                        paint_background_height = 0.0;
                        paints_on_page = named_page_matches(node_id);
                        paints_as_absolute_continuation = paints_on_page;
                    }
                }
                // The first fragment of the same containing block extends to
                // the page boundary when a direct in-flow child starts on the
                // next page.  Its normal layout height only covers the first
                // one-pass OOF content box, so preserve the border/background
                // through the break without changing descendant coordinates.
                if paints_on_page
                    && !paints_as_absolute_continuation
                    && is_absolute_fragment_candidate
                    && let Some(descendant_end) = node
                        .children
                        .iter()
                        .filter_map(|child_id| {
                            let child = document.get_node(*child_id)?;
                            let child_cv = cascade.computed.get(*child_id)?;
                            if !matches!(
                                child_cv.position,
                                PositionValue::Static
                                    | PositionValue::Relative
                                    | PositionValue::Sticky
                            ) {
                                return None;
                            }
                            let child_y = abs_y + child.unrounded_layout.location.y;
                            (child_y >= page_bottom)
                                .then_some(child_y + child.unrounded_layout.size.height)
                        })
                        .fold(None, |maximum, value| {
                            Some(maximum.map_or(value, |current: f32| current.max(value)))
                        })
                {
                    paint_height = paint_height
                        .max((descendant_end - abs_y + cv.border.bottom.width().px()).max(0.0));
                    // Keep the element background inside the current page's
                    // content extent.  The border continuation is painted
                    // separately and may reach the page edge.
                    paint_background_height = (page_bottom - abs_y).max(0.0);
                }

                // The pseudo is not an arena child yet, so Taffy cannot
                // include its line box in an auto-height originating box.
                // Expand only the auto-height paint geometry; fixed-height
                // elements retain their normal overflow behavior. This is a
                // paint-side bridge: the arena/Taffy flow remains unchanged
                // until generated inline boxes can participate in layout.
                if !paints_as_absolute_continuation
                    && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
                {
                    let pseudo_height = generated_flow_height(
                        document,
                        cascade,
                        node_id,
                        counter_snapshots,
                        generated_flow_heights,
                    );
                    let pseudo_border_height = cv.border.top.width().px()
                        + paint_padding.top
                        + pseudo_height
                        + paint_padding.bottom
                        + cv.border.bottom.width().px();
                    if pseudo_border_height > paint_height {
                        let was_intrinsic_height =
                            (paint_background_height - layout.size.height).abs() <= f32::EPSILON;
                        paint_height = pseudo_border_height;
                        if was_intrinsic_height {
                            paint_background_height = paint_height;
                        }
                    }
                }
                let generated_x = paint_x + cv.border.left.width().px() + paint_padding.left;
                let generated_y = paint_y + cv.border.top.width().px() + paint_padding.top;
                // Direct text children are the only inline descendants whose
                // current minimal layout path has a reliable horizontal box.
                // Use their right edge for `::after`; element children stay
                // on the existing block-flow path until a full IFC lands.
                let generated_content_advance = node
                    .children
                    .iter()
                    .filter_map(|child_id| document.get_node(*child_id))
                    .filter(|child| child.kind() == NodeKind::Text)
                    .map(|child| {
                        (child.unrounded_layout.location.x + child.unrounded_layout.size.width
                            - cv.border.left.width().px()
                            - paint_padding.left)
                            .max(0.0)
                    })
                    .fold(0.0, f32::max);
                // CSS Color 4 §3.3: opacity applies to the complete
                // element group, including its background, border, text,
                // generated content, and descendants. Keep this layer open
                // until the deferred `PaintAfter` and overflow clip frames
                // have finished so overlapping descendants are composited as
                // one group instead of being alpha-blended individually.
                let has_opacity_layer = raikiri_dom::paint_rules::opacity_layer(cv).is_some();
                if has_opacity_layer {
                    let opacity_clip = Rect::new(
                        0.0,
                        0.0,
                        page_box.width.max(0.0) as f64,
                        page_box.height.max(0.0) as f64,
                    );
                    scene.scene.push_layer(
                        Mix::Normal,
                        cv.opacity,
                        Affine::IDENTITY,
                        &opacity_clip,
                        None,
                        None,
                    );
                    // This frame is pushed first so it closes after the
                    // optional overflow clip and generated `::after` paint.
                    stack.push(PaintFrame::PopOpacity);
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushOpacity(node_id, cv.opacity));
                    }
                }
                let mut before_advance = 0.0;
                let has_column_rules = document.column_rules(cascade, node_id).next().is_some();
                if paints_on_page
                    && !raikiri_dom::paint_rules::hides_empty_table_cell(document, cascade, node_id)
                {
                    if let Some(t) = trace.as_deref_mut()
                        && generates_box
                        && !raikiri_dom::paint_rules::hides_own_box(cv)
                    {
                        t.push(crate::PaintTraceEvent::Box(node_id));
                    }
                    let grid = node.table_grid_box();
                    let own_paint_x = paint_x + grid.map_or(0.0, |rect| rect.x);
                    let own_paint_width = grid.map_or(layout.size.width, |rect| rect.width);
                    let own_paint_y = paint_y + grid.map_or(0.0, |rect| rect.y);
                    let own_paint_height = grid.map_or(paint_height, |rect| rect.height);
                    let own_background_height =
                        grid.map_or(paint_background_height, |rect| rect.height);
                    // The box's own geometry, which its outline follows.
                    let (fragment_paint_y, fragment_paint_height) = (own_paint_y, own_paint_height);
                    // A box that is one slice of a box broken across outer
                    // columns paints the whole box's decorations, cut at the
                    // breaks (CSS Fragmentation 3 §5.4).
                    let block_slice = (cv.box_decoration_break
                        == raikiri_style::property::BoxDecorationBreak::Slice)
                        .then(|| document.fragment_block_slice(node_id, fragment_id))
                        .flatten();
                    let (own_paint_y, own_paint_height, own_background_height) = match block_slice {
                        Some((start, total)) => {
                            const UNBOUNDED: f64 = 1.0e7;
                            let top = if start > 0.0 {
                                f64::from(own_paint_y)
                            } else {
                                -UNBOUNDED
                            };
                            let bottom = if start + own_paint_height < total - 0.01 {
                                f64::from(own_paint_y + own_paint_height)
                            } else {
                                UNBOUNDED
                            };
                            scene.push_clip_layer(
                                Affine::IDENTITY,
                                &Rect::new(-UNBOUNDED, top, UNBOUNDED, bottom),
                            );
                            (own_paint_y - start, total, total)
                        }
                        None => (own_paint_y, own_paint_height, own_background_height),
                    };
                    // A `display: contents` element has no box whose
                    // background, borders or outline could paint; its
                    // generated content still does. Neither does a box that
                    // is not visible, whose descendants may still be.
                    let paints_own_box =
                        generates_box && !raikiri_dom::paint_rules::hides_own_box(cv);
                    if paints_own_box {
                        // Paint element background (CSS Backgrounds 3 §2.2).
                        // Shift applies to the box itself per CSS 2.1 §10.8.1.
                        let clip_background = page_content_clip.filter(|_| {
                            !inside_fixed
                                && !fixed_in_viewport
                                && matches!(
                                    cv.position,
                                    PositionValue::Static
                                        | PositionValue::Relative
                                        | PositionValue::Sticky
                                )
                        });
                        if let Some(clip) = clip_background {
                            scene.scene.push_clip_layer(page_clip_transform, &clip);
                        }
                        paint_element_box_shadows(
                            scene,
                            own_paint_width,
                            own_paint_height,
                            own_paint_x,
                            own_paint_y,
                            &paint_border_radius,
                            &cv.box_shadow,
                            cv.color,
                        );
                        let table_part_clip =
                            document.anonymous_table_part_background_cells(node_id);
                        if let Some(cells) = table_part_clip {
                            let mut path = BezPath::new();
                            for cell in cells {
                                let x = f64::from(cell.x);
                                let y = f64::from(cell.y);
                                let right = x + f64::from(cell.width);
                                let bottom = y + f64::from(cell.height);
                                path.move_to((x, y));
                                path.line_to((right, y));
                                path.line_to((right, bottom));
                                path.line_to((x, bottom));
                                path.close_path();
                            }
                            scene.scene.push_clip_layer(
                                Affine::translate((f64::from(own_paint_x), f64::from(own_paint_y))),
                                &path,
                            );
                        }
                        // Propagation transfers only the background to the canvas;
                        // shadows and borders still belong to the element's box.
                        if canvas_owner != Some(node_id) {
                            paint_element_background(
                                image_cache,
                                scene,
                                own_paint_width,
                                own_background_height,
                                own_paint_x,
                                own_paint_y,
                                cv.background_color,
                                cv.background_layers(),
                                cv.color,
                                &paint_border_radius,
                                painted_border,
                                &paint_padding,
                                BackgroundEnv {
                                    border_hides_edge: matches!(
                                        cv.border_image.source,
                                        BackgroundImage::None
                                    ) && border_hides_background_edge(
                                        painted_border,
                                        cv.color,
                                    ),
                                    device_grid: image_cache.device_grid(scene.transform),
                                    fixed_viewport: fixed_background_viewport(
                                        page_area,
                                        root_transform,
                                        inside_fixed_containing_block
                                            || establishes_fixed_containing_block,
                                    ),
                                },
                                pixel_source,
                                warnings,
                            );
                        }
                        if table_part_clip.is_some() {
                            scene.pop_layer();
                        }
                        if clip_background.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // An opaque background that uses the same solid color as
                    // every border side already paints the complete visible
                    // union in `paint_element_background`; avoid compositing
                    // the same anti-aliased rounded edge a second time.
                    let border_covered_by_background = !cv.has_background_image()
                        && cv.background_color.a == 255
                        && matches!(
                            cv.final_background_clip(),
                            raikiri_style::property::VisualBox::PaddingBox
                                | raikiri_style::property::VisualBox::ContentBox
                        )
                        && [
                            &painted_border.top,
                            &painted_border.right,
                            &painted_border.bottom,
                            &painted_border.left,
                        ]
                        .iter()
                        .all(|side| {
                            side.style() == BorderStyle::Solid
                                && side.width().px() > 0.0
                                && matches!(side.color, BorderColor::Resolved(c) if c == cv.background_color)
                        });
                    // A border image replaces the border styles (CSS
                    // Backgrounds 3 §6).
                    // In the collapsing border model, border images do not
                    // apply to internal table boxes (CSS Backgrounds 3 §6.1).
                    let collapsed_internal_table_box = matches!(
                        cv.border_collapse,
                        raikiri_style::property::BorderCollapseValue::Collapse
                    ) && matches!(
                        cv.display,
                        DisplayValue::TableRowGroup
                            | DisplayValue::TableHeaderGroup
                            | DisplayValue::TableFooterGroup
                            | DisplayValue::TableRow
                            | DisplayValue::TableColumnGroup
                            | DisplayValue::TableColumn
                            | DisplayValue::TableCell
                    );
                    let border_image_drawn = paints_own_box
                        && !collapsed_internal_table_box
                        && if paints_as_absolute_continuation && unfragmented_top < own_paint_y {
                            // A continuation fragment slices the image laid
                            // out over the whole box, so it shows no top edge
                            // at the break (CSS Fragmentation 3 §5.4).
                            const UNBOUNDED: f64 = 1.0e7;
                            scene.push_clip_layer(
                                Affine::IDENTITY,
                                &Rect::new(
                                    -UNBOUNDED,
                                    f64::from(own_paint_y),
                                    UNBOUNDED,
                                    UNBOUNDED,
                                ),
                            );
                            let drawn = paint_element_border_image(
                                image_cache,
                                scene,
                                own_paint_width,
                                own_paint_y + own_paint_height - unfragmented_top,
                                own_paint_x,
                                unfragmented_top,
                                cv,
                                &cv.border_image,
                                painted_border,
                                pixel_source,
                                warnings,
                                scene.transform,
                            );
                            scene.pop_layer();
                            drawn
                        } else {
                            paint_element_border_image(
                                image_cache,
                                scene,
                                own_paint_width,
                                own_paint_height,
                                own_paint_x,
                                own_paint_y,
                                cv,
                                &cv.border_image,
                                painted_border,
                                pixel_source,
                                warnings,
                                scene.transform,
                            )
                        };
                    let border_covered_by_background =
                        border_covered_by_background || border_image_drawn;
                    // Paint border on top of background (CSS Backgrounds 3 §5).
                    let paints_continuation_border =
                        paints_own_box && paints_as_absolute_continuation;
                    if !border_covered_by_background && paints_continuation_border {
                        paint_element_border_with_top(
                            scene,
                            own_paint_width, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            own_paint_height, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            own_paint_x, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            own_paint_y, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            painted_border, // cov:ignore: absolute continuation fragments are exercised by the ignored fragmentation WPT reftests.
                            cv.color,
                            false,
                        );
                    } else if paints_own_box && !border_covered_by_background {
                        paint_element_border_rounded(
                            scene,
                            own_paint_width,
                            own_paint_height,
                            own_paint_x,
                            own_paint_y,
                            painted_border,
                            cv.color,
                            &paint_border_radius,
                        );
                    }
                    if block_slice.is_some() {
                        scene.pop_layer();
                    }
                    // Outlines do not consume box-model space and are painted
                    // outside the border edge (CSS Basic UI §4). Each box of a
                    // sliced box draws a whole outline around itself, like the
                    // pieces of an inline box.
                    if paints_own_box {
                        paint_element_outline(
                            scene,
                            own_paint_width,
                            fragment_paint_height,
                            own_paint_x,
                            fragment_paint_y,
                            &cv.outline,
                            cv.outline_offset,
                            cv.color,
                        );
                    }
                    // Resolve the source's natural dimensions first. SVG
                    // sources use the resulting concrete object dimensions
                    // for a bounded, size-specific raster request.
                    let is_replaced = node.is_inline_svg_root()
                        || document.is_canvas_element(node_id)
                        || node.tag_name() == Some("img");
                    // The replaced content paints inside its overflow clip,
                    // the same clip the page paint order puts around it.
                    let content_clip = is_replaced
                        .then(|| {
                            raikiri_dom::paint_rules::replaced_content_clip(
                                cv,
                                PaintRect::new(
                                    paint_x,
                                    paint_y,
                                    layout.size.width,
                                    layout.size.height,
                                ),
                                PaintInsets::new(
                                    layout.border.top,
                                    layout.border.right,
                                    layout.border.bottom,
                                    layout.border.left,
                                ),
                                PaintInsets::new(
                                    layout.padding.top,
                                    layout.padding.right,
                                    layout.padding.bottom,
                                    layout.padding.left,
                                ),
                            )
                        })
                        .flatten();
                    if let Some(clip) = content_clip {
                        let rect = push_overflow_clip(scene, clip, &page_box);
                        if let Some(t) = trace.as_deref_mut() {
                            t.push(crate::PaintTraceEvent::PushOverflowClip(
                                node_id,
                                [rect.x0, rect.y0, rect.x1, rect.y1],
                            ));
                        }
                    }
                    if is_replaced && let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::Replaced(node_id));
                    }
                    let painted_image = if node.is_inline_svg_root() {
                        paint_inline_svg(
                            scene,
                            document,
                            cascade,
                            cv,
                            node_id,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &paint_padding,
                            warnings,
                        )
                    } else if document.is_canvas_element(node_id) {
                        let fallback_bitmap;
                        let bitmap = if let Some(bitmap) = document.canvas_bitmap_ref(node_id) {
                            bitmap
                        } else {
                            let (w, h) = document
                                .canvas_size(node_id)
                                .unwrap_or(raikiri_dom::document::DEFAULT_CANVAS_SIZE);
                            fallback_bitmap = raikiri_dom::CanvasBitmap::transparent(w, h);
                            &fallback_bitmap
                        };
                        paint_canvas(
                            scene,
                            bitmap,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            &paint_padding,
                            cv.object_fit,
                            &cv.object_position,
                            !raikiri_dom::paint_rules::hides_own_box(cv),
                        )
                    } else if let Some(source) = pixel_source
                        && let Some(src_url) = img_src_url(document, node_id)
                        && let Some(intrinsic) = source.intrinsic_size(&src_url)
                    {
                        paint_image(
                            image_cache,
                            scene,
                            source,
                            &src_url,
                            intrinsic,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            &cv.border,
                            &layout.padding,
                            cv.object_fit,
                            &cv.object_position,
                            !raikiri_dom::paint_rules::hides_own_box(cv),
                            node_id,
                            warnings,
                        )
                    } else {
                        false
                    };
                    if !painted_image
                        && paints_own_box
                        && pixel_source.is_none()
                        && node.tag_name() == Some("img")
                        && let Some(src) = node.attribute("src")
                        && let Some(color) = infer_url_color(src)
                    {
                        // Use the same background paint path as a normal
                        // block so the fallback has identical raster edges.
                        let clip_background = page_content_clip.filter(|_| {
                            !inside_fixed
                                && !fixed_in_viewport
                                && matches!(
                                    cv.position,
                                    PositionValue::Static
                                        | PositionValue::Relative
                                        | PositionValue::Sticky
                                )
                        });
                        if let Some(clip) = clip_background {
                            scene.scene.push_clip_layer(page_clip_transform, &clip);
                        }
                        paint_element_background(
                            image_cache,
                            scene,
                            layout.size.width,
                            layout.size.height,
                            paint_x,
                            paint_y,
                            color,
                            std::slice::from_ref(&color_only_background_layer(cv)),
                            cv.color,
                            &paint_border_radius,
                            &cv.border,
                            &paint_padding,
                            BackgroundEnv {
                                device_grid: image_cache.device_grid(scene.transform),
                                ..BackgroundEnv::default()
                            },
                            pixel_source,
                            warnings,
                        );
                        if clip_background.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // The fallback stands in for the image content, so it
                    // stays inside the same clip.
                    if content_clip.is_some() {
                        if let Some(t) = trace.as_deref_mut() {
                            t.push(crate::PaintTraceEvent::PopClip);
                        }
                        scene.pop_layer();
                    }
                    if has_column_rules && !raikiri_dom::paint_rules::hides_own_box(cv) {
                        // Rules obey the owner's overflow, while an outside marker
                        // remains outside that clip and paints above the rule.
                        let rule_clip = raikiri_dom::paint_rules::overflow_clip(
                            document,
                            cascade,
                            node_id,
                            PaintRect::new(paint_x, paint_y, layout.size.width, paint_height),
                            PaintInsets::new(
                                layout.border.top,
                                layout.border.right,
                                layout.border.bottom,
                                layout.border.left,
                            ),
                        );
                        if let Some(resolved) = rule_clip {
                            push_overflow_clip(scene, resolved, &page_box);
                        }
                        for mut rule in
                            document.fragment_column_rules(cascade, node_id, fragment_id)
                        {
                            rule.rect.x += paint_x;
                            rule.rect.y += paint_y;
                            rule.pattern_origin += paint_y;
                            let content_top = margins.top + insets.top;
                            let start = rule.rect.y.max(content_top);
                            let end =
                                (rule.rect.y + rule.rect.height).min(content_top + content_height);
                            rule.rect.y = start;
                            rule.rect.height = (end - start).max(0.0);
                            crate::column_rules::paint(scene, rule);
                        }
                        if rule_clip.is_some() {
                            scene.pop_layer();
                        }
                    }
                    // A generated list marker belongs to the first principal
                    // fragment; inherited ancestor fragment state is unrelated.
                    if !paints_as_absolute_continuation
                        && cv.display == DisplayValue::ListItem
                        && fragment_id.is_none_or(|id| {
                            layout_fragments[id].node_id != node_id
                                || layout_fragments[id].fragment_index == 0
                        })
                    {
                        // An empty list item still owns a generated marker.
                        // Its principal box has zero width/height, which the
                        // marker helper intentionally rejects for ordinary
                        // zero-sized fragments. Give this empty-item marker a
                        // minimal paint extent without changing that helper's
                        // defensive behavior for other callers.
                        let empty_item_marker = node.children.is_empty();
                        let marker_width = if empty_item_marker {
                            layout.size.width.max(1.0)
                        } else {
                            layout.size.width
                        };
                        let marker_height = if empty_item_marker {
                            layout.size.height.max(cv.font_size.px().max(1.0))
                        } else {
                            layout.size.height
                        };
                        paint_list_marker_with_snapshots(
                            image_cache,
                            scene,
                            document,
                            cascade,
                            node_id,
                            paint_x,
                            paint_y,
                            marker_width,
                            marker_height,
                            layout.padding.left,
                            (
                                content_origin_y,
                                page_offset_y + page_bottom,
                                inside_fixed || fixed_in_viewport,
                            ),
                            counter_snapshots,
                            pixel_source,
                        );
                    }
                    // Generated content is an immediate child of its
                    // originating box.  The minimal layout engine does not
                    // allocate an arena node for it, so paint literal
                    // `::before` content at the content-box inline start.
                    // The returned advance is used by the deferred `::after`
                    // frame below to preserve source order for an empty or
                    // otherwise unlaid-out originating box.
                    before_advance = if !paints_as_absolute_continuation && !has_column_rules {
                        paint_generated_pseudo(
                            scene,
                            document,
                            cascade,
                            node_id,
                            raikiri_style::PseudoElem::Before,
                            generated_x,
                            generated_y,
                            layout.size.width,
                            paint_height,
                            counter_snapshots,
                        )
                    } else {
                        0.0
                    };
                }
                // The fragmentainer clip encloses the element and its children;
                // close it after any element-local overflow clip.
                if multicol_clip_pushed {
                    stack.push(PaintFrame::PopClip);
                }
                if fragmentainer_clip_pushed {
                    stack.push(PaintFrame::PopClip);
                }
                // CSS Overflow 3 §3.1: non-visible overflow clips descendants to
                // the padding box. Min edges are floored outward so a fractional
                // padding-box origin does not antialias-cut pixel-snapped descendant
                // backgrounds (which round to integers); this mirrors the multicol
                // clip origin flooring. The clip is pushed only after painting the
                // element itself, then popped after its complete subtree via the
                // explicit stack frame.
                let mut overflow_clip_pushed = None;
                if let Some(resolved) = raikiri_dom::paint_rules::overflow_clip(
                    document,
                    cascade,
                    node_id,
                    PaintRect::new(paint_x, paint_y, layout.size.width, paint_height),
                    PaintInsets::new(
                        layout.border.top,
                        layout.border.right,
                        layout.border.bottom,
                        layout.border.left,
                    ),
                ) {
                    let transform = scene.transform;
                    let clip = push_overflow_clip(scene, resolved, &page_box);
                    stack.push(PaintFrame::PopClip);
                    overflow_clip_pushed = Some((resolved, transform));
                    if let Some(t) = trace.as_deref_mut() {
                        t.push(crate::PaintTraceEvent::PushOverflowClip(
                            node_id,
                            [clip.x0, clip.y0, clip.x1, clip.y1],
                        ));
                    }
                }
                // A column rule is below all content, including a standalone
                // generated overlay that the inline engine did not lay out.
                if has_column_rules
                    && paints_on_page
                    && !paints_as_absolute_continuation
                    && !raikiri_dom::paint_rules::hides_empty_table_cell(document, cascade, node_id)
                {
                    before_advance = paint_generated_pseudo(
                        scene,
                        document,
                        cascade,
                        node_id,
                        raikiri_style::PseudoElem::Before,
                        generated_x,
                        generated_y,
                        layout.size.width,
                        paint_height,
                        counter_snapshots,
                    );
                }
                // Push this after the clip-pop frame and before children.
                // Children therefore paint first, then `::after`, then the
                // clip closes.  A full IFC would place the after run after
                // laid-out inline descendants; this minimal path uses the
                // generated-before advance as its only inline position.
                let after_pushed = paints_on_page && !paints_as_absolute_continuation;
                if after_pushed {
                    stack.push(PaintFrame::PaintAfter {
                        node_id,
                        x: generated_x,
                        y: generated_y,
                        width: layout.size.width,
                        height: paint_height,
                        before_advance,
                        content_advance: generated_content_advance,
                    });
                }
                let child_font_size = cv.font_size.px();
                // `text-decoration-line` is non-inherited at the computed-value
                // layer, but its originating line is propagated to descendants
                // by CSS Text Decoration. Keep that paint-only context separate
                // from `CascadeResult`'s inheritance result.
                let child_decorations = text::decorations_for_element(
                    &decorations,
                    NodeId::new(node_id as u64),
                    cv,
                    child_shift_y,
                );
                // Paint positioned siblings in stacking order while keeping
                // source order for equal stack levels. Flex/grid containers use
                // order-modified document order within each stacking bucket,
                // as required by CSS Display's `order` property behavior.
                // This is intentionally local to the current parent; full
                // nested stacking-context isolation remains outside this
                // minimal painter.
                let mut children = if node.is_inline_svg_root()
                    || document.anonymous_table_contents_paint_only(node_id)
                {
                    Vec::new()
                } else if node.is_ifc_root() {
                    // The paragraph's own text and inline elements are drawn
                    // from the lines; only the boxes laid out beside them are
                    // visited like ordinary children.
                    node.ifc_boxes()
                } else if let Some(children) = document.anonymous_table_paint_sequence(node_id) {
                    children
                } else {
                    node.children.clone()
                };
                raikiri_dom::paint_rules::sort_paint_children(&mut children, cv.display, cascade);

                // The current Taffy bridge treats inline children as zero-sized
                // block items.  Keep that layout authority, but provide the
                // measured paint-time inline advance for generated pseudo text
                // so a sequence such as `span::before { content: counter(c) }`
                // remains in source order.  This is deliberately limited to
                // siblings that actually carry generated content; ordinary
                // inline layout remains unchanged until a full IFC lands.
                let child_generated_advances: Vec<(usize, bool, f32)> = children
                    .iter()
                    .map(|&child| {
                        let is_text = document
                            .get_node(child)
                            .is_some_and(|child_node| child_node.kind() == NodeKind::Text);
                        let advance = if !is_text
                            && cascade
                                .computed
                                .get(child)
                                .is_some_and(|child_cv| child_cv.display.is_inline_flow())
                        {
                            generated_pseudo_text_advance(
                                document,
                                cascade,
                                child,
                                raikiri_style::PseudoElem::Before,
                                counter_snapshots,
                            ) + generated_pseudo_text_advance(
                                document,
                                cascade,
                                child,
                                raikiri_style::PseudoElem::After,
                                counter_snapshots,
                            )
                        } else {
                            0.0
                        };
                        (child, is_text, advance)
                    })
                    .collect();
                let inline_offsets = if child_generated_advances
                    .iter()
                    .any(|(_, _, advance)| *advance > 0.0)
                {
                    let family = cv
                        .font_family
                        .first()
                        .cloned()
                        .unwrap_or_else(|| raikiri_style::FontFamilyName::generic("serif"));
                    let collapsed_space = text::measure_margin_text_advance(
                        document,
                        " ",
                        cv.font_size.px(),
                        std::slice::from_ref(&family),
                    );
                    let mut flow_advance = 0.0;
                    let mut saw_generated_inline = false;
                    let mut offsets = Vec::new();
                    for (index, (child, is_text, advance)) in
                        child_generated_advances.iter().enumerate()
                    {
                        if *is_text {
                            let has_following_generated_inline = child_generated_advances
                                .iter()
                                .skip(index + 1)
                                .any(|(_, _, following)| *following > 0.0);
                            if saw_generated_inline && has_following_generated_inline {
                                flow_advance += collapsed_space;
                            }
                        } else if *advance > 0.0 {
                            offsets.push((*child, flow_advance));
                            flow_advance += *advance;
                            saw_generated_inline = true;
                        }
                    }
                    offsets
                } else {
                    Vec::new()
                };
                let vertical_generated_offsets = if !cv.display.is_inline_flow() {
                    let mut flow_extra = generated_pseudo_text_height(
                        document,
                        cascade,
                        node_id,
                        raikiri_style::PseudoElem::Before,
                        counter_snapshots,
                    );
                    let mut offsets = Vec::new();
                    for &child in &children {
                        offsets.push((child, flow_extra));
                        let Some(child_node) = document.get_node(child) else {
                            continue; // cov:ignore: document child ids are arena-valid
                        };
                        let child_cv = &cascade.computed[child];
                        let child_is_normal_flow = matches!(
                            child_cv.position,
                            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                        );
                        if child_node.kind() != NodeKind::Text
                            && !child_cv.display.is_inline_flow()
                            && child_is_normal_flow
                        {
                            let required_height = generated_flow_height(
                                document,
                                cascade,
                                child,
                                counter_snapshots,
                                generated_flow_heights,
                            );
                            flow_extra += (required_height
                                - child_node.unrounded_layout.size.height)
                                .max(0.0);
                        }
                    }
                    offsets
                } else {
                    Vec::new()
                };

                // Reverse push makes the lowest stack level paint first.
                // Relative insets are already present in `layout.location`
                // except for the manual placement paths identified above.
                let child_parent_x = abs_x + pos_dx + fixed_dx;
                let child_parent_y = abs_y + pos_dy + fixed_dy;
                let child_paint_offset_y = paint_offset_y + pos_dy + fixed_dy;
                // A paragraph inside a fixed box, or a fixed box that is a
                // paragraph itself, repeats on every page and is not clipped
                // like flowed text, as the text branch below does.
                let ifc_inside_fixed = inside_fixed || fixed_in_viewport;
                // The page a text node belongs to is looked up on the text node
                // itself; the block's own page value can differ (an absolutely
                // positioned box named for a page), so ask as the first text
                // child would.
                let ifc_page_probe = if node.is_ifc_root() {
                    first_text_descendant(document, node_id).unwrap_or(node_id)
                } else {
                    node_id
                };
                let mut lines_frame = None;
                if node.is_ifc_root()
                    && named_page_matches(ifc_page_probe)
                    && (ifc_inside_fixed
                        || box_intersects_page(
                            abs_y,
                            ifc_paint_extent(node, &layout),
                            page_top,
                            page_bottom,
                        ))
                {
                    // The block's own background and border were painted
                    // above; its lines are drawn at the content-box origin,
                    // where a child text node would have been placed. That
                    // origin comes from the taffy layout's border and padding
                    // (the text child's `location`), not from
                    // `used_padding_for_paint`, which places the element's own
                    // generated content.
                    let content_x = child_parent_x
                        + page_offset_x
                        + child_transform_x
                        + layout.border.left
                        + layout.padding.left;
                    let content_y = child_parent_y
                        + page_offset_y
                        + child_transform_y
                        + layout.border.top
                        + layout.padding.top;
                    lines_frame = Some(PaintFrame::Lines(LinesFrame {
                        node_id,
                        x: content_x,
                        y: content_y,
                        shift_y: child_shift_y,
                        translation_y: child_transform_y + child_paint_offset_y,
                        decorations: child_decorations.clone(),
                        fragmentainer,
                        inside_fixed: ifc_inside_fixed,
                        paint_transform: scene.transform,
                        fixed_viewport: fixed_background_viewport(
                            page_area,
                            root_transform,
                            inside_fixed_containing_block || establishes_fixed_containing_block,
                        ),
                        // Lines clipped to the page content box hold no
                        // boxes, which that clip does not cover.
                        hold: !children.is_empty()
                            && node.ifc_lines().is_some()
                            && text_page_clip(ifc_inside_fixed).is_none(),
                        boxes_in_groups: children.iter().any(|&child| {
                            !crate::ifc_text::enclosing_inline_opacities(
                                crate::ifc_text::Paragraph {
                                    document,
                                    cascade,
                                    root_id: node_id,
                                },
                                child,
                            )
                            .is_empty()
                        }),
                    }));
                }
                let hold_lines = matches!(
                    lines_frame,
                    Some(PaintFrame::Lines(LinesFrame { hold: true, .. }))
                );
                let own_multicol_clip_height = match cv.column_count {
                    ColumnCountValue::Count(count)
                        if count > 1
                            && node.is_multicol_container()
                            && layout.size.height > 0.0 =>
                    {
                        Some(layout.size.height)
                    }
                    _ => None,
                };
                // A `display: contents` element has no box, so the columns of
                // the container it sits in hold its children.
                let own_multicol_clip_height = own_multicol_clip_height
                    .or((cv.display == DisplayValue::Contents)
                        .then_some(column_clip_height)
                        .flatten());
                // A plain block in a block container leaves its inline
                // content to the end of the layer (CSS 2.1 Appendix E): its
                // lines and the boxes laid out beside them paint after the
                // backgrounds of the blocks that follow it.
                let defers_here = !multicol_clip_pushed
                    && !fragmentainer_clip_pushed
                    && fragmentainer.is_none()
                    && !is_fragment_visit
                    && !fixed_in_viewport;
                let defers = defers_here
                    && overflow_clip_pushed.is_none()
                    && raikiri_dom::paint_rules::defers_inline_content(document, cascade, node_id);
                // A plain block that clips collects its inline content in a
                // layer of its own, which it hands to the enclosing layer
                // inside its clip.
                let merge = overflow_clip_pushed
                    .filter(|_| {
                        defers_here
                            && deferral_scopes.last().is_some_and(|layer| layer.allowed)
                            && raikiri_dom::paint_rules::merges_clipped_inline_content(
                                document, cascade, node_id,
                            )
                    })
                    .map(|(clip, transform)| LayerClose::Merge {
                        node_id,
                        clip,
                        transform,
                    });
                // Frames of the children, last child first.
                let mut child_frames = Vec::new();
                let mut below_lines_frames = Vec::new();
                // In-flow blocks inside the paragraph's inline content, whose
                // backgrounds paint before its lines.
                let mut block_frames = Vec::new();
                // With content positioned below the blocks, the lines place
                // the blocks between that content and the rest.
                let negative_content = node.is_ifc_root()
                    && raikiri_dom::paint_rules::holds_negative_stack_level(
                        document, cascade, node_id,
                    );
                let blocks_first = node.is_ifc_root() && !children.is_empty();
                for child in children.into_iter().rev() {
                    if let Some(cell) = document.anonymous_table_cell(child) {
                        child_frames.push(PaintFrame::AnonymousParagraph(
                            AnonymousParagraphFrame {
                                key: child,
                                cell,
                                x: child_parent_x + cell.unrounded_layout.location.x,
                                y: child_parent_y + cell.unrounded_layout.location.y,
                                shift_y: child_shift_y,
                                transform_x: child_transform_x,
                                transform_y: child_transform_y,
                                paint_offset_y: child_paint_offset_y,
                                fragmentainer,
                                inside_fixed: ifc_inside_fixed,
                                decorations: child_decorations.clone(),
                                fixed_viewport: fixed_background_viewport(
                                    page_area,
                                    root_transform,
                                    inside_fixed_containing_block
                                        || establishes_fixed_containing_block,
                                ),
                                defer: defers,
                            },
                        ));
                        continue;
                    }
                    let child_fragment_clip_height = match own_multicol_clip_height {
                        Some(clip_height)
                            if document.get_node(child).is_some_and(|child_node| {
                                (cascade.computed[child].display.is_grid_container()
                                    || matches!(
                                        cascade.computed[child].display,
                                        DisplayValue::Table | DisplayValue::InlineTable
                                    )) && child_node.kind() == NodeKind::Element
                            }) =>
                        {
                            Some(clip_height)
                        }
                        // A zero-height block at a multicolumn break can own
                        // an overflowing parallel flow. It has no fragment
                        // area in the column, so its descendants must not
                        // paint into the following sibling's fragment. A
                        // block that layout broke into column pieces places
                        // its descendants in those pieces itself. A
                        // `display: contents` element has no box at all, so
                        // its children are the ones the columns hold.
                        Some(_) // cov:ignore: exercised by ignored exact paged-text WPT.
                            if !fragmented_nodes.contains(&child) && document.get_node(child).is_some_and(|child_node| { // cov:ignore: exercised by ignored exact paged-text WPT.
                                child_node.kind() == NodeKind::Element // cov:ignore: exercised by ignored exact paged-text WPT.
                                    && cascade.computed[child].display != DisplayValue::Contents
                                    && child_node.unrounded_layout.size.height <= 0.0 // cov:ignore: exercised by ignored exact paged-text WPT.
                                    && !child_node.children.is_empty() // cov:ignore: exercised by ignored exact paged-text WPT.
                            }) => // cov:ignore: exercised by ignored exact paged-text WPT.
                        {
                            Some(0.0) // cov:ignore: exercised by ignored exact paged-text WPT.
                        }
                        Some(_) => None,
                        None => fragment_clip_height,
                    };
                    let is_direct_text = document
                        .get_node(child)
                        .is_some_and(|node| node.kind() == NodeKind::Text);
                    let generated_text_shift = if is_direct_text { before_advance } else { 0.0 };
                    let inline_generated_offset = inline_offsets
                        .iter()
                        .find_map(|(offset_child, offset)| {
                            (*offset_child == child).then_some(*offset)
                        })
                        .unwrap_or(0.0);
                    let vertical_generated_offset = vertical_generated_offsets
                        .iter()
                        .find_map(|(offset_child, offset)| {
                            (*offset_child == child).then_some(*offset)
                        })
                        .unwrap_or(0.0);
                    // A box beside held lines joins the groups of the
                    // paragraph, which composite it with the rest of their
                    // paint once the lines and boxes are all recorded.
                    // A box beside other lines inside translucent inline
                    // elements paints inside their opacity. Each box gets
                    // layers of its own, apart from the group the elements'
                    // text composites in.
                    let inline_opacities = if node.is_ifc_root() && !hold_lines {
                        crate::ifc_text::enclosing_inline_opacities(
                            crate::ifc_text::Paragraph {
                                document,
                                cascade,
                                root_id: node_id,
                            },
                            child,
                        )
                    } else {
                        Vec::new()
                    };
                    // A box with a negative stack level paints below the
                    // paragraph's lines (CSS 2.1 Appendix E steps 3 and 7).
                    let below_lines = node.is_ifc_root()
                        && cascade.computed.get(child).is_some_and(|child_cv| {
                            child_cv.position != PositionValue::Static
                                && matches!(child_cv.z_index, ZIndexValue::Integer(level) if level < 0)
                        });
                    let block = blocks_first
                        && raikiri_dom::paint_rules::paints_before_paragraph_lines(
                            document, cascade, node_id, child,
                        );
                    let frames = if block {
                        &mut block_frames
                    } else if below_lines {
                        &mut below_lines_frames
                    } else {
                        &mut child_frames
                    };
                    // A box beside held lines joins the groups of the
                    // paragraph, which composite it with the rest of their
                    // paint once the lines and boxes are all recorded.
                    let captured = (hold_lines && !block) || (block && negative_content);
                    if captured {
                        frames.push(PaintFrame::EndBox {
                            root: node_id,
                            node_id: child,
                            early: below_lines || block,
                        });
                    }
                    frames.extend(inline_opacities.iter().map(|_| PaintFrame::PopOpacity));
                    frames.push(PaintFrame::Visit(VisitFrame {
                        node_id: child,
                        parent_abs_x: child_parent_x
                            + generated_text_shift
                            + inline_generated_offset,
                        parent_abs_y: child_parent_y + vertical_generated_offset,
                        parent_font_size: child_font_size,
                        shift_y: child_shift_y,
                        transform_x: child_transform_x,
                        transform_y: child_transform_y,
                        paint_offset_y: child_paint_offset_y,
                        fragment_clip_height: child_fragment_clip_height,
                        column_clip_height: (cascade.computed[child].display
                            == DisplayValue::Contents)
                            .then_some(own_multicol_clip_height)
                            .flatten(),
                        fragment_id,
                        fragment_origin_x,
                        fragment_origin_y,
                        fragmentainer,
                        fragment_rect: None,
                        fragment_clip: None,
                        is_fragment_visit: false,
                        inside_fixed: inside_fixed || fixed_in_viewport,
                        inside_fixed_containing_block: inside_fixed_containing_block
                            || establishes_fixed_containing_block,
                        decorations: child_decorations.clone(),
                    }));
                    frames.extend(
                        inline_opacities
                            .iter()
                            .rev()
                            .map(|&(node_id, alpha)| PaintFrame::PushOpacity { node_id, alpha }),
                    );
                    if captured {
                        frames.push(PaintFrame::BeginBox {
                            root: node_id,
                            node_id: child,
                            early: below_lines || block,
                        });
                    }
                }
                let finish_lines = hold_lines.then_some(PaintFrame::FinishLines { root: node_id });
                // A `display: contents` element has no box, so its children
                // share the enclosing layer.
                if !defers && generates_box {
                    let allowed = deferral_scopes.last().is_some_and(|layer| layer.allowed)
                        && !node.is_multicol_container();
                    let close = merge.unwrap_or(
                        if raikiri_dom::paint_rules::hoists_positioned_content(
                            document, cascade, node_id,
                        ) {
                            LayerClose::Hoist
                        } else {
                            LayerClose::Paint
                        },
                    );
                    let mut layer = DeferralLayer::new(allowed, close);
                    layer.after = after_pushed;
                    deferral_scopes.push(layer);
                    stack.push(PaintFrame::EndDeferral);
                }
                if (defers || merge.is_some()) && node.is_ifc_root() {
                    if let Some(layer) = deferral_scopes.last_mut() {
                        let pending = &mut layer.pending;
                        pending.extend(below_lines_frames.into_iter().rev());
                        if negative_content && !block_frames.is_empty() {
                            pending.push(PaintFrame::TraceBelowBlocks { root: node_id });
                            pending.extend(std::mem::take(&mut block_frames).into_iter().rev());
                        }
                        pending.extend(lines_frame);
                        pending.extend(child_frames.into_iter().rev());
                        pending.extend(finish_lines);
                        // A standalone `::after` still follows the content
                        // it is generated after.
                        if defers && after_pushed {
                            pending.extend(stack.pop());
                        }
                    }
                } else {
                    stack.extend(finish_lines);
                    stack.extend(child_frames);
                    stack.extend(lines_frame);
                    if negative_content && !block_frames.is_empty() {
                        stack.extend(std::mem::take(&mut block_frames));
                        stack.push(PaintFrame::TraceBelowBlocks { root: node_id });
                    }
                    stack.extend(below_lines_frames);
                }
                stack.extend(block_frames);
            }
            NodeKind::Text => {
                let layout = node_layout;
                let abs_x = parent_abs_x + layout.location.x;
                let abs_y = parent_abs_y + layout.location.y;
                if named_page_matches(node_id)
                    && (inside_fixed
                        || box_intersects_page(abs_y, layout.size.height, page_top, page_bottom))
                {
                    let text_clip = text_page_clip(inside_fixed);
                    if let Some(clip) = &text_clip {
                        scene.scene.push_clip_layer(page_clip_transform, clip);
                    }
                    if node.is_ifc_root() {
                        // A text node laid out as an anonymous flex or grid
                        // item is a paragraph of its own: its lines start at
                        // its own box, which has no edges.
                        if let Some(t) = trace.as_deref_mut() {
                            t.push(crate::PaintTraceEvent::Text(node_id));
                        }
                        let paint_transform = scene.transform;
                        crate::ifc_text::draw_ifc_lines_with_resources(
                            image_cache,
                            scene,
                            document,
                            cascade,
                            node_id,
                            crate::ifc_text::IfcPosition {
                                x: abs_x + page_offset_x + transform_x,
                                y: abs_y + page_offset_y + transform_y,
                                shift_y,
                            },
                            &decorations,
                            fragmentainer,
                            custom_highlights,
                            pixel_source,
                            warnings,
                            page_box,
                            page_lines
                                .filter(|_| !inside_fixed)
                                .map(|lines| lines.translated(transform_y + paint_offset_y)),
                            paint_transform,
                            fixed_background_viewport(
                                page_area,
                                root_transform,
                                inside_fixed_containing_block,
                            ),
                        );
                    }
                    // Any other text node lies outside every paragraph and
                    // has no lines to draw.
                    if text_clip.is_some() {
                        scene.pop_layer();
                    }
                }
            }
            NodeKind::Document => {
                // The walk starts at the document element.
                // A Document node may have children (unattached `<html>`), but
                // they are not handled yet. Skip the subtree defensively.
            }
            _ => {
                // NodeKind is #[non_exhaustive]: `Comment` / `ProcessingInstruction` /
                // `DocumentFragment` arrives here (not paintable). In practice,
                // mark_in_document_flags clears IS_IN_DOCUMENT for Comment/PI,
                // so the earlier is_in_document() gate should filter those
                // before this walker reaches them. Keep this kind gate as a
                // second defense: if both gates fail, skip the whole subtree.
                // Apply the same behavior to future CDATA / DocumentType kinds.
                // Retain the defensive fallback for unknown node kinds.
            }
        }
    }
}
