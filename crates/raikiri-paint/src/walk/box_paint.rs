//! What an element paints of its own box: its decorations, replaced content,
//! column rules, list marker and generated `::before` content.

use super::element_box::ElementBox;
use super::page_geometry::PageGeometry;
use super::paint_walk::PaintWalk;
use super::{
    BackgroundEnv, InlineSvg, background_padding_for_paint, border_hides_background_edge,
    color_only_background_layer, fixed_background_viewport, img_src_url, infer_url_color,
    paint_canvas, paint_element_background, paint_element_border_image,
    paint_element_border_rounded, paint_element_border_with_top, paint_element_box_shadows,
    paint_element_outline, paint_generated_pseudo, paint_image, paint_list_marker_with_snapshots,
    paint_outline_break_bars, push_overflow_clip, solid_outline,
};
use anyrender::PaintScene;
use kurbo::{Affine, BezPath, Rect};
use raikiri_style::property::{
    BackgroundImage, BorderCollapseValue, BorderColor, BorderStyle, DisplayValue, PositionValue,
    VisualBox,
};
use raikiri_style::{ComputedBorder, ComputedValues, Sides};
use raikiri_traits::{PaintInsets, PaintRect};

/// The geometry that the decorations of a box paint in: its own box, which
/// for a table is its grid without the captions.
#[derive(Clone, Copy)]
struct OwnBox {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    background_height: f32,
}

/// How the box of an element is cut at the page area on one page.
#[derive(Clone, Copy)]
struct PageCut {
    /// The page area, in page coordinates.
    area: Rect,
    /// The box comes from the page before: it starts above the page area on a
    /// page that is not the first.
    from_before: bool,
    /// The box goes on to the next page.
    after: bool,
}

/// The clip that cuts the rectangle `(x, y, width, height)` at the edges of
/// `area` that it leaves by more than rounding error, or `None` when it stays
/// inside. `top_cuts` tells whether the top edge cuts at all: an absolutely
/// positioned box above the page area of the first page is not cut there.
///
/// A rectangle that stays inside needs no clip, which would only change the
/// anti-aliasing of its edges.
fn page_area_clip(
    (x, y, width, height): (f32, f32, f32, f32),
    area: Rect,
    top_cuts: bool,
) -> Option<Rect> {
    const TOLERANCE: f64 = 0.01;
    const UNBOUNDED: f64 = 1.0e7;
    let left = f64::from(x) < area.x0 - TOLERANCE;
    let top = top_cuts && f64::from(y) < area.y0 - TOLERANCE;
    let right = f64::from(x + width) > area.x1 + TOLERANCE;
    let bottom = f64::from(y + height) > area.y1 + TOLERANCE;
    (left || top || right || bottom).then(|| {
        Rect::new(
            if left { area.x0 } else { -UNBOUNDED },
            if top { area.y0 } else { -UNBOUNDED },
            if right { area.x1 } else { UNBOUNDED },
            if bottom { area.y1 } else { UNBOUNDED },
        )
    })
}

/// The full winning lines paint around the cell's logical border centers.
pub(super) fn collapsed_block_border_paint_rect(
    node: &raikiri_dom::Node,
    rect: PaintRect,
    border: &Sides<ComputedBorder>,
) -> PaintRect {
    if !node.centered_collapsed_block_borders() || rect.height <= 0.0 {
        return rect;
    }
    let top = border.top.width().px() / 2.0;
    let bottom = border.bottom.width().px() / 2.0;
    PaintRect::new(rect.x, rect.y - top, rect.width, rect.height + top + bottom)
}

/// The clip that the content of the replaced element `element` paints inside.
fn replaced_content_clip(element: &ElementBox<'_>) -> Option<raikiri_traits::PaintClip> {
    let ElementBox {
        cv,
        layout,
        paint_x,
        paint_y,
        ..
    } = *element;
    raikiri_dom::paint_rules::replaced_content_clip(
        cv,
        PaintRect::new(paint_x, paint_y, layout.size.width, layout.size.height),
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
}

/// Whether an opaque background in the same solid color as every border side
/// already paints the complete visible union of the border in
/// `paint_element_background`.
fn border_covered_by_solid_background(
    cv: &ComputedValues,
    painted_border: &Sides<ComputedBorder>,
) -> bool {
    !cv.has_background_image()
        && cv.background_color.a == 255
        && matches!(
            cv.final_background_clip(),
            VisualBox::PaddingBox | VisualBox::ContentBox
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
        })
}

