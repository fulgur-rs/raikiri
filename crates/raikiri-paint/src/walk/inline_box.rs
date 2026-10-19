//! The painting of one piece of an inline element: its shadows, background,
//! borders and outline.

use super::{
    BackgroundEnv, BorderImage, ImageBrushCache, InlineProgression, VisibleArea,
    paint_element_background_layers, paint_element_border_image, paint_element_border_rounded,
    paint_element_box_shadows, paint_element_outline, paintable_border_radius,
};
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use raikiri_style::property::{
    BackgroundImage, BorderImageOutsetSide, BorderImageWidthSide, BorderStyle, Sides,
};
use raikiri_style::{ComputedBorderImage, ComputedBorderRadius, ComputedValues};
use raikiri_traits::{ImagePixelSource, RenderWarning};

/// Paint the shadows, background, borders and outline of one piece of an
/// inline element, the way the element visit paints a box. `x` and `y` are
/// the page position of the paragraph's content box; `progression` gives the
/// axis the element's pieces run along; `transform` is what the piece is
/// painted through inside the root element's transform; `fixed_viewport` is
/// the page area its `fixed` background layers are positioned in, and `None`
/// positions them like `scroll` ones.
///
/// A side whose edge the piece does not carry (the element continues on
/// another line there) has no border, padding or corner radius: the box is
/// sliced (CSS Fragmentation 3 `box-decoration-break: slice`). The padding is
/// what the piece leaves around its content box once the border is taken off.
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_inline_box(
    image_cache: &ImageBrushCache,
    scene: &mut impl PaintScene,
    cv: &ComputedValues,
    piece: &raikiri_dom::InlineBoxPiece,
    background_slice: Option<&crate::ifc_text::BackgroundSlice>,
    progression: InlineProgression,
    transformed: bool,
    x: f32,
    y: f32,
    pixel_source: Option<&dyn ImagePixelSource>,
    warnings: &mut Vec<RenderWarning>,
    transform: Affine,
    fixed_viewport: Option<Rect>,
) {
    let inline_box = InlineBox::new(
        cv,
        piece,
        background_slice,
        progression,
        transformed,
        (x, y),
    );
    let env = InlineBoxEnv {
        image_cache,
        pixel_source,
        transform,
        fixed_viewport,
    };
    inline_box.paint_shadows(scene);
    inline_box.paint_background(scene, &env, warnings);
    if !inline_box.paint_border_image(scene, &env, warnings) {
        inline_box.paint_border(scene);
    }
    inline_box.paint_outline(scene);
}

/// What painting a piece of an inline element reads besides the piece.
struct InlineBoxEnv<'a> {
    image_cache: &'a ImageBrushCache,
    pixel_source: Option<&'a dyn ImagePixelSource>,
    /// What the piece is painted through inside the root element's transform.
    transform: Affine,
    /// The page area `fixed` background layers are positioned in.
    fixed_viewport: Option<Rect>,
}

/// One piece of an inline element: the edges it carries and where it lies on
/// the page and along the axis its element's pieces run along.
struct InlineBox<'a> {
    cv: &'a ComputedValues,
    progression: InlineProgression,
    /// The border box of the piece, in the paragraph.
    outer: raikiri_dom::BoxRect,
    /// The page position of the paragraph's content box.
    x: f32,
    y: f32,
    /// The page position of the piece's border box.
    abs_x: f32,
    abs_y: f32,
    /// The edges at the low (left or top) and high (right or bottom) ends of
    /// the axis.
    low_edge: bool,
    high_edge: bool,
    /// The borders of the sides the piece carries.
    border: Sides<raikiri_style::resolve::ComputedBorder>,
    /// The corner radii of the corners the piece carries.
    radius: ComputedBorderRadius,
    /// The corner radii of the whole element, which a joined box keeps.
    full_radius: ComputedBorderRadius,
    /// What the piece leaves around its content box once the border is taken
    /// off.
    padding: taffy::Rect<f32>,
    /// The start and size of the border box along the axis.
    piece_start: f32,
    piece_size: f32,
    /// The page coordinate the paragraph's boxes are measured from along the
    /// axis.
    along_origin: f32,
    /// The page coordinates of the piece's ends along the axis and across it.
    piece_along: (f64, f64),
    piece_across: (f64, f64),
    has_background_image: bool,
    /// A joined box that reaches past this piece along the axis.
    joined: Option<&'a crate::ifc_text::BackgroundSlice>,
}

