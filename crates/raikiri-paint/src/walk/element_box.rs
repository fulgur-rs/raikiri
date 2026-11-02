//! Where the box of an element is painted on the page.

use super::frames::{PaintFrame, VisitFrame};
use super::page_geometry::PageGeometry;
use super::paint_walk::PaintWalk;
use super::{
    OpenBlockEdges, box_intersects_page, element_transform, fixed_position_px,
    generated_flow_height, outline_outset, page_bounds_in_scene, paintable_border_radius,
    snap_column_clip_outward, transform_translation, translation_only, used_padding_for_paint,
    vertical_align_shift_px,
};
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use raikiri_dom::{Document, FragmentRect, Node};
use raikiri_layout_geometry::{BoxLayout, Edges};
use raikiri_style::property::{BorderStyle, PositionValue};
use raikiri_style::{
    CascadeResult, ComputedBorder, ComputedBorderRadius, ComputedLengthPercentageOrAuto,
    ComputedValues, Sides,
};
use raikiri_traits::NodeKind;

/// What the visit of a node reads of its frame to find the box of an element.
#[derive(Clone, Copy)]
pub(super) struct BoxInputs {
    pub(super) node_id: usize,
    pub(super) parent_abs_x: f32,
    pub(super) parent_abs_y: f32,
    pub(super) parent_font_size: f32,
    pub(super) shift_y: f32,
    pub(super) transform_x: f32,
    pub(super) transform_y: f32,
    pub(super) fragment_clip_height: Option<f32>,
    pub(super) fragment_id: Option<usize>,
    pub(super) fragment_clip: Option<FragmentRect>,
    pub(super) is_fragment_visit: bool,
    pub(super) inside_fixed: bool,
    pub(super) inside_fixed_containing_block: bool,
}

impl BoxInputs {
    /// What the visit `visit` hands to the search for the box of its element.
    pub(super) fn of(visit: &VisitFrame) -> Self {
        Self {
            node_id: visit.node_id,
            parent_abs_x: visit.parent_abs_x,
            parent_abs_y: visit.parent_abs_y,
            parent_font_size: visit.parent_font_size,
            shift_y: visit.shift_y,
            transform_x: visit.transform_x,
            transform_y: visit.transform_y,
            fragment_clip_height: visit.fragment_clip_height,
            fragment_id: visit.fragment_id,
            fragment_clip: visit.fragment_clip,
            is_fragment_visit: visit.is_fragment_visit,
            inside_fixed: visit.inside_fixed,
            inside_fixed_containing_block: visit.inside_fixed_containing_block,
        }
    }
}

/// Where the box of an element lies and which of its parts paint on this page.
#[derive(Clone, Copy)]
pub(super) struct ElementBox<'w> {
    pub(super) node: &'w Node,
    pub(super) node_id: usize,
    pub(super) cv: &'w ComputedValues,
    pub(super) layout: BoxLayout,
    pub(super) fragment_id: Option<usize>,
    pub(super) inside_fixed: bool,
    pub(super) inside_fixed_containing_block: bool,
    pub(super) abs_x: f32,
    pub(super) abs_y: f32,
    pub(super) generates_box: bool,
    pub(super) paint_padding: Edges<f32>,
    pub(super) paint_border_radius: ComputedBorderRadius,
    pub(super) child_shift_y: f32,
    pub(super) pos_dx: f32,
    pub(super) pos_dy: f32,
    pub(super) fixed_dx: f32,
    pub(super) fixed_dy: f32,
    pub(super) child_transform_x: f32,
    pub(super) child_transform_y: f32,
    pub(super) placed_in_column: bool,
    pub(super) fixed_in_viewport: bool,
    pub(super) establishes_fixed_containing_block: bool,
    pub(super) paint_x: f32,
    pub(super) paint_y: f32,
    pub(super) paint_height: f32,
    pub(super) paint_background_height: f32,
    pub(super) multicol_clip_pushed: bool,
    pub(super) fragmentainer_clip_pushed: bool,
    pub(super) painted_border: &'w Sides<ComputedBorder>,
    pub(super) paints_on_page: bool,
    pub(super) paints_as_absolute_continuation: bool,
    pub(super) unfragmented_top: f32,
    pub(super) generated_x: f32,
    pub(super) generated_y: f32,
    pub(super) generated_content_advance: f32,
}