/// Whether `cv` is an internal table box in the collapsing border model,
/// where border images do not apply (CSS Backgrounds 3 §6.1).
fn collapsed_internal_table_box(cv: &ComputedValues) -> bool {
    matches!(cv.border_collapse, BorderCollapseValue::Collapse)
        && matches!(
            cv.display,
            DisplayValue::TableRowGroup
                | DisplayValue::TableHeaderGroup
                | DisplayValue::TableFooterGroup
                | DisplayValue::TableRow
                | DisplayValue::TableColumnGroup
                | DisplayValue::TableColumn
                | DisplayValue::TableCell
        )
}

impl<'w, S: PaintScene> PaintWalk<'w, S> {
    /// Paint the parts of the box of `element` in their order, and return
    /// the advance of its generated `::before` content.
    pub(super) fn paint_box(&mut self, element: &ElementBox<'w>, has_column_rules: bool) -> f32 {
        if element.paints_on_page
            && !raikiri_dom::paint_rules::hides_empty_table_cell(
                self.document,
                self.cascade,
                element.node_id,
            )
        {
            self.paint_box_decorations(element);
            self.paint_replaced_content(element);
            self.paint_column_rules(element, has_column_rules);
            self.paint_list_marker(element);
            self.paint_generated_before(element, has_column_rules)
        } else {
            0.0
        }
    }

    /// Paint the background, shadows, border and outline of the box of `element`, and trace it.
    pub(super) fn paint_box_decorations(&mut self, element: &ElementBox<'w>) {
        let ElementBox {
            node,
            node_id,
            cv,
            layout,
            generates_box,
            paint_x,
            paint_y,
            paint_height,
            paint_background_height,
            ..
        } = *element;
        if let Some(t) = self.trace.as_deref_mut()
            && generates_box
            && !raikiri_dom::paint_rules::hides_own_box(cv)
        {
            t.push(crate::PaintTraceEvent::Box(node_id));
        }
        let grid = node.table_grid_box();
        let own = OwnBox {
            x: paint_x + grid.map_or(0.0, |rect| rect.x),
            y: paint_y + grid.map_or(0.0, |rect| rect.y),
            width: grid.map_or(layout.size.width, |rect| rect.width),
            height: grid.map_or(paint_height, |rect| rect.height),
            background_height: grid.map_or(paint_background_height, |rect| rect.height),
        };
        // The box's own geometry, which its outline follows.
        let (fragment_paint_y, fragment_paint_height) = (own.y, own.height);
        let (own, sliced) = self.open_block_slice(element, own);
        // A `display: contents` element has no box whose
        // background, borders or outline could paint; its
        // generated content still does. Neither does a box that
        // is not visible, whose descendants may still be.
        let paints_own_box = generates_box && !raikiri_dom::paint_rules::hides_own_box(cv);
        if paints_own_box {
            self.paint_background(element, &own);
        }
        self.paint_borders(element, &own, paints_own_box);
        let scene = &mut *self.scene;
        if sliced {
            scene.pop_layer();
        }
        // Outlines do not consume box-model space and are painted
        // outside the border edge (CSS Basic UI §4). Each box of a
        // sliced box draws a whole outline around itself, like the
        // pieces of an inline box.
        if paints_own_box {
            self.paint_outline(
                element,
                (own.x, fragment_paint_y, own.width, fragment_paint_height),
            );
        }
    }