impl<'a> InlineBox<'a> {
    /// The piece `piece` of the element with the style `cv`, in the paragraph
    /// whose content box is at `(x, y)`. `transformed` tells whether the
    /// element's transform applies to the piece.
    fn new(
        cv: &'a ComputedValues,
        piece: &raikiri_dom::InlineBoxPiece,
        background_slice: Option<&'a crate::ifc_text::BackgroundSlice>,
        progression: InlineProgression,
        transformed: bool,
        (x, y): (f32, f32),
    ) -> Self {
        let outer = piece.border_box;
        let content = piece.content_box;
        // With `box-decoration-break: clone` every fragment is drawn as a whole
        // box (CSS Fragmentation 3 §5.4): the inline layout gives each fragment
        // both edges, and its background is laid out over its own box.
        let clone = cv.box_decoration_break == raikiri_style::property::BoxDecorationBreak::Clone;
        let background_slice = background_slice.filter(|_| !clone);
        // The edges at the low (left or top) and high (right or bottom) ends of
        // the axis. The start edge is at the low end when the element's own
        // direction is left to right, whatever the paragraph's direction.
        let (low_edge, high_edge) = if progression.reversed {
            (piece.has_end_edge, piece.has_start_edge)
        } else {
            (piece.has_start_edge, piece.has_end_edge)
        };
        let has_background_image = cv.has_background_image();
        let (border, radius, full_radius) =
            carried_edges(cv, progression, (low_edge, high_edge), transformed);
        let padding = taffy::Rect {
            left: (content.x - outer.x - border.left.width().px()).max(0.0),
            right: ((outer.x + outer.width)
                - (content.x + content.width)
                - border.right.width().px())
            .max(0.0),
            top: (content.y - outer.y - border.top.width().px()).max(0.0),
            bottom: ((outer.y + outer.height)
                - (content.y + content.height)
                - border.bottom.width().px())
            .max(0.0),
        };
        let abs_x = x + outer.x;
        let abs_y = y + outer.y;
        let (piece_start, piece_size) = progression.span(outer);
        // The page coordinates of the piece's ends along the axis and across it.
        let along_origin = if progression.vertical { y } else { x };
        let piece_along = (
            f64::from(along_origin + piece_start),
            f64::from(along_origin + piece_start + piece_size),
        );
        let piece_across = if progression.vertical {
            (f64::from(abs_x), f64::from(abs_x + outer.width))
        } else {
            (f64::from(abs_y), f64::from(abs_y + outer.height))
        };
        let joined = background_slice
            .filter(|slice| progression.span(slice.outer) != (piece_start, piece_size));
        Self {
            cv,
            progression,
            outer,
            x,
            y,
            abs_x,
            abs_y,
            low_edge,
            high_edge,
            border,
            radius,
            full_radius,
            padding,
            piece_start,
            piece_size,
            along_origin,
            piece_along,
            piece_across,
            has_background_image,
            joined,
        }
    }
}