/// Where the box of an element lies before the clips of fragmentainers cut
/// it.
struct Placement<'w> {
    cv: &'w ComputedValues,
    abs_x: f32,
    abs_y: f32,
    generates_box: bool,
    paint_padding: Edges<f32>,
    paint_border_radius: ComputedBorderRadius,
    child_shift_y: f32,
    pos_dx: f32,
    pos_dy: f32,
    transformable: bool,
    child_transform_x: f32,
    child_transform_y: f32,
    placed_in_column: bool,
    fixed_in_viewport: bool,
    establishes_fixed_containing_block: bool,
    fixed_dx: f32,
    fixed_dy: f32,
    paint_x: f32,
    paint_y: f32,
}

/// The extent of the box of an element after the clips of the fragmentainers
/// cut it.
struct FragmentCut {
    paint_y: f32,
    paint_height: f32,
    paint_background_height: f32,
    multicol_clip_pushed: bool,
    fragmentainer_clip_pushed: bool,
}

/// Whether the box of an element paints on this page, and the extent it
/// paints in.
struct PageExtent {
    paint_y: f32,
    paint_height: f32,
    paint_background_height: f32,
    paints_on_page: bool,
    paints_as_absolute_continuation: bool,
    unfragmented_top: f32,
}

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Find where the box of the element `inputs.node_id`, laid out as
    /// `layout`, is painted. Opens the clips of the fragmentainers that cut it
    /// and the transform that moves it, or gives `None` when the transform
    /// hides the element and its subtree.
    pub(super) fn element_box(
        &mut self,
        inputs: BoxInputs,
        node: &'w Node,
        layout: BoxLayout,
    ) -> Option<ElementBox<'w>> {
        let BoxInputs {
            node_id,
            fragment_id,
            inside_fixed,
            inside_fixed_containing_block,
            ..
        } = inputs;
        let placement = self.placement(&inputs, node, layout);
        let Placement {
            cv,
            paint_padding,
            paint_x,
            paint_y,
            ..
        } = placement;
        if placement.transformable
            && !translation_only(cv)
            && !self.push_transform(cv, layout, paint_x, paint_y)
        {
            return None;
        }
        let cut = self.cut_by_fragmentainers(&inputs, layout, paint_x, paint_y);
        // A collapsed-border table cell paints the borders that won
        // conflict resolution with its rows, neighbours and table.
        let painted_border = self
            .document
            .get_node(node_id)
            .and_then(|node| node.collapsed_border())
            .unwrap_or(&cv.border);
        let mut extent = self.page_extent(&inputs, node, layout, &placement, &cut);
        self.grow_for_generated_flow(node_id, layout, &placement, &mut extent);
        let generated_x = paint_x + cv.border.left.width().px() + paint_padding.left;
        let generated_y = extent.paint_y + cv.border.top.width().px() + paint_padding.top;
        Some(ElementBox {
            node,
            node_id,
            cv,
            layout,
            fragment_id,
            inside_fixed,
            inside_fixed_containing_block,
            abs_x: placement.abs_x,
            abs_y: placement.abs_y,
            generates_box: placement.generates_box,
            paint_padding,
            paint_border_radius: placement.paint_border_radius,
            child_shift_y: placement.child_shift_y,
            pos_dx: placement.pos_dx,
            pos_dy: placement.pos_dy,
            fixed_dx: placement.fixed_dx,
            fixed_dy: placement.fixed_dy,
            child_transform_x: placement.child_transform_x,
            child_transform_y: placement.child_transform_y,
            placed_in_column: placement.placed_in_column,
            fixed_in_viewport: placement.fixed_in_viewport,
            establishes_fixed_containing_block: placement.establishes_fixed_containing_block,
            paint_x,
            paint_y: extent.paint_y,
            paint_height: extent.paint_height,
            paint_background_height: extent.paint_background_height,
            multicol_clip_pushed: cut.multicol_clip_pushed,
            fragmentainer_clip_pushed: cut.fragmentainer_clip_pushed,
            painted_border,
            paints_on_page: extent.paints_on_page,
            paints_as_absolute_continuation: extent.paints_as_absolute_continuation,
            unfragmented_top: extent.unfragmented_top,
            generated_x,
            generated_y,
            generated_content_advance: generated_content_advance(
                self.document,
                node,
                cv,
                paint_padding,
            ),
        })
    }

    /// Grow the paint height of the auto-height element `node_id` to hold
    /// the line boxes of its generated content.
    ///
    /// The pseudo is not an arena child yet, so Taffy cannot include its line
    /// box in an auto-height originating box. Only the auto-height paint
    /// geometry expands; fixed-height elements retain their normal overflow
    /// behavior. This is a paint-side bridge: the arena/Taffy flow remains
    /// unchanged until generated inline boxes can participate in layout.
    fn grow_for_generated_flow(
        &mut self,
        node_id: usize,
        layout: BoxLayout,
        placement: &Placement<'w>,
        extent: &mut PageExtent,
    ) {
        let Placement {
            cv, paint_padding, ..
        } = *placement;
        let (document, cascade, counter_snapshots) =
            (self.document, self.cascade, self.counter_snapshots);
        if !extent.paints_as_absolute_continuation
            && matches!(cv.height, ComputedLengthPercentageOrAuto::Auto)
        {
            let pseudo_height = generated_flow_height(
                document,
                cascade,
                node_id,
                counter_snapshots,
                &mut self.generated_flow_heights,
            );
            let pseudo_border_height = cv.border.top.width().px()
                + paint_padding.top
                + pseudo_height
                + paint_padding.bottom
                + cv.border.bottom.width().px();
            if pseudo_border_height > extent.paint_height {
                let was_intrinsic_height =
                    (extent.paint_background_height - layout.size.height).abs() <= f32::EPSILON;
                extent.paint_height = pseudo_border_height;
                if was_intrinsic_height {
                    extent.paint_background_height = extent.paint_height;
                }
            }
        }
    }

    /// Where the box of the element `inputs.node_id` lies, before any
    /// transform or fragmentainer clip applies.
    fn placement(&self, inputs: &BoxInputs, node: &'w Node, layout: BoxLayout) -> Placement<'w> {
        let BoxInputs {
            node_id,
            parent_abs_x,
            parent_abs_y,
            parent_font_size,
            shift_y,
            transform_x,
            transform_y,
            inside_fixed_containing_block,
            ..
        } = *inputs;
        let (document, cascade, root_id) = (self.document, self.cascade, self.root_id);
        let PageGeometry {
            page_offset_x,
            page_offset_y,
            ..
        } = self.geometry;
        let abs_x = parent_abs_x + layout.location.x;
        let abs_y = parent_abs_y + layout.location.y;
        let cv = &cascade.computed[node_id];
        let generates_box = node_id == root_id || raikiri_dom::paint_rules::generates_box(cv);
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
        let own_shift = self.own_vertical_align_shift(node, node_id, cv, parent_font_size);
        let child_shift_y = shift_y + own_shift;
        let (placed_in_column, (pos_dx, pos_dy)) = self.paint_inset(inputs, node, cv);
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
            self.fixed_offset(cv, &layout, (abs_x, abs_y))
        } else {
            (0.0, 0.0)
        };
        let paint_x = abs_x + page_offset_x + pos_dx + fixed_dx + child_transform_x;
        let paint_y = abs_y + page_offset_y + child_shift_y + pos_dy + fixed_dy + child_transform_y;
        Placement {
            cv,
            abs_x,
            abs_y,
            generates_box,
            paint_padding,
            paint_border_radius,
            child_shift_y,
            pos_dx,
            pos_dy,
            transformable,
            child_transform_x,
            child_transform_y,
            placed_in_column,
            fixed_in_viewport,
            establishes_fixed_containing_block,
            fixed_dx,
            fixed_dy,
            paint_x,
            paint_y,
        }
    }

    /// How far `vertical-align` shifts the element `node_id`, whose computed
    /// values are `cv`, below the baseline of a parent of font size
    /// `parent_font_size`.
    fn own_vertical_align_shift(
        &self,
        node: &Node,
        node_id: usize,
        cv: &ComputedValues,
        parent_font_size: f32,
    ) -> f32 {
        let (document, cascade, root_id) = (self.document, self.cascade, self.root_id);
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
        let blockified_item = document
            .layout_parent_of(node_id)
            .is_some_and(|parent| cascade.computed[parent].display.is_flex_or_grid_container());
        if node_id == root_id || placed_by_inline_engine || blockified_item {
            0.0
        } else {
            vertical_align_shift_px(cv.vertical_align, cv.display, parent_font_size)
        }
    }

    /// Whether the element of `inputs` is placed in a column of a multicol
    /// container, and the offsets that paint adds to the position of its
    /// box for its relative insets.
    fn paint_inset(
        &self,
        inputs: &BoxInputs,
        node: &Node,
        cv: &ComputedValues,
    ) -> (bool, (f32, f32)) {
        let BoxInputs {
            node_id,
            fragment_id,
            is_fragment_visit,
            ..
        } = *inputs;
        let (document, cascade, root_id, layout_fragments) = (
            self.document,
            self.cascade,
            self.root_id,
            self.layout_fragments,
        );
        // Taffy and the IFC block/atomic placement paths already apply
        // relative insets. Floats, the boxes a multicol container
        // places in its columns and manually placed table parts still
        // use paint-side insets. The container's own fragment and the
        // fragment of a box deeper inside a column child stand where
        // block flow put the box, inset included.
        let placed_in_column = is_fragment_visit
            && fragment_id.is_some_and(|id| {
                layout_fragments[id].parent.is_some_and(|parent| {
                    document
                        .get_node(layout_fragments[parent].node_id)
                        .is_some_and(|owner| owner.is_multicol_container())
                })
            });
        let needs_paint_inset = raikiri_dom::placed_without_relative_offset(
            document,
            cascade,
            Some(root_id),
            node_id,
            cv,
        ) || (placed_in_column && !cv.display.is_inline_level());
        let offset = if needs_paint_inset {
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
        (placed_in_column, offset)
    }

    /// How far a viewport-fixed box laid out as `layout` at (`abs_x`,
    /// `abs_y`) moves to stand at its fixed position on the page.
    fn fixed_offset(
        &self,
        cv: &ComputedValues,
        layout: &BoxLayout,
        (abs_x, abs_y): (f32, f32),
    ) -> (f32, f32) {
        let PageGeometry {
            content_height,
            fixed_rtl,
            fixed_content_width,
            fixed_flow_shift_x,
            fixed_origin_y,
            ..
        } = self.geometry;
        fixed_position_px(
            cv,
            layout,
            (abs_x, abs_y),
            fixed_rtl,
            self.page_box.width,
            fixed_origin_y,
            fixed_content_width,
            content_height,
        )
        .map(|(x, y)| (x - abs_x - fixed_flow_shift_x, y - abs_y))
        .unwrap_or((0.0, 0.0))
    }

    /// Move the scene by the transform of the element, and push the frame
    /// that restores it. Give `false` when the transform hides the element
    /// and its subtree.
    fn push_transform(
        &mut self,
        cv: &ComputedValues,
        layout: BoxLayout,
        paint_x: f32,
        paint_y: f32,
    ) -> bool {
        let matrix = element_transform(cv, paint_x, paint_y, layout.size.width, layout.size.height);
        // spec: https://www.w3.org/TR/css-transforms-1/#transform-function-lists
        // A singular matrix hides the element and its subtree.
        if matrix.determinant() == 0.0 || !matrix.as_coeffs().iter().all(|value| value.is_finite())
        {
            return false;
        }
        self.stack
            .push(PaintFrame::RestoreTransform(self.scene.transform));
        self.scene.transform *= matrix;
        true
    }

    /// Open the clips of the fragmentainers that cut the box of the element
    /// `inputs.node_id`, and say how far they cut it.
    fn cut_by_fragmentainers(
        &mut self,
        inputs: &BoxInputs,
        layout: BoxLayout,
        paint_x: f32,
        paint_y: f32,
    ) -> FragmentCut {
        let BoxInputs {
            node_id,
            fragment_clip_height,
            fragment_clip,
            fragment_id,
            ..
        } = *inputs;
        let image_cache = self.image_cache;
        let open_edges = fragment_id
            .and_then(|id| self.layout_fragments.get(id))
            .map_or(
                OpenBlockEdges {
                    top: false,
                    bottom: false,
                },
                OpenBlockEdges::of,
            );
        let scene = &mut *self.scene;
        let trace = &mut self.trace;
        let mut paint_y = paint_y;
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
            // Layout does not break the box across the columns, so they cut it
            // off at the bottom, where the container ends. They clip nothing
            // else: what of the box, its descendants, its outline and its
            // shadows overflows it at the top and at the sides shows, as the
            // overflow of any box in a column does (CSS Multi-column 1,
            // section 8). The clip is as open there as the clip of a box whose
            // overflow is visible on that axis.
            //
            // A box they cut off at the bottom has no bottom edge for the
            // outline to follow, and a box with no area paints no outline.
            let cv = &self.cascade.computed[node_id];
            let outset = f64::from(outline_outset(
                layout.size.width,
                layout.size.height,
                &cv.outline,
                cv.outline_offset,
            ));
            let bottom_outset = if layout.size.height <= clip_height {
                outset
            } else {
                0.0
            };
            let page = page_bounds_in_scene(scene.transform, &self.page_box);
            // A box that ends above the page has nothing to show on it.
            let bottom = ((clip_origin_y + clip_height) as f64 + bottom_outset).max(page.y0);
            let clip = Rect::new(page.x0, page.y0, page.x1, bottom);
            let clip = snap_column_clip_outward(
                clip,
                image_cache.device_grid(scene.transform),
                // The column clip of a box that fills the columns: its
                // block edges are the column's.
                OpenBlockEdges {
                    top: false,
                    bottom: false,
                },
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
                open_edges,
            );
            scene.push_clip_layer(Affine::IDENTITY, &clip);
            fragmentainer_clip_pushed = true;
            if let Some(t) = trace.as_deref_mut() {
                t.push(crate::PaintTraceEvent::PushFragmentainerClip(node_id));
            }
        }
        FragmentCut {
            paint_y,
            paint_height,
            paint_background_height,
            multicol_clip_pushed,
            fragmentainer_clip_pushed,
        }
    }

    /// Whether the box of the element `inputs.node_id` paints on this page,
    /// and the extent it paints in, which a box that continues from an
    /// earlier page or breaks into the next one stretches or moves.
    fn page_extent(
        &self,
        inputs: &BoxInputs,
        node: &'w Node,
        layout: BoxLayout,
        placement: &Placement<'w>,
        cut: &FragmentCut,
    ) -> PageExtent {
        let FragmentCut {
            paint_y,
            paint_height,
            paint_background_height,
            ..
        } = *cut;
        let mut extent = PageExtent {
            paint_y,
            paint_height,
            paint_background_height,
            paints_on_page: self.paints_on_page(inputs, node, layout, placement, paint_y),
            paints_as_absolute_continuation: false,
            // Where the box's border box starts on this page's coordinates
            // before it is cut into a continuation fragment.
            unfragmented_top: paint_y,
        };
        if is_absolute_fragment_candidate(placement.cv) {
            self.continue_absolute_fragment(inputs.node_id, node, placement, &mut extent);
            self.extend_absolute_fragment(node, placement, &mut extent);
        }
        extent
    }

    /// Whether the box of the element `inputs.node_id`, laid out as `layout`
    /// and placed by `placement`, paints on this page once the clips of the
    /// fragmentainers have cut its top down to `paint_y`.
    fn paints_on_page(
        &self,
        inputs: &BoxInputs,
        node: &'w Node,
        layout: BoxLayout,
        placement: &Placement<'w>,
        paint_y: f32,
    ) -> bool {
        let BoxInputs {
            node_id,
            inside_fixed,
            ..
        } = *inputs;
        let Placement {
            cv,
            abs_y,
            fixed_in_viewport,
            paint_x,
            ..
        } = *placement;
        let scene_transform = self.scene.transform;
        let PageGeometry {
            page_top,
            page_bottom,
            flow_band,
            ..
        } = self.geometry;
        // A hidden table must not paint its own collapsed border.
        let visibility_hidden_table = raikiri_dom::paint_rules::is_visibility_hidden_table(cv);
        !visibility_hidden_table
            && (node_id == self.root_id
                || ((fixed_in_viewport || self.named_page_matches(node_id))
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
                                    (paint_x + grid.x, paint_y + grid.y, grid.width, grid.height)
                                },
                            );
                            band.holds_box(scene_transform, own_box)
                        })))))
    }

    /// Whether the page named by the `page` property of the element
    /// `node_id` is the page being painted.
    fn named_page_matches(&self, node_id: usize) -> bool {
        raikiri_dom::paint_rules::named_page_matches(
            self.document,
            self.cascade,
            node_id,
            self.active_page_name,
        )
    }

    /// Give the box of the element `node_id` the continuation of its border
    /// on this page when it has no part here but a child fragment of it
    /// does.
    fn continue_absolute_fragment(
        &self,
        node_id: usize,
        node: &Node,
        placement: &Placement<'w>,
        extent: &mut PageExtent,
    ) {
        let Placement {
            cv,
            abs_y,
            pos_dy,
            child_transform_y,
            ..
        } = *placement;
        let PageGeometry {
            margins,
            insets,
            page_top,
            page_bottom,
            flow_band,
            ..
        } = self.geometry;
        // Continuations cut boxes along y; a page that follows the x
        // axis holds only the boxes across its page area.
        if !extent.paints_on_page
            && flow_band.is_none()
            && let Some(descendant_end) = in_flow_child_end(
                self.document,
                self.cascade,
                node,
                abs_y,
                |child_y, child_height| {
                    box_intersects_page(child_y, child_height, page_top, page_bottom)
                        .then_some(child_y + child_height)
                },
            )
        {
            let fragment_top = page_top - margins.top - insets.top;
            let continuation_height =
                (descendant_end - fragment_top + cv.border.bottom.width().px()).max(0.0);
            if continuation_height > 0.0 {
                extent.paint_y =
                    fragment_top + margins.top + insets.top - page_top + pos_dy + child_transform_y;
                extent.paint_height = continuation_height;
                // The local reftest oracle carries only the
                // continuation border; the ancestor background is
                // not repeated behind the child fragment.
                extent.paint_background_height = 0.0;
                extent.paints_on_page = self.named_page_matches(node_id);
                extent.paints_as_absolute_continuation = extent.paints_on_page;
            }
        }
    }

    /// Stretch the box of the element `node`, which paints on this page, to
    /// the page boundary when a direct in-flow child starts on the next page.
    ///
    /// The first fragment of the same containing block extends that far. Its
    /// normal layout height only covers the first one-pass OOF content box,
    /// so the border and background continue through the break without
    /// changing descendant coordinates.
    fn extend_absolute_fragment(
        &self,
        node: &Node,
        placement: &Placement<'w>,
        extent: &mut PageExtent,
    ) {
        let Placement { cv, abs_y, .. } = *placement;
        let page_bottom = self.geometry.page_bottom;
        if extent.paints_on_page
            && !extent.paints_as_absolute_continuation
            && let Some(descendant_end) = in_flow_child_end(
                self.document,
                self.cascade,
                node,
                abs_y,
                |child_y, child_height| (child_y >= page_bottom).then_some(child_y + child_height),
            )
        {
            extent.paint_height = extent
                .paint_height
                .max((descendant_end - abs_y + cv.border.bottom.width().px()).max(0.0));
            // Keep the element background inside the current page's
            // content extent.  The border continuation is painted
            // separately and may reach the page edge.
            extent.paint_background_height = (page_bottom - abs_y).max(0.0);
        }
    }
}