    /// Paint the outline around the box `(x, y, width, height)` of `element`.
    ///
    /// On a page it is cut at the page area, and where a page break cuts the
    /// box the outline is a bar along the edge of the page area, inside it, as
    /// in Chromium (see [`paint_outline_break_bars`]).
    fn paint_outline(
        &mut self,
        element: &ElementBox<'w>,
        (x, y, width, height): (f32, f32, f32, f32),
    ) {
        let cv = element.cv;
        let Some(solid) = solid_outline(&cv.outline, cv.outline_offset, cv.color) else {
            return;
        };
        let outset = solid.width + solid.offset;
        // The ring lies `outset` outside the box. It is cut at the edges of
        // the page area that it leaves.
        let ring = (
            x - outset,
            y - outset,
            width + 2.0 * outset,
            height + 2.0 * outset,
        );
        // The page cut of the box, where the page area clips the ring.
        let cut = self.page_cut(element).filter(|cut| {
            width > 0.0 && height > 0.0 && page_area_clip(ring, cut.area, true).is_some()
        });
        let clip = cut.and_then(|cut| page_area_clip(ring, cut.area, true));
        if let Some(clip) = clip {
            self.scene
                .scene
                .push_clip_layer(self.geometry.page_clip_transform, &clip);
        }
        let scene = &mut *self.scene;
        paint_element_outline(
            scene,
            width,
            height,
            x,
            y,
            &cv.outline,
            cv.outline_offset,
            cv.color,
        );
        if let Some(cut) = cut {
            paint_outline_break_bars(
                scene,
                (x, y, width, height),
                &solid,
                (
                    cut.from_before.then_some(cut.area.y0),
                    cut.after.then_some(cut.area.y1),
                ),
            );
            scene.pop_layer();
        }
    }

    /// Clip the decorations of `element` to the slice it paints, when it is
    /// one slice of a box broken across outer columns, and give the extent of
    /// the whole box it paints in, and whether it clipped.
    fn open_block_slice(&mut self, element: &ElementBox<'w>, own: OwnBox) -> (OwnBox, bool) {
        let ElementBox {
            node_id,
            cv,
            fragment_id,
            ..
        } = *element;
        let document = self.document;
        let scene = &mut *self.scene;
        let OwnBox {
            x: own_paint_x,
            y: own_paint_y,
            width: own_paint_width,
            height: own_paint_height,
            background_height: own_background_height,
        } = own;
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
        (
            OwnBox {
                x: own_paint_x,
                y: own_paint_y,
                width: own_paint_width,
                height: own_paint_height,
                background_height: own_background_height,
            },
            block_slice.is_some(),
        )
    }

    /// Paint the shadows and the background of the box `own` of `element`.
    fn paint_background(&mut self, element: &ElementBox<'w>, own: &OwnBox) {
        let ElementBox {
            node_id,
            cv,
            paint_border_radius,
            ..
        } = *element;
        let OwnBox {
            x: own_paint_x,
            y: own_paint_y,
            width: own_paint_width,
            height: own_paint_height,
            ..
        } = *own;
        let document = self.document;
        let clipped_to_page_content = self.push_background_page_clip(element);
        paint_element_box_shadows(
            &mut *self.scene,
            own_paint_width,
            own_paint_height,
            own_paint_x,
            own_paint_y,
            &paint_border_radius,
            &cv.box_shadow,
            cv.color,
        );
        let table_part_clip = document.anonymous_table_part_background_cells(node_id);
        if let Some(cells) = table_part_clip {
            self.push_table_part_clip(cells, own);
        }
        // Propagation transfers only the background to the canvas;
        // shadows and borders still belong to the element's box.
        if self.canvas_owner != Some(node_id) {
            self.paint_background_layers(element, own);
        }
        if table_part_clip.is_some() {
            self.scene.pop_layer();
        }
        if clipped_to_page_content {
            self.scene.pop_layer();
        }
    }

    /// Clip the shadows and the background of `element` to the page content
    /// box, if they are clipped to it, and return whether a clip was pushed.
    fn push_background_page_clip(&mut self, element: &ElementBox<'w>) -> bool {
        let ElementBox {
            cv,
            inside_fixed,
            fixed_in_viewport,
            ..
        } = *element;
        let PageGeometry {
            page_clip_transform,
            page_content_clip,
            ..
        } = self.geometry;
        // Paint element background (CSS Backgrounds 3 §2.2).
        // Shift applies to the box itself per CSS 2.1 §10.8.1.
        let clip_background = page_content_clip
            .filter(|_| {
                !inside_fixed
                    && !fixed_in_viewport
                    && matches!(
                        cv.position,
                        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                    )
            })
            .or_else(|| self.absolute_page_cut(element));
        if let Some(clip) = clip_background {
            self.scene.scene.push_clip_layer(page_clip_transform, &clip);
        }
        clip_background.is_some()
    }