impl InlineBox<'_> {
    /// Paint the box shadows of the piece. A joined box that reaches past the
    /// piece casts one shadow, which the piece shows the part of.
    fn paint_shadows(&self, scene: &mut impl PaintScene) {
        let cv = self.cv;
        let progression = self.progression;
        let (piece_start, piece_size) = (self.piece_start, self.piece_size);
        let (piece_along, piece_across) = (self.piece_along, self.piece_across);
        let along_origin = self.along_origin;
        let shadow_slice = self.joined.filter(|_| !cv.box_shadow.is_empty());
        if let Some(slice) = shadow_slice {
            // Only artificial inline edges clip a sliced shadow. Its real outer
            // edges and its block-axis overflow remain visible.
            let (mut overflow_x, mut overflow_y) = (1.0_f32, 1.0_f32);
            for shadow in cv.box_shadow.iter() {
                let spread = shadow.spread_radius.px().abs();
                let blur = shadow.blur_radius.px().max(0.0) * 4.0;
                overflow_x = overflow_x.max(shadow.offset_x.px().abs() + spread + blur + 1.0);
                overflow_y = overflow_y.max(shadow.offset_y.px().abs() + spread + blur + 1.0);
            }
            let (overflow_along, overflow_across) = if progression.vertical {
                (f64::from(overflow_y), f64::from(overflow_x))
            } else {
                (f64::from(overflow_x), f64::from(overflow_y))
            };
            let (slice_start, slice_size) = progression.span(slice.outer);
            let low = if slice_start < piece_start {
                piece_along.0
            } else {
                f64::from(along_origin + slice_start) - overflow_along
            };
            let high = if slice_start + slice_size > piece_start + piece_size {
                piece_along.1
            } else {
                f64::from(along_origin + slice_start + slice_size) + overflow_along
            };
            scene.push_clip_layer(
                Affine::IDENTITY,
                &progression.rect(
                    (low, high),
                    (
                        piece_across.0 - overflow_across,
                        piece_across.1 + overflow_across,
                    ),
                ),
            );
            let joined_box = progression.with_span(self.outer, (slice_start, slice_size));
            paint_element_box_shadows(
                scene,
                joined_box.width,
                joined_box.height,
                self.x + joined_box.x,
                self.y + joined_box.y,
                &self.full_radius,
                &cv.box_shadow,
                cv.color,
            );
            scene.pop_layer();
        } else {
            paint_element_box_shadows(
                scene,
                self.outer.width,
                self.outer.height,
                self.abs_x,
                self.abs_y,
                &self.radius,
                &cv.box_shadow,
                cv.color,
            );
        }
    }

    /// Paint the background layers of the piece.
    fn paint_background(
        &self,
        scene: &mut impl PaintScene,
        env: &InlineBoxEnv<'_>,
        warnings: &mut Vec<RenderWarning>,
    ) {
        let cv = self.cv;
        let outer = self.outer;
        let progression = self.progression;
        let piece_rect = Rect::new(
            f64::from(self.abs_x),
            f64::from(self.abs_y),
            f64::from(self.abs_x + outer.width),
            f64::from(self.abs_y + outer.height),
        );
        // A typographic pseudo split across source owners has one background
        // positioning area. Slice that image before applying each owner's offset;
        // fragment sizes across the axis remain local when their inherited fonts
        // differ.
        let slice = self.joined.filter(|_| self.has_background_image);
        let mut background_outer = outer;
        let mut background_padding = self.padding;
        if let Some(slice) = slice {
            background_outer = progression.with_span(outer, progression.span(slice.outer));
            if progression.vertical {
                background_padding.top =
                    (slice.content.y - slice.outer.y - cv.border.top.width().px()).max(0.0);
                background_padding.bottom = ((slice.outer.y + slice.outer.height)
                    - (slice.content.y + slice.content.height)
                    - cv.border.bottom.width().px())
                .max(0.0);
            } else {
                background_padding.left =
                    (slice.content.x - slice.outer.x - cv.border.left.width().px()).max(0.0);
                background_padding.right = ((slice.outer.x + slice.outer.width)
                    - (slice.content.x + slice.content.width)
                    - cv.border.right.width().px())
                .max(0.0);
            }
            scene.push_clip_layer(Affine::IDENTITY, &piece_rect);
        }
        paint_element_background_layers(
            env.image_cache,
            scene,
            (
                background_outer.width,
                background_outer.height,
                self.x + background_outer.x,
                self.y + background_outer.y,
            ),
            cv.background_color,
            cv.background_layers(),
            cv.color,
            if slice.is_some() {
                &self.full_radius
            } else {
                &self.radius
            },
            if slice.is_some() {
                &cv.border
            } else {
                &self.border
            },
            &background_padding,
            BackgroundEnv {
                device_grid: env.image_cache.device_grid(env.transform),
                fixed_viewport: env.fixed_viewport,
                ..BackgroundEnv::default()
            },
            env.pixel_source,
            warnings,
            slice.map(|_| piece_rect),
        );
        if slice.is_some() {
            scene.pop_layer();
        }
    }

    /// Paint the border image of the piece and tell whether one was drawn.
    ///
    /// With `box-decoration-break: slice` the border image is laid out once
    /// over the joined box and clipped to this fragment (CSS Fragmentation 3
    /// §5.4). Without a joined box, the fragment shows it on the edges it
    /// keeps.
    fn paint_border_image(
        &self,
        scene: &mut impl PaintScene,
        env: &InlineBoxEnv<'_>,
        warnings: &mut Vec<RenderWarning>,
    ) -> bool {
        let cv = self.cv;
        let progression = self.progression;
        let piece_along = self.piece_along;
        if let Some(joined) = self.joined {
            // Only the cuts between fragments clip the image; the outset beyond
            // the box's own edges stays visible (CSS Backgrounds 3 §6.4). The
            // image itself bounds what is drawn past those edges.
            const UNBOUNDED: f64 = 1.0e7;
            let low = if self.low_edge {
                -UNBOUNDED
            } else {
                piece_along.0
            };
            let high = if self.high_edge {
                UNBOUNDED
            } else {
                piece_along.1
            };
            scene.push_clip_layer(
                Affine::IDENTITY,
                &progression.rect((low, high), (-UNBOUNDED, UNBOUNDED)),
            );
            let joined_box = progression.with_span(self.outer, progression.span(joined.outer));
            let visible = if progression.vertical {
                VisibleArea {
                    x: VisibleArea::UNBOUNDED.x,
                    y: (low, high),
                }
            } else {
                VisibleArea {
                    x: (low, high),
                    y: VisibleArea::UNBOUNDED.y,
                }
            };
            let drawn = BorderImage::new(
                env.image_cache,
                (joined_box.width, joined_box.height),
                (self.x + joined_box.x, self.y + joined_box.y),
                cv,
                &cv.border_image,
                &cv.border,
                env.transform,
            )
            .within(visible)
            .paint(scene, env.pixel_source, warnings);
            scene.pop_layer();
            drawn
        } else {
            let cut = self.border_image_without_cut_edges();
            paint_element_border_image(
                env.image_cache,
                scene,
                self.outer.width,
                self.outer.height,
                self.abs_x,
                self.abs_y,
                cv,
                cut.as_ref().unwrap_or(&cv.border_image),
                &self.border,
                env.pixel_source,
                warnings,
                env.transform,
            )
        }
    }

    /// The border image of the element with no width or outset at the edges
    /// the piece does not carry, or `None` when the image is whole there.
    ///
    /// A cut between line fragments shows no image: its widths and outset are
    /// taken as zero there, whatever form `border-image-width` takes.
    fn border_image_without_cut_edges(&self) -> Option<ComputedBorderImage> {
        let cv = self.cv;
        if (self.low_edge && self.high_edge)
            || matches!(cv.border_image.source, BackgroundImage::None)
        {
            return None;
        }
        let mut image = cv.border_image.clone();
        let (low_width, low_outset, high_width, high_outset) = if self.progression.vertical {
            (
                &mut image.width.top,
                &mut image.outset.top,
                &mut image.width.bottom,
                &mut image.outset.bottom,
            )
        } else {
            (
                &mut image.width.left,
                &mut image.outset.left,
                &mut image.width.right,
                &mut image.outset.right,
            )
        };
        if !self.low_edge {
            *low_width = BorderImageWidthSide::Number(0.0);
            *low_outset = BorderImageOutsetSide::Number(0.0);
        }
        if !self.high_edge {
            *high_width = BorderImageWidthSide::Number(0.0);
            *high_outset = BorderImageOutsetSide::Number(0.0);
        }
        Some(image)
    }

    /// Paint the borders of the sides the piece carries.
    fn paint_border(&self, scene: &mut impl PaintScene) {
        paint_element_border_rounded(
            scene,
            self.outer.width,
            self.outer.height,
            self.abs_x,
            self.abs_y,
            &self.border,
            self.cv.color,
            &self.radius,
        );
    }

    /// Paint the outline of the piece.
    fn paint_outline(&self, scene: &mut impl PaintScene) {
        let cv = self.cv;
        paint_element_outline(
            scene,
            self.outer.width,
            self.outer.height,
            self.abs_x,
            self.abs_y,
            &cv.outline,
            cv.outline_offset,
            cv.color,
        );
    }
}