/// Whether `cv` is an absolutely positioned box with a border and a
/// background that a child fragment on another page can continue.
///
/// An absolutely positioned containing block can own a child fragment on a
/// later page even when its first layout box ends before that page. The
/// parent's border and background continue for this narrow OOF shape. Full
/// OOF fragmentation remains outside this painter, so the gate leaves
/// ordinary flow, fixed-position, transformed, and borderless absolute boxes
/// alone.
fn is_absolute_fragment_candidate(cv: &ComputedValues) -> bool {
    matches!(cv.position, PositionValue::Absolute)
        && cv.transform.is_empty()
        && (cv.border.top.width().px() > 0.0
            || cv.border.right.width().px() > 0.0
            || cv.border.bottom.width().px() > 0.0
            || cv.border.left.width().px() > 0.0)
        && cv.background_color.a != 0
}

/// How far the direct text children of `node` reach along the line, from its
/// content box.
///
/// Direct text children are the only inline descendants whose current minimal
/// layout path has a reliable horizontal box. Their right edge places
/// `::after`; element children stay on the existing block-flow path until a
/// full IFC lands.
fn generated_content_advance(
    document: &Document,
    node: &Node,
    cv: &ComputedValues,
    paint_padding: Edges<f32>,
) -> f32 {
    node.children
        .iter()
        .filter_map(|child_id| document.get_node(*child_id))
        .filter(|child| child.kind() == NodeKind::Text)
        .map(|child| {
            (child.layout().location.x + child.layout().size.width
                - cv.border.left.width().px()
                - paint_padding.left)
                .max(0.0)
        })
        .fold(0.0, f32::max)
}

/// The farthest bottom edge, among the in-flow children of `node` laid out
/// from `abs_y`, that `end` reports for the top and height of a child.
fn in_flow_child_end(
    document: &Document,
    cascade: &CascadeResult,
    node: &Node,
    abs_y: f32,
    end: impl Fn(f32, f32) -> Option<f32>,
) -> Option<f32> {
    node.children
        .iter()
        .filter_map(|child_id| {
            let child = document.get_node(*child_id)?;
            let child_cv = cascade.computed.get(*child_id)?;
            if !matches!(
                child_cv.position,
                PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
            ) {
                return None;
            }
            let child_y = abs_y + child.layout().location.y;
            end(child_y, child.layout().size.height)
        })
        .fold(None, |maximum, value| {
            Some(maximum.map_or(value, |current: f32| current.max(value)))
        })
}