    /// Clip the border of `element`, which paints in `border_box`, at the
    /// edges of the page area that it leaves, and return whether a clip was
    /// pushed.
    ///
    /// An absolutely positioned box is cut at the top only where it comes
    /// from the page before, as its background is.
    fn push_border_page_clip(
        &mut self,
        element: &ElementBox<'w>,
        border_box: (f32, f32, f32, f32),
    ) -> bool {
        let absolute = matches!(element.cv.position, PositionValue::Absolute);
        let clip = self.page_cut(element).and_then(|cut| {
            page_area_clip(
                border_box,
                cut.area,
                if absolute { cut.from_before } else { true },
            )
        });
        if let Some(clip) = clip {
            self.scene
                .scene
                .push_clip_layer(self.geometry.page_clip_transform, &clip);
        }
        clip.is_some()
    }

    /// What the absolutely positioned box of `element` is clipped to on this
    /// page, or `None` when it is not cut.
    ///
    /// An absolutely positioned box is laid out whole, not broken at the
    /// pages. What it paints is cut to the page area where the box goes on to
    /// the next page or comes from the page before, as each fragment of a
    /// broken box is (CSS Fragmentation 3, section 5.4). On the first page a
    /// box above the page area lies in the margin, and is not cut.
    fn absolute_page_cut(&self, element: &ElementBox<'w>) -> Option<Rect> {
        const UNBOUNDED: f64 = 1.0e7;
        if !matches!(element.cv.position, PositionValue::Absolute) {
            return None;
        }
        let PageCut {
            area,
            from_before,
            after,
        } = self.page_cut(element)?;
        (from_before || after).then(|| {
            Rect::new(
                -UNBOUNDED,
                if from_before { area.y0 } else { -UNBOUNDED },
                UNBOUNDED,
                if after { area.y1 } else { UNBOUNDED },
            )
        })
    }

    /// Where the box of `element` lies against the page area on this page, or
    /// `None` for a box that the page area does not cut: a fixed one, or one
    /// on a page that follows the x axis.
    fn page_cut(&self, element: &ElementBox<'w>) -> Option<PageCut> {
        let ElementBox {
            cv,
            node_id,
            inside_fixed,
            fixed_in_viewport,
            paint_y,
            paint_height,
            child_transform_y,
            ..
        } = *element;
        let PageGeometry {
            page_content_clip,
            page_top,
            flow_band,
            ..
        } = self.geometry;
        let area = page_content_clip?;
        if inside_fixed
            || fixed_in_viewport
            || flow_band.is_some()
            || matches!(cv.position, PositionValue::Fixed)
        {
            return None;
        }
        let from_before = page_top > 0.0 && f64::from(paint_y) < area.y0 - 0.01;
        let after = f64::from(paint_y + paint_height) > area.y1 + 0.01;
        // A relatively positioned box lies where its offset puts it, not
        // where a break cut it, and neither does a box that a transform or the
        // relative offset of an ancestor moved. Only a box that crosses an
        // edge of the page area needs the check.
        let broken = (from_before || after)
            && match cv.position {
                PositionValue::Absolute => true,
                PositionValue::Static => {
                    child_transform_y == 0.0 && !self.moved_by_relative_ancestor(node_id)
                }
                _ => false,
            };
        Some(PageCut {
            area,
            from_before: broken && from_before,
            after: broken && after,
        })
    }

    /// Whether a relative offset of an ancestor of the node `node_id` moves
    /// it vertically.
    fn moved_by_relative_ancestor(&self, node_id: usize) -> bool {
        self.document.ancestors(node_id).any(|ancestor| {
            let cv = &self.cascade.computed[ancestor];
            raikiri_dom::used_relative_offset(self.document, self.cascade, cv, ancestor).1 != 0.0
        })
    }

    /// Clip to the cell areas of a source row or row group, which the box
    /// `own` of the part positions.
    fn push_table_part_clip(&mut self, cells: &[PaintRect], own: &OwnBox) {
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
        self.scene.scene.push_clip_layer(
            Affine::translate((f64::from(own.x), f64::from(own.y))),
            &path,
        );
    }