/// The border and the corner radii the piece of an element carries, and the
/// corner radii of the whole element.
///
/// A side whose edge the piece does not carry has no border, and the corners
/// at that edge are square. `transformed` tells whether the element's
/// transform applies to the piece.
fn carried_edges(
    cv: &ComputedValues,
    progression: InlineProgression,
    (low_edge, high_edge): (bool, bool),
    transformed: bool,
) -> (
    Sides<raikiri_style::resolve::ComputedBorder>,
    ComputedBorderRadius,
    ComputedBorderRadius,
) {
    let mut border = cv.border;
    let square = raikiri_style::property::CornerRadius::circular(
        raikiri_style::resolve::ComputedLengthPercentageOrCalc::Px(0.0),
    );
    // The same rule as the element visit: radii only for a box that draws a
    // border or a background, and never under a filter or an applied
    // transform. `transformed` is false for a non-replaced inline box, which
    // is not transformable (CSS Transforms 1 §3), whatever its `transform`.
    let has_background_image = cv.has_background_image();
    let joined_radius = |border: &Sides<raikiri_style::resolve::ComputedBorder>| {
        let has_border = [&border.top, &border.right, &border.bottom, &border.left]
            .iter()
            .any(|side| side.width().px() > 0.0 && side.style() != BorderStyle::None);
        paintable_border_radius(
            &cv.border_radius,
            (has_border || cv.background_color.a > 0 || has_background_image)
                && !transformed
                && cv.filter.is_empty(),
        )
    };
    let full_radius = joined_radius(&cv.border);
    if progression.vertical {
        if !low_edge {
            border.top = border.top.without_line();
        }
        if !high_edge {
            border.bottom = border.bottom.without_line();
        }
    } else {
        if !low_edge {
            border.left = border.left.without_line();
        }
        if !high_edge {
            border.right = border.right.without_line();
        }
    }
    let mut radius = joined_radius(&border);
    if !low_edge {
        if progression.vertical {
            radius.top_left = square;
            radius.top_right = square;
        } else {
            radius.top_left = square;
            radius.bottom_left = square;
        }
    }
    if !high_edge {
        if progression.vertical {
            radius.bottom_left = square;
            radius.bottom_right = square;
        } else {
            radius.top_right = square;
            radius.bottom_right = square;
        }
    }
    (border, radius, full_radius)
}