    /// Paint the background color and layers of the box `own` of `element`.
    fn paint_background_layers(&mut self, element: &ElementBox<'w>, own: &OwnBox) {
        let ElementBox {
            cv,
            layout,
            paint_padding,
            paint_border_radius,
            painted_border,
            ..
        } = *element;
        let OwnBox {
            x: own_paint_x,
            y: own_paint_y,
            width: own_paint_width,
            background_height: own_background_height,
            ..
        } = *own;
        let paint_padding = background_padding_for_paint(cv, &layout, paint_padding);
        let env = self.background_env(element);
        paint_element_background(
            self.image_cache,
            &mut *self.scene,
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
            env,
            self.pixel_source,
            &mut *self.warnings,
        );
    }

    /// What surrounds the box of `element` and shapes how its background is
    /// painted.
    fn background_env(&self, element: &ElementBox<'w>) -> BackgroundEnv {
        let ElementBox {
            cv,
            inside_fixed_containing_block,
            establishes_fixed_containing_block,
            painted_border,
            ..
        } = *element;
        BackgroundEnv {
            border_hides_edge: matches!(cv.border_image.source, BackgroundImage::None)
                && border_hides_background_edge(painted_border, cv.color),
            border_insets: element
                .node
                .centered_collapsed_block_borders()
                .then_some(element.layout.border),
            device_grid: self.image_cache.device_grid(self.scene.transform),
            fixed_viewport: fixed_background_viewport(
                self.geometry.page_area,
                self.root_transform,
                inside_fixed_containing_block || establishes_fixed_containing_block,
            ),
        }
    }

    /// Paint the border of the box `own` of `element` over its background,
    /// or its border image, which replaces the border styles.
    fn paint_borders(&mut self, element: &ElementBox<'w>, own: &OwnBox, paints_own_box: bool) {
        let ElementBox {
            cv,
            paint_border_radius,
            painted_border,
            paints_as_absolute_continuation,
            ..
        } = *element;
        let OwnBox {
            x: own_paint_x,
            y: own_paint_y,
            width: own_paint_width,
            height: own_paint_height,
            ..
        } = *own;
        let border_box = collapsed_block_border_paint_rect(
            element.node,
            PaintRect::new(own_paint_x, own_paint_y, own_paint_width, own_paint_height),
            painted_border,
        );
        let (own_paint_y, own_paint_height) = (border_box.y, border_box.height);
        // The border is cut at the page area where it leaves it.
        let clipped_to_page_content = paints_own_box
            && self.push_border_page_clip(
                element,
                (
                    border_box.x,
                    border_box.y,
                    border_box.width,
                    border_box.height,
                ),
            );
        // A border image replaces the border styles (CSS
        // Backgrounds 3 §6).
        let border_image_drawn = paints_own_box
            && !collapsed_internal_table_box(cv)
            && self.paint_border_image(element, own);
        let border_covered_by_background = (!element.node.centered_collapsed_block_borders()
            && border_covered_by_solid_background(cv, painted_border))
            || border_image_drawn;
        let scene = &mut *self.scene;
        // Paint border on top of background (CSS Backgrounds 3 §5).
        let paints_continuation_border = paints_own_box && paints_as_absolute_continuation;
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
        if clipped_to_page_content {
            scene.pop_layer();
        }
    }

    /// Paint the border image of the box `own` of `element`, and return
    /// whether one was drawn.
    fn paint_border_image(&mut self, element: &ElementBox<'w>, own: &OwnBox) -> bool {
        let ElementBox {
            cv,
            painted_border,
            paints_as_absolute_continuation,
            unfragmented_top,
            ..
        } = *element;
        let OwnBox {
            x: own_paint_x,
            y: own_paint_y,
            width: own_paint_width,
            height: own_paint_height,
            ..
        } = *own;
        let (image_cache, pixel_source) = (self.image_cache, self.pixel_source);
        let scene = &mut *self.scene;
        let warnings = &mut *self.warnings;
        if paints_as_absolute_continuation && unfragmented_top < own_paint_y {
            // A continuation fragment slices the image laid
            // out over the whole box, so it shows no top edge
            // at the break (CSS Fragmentation 3 §5.4).
            const UNBOUNDED: f64 = 1.0e7;
            scene.push_clip_layer(
                Affine::IDENTITY,
                &Rect::new(-UNBOUNDED, f64::from(own_paint_y), UNBOUNDED, UNBOUNDED),
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
        }
    }

    /// Paint the content of the replaced element `element`: an inline SVG, a canvas or an image, inside its overflow clip, with the stand-in for an image that did not paint.
    pub(super) fn paint_replaced_content(&mut self, element: &ElementBox<'w>) {
        let ElementBox { node, node_id, .. } = *element;
        // Resolve the source's natural dimensions first. SVG
        // sources use the resulting concrete object dimensions
        // for a bounded, size-specific raster request.
        let is_replaced = node.is_inline_svg_root()
            || self.document.is_canvas_element(node_id)
            || node.tag_name() == Some("img");
        // The replaced content paints inside its overflow clip,
        // the same clip the page paint order puts around it.
        let content_clip = is_replaced
            .then(|| replaced_content_clip(element))
            .flatten();
        if let Some(clip) = content_clip {
            let rect = push_overflow_clip(&mut *self.scene, clip, &self.page_box);
            if let Some(t) = self.trace.as_deref_mut() {
                t.push(crate::PaintTraceEvent::PushOverflowClip(
                    node_id,
                    [rect.x0, rect.y0, rect.x1, rect.y1],
                ));
            }
        }
        if is_replaced && let Some(t) = self.trace.as_deref_mut() {
            t.push(crate::PaintTraceEvent::Replaced(node_id));
        }
        let painted_image = self.paint_replaced_source(element);
        self.paint_image_stand_in(element, painted_image);
        // The fallback stands in for the image content, so it
        // stays inside the same clip.
        if content_clip.is_some() {
            if let Some(t) = self.trace.as_deref_mut() {
                t.push(crate::PaintTraceEvent::PopClip);
            }
            self.scene.pop_layer();
        }
    }

    /// Paint the source of the replaced element `element`: an inline SVG, a canvas or an image. Returns whether an image painted.
    fn paint_replaced_source(&mut self, element: &ElementBox<'w>) -> bool {
        let ElementBox {
            node,
            node_id,
            cv,
            layout,
            paint_padding,
            paint_x,
            paint_y,
            ..
        } = *element;
        let (document, cascade, image_cache, pixel_source) = (
            self.document,
            self.cascade,
            self.image_cache,
            self.pixel_source,
        );
        let scene = &mut *self.scene;
        let warnings = &mut *self.warnings;
        if node.is_inline_svg_root() {
            InlineSvg::new(
                document,
                cascade,
                cv,
                node_id,
                (layout.size.width, layout.size.height),
                (paint_x, paint_y),
                &paint_padding,
            )
            .paint(scene, warnings)
        } else if document.is_canvas_element(node_id) {
            let Some(bitmap) = document.canvas_bitmap_snapshot(node_id) else {
                // An unmaterialized canvas is transparent and emits no image.
                return true;
            };
            paint_canvas(
                scene,
                &bitmap,
                layout.size.width,
                layout.size.height,
                paint_x,
                paint_y,
                &cv.border,
                &paint_padding,
                cv.object_fit,
                &cv.object_position,
                !raikiri_dom::paint_rules::hides_own_box(cv),
                image_cache.device_grid(scene.transform),
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
                image_cache.device_grid(scene.transform),
                node_id,
                warnings,
            )
        } else {
            false
        }
    }

    /// Paint the stand-in for the image of `element` when it did not paint (`painted_image` is false): the color that its URL suggests.
    fn paint_image_stand_in(&mut self, element: &ElementBox<'w>, painted_image: bool) {
        let ElementBox {
            node,
            cv,
            layout,
            inside_fixed,
            generates_box,
            paint_padding,
            paint_border_radius,
            fixed_in_viewport,
            paint_x,
            paint_y,
            ..
        } = *element;
        let (image_cache, pixel_source) = (self.image_cache, self.pixel_source);
        let PageGeometry {
            page_clip_transform,
            page_content_clip,
            ..
        } = self.geometry;
        let scene = &mut *self.scene;
        let warnings = &mut *self.warnings;
        let paints_own_box = generates_box && !raikiri_dom::paint_rules::hides_own_box(cv);
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
                        PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
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
    }

    /// Paint the rules between the columns of `element`, inside its overflow clip.
    pub(super) fn paint_column_rules(&mut self, element: &ElementBox<'w>, has_column_rules: bool) {
        let ElementBox {
            node_id,
            cv,
            layout,
            fragment_id,
            paint_x,
            paint_y,
            paint_height,
            ..
        } = *element;
        let (document, cascade, page_box) = (self.document, self.cascade, self.page_box);
        let PageGeometry {
            margins,
            insets,
            content_height,
            ..
        } = self.geometry;
        let scene = &mut *self.scene;
        if has_column_rules && !raikiri_dom::paint_rules::hides_own_box(cv) {
            // Rules and markers obey the owner's overflow independently,
            // preserving their order before the owner's inline content.
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
            for mut rule in document.fragment_column_rules(cascade, node_id, fragment_id) {
                rule.rect.x += paint_x;
                rule.rect.y += paint_y;
                rule.pattern_origin += paint_y;
                let content_top = margins.top + insets.top;
                let start = rule.rect.y.max(content_top);
                let end = (rule.rect.y + rule.rect.height).min(content_top + content_height);
                rule.rect.y = start;
                rule.rect.height = (end - start).max(0.0);
                crate::column_rules::paint(scene, rule);
            }
            if rule_clip.is_some() {
                scene.pop_layer();
            }
        }
    }

    /// Paint the generated marker of the list item `element`.
    pub(super) fn paint_list_marker(&mut self, element: &ElementBox<'w>) {
        let ElementBox {
            node,
            node_id,
            cv,
            layout,
            fragment_id,
            inside_fixed,
            fixed_in_viewport,
            paint_x,
            paint_y,
            paint_height,
            paints_as_absolute_continuation,
            ..
        } = *element;
        let (
            document,
            cascade,
            counter_snapshots,
            image_cache,
            pixel_source,
            content_origin_y,
            layout_fragments,
        ) = (
            self.document,
            self.cascade,
            self.counter_snapshots,
            self.image_cache,
            self.pixel_source,
            self.content_origin_y,
            self.layout_fragments,
        );
        let PageGeometry {
            page_bottom,
            page_offset_y,
            ..
        } = self.geometry;
        let scene = &mut *self.scene;
        // Prepared outside images follow the page of their first line;
        // other markers belong to the first principal fragment, regardless
        // of inherited ancestor fragment state.
        if raikiri_dom::generated_content::inside_marker_in_flow(cascade, node_id)
            && node.is_ifc_root()
        {
            return;
        }
        if !paints_as_absolute_continuation
            && cv.display == DisplayValue::ListItem
            && (document
                .list_marker_image_top(
                    node_id,
                    (content_origin_y, inside_fixed || fixed_in_viewport),
                    paint_y,
                )
                .is_some_and(|top| top.is_some())
                || fragment_id.is_none_or(|id| {
                    layout_fragments[id].node_id != node_id
                        || layout_fragments[id].fragment_index == 0
                }))
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
            // Clip to the actual owner, including a zero-height empty item.
            // The marker's minimum drawing extent must not enlarge this clip.
            let marker_clip = raikiri_dom::paint_rules::overflow_clip(
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
            if let Some(clip) = marker_clip {
                push_overflow_clip(scene, clip, &self.page_box);
            }
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
            if marker_clip.is_some() {
                scene.pop_layer();
            }
        }
    }

    /// Paint the generated `::before` content of `element` and give its advance.
    pub(super) fn paint_generated_before(
        &mut self,
        element: &ElementBox<'w>,
        has_column_rules: bool,
    ) -> f32 {
        let ElementBox {
            node_id,
            layout,
            paint_height,
            paints_as_absolute_continuation,
            generated_x,
            generated_y,
            ..
        } = *element;
        let (document, cascade, counter_snapshots) =
            (self.document, self.cascade, self.counter_snapshots);
        let scene = &mut *self.scene;
        // Generated content is an immediate child of its
        // originating box.  The minimal layout engine does not
        // allocate an arena node for it, so paint literal
        // `::before` content at the content-box inline start.
        // The returned advance is used by the deferred `::after`
        // frame below to preserve source order for an empty or
        // otherwise unlaid-out originating box.
        if !paints_as_absolute_continuation && !has_column_rules {
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
        }
    }
}
