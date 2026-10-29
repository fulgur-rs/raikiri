use super::inline_svg::{InlineSvgCache, PictureKey, PictureRequest, PreparedSvg, build_picture};
use super::{
    DomView, Fragment, GeneratedImageId, MarginBox, OverflowClip, PaintEvent, PositionedGlyphRun,
    TextLineId,
};
use crate::RenderError;
use crate::SvgRendering;
use crate::svg_rendering::SvgSettings;
use raikiri_style::property::StringFetchMode;
use raikiri_style::{ComputedValues, PageCascadeResult};
use raikiri_traits::{NodeId, PaintInsets, PaintRect, PaintTransform};

/// How the page is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PageMode {
    /// A fixed-size page of paged media.
    Paged,
}

/// Page geometry in CSS px, origin at the top-left of the page box.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageGeometry {
    /// The whole page box.
    pub page_box: PaintRect,
    /// Resolved page margins.
    pub margins: PaintInsets,
    /// The flow area: origin after margins and `@page` border / padding;
    /// width is the flow width (horizontal insets are not subtracted).
    pub content_box: PaintRect,
    /// Layout mode.
    pub mode: PageMode,
}

/// The picture of an inline SVG root and its resolved placement.
///
/// Raikiri decides how the picture is drawn. [`Self::content`] is a scene
/// when the document is inside the vector subset and the root does not ask
/// for pixels (see [`SvgRendering`]); it is pixels when the root asks for
/// them, and when a scene cannot stand for the picture: the document uses a
/// `mask` or a `pattern`, is over a budget of the scene, or strokes text under
/// a transform that is not a rotation, a reflection and a uniform scale. A
/// consumer draws what it is given and has no second path to take.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InlineSvg {
    /// What to draw. A scene is `viewport` in size, in CSS pixels, with its
    /// origin at the top-left corner of the viewport. Pixels are the picture
    /// of the CSS pixel grid of the viewport at
    /// [`Page::svg_raster_scale`] device pixels per CSS pixel, so they are
    /// drawn from the same corner at their size divided by that scale. Draw
    /// either clipped to the viewport.
    pub content: ImageContent,
    /// Whole content box in CSS page coordinates, before pagination cuts.
    pub viewport: PaintRect,
    /// Host root opacity represented by the page's paint events.
    ///
    /// The content holds no host opacity, whichever it is: the page's paint
    /// events composite it around the picture, or, for a consumer that does
    /// not take the events, this is the opacity to draw the picture with.
    /// Descendant opacity stays in the content.
    pub host_opacity: Option<f32>,
}

/// One laid-out page, borrowed from a [`super::DocumentLayout`].
#[derive(Clone, Copy)]
pub struct Page<'a> {
    pub(super) slice: &'a raikiri_dom::PageSlice,
    pub(super) geometry: &'a raikiri_dom::ResolvedPageGeometry,
    pub(super) style: &'a PageCascadeResult,
    pub(super) document: &'a raikiri_dom::Document,
    pub(super) cascade: &'a raikiri_style::CascadeResult,
    pub(super) page_count: u32,
    /// Whether `page_count` is a placeholder for a number of pages not
    /// known yet.
    pub(super) page_count_deferred: bool,
    /// The `@page` cascade of the left page paired with this right page.
    pub(super) paired_style: Option<&'a PageCascadeResult>,
    /// The document's running elements; `None` on a page that is itself a
    /// laid-out running element.
    pub(super) running: Option<super::running::RunningSource<'a>>,
    /// Prepared inline SVG sources and pictures of [`Self::document`].
    pub(super) svg_sources: &'a InlineSvgCache,
    /// What the layout was asked for about SVG pictures.
    pub(super) svg: SvgSettings,
    /// The `element()` and `string()` values of [`Self::document`], shared
    /// by its pages.
    pub(super) margin_values: &'a raikiri_dom::MarginBoxNamedValues,
}

/// A running element placed in a page-margin box.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub struct PlacedRunningElement<'a> {
    /// The element laid out at the width of the margin box's content box.
    pub layout: &'a super::RunningElementLayout,
    /// Where the origin of [`super::RunningElementLayout::page`] goes on
    /// this page, in CSS px.
    pub origin: (f32, f32),
}

/// A [`PlacedRunningElement`] that owns its layout.
pub(crate) struct OwnedRunningElement {
    pub(crate) layout: super::RunningElementLayout,
    pub(crate) origin: (f32, f32),
}

/// Resolved raster pixels and their complete object placement on a page.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct RasterImage {
    /// Source element.
    pub node: NodeId,
    /// Absolute URL resolved by layout, without changing source attributes.
    pub url: url::Url,
    /// Shared straight RGBA8 pixels from the configured source.
    pub pixels: std::sync::Arc<raikiri_traits::DecodedImage>,
    /// Object rectangle after object-fit and object-position, or the placement
    /// of a marker or a generated-content image.
    /// Like [`Fragment::paint_rect`], it is in layout coordinates, which the
    /// enclosing [`PaintEvent::PushTransform`] events of its paint event move.
    pub rect: PaintRect,
    /// Whole content-edge clip or marker rectangle, before pagination cuts.
    pub clip: raikiri_traits::PaintClip,
    /// Innermost node whose overflow clip applies. Standalone markers start
    /// at their parent's clip; inline image markers and replaced content
    /// include their own element's clip.
    pub clip_owner: Option<NodeId>,
}

/// What an image is drawn with.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum ImageContent {
    /// Shared straight RGBA8 pixels from the configured source.
    Raster(std::sync::Arc<raikiri_traits::DecodedImage>),
    /// A scene built for the box the image is placed in, in CSS px, with its
    /// origin at the top-left corner of that box.
    Vector(std::sync::Arc<raikiri_traits::vector::VectorImage>),
}

/// An image and its complete object placement on a page, drawn as pixels or
/// as a scene.
///
/// The fields other than [`Self::content`] are those of [`RasterImage`].
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlacedImage {
    /// Source element.
    pub node: NodeId,
    /// Absolute URL resolved by layout, without changing source attributes.
    pub url: url::Url,
    /// What to draw: pixels, or a scene of the size of [`Self::rect`].
    pub content: ImageContent,
    /// Object rectangle, as for [`RasterImage::rect`].
    pub rect: PaintRect,
    /// Whole content-edge clip or marker rectangle, before pagination cuts.
    pub clip: raikiri_traits::PaintClip,
    /// Innermost node whose overflow clip applies, as for
    /// [`RasterImage::clip_owner`].
    pub clip_owner: Option<NodeId>,
}

/// Where an image sits, before its pixels or scene are asked for.
struct Placement {
    node: NodeId,
    url: url::Url,
    rect: PaintRect,
    clip: raikiri_traits::PaintClip,
    clip_owner: Option<NodeId>,
    /// The size to ask pixels for, which already follows the raster scale.
    raster: raikiri_traits::ImageRasterSize,
    /// The rendering the owner of the image names itself, when that owner is
    /// not [`Self::node`]: the `::before` or `::after` pseudo-element whose
    /// `content` is the image. `None` leaves it to the element of `node`.
    rendering: Option<SvgRendering>,
}

impl Placement {
    fn into_raster(self, source: &dyn raikiri_traits::ImagePixelSource) -> Option<RasterImage> {
        let pixels = checked_pixels(source, &self.url, self.raster)?;
        Some(RasterImage {
            node: self.node,
            url: self.url,
            pixels,
            rect: self.rect,
            clip: self.clip,
            clip_owner: self.clip_owner,
        })
    }
}

impl<'a> Page<'a> {
    /// This page with `placeholder` standing in for the number of pages.
    pub(super) fn with_deferred_page_count(mut self, placeholder: u32) -> Self {
        self.page_count = placeholder;
        self.page_count_deferred = true;
        self
    }

    /// Zero-based page index.
    pub fn index(&self) -> u32 {
        self.slice.page_index
    }

    /// Named page, if the page context selected one.
    pub fn name(&self) -> Option<&'a str> {
        self.slice.page_name.as_deref()
    }

    /// Page geometry.
    pub fn geometry(&self) -> PageGeometry {
        let f = self.geometry;
        PageGeometry {
            page_box: PaintRect::new(0.0, 0.0, f.page_box.width, f.page_box.height),
            margins: PaintInsets::new(
                f.margins.top,
                f.margins.right,
                f.margins.bottom,
                f.margins.left,
            ),
            content_box: PaintRect::new(
                f.content_box.x,
                f.content_box.y,
                f.content_box.width,
                f.content_box.height,
            ),
            mode: PageMode::Paged,
        }
    }

    /// All fragments on this page. The order is not the paint order.
    pub fn fragments(&self) -> impl Iterator<Item = Fragment<'a>> + 'a + use<'a> {
        self.document.page_fragments(self.slice.page_index)
    }

    /// Resolve an image fragment from already cached pixels.
    ///
    /// Pass a fragment from this page and the source returned by
    /// [`RenderResources::image_pixel_source_ref`](crate::RenderResources::image_pixel_source_ref)
    /// so painting retains the configured resource policy and byte limits.
    /// Empty, hidden, missing, and ordinary non-image elements return `None`.
    /// Ancestor overflow and opacity remain represented by page paint events.
    pub fn raster_image(
        &self,
        fragment: &Fragment<'a>,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<RasterImage> {
        self.image_placement(fragment, source)?.into_raster(source)
    }

    /// Like [`Self::raster_image`], but the image is a scene when its source
    /// has one and the element is not asked to draw it as pixels.
    ///
    /// An SVG image is drawn as a scene unless the element that owns it says
    /// `--raikiri-svg-rendering: raster` or the layout was asked for
    /// [`SvgRendering::Raster`], and unless the source has no scene for it
    /// (see [`raikiri_traits::ImagePixelSource::get_vector`]); then it is
    /// pixels, at [`Self::svg_raster_scale`]. Every other image is pixels.
    pub fn placed_image(
        &self,
        fragment: &Fragment<'a>,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<PlacedImage> {
        let placement = self.image_placement(fragment, source)?;
        self.place(placement, source)
    }

    /// Everything about the image of an `<img>` fragment but its pixels.
    fn image_placement(
        &self,
        fragment: &Fragment<'a>,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<Placement> {
        let node = usize::try_from(fragment.node().0).ok()?;
        let url = self.document.resolved_image_url(node)?;
        let computed = self.computed(fragment.node())?;
        if computed.visibility != raikiri_style::property::Visibility::Visible {
            return None;
        }
        let content = fragment.content_rect()?;
        let natural = source.intrinsic_size(&url)?;
        let (x, y, width, height) = raikiri_dom::image_geometry::object_image_rect(
            (
                content.x as f64,
                content.y as f64,
                content.width as f64,
                content.height as f64,
            ),
            natural,
            computed.object_fit,
            &computed.object_position,
        )?;
        let rect = PaintRect::new(x as f32, y as f32, width as f32, height as f32);
        if !is_finite_rect(&rect) {
            return None;
        }
        Some(Placement {
            node: fragment.node(),
            url,
            raster: self.raster_size_for(raster_size(&rect)),
            rect,
            clip: raikiri_traits::PaintClip::new(content),
            clip_owner: Some(fragment.node()),
            rendering: None,
        })
    }

    /// `size` times the raster scale of the layout.
    fn raster_size_for(
        &self,
        size: raikiri_traits::ImageRasterSize,
    ) -> raikiri_traits::ImageRasterSize {
        let scale = self.svg_raster_scale();
        raikiri_traits::ImageRasterSize {
            width: size.width * scale,
            height: size.height * scale,
        }
    }

    /// The pixels or the scene of `placement`.
    fn place(
        &self,
        placement: Placement,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<PlacedImage> {
        let rendering = placement
            .rendering
            .unwrap_or_else(|| self.svg_rendering(placement.node));
        let content = self.content_in(
            rendering,
            &placement.url,
            raster_size(&placement.rect),
            placement.raster,
            source,
        )?;
        Some(PlacedImage {
            node: placement.node,
            url: placement.url,
            content,
            rect: placement.rect,
            clip: placement.clip,
            clip_owner: placement.clip_owner,
        })
    }

    /// What to draw for the image at `url` in a box of `size` CSS px, for
    /// an element `owner` that decides how SVG is drawn. Use it for the
    /// images the consumer places itself, such as the layers of a
    /// `background-image`.
    ///
    /// The image is a scene when `source` has one and the rendering of
    /// `owner` is [`SvgRendering::Vector`] (see [`Self::svg_rendering`]);
    /// otherwise it is pixels asked for at `size` times
    /// [`Self::svg_raster_scale`]. `None` when the source has neither.
    pub fn image_content(
        &self,
        owner: NodeId,
        url: &url::Url,
        size: raikiri_traits::ImageRasterSize,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<ImageContent> {
        let rendering = self.svg_rendering(owner);
        self.content_in(rendering, url, size, self.raster_size_for(size), source)
    }

    fn content_in(
        &self,
        rendering: SvgRendering,
        url: &url::Url,
        size: raikiri_traits::ImageRasterSize,
        raster: raikiri_traits::ImageRasterSize,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<ImageContent> {
        if rendering == SvgRendering::Vector
            && let Some(scene) =
                source.get_vector(url, &raikiri_traits::ImageVectorRequest::new(size))
        {
            return Some(ImageContent::Vector(scene));
        }
        checked_pixels(source, url, raster).map(ImageContent::Raster)
    }

    /// Cached raster pixels of a prepared image marker from [`PaintEvent::MarkerImage`].
    ///
    /// Use [`crate::RenderResources::image_pixel_source_ref`] to preserve source
    /// policy and byte limits. This method uses the absolute URL and dimensions
    /// retained by layout; it does not fetch or place content in the consumer.
    pub fn raster_marker(
        &self,
        owner: NodeId,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<RasterImage> {
        self.marker_placement(owner)?.into_raster(source)
    }

    /// Like [`Self::raster_marker`], but the image is a scene under the
    /// conditions of [`Self::placed_image`].
    pub fn placed_marker(
        &self,
        owner: NodeId,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<PlacedImage> {
        let placement = self.marker_placement(owner)?;
        self.place(placement, source)
    }

    fn marker_placement(&self, owner: NodeId) -> Option<Placement> {
        let (url, rect) = self.document.page_marker_image(self.index(), owner)?;
        if !is_drawable_rect(&rect) {
            return None;
        }
        let element = usize::try_from(owner.0).ok()?;
        let clip_owner =
            if raikiri_dom::generated_content::inside_marker_in_flow(self.cascade, element)
                && self
                    .document
                    .get_node(element)
                    .is_some_and(|node| node.is_ifc_root())
            {
                Some(owner)
            } else {
                self.dom().parent(owner)
            };
        Some(Placement {
            node: owner,
            url: url.clone(),
            raster: self.raster_size_for(raster_size(&rect)),
            rect,
            clip: raikiri_traits::PaintClip::new(rect),
            clip_owner,
            rendering: None,
        })
    }

    /// Where an image of `::before` or `::after` content from
    /// [`PaintEvent::GeneratedImage`] sits in the text: the line holding it,
    /// and its UTF-8 offset in that line's processed text when known.
    ///
    /// Within one line, the image and the glyph runs of [`Self::text_runs`]
    /// follow the logical (reading) order of the offsets, compared with
    /// [`PositionedGlyphRun::text_offset`]. This orders the text and images
    /// of one `content` list, such as `'Note: ' url(icon.png) ' end'`,
    /// although paint lists the images before the line's text. The offset is
    /// `None` on a line that mixes bidi directions.
    pub fn generated_image_text_position(
        &self,
        image: GeneratedImageId,
    ) -> Option<(TextLineId, Option<usize>)> {
        self.document
            .page_generated_image_text_position(self.index(), image)
    }

    /// The alternative text of the `content` of the `kind` pseudo-element of
    /// `owner`: the components after `/`, such as `"Note"` in
    /// `content: url(icon.png) / "Note"`, with `attr()` and counters
    /// resolved (CSS Content 3 <https://www.w3.org/TR/css-content-3/#alt>).
    ///
    /// Assistive technology reads it in place of the pseudo-element's text
    /// and images; an empty string marks them as decorative. `None` when
    /// the value gives no alternative text, and for `::first-letter`, which
    /// has no `content`.
    pub fn generated_alt_text(&self, owner: NodeId, kind: crate::GeneratedKind) -> Option<String> {
        let pseudo = match kind {
            crate::GeneratedKind::Before => raikiri_style::PseudoElem::Before,
            crate::GeneratedKind::After => raikiri_style::PseudoElem::After,
            crate::GeneratedKind::Marker => raikiri_style::PseudoElem::Marker,
            _ => return None,
        };
        let element = usize::try_from(owner.0).ok()?;
        let snapshots = self
            .document
            .shared_compact_counter_snapshots(self.cascade)
            .ok()?;
        raikiri_dom::generated_content::generated_alt_text_compact(
            self.document,
            self.cascade,
            element,
            pseudo,
            &snapshots,
        )
    }

    /// Cached raster pixels of an image of `::before` or `::after` content
    /// from [`PaintEvent::GeneratedImage`].
    ///
    /// Like [`Self::raster_marker`], this uses the absolute URL and the
    /// rectangle retained by layout, and does not fetch or place content in
    /// the consumer. The image is clipped by its element's overflow clip, as
    /// the pseudo-element is a child of the element.
    pub fn raster_generated_image(
        &self,
        image: GeneratedImageId,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<RasterImage> {
        self.generated_image_placement(image)?.into_raster(source)
    }

    /// Like [`Self::raster_generated_image`], but the image is a scene under
    /// the conditions of [`Self::placed_image`].
    pub fn placed_generated_image(
        &self,
        image: GeneratedImageId,
        source: &dyn raikiri_traits::ImagePixelSource,
    ) -> Option<PlacedImage> {
        let placement = self.generated_image_placement(image)?;
        self.place(placement, source)
    }

    fn generated_image_placement(&self, image: GeneratedImageId) -> Option<Placement> {
        let (url, content) = self.document.page_generated_image(self.index(), image)?;
        let owner = usize::try_from(image.owner.0).ok();
        let pseudo = match image.kind {
            crate::GeneratedKind::After => raikiri_style::PseudoElem::After,
            _ => raikiri_style::PseudoElem::Before,
        };
        // An image that replaces its pseudo-element is drawn at its
        // `object-fit` rectangle, clipped to its box.
        let object = owner
            .and_then(|owner| {
                self.document
                    .content_image_object_rect(owner, pseudo, image.index)
            })
            .unwrap_or(PaintRect::new(0.0, 0.0, content.width, content.height));
        let rect = PaintRect::new(
            content.x + object.x,
            content.y + object.y,
            object.width,
            object.height,
        );
        if !is_drawable_rect(&rect) {
            return None;
        }
        // The size layout prepared the pixels at, which stays bounded when
        // `object-fit` makes the object far larger than its box.
        let raster = owner
            .and_then(|owner| {
                self.document
                    .content_image_raster_size(owner, pseudo, image.index)
            })
            .unwrap_or_else(|| raster_size(&rect));
        // The pseudo-element owns the image, so what it names itself wins
        // over what the element it was generated for names.
        let rendering = owner
            .and_then(|owner| {
                self.cascade
                    .pseudo
                    .get(&(raikiri_style::StyleNodeId::new(owner as u64), pseudo))
            })
            .and_then(|style| SvgRendering::declared_by(style.as_ref()));
        Some(Placement {
            node: image.owner,
            url: url.clone(),
            raster: self.raster_size_for(raster),
            rect,
            clip: raikiri_traits::PaintClip::new(content),
            clip_owner: Some(image.owner),
            rendering,
        })
    }

    /// The picture of an SVG root placed by this page, and where to draw it.
    ///
    /// Pass a fragment from [`Self::fragments`]. Ordinary elements, text and
    /// empty SVG viewports return `None`. Hidden roots retain content so their
    /// explicitly visible descendants can still be drawn. The host box is drawn
    /// separately; source admission, preparation and picture errors are
    /// returned. [`InlineSvg`] says when the picture is a scene and when it is
    /// pixels. A picture is built once for roots with the same markup, style and
    /// viewport size, and shared by them and by the pages that show them, as
    /// far as the cache has room for it.
    /// Fixed boxes placed through other insets than `top` and `left` lengths
    /// are rejected, including when the fixed box is an ancestor. Their paint
    /// placement is not represented by the fragment projection yet.
    pub fn inline_svg(
        &self,
        fragment: &Fragment<'a>,
    ) -> Result<Option<InlineSvg>, raikiri_svg::SvgSceneError> {
        let Some((viewport, prepared)) = self.prepared_inline_svg(fragment)? else {
            return Ok(None);
        };
        let key = PictureKey::new(&prepared, viewport);
        let content = match self.svg_sources.picture(&key) {
            Some(content) => content,
            None => {
                let fonts = self.svg_fonts();
                let content = build_picture(&PictureRequest {
                    source: &prepared.source,
                    viewport,
                    host_opacity: prepared.host_opacity,
                    rendering: prepared.rendering,
                    raster_scale: self.svg.raster_scale,
                    fonts: &fonts,
                })?;
                self.svg_sources.keep(key, &content);
                content
            }
        };
        Ok(Some(InlineSvg {
            content,
            viewport,
            host_opacity: prepared.host_opacity,
        }))
    }

    /// The viewport and the prepared source of the SVG root of `fragment`.
    /// See [`Self::inline_svg`] for who gets `None` and which roots are
    /// rejected.
    pub(super) fn prepared_inline_svg(
        &self,
        fragment: &Fragment<'a>,
    ) -> Result<Option<(PaintRect, PreparedSvg)>, raikiri_svg::SvgError> {
        let Some(id) = usize::try_from(fragment.node().0).ok() else {
            return Ok(None); // cov:ignore: Fragment node IDs originate from usize arena indices.
        };
        if !self
            .document
            .get_node(id)
            .is_some_and(|node| node.is_inline_svg_root())
        {
            return Ok(None);
        }
        let Some(viewport) = fragment.content_rect() else {
            return Ok(None); // cov:ignore: Page fragments of an SVG element always cache its content box.
        };
        let Some(computed) = self.computed(fragment.node()) else {
            return Ok(None); // cov:ignore: Completed layout cascades cover the entire source node arena.
        };
        if viewport.width <= 0.0 || viewport.height <= 0.0 {
            return Ok(None);
        }
        let dom = self.dom();
        let mut current = Some(fragment.node());
        while let Some(node) = current {
            if let Some(style) = self.computed(node)
                && style.position == raikiri_style::property::PositionValue::Fixed
                && !(matches!(
                    style.left,
                    raikiri_style::resolve::ComputedLengthPercentageOrAuto::Px(_)
                ) && matches!(
                    style.top,
                    raikiri_style::resolve::ComputedLengthPercentageOrAuto::Px(_)
                ))
            {
                return Err(raikiri_svg::SvgError::InvalidDocument(
                    "SVG fixed placement is not represented by the fragment projection".into(),
                ));
            }
            current = dom.parent(node);
        }
        let key = (id, viewport.width.to_bits(), viewport.height.to_bits());
        if let Some(prepared) = self.svg_sources.get(&key) {
            return Ok(Some((viewport, prepared)));
        }
        let rendering = self.svg_rendering(fragment.node());
        let Some(prepared) = self.prepare_inline_svg(id, viewport, computed, rendering)? else {
            return Ok(None); // cov:ignore: The checked SVG namespace/root predicate guarantees a serializable root.
        };
        let prepared = self.svg_sources.insert(key, prepared);
        Ok(Some((viewport, prepared)))
    }

    /// The fonts that the SVG text of this page is set in: those the page's
    /// document was laid out with.
    ///
    /// [`Self::inline_svg`] sets the text of its picture in them. Pass it to
    /// [`raikiri_svg::SvgResources::with_fonts`] when building the scene of
    /// another SVG document.
    pub fn svg_fonts(&self) -> crate::PageSvgFonts<'a> {
        crate::PageSvgFonts::new(
            self.document
                .font_collection()
                .expect("a laid-out document has fonts"),
            self.svg_sources.faces(),
        )
    }

    /// How the SVG picture that `node` owns is to be drawn.
    ///
    /// The owner is the `<img>`, the element that has the `background-image`,
    /// or the `<svg>` root. (For an image that is the `content` of `::before`
    /// or `::after`, [`Self::placed_generated_image`] lets the pseudo-element
    /// decide.) Its `--raikiri-svg-rendering` decides, as
    /// [`SvgRendering`] describes; a value that names no rendering is passed
    /// over for the one the element would inherit without it, and the default
    /// of [`crate::LayoutOptions::svg_rendering`] applies when no element up
    /// to the root names one. A node the page does not have gets the default.
    pub fn svg_rendering(&self, node: NodeId) -> SvgRendering {
        let dom = self.dom();
        let mut current = Some(node);
        while let Some(node) = current {
            if let Some(rendering) = self.computed(node).and_then(SvgRendering::declared_by) {
                return rendering;
            }
            current = dom.parent(node);
        }
        self.svg.rendering
    }

    /// Device pixels per CSS pixel for an SVG drawn as pixels: the
    /// [`crate::LayoutOptions::svg_raster_scale`] of the layout, `1` unless
    /// it was set.
    pub fn svg_raster_scale(&self) -> f32 {
        self.svg.raster_scale
    }

    /// Serializes, parses and restyles the SVG root `id`.
    fn prepare_inline_svg(
        &self,
        id: usize,
        viewport: PaintRect,
        computed: &ComputedValues,
        rendering: SvgRendering,
    ) -> Result<Option<PreparedSvg>, raikiri_svg::SvgError> {
        let Some(source) = self
            .document
            .serialize_svg_subtree(id)
            .map_err(raikiri_svg::SvgError::InvalidDocument)?
        else {
            return Ok(None); // cov:ignore: The checked SVG namespace/root predicate guarantees a serializable root.
        };
        let document = raikiri_svg::SvgDocument::parse(source.as_bytes())?;
        let host_opacity = self
            .cascade
            .opacity_specified
            .get(id)
            .copied()
            .unwrap_or(false)
            .then_some(computed.opacity);
        let color = computed.color;
        let node_styles = raikiri_dom::inline_svg_style::inline_svg_element_declarations(
            self.document,
            self.cascade,
            id,
        )
        .map_err(raikiri_svg::SvgError::InvalidDocument)?;
        let element_styles: Vec<_> = node_styles
            .iter()
            .map(|node| raikiri_svg::SvgElementStyle {
                element_index: node.element_index,
                declarations: &node.declarations,
            })
            .collect();
        let source = document.styled_source_with_resolved_styles(
            raikiri_svg::SvgViewport {
                width: viewport.width,
                height: viewport.height,
            },
            raikiri_svg::SvgRootStyle {
                inherited_color: [color.r, color.g, color.b, color.a],
                opacity: host_opacity.unwrap_or(1.0),
                neutralize_root_opacity: host_opacity.is_some(),
                host_controls_root_background: self
                    .cascade
                    .background_color_specified
                    .get(id)
                    .copied()
                    .unwrap_or(false),
                visible: computed.visibility == raikiri_style::property::Visibility::Visible,
            },
            [color.r, color.g, color.b, color.a],
            raikiri_svg::SvgRootFont {
                size: computed.font_size.0,
                weight: computed.font_weight,
                style: computed.font_style.as_css_str(),
                family: &raikiri_dom::inline_svg_style::svg_font_family(computed),
            },
            &element_styles,
        )?;
        Ok(Some(PreparedSvg::new(source, host_opacity, rendering)))
    }

    /// Local overflow clips of this page's placements and their ancestors,
    /// ordered by source node. A clipping ancestor can have no fragment on
    /// this page when its content reaches the page along an open axis.
    ///
    /// This supplies resolved shapes, not the order in which to push clips.
    /// Apply the clipping ancestors of each painted item in DOM order, or use
    /// the nested clip events in [`Self::paint_order`].
    pub fn overflow_clips(&self) -> impl Iterator<Item = OverflowClip> + 'a + use<'a> {
        self.document.page_overflow_clips(self.slice.page_index)
    }

    /// Positioned glyph runs of the body text on this page, in document
    /// order, at the positions the built-in painter draws them.
    ///
    /// A line belongs to the page that holds its center, the same rule as
    /// [`Fragment::line_range`], so every line appears on exactly one page;
    /// a paragraph repeated on every page (inside `position: fixed`) appears
    /// on each. Propagated text decorations and used text shadows are
    /// included. Margin box text comes from [`Self::margin_boxes`].
    /// Text list markers are included, including standalone markers of
    /// empty items. A standalone marker belongs to its item's first
    /// principal fragment and is not repeated on continuation pages. Missing
    /// image markers retain their text fallback; usable images have their own
    /// [`PaintEvent::MarkerImage`] event.
    /// Relative offsets and transforms are included: inside columns they
    /// move the positions, elsewhere [`PositionedGlyphRun::transform`] maps
    /// the positions to the page.
    /// A paragraph whose text is placed by geometry the runs do not model
    /// has no runs here: a vertical writing mode, overlapping or unrooted
    /// column placements, a transform other than a translation on the
    /// paragraph or an ancestor inside columns, a singular transform, a
    /// percentage translation of a box split into column fragments, a
    /// relative offset or translation inside columns that moves an overflow
    /// or column clip around the paragraph, or a fixed box not placed by
    /// `top` and `left` lengths or inside a transformed, filtered or moved
    /// box. Each such paragraph is reported
    /// once in [`super::DocumentLayout::warnings`] with
    /// [`raikiri_traits::WarningKind::TextRunsOmitted`].
    pub fn text_runs(&self) -> Vec<PositionedGlyphRun<'a>> {
        self.document
            .page_text_runs(self.cascade, self.slice.page_index)
    }

    /// Body content of this page in paint order: element boxes, paragraph
    /// text and replaced content, with the clips and group opacities that
    /// enclose them. The page background, page border and margin boxes are
    /// drawn before these events.
    ///
    /// Every fragment in an event is one of [`Self::fragments`]. Siblings
    /// follow the built-in painter: negative `z-index` first, then in-flow
    /// boxes, floats, and positioned boxes with `z-index: auto` or a
    /// non-negative `z-index`; flex and grid items use `order` within a
    /// stacking level. A paragraph's inline boxes come before its text. An
    /// inline element on the first line to which a `::first-line` gives a
    /// color or font size other than its own is listed by
    /// [`PaintEvent::LineBox`] pieces in place of its [`PaintEvent::Box`], as
    /// [`Self::paint_order_for_text_runs`] describes.
    /// Clip rectangles are in the same space as [`Fragment::paint_rect`]; an
    /// overflow clip is built from the element's whole box, so it runs past
    /// the page where a page break cuts the box.
    /// An element that the built-in painter moves from its layout location,
    /// by a relative offset that layout leaves to paint or by `transform`,
    /// is enclosed with its subtree in a [`PaintEvent::PushTransform`] and
    /// [`PaintEvent::PopTransform`] pair; inside it, events keep their
    /// layout coordinates. Elements inside columns are not enclosed this
    /// way, and an element with a singular transform is left out with its
    /// subtree.
    /// Image markers are included. Generated text uses
    /// [`Self::paint_order_for_text_runs`]. Explicit column fragment clips are
    /// included. Legacy column-height clipping retains an approximation
    /// warning for each outermost multicolumn container (reported once in
    /// [`super::DocumentLayout::warnings`] with
    /// [`raikiri_traits::WarningKind::PaintOrderApproximated`]).
    ///
    /// Where the built-in painter differs: it records root and body boxes on every
    /// page, while this list includes each only where its used box reaches; and it
    /// draws each line's inline element backgrounds just before that line's
    /// text, while this list has all of a paragraph's inline element boxes
    /// before all of its text. A `::first-line` with opacity below one is
    /// not a group here: its line is listed with the rest of its paragraph
    /// and drawn opaque. [`Self::paint_order_for_text_runs`] lists the group.
    pub fn paint_order(&self) -> Vec<PaintEvent<'a>> {
        self.document.page_paint_order(
            self.cascade,
            self.slice.page_index,
            self.slice.page_name.as_deref(),
        )
    }

    /// Body paint order using the supplied [`Self::text_runs`] instead of
    /// DOM text fragments. Each [`PaintEvent::TextLine`] identifies the runs
    /// of one paragraph line, including generated text and ellipses, inside
    /// the paragraph's ancestor clips and opacity groups.
    /// Standalone list markers have their own line events, before the list
    /// item's overflow clip and inside its opacity group.
    ///
    /// A non-atomic inline element with opacity below one is its own opacity
    /// group, listed after the rest of its paragraph like other stacking
    /// contexts with `z-index: auto`: a [`PaintEvent::PushOpacity`], the
    /// element's inline boxes and generated boxes and those of its
    /// descendants, one [`PaintEvent::GroupTextLine`] per line with runs in
    /// the group, its floats and atomic inlines, the groups nested in it, and
    /// the [`PaintEvent::PopOpacity`]. A `::before`, `::after` or
    /// `::first-letter` with opacity below one is such a group too, holding
    /// its generated boxes, its images and its text, named by the
    /// pseudo-element's own id instead of an element's. So is an atomic
    /// inline-level `::before` or `::after` with a `transform`, listed inside
    /// a [`PaintEvent::PushTransform`] that turns it about the
    /// pseudo-element's box. That transform encloses the group's
    /// [`PaintEvent::PushOpacity`] when the pseudo-element is translucent
    /// too; otherwise the group has no opacity events.
    /// A `::first-line` with opacity below one, on the paragraph or on an
    /// ancestor block whose first line it holds, makes a group of the
    /// paragraph's first line, listed
    /// before the paragraph's other inline groups: a
    /// [`PaintEvent::PushOpacity`], the content of the first line in the
    /// order above, and the [`PaintEvent::PopOpacity`]. An inline element on
    /// the first line and on later ones is split by line: in place of its
    /// [`PaintEvent::Box`], its piece on the first line is a
    /// [`PaintEvent::LineBox`] in the group, and its other pieces are
    /// [`PaintEvent::LineBox`] events with the other lines. The piece on the
    /// first line carries the values the `::first-line` gives the element, in
    /// [`crate::LineBox::style`]. Floats and atomic inlines not on the first
    /// line stay with the other lines.
    /// Whether or not the `::first-line` is translucent, an inline element
    /// on the first line to which it gives a color or font size other than
    /// the element's own is listed by [`PaintEvent::LineBox`] pieces in the
    /// same way, also when it has no box on later lines.
    /// [`Self::text_run_opacity_group`] tells
    /// which line event draws a run: a run of no group belongs to the
    /// `TextLine` event of its line, and any other run to the `GroupTextLine`
    /// event of its line and group.
    ///
    /// Pass the runs from this page. No text is extracted or shaped again.
    /// Boxes, replaced content and the approximations of [`Self::paint_order`]
    /// retain their existing order; paragraphs omitted by [`Self::text_runs`]
    /// have no text events here.
    pub fn paint_order_for_text_runs(
        &self,
        runs: &[PositionedGlyphRun<'a>],
    ) -> Vec<PaintEvent<'a>> {
        self.document
            .page_paint_order_for_text_runs(self.cascade, self.slice.page_index, runs)
    }

    /// The element whose inline opacity group draws `run`, a run of
    /// [`Self::text_runs`]: the innermost non-atomic inline element with
    /// opacity below one around the run's text inside its paragraph.
    /// `None` when the run is drawn with the rest of its paragraph. Elements
    /// with `display: contents` generate no box and form no group.
    ///
    /// The letters of a translucent `::first-letter` and the text of a
    /// translucent or transformed atomic `::before` or `::after` belong to
    /// that pseudo-element's group. Its id is not a node of [`Self::dom`]; it is the group of the
    /// matching [`PaintEvent::GroupTextLine`] events. The fragments of a
    /// `::first-letter` keep the style they are given for the text they wrap,
    /// so where its letters come from text with different inherited values,
    /// or lie in different inline groups, its fragments form one group per
    /// opacity and enclosing group, each with an id of its own; a fragment
    /// whose opacity is one forms none.
    pub fn text_run_opacity_group(&self, run: &PositionedGlyphRun<'_>) -> Option<NodeId> {
        self.document.text_run_opacity_group(self.cascade, run)
    }

    /// Page-margin boxes of this page (CSS Paged Media 3 §4.2) in drawing
    /// order (§3.1): by ascending [`MarginBox::z_index`], and among boxes
    /// of one level clockwise from the top-left corner.
    ///
    /// The canvas background, the page border and the document contents
    /// form a single layer at level zero. Draw the boxes with a negative
    /// `z_index` after the page background and behind that layer, and the
    /// others after [`Self::paint_order`], in front of it.
    ///
    /// Generated content is resolved for this page: `counter(page)`,
    /// `counter(pages)`, quotes, `string()` as the value on this page, and
    /// the document-wide value of `element()`. Each box's text is available
    /// as glyph runs through [`MarginBox::text_runs`].
    ///
    /// On a page streamed before the number of pages is known,
    /// `counter(pages)` shows a placeholder and each box lists it in
    /// [`MarginBox::deferred`]; see [`crate::StreamPage::page`].
    pub fn margin_boxes(&self) -> Vec<MarginBox> {
        // The first page is a right page, as in the page context queries.
        let page_is_left = self.slice.page_index % 2 == 1;
        let paired_page_increment = self.paired_style.and_then(|paired| {
            match paired
                .declarations()
                .get(&raikiri_style::PropertyKey::CounterIncrement)
            {
                Some(raikiri_style::PropertyValue::CounterIncrement(entries)) => entries
                    .iter()
                    .find(|(name, _)| name.as_str() == "page")
                    .map(|(_, value)| *value),
                _ => None,
            }
        });
        let mut context = raikiri_dom::MarginBoxPageContext::new(
            self.slice.page_index,
            self.page_count,
            page_is_left,
        )
        .with_paired_page_increment(paired_page_increment);
        if self.page_count_deferred {
            context = context.with_deferred_page_count();
        }
        raikiri_dom::page_margin_boxes_with_values(
            self.document,
            self.cascade,
            self.style,
            self.geometry.page_box,
            context,
            self.margin_values,
        )
    }

    /// The laid-out document, its cascade, the page box and the page's
    /// origin in the shared flow, for painting the page with the built-in
    /// painter.
    #[doc(hidden)]
    pub fn paint_inputs(
        &self,
    ) -> (
        &'a raikiri_dom::Document,
        &'a raikiri_style::CascadeResult,
        raikiri_traits::PageBox,
        f32,
    ) {
        (
            self.document,
            self.cascade,
            self.geometry.page_box,
            self.slice.content_origin_y,
        )
    }

    /// The root element's box on this page, which the canvas background's
    /// images are sized and positioned against (CSS Backgrounds 3 §2.11.2).
    ///
    /// The canvas runs through every page as one surface, so on a later page
    /// the box sits above the page by the flow the earlier pages hold: an
    /// image that does not repeat shows only where the box reaches, and
    /// tiles continue from the page before. Paint the canvas background
    /// over the page area whatever the box covers.
    ///
    /// The box carries the root's `transform`, which [`Self::paint_order`]
    /// leaves out: it moves the whole canvas, its background images and
    /// every paint event included (CSS Transforms 1 §6). `None` when the
    /// document has no root element.
    ///
    /// An auto-height root is as tall as the laid-out content. A page
    /// streamed before the input ended comes from a layout of the content
    /// that had arrived, so the box ends where that content ended, and one
    /// from a layout resumed after delivered pages starts where it resumed.
    /// The canvas background does not show it: a streaming layout holds the
    /// pages back while images of the canvas fill the box or are placed by
    /// its height, and lays the whole document out for tiles that start at
    /// its top.
    pub fn root_box(&self) -> Option<super::RootBox> {
        let page_count = if self.page_count_deferred {
            self.slice.page_index + 1
        } else {
            self.page_count
        };
        raikiri_dom::root_box(
            self.document,
            self.cascade,
            self.geometry.page_box,
            self.slice.content_origin_y,
            page_count,
        )
    }

    /// The running element (`position: running(<name>)`) that a page margin
    /// box with `content: element(<name>, <fetch>)` shows on this page, per
    /// CSS GCPM 3 §1.2.2 <https://www.w3.org/TR/css-gcpm-3/#element-syntax>.
    ///
    /// A running element is assigned on the page of the rendered content
    /// that follows it in document order. `first` (the default keyword) and
    /// `last` take the first or last element assigned on this page, `start`
    /// the first one only when no content of this page comes before it, and
    /// `first-except` none on a page with an assignment; otherwise each
    /// falls back to the last element assigned on an earlier page. `None`
    /// when no element of that name applies yet.
    ///
    /// Lay the element out with
    /// [`DocumentLayout::layout_running_element`](super::DocumentLayout::layout_running_element).
    pub fn running_element(&self, name: &str, fetch: StringFetchMode) -> Option<NodeId> {
        self.running?
            .index
            .select(name, fetch, self.slice.page_index)
    }

    /// The running element `margin_box` shows on this page, laid out at the
    /// width of its content box and aligned in it by the box's
    /// `vertical-align` (CSS GCPM 3 §1.2.2). `None` when the box shows no
    /// running element or none applies on this page.
    ///
    /// Draw the element's page at [`PlacedRunningElement::origin`] in place
    /// of [`MarginBox::text`], clipped by [`MarginBox::overflow_clip`] when
    /// the box has one; with `overflow: visible` the element may overflow
    /// the box.
    pub fn margin_box_running_element(
        &self,
        margin_box: &MarginBox,
    ) -> Result<Option<PlacedRunningElement<'a>>, RenderError> {
        let (Some(source), Some(running)) = (self.running, margin_box.running.as_ref()) else {
            return Ok(None);
        };
        let Some(node) = source
            .index
            .select(&running.name, running.fetch, self.slice.page_index)
        else {
            return Ok(None);
        };
        let content = running.content_box;
        let Some(layout) = source.layout(node, content.width)? else {
            return Ok(None); // cov:ignore: a selected node is always a running element.
        };
        let free = (content.height - layout.height()).max(0.0);
        Ok(Some(PlacedRunningElement {
            layout,
            origin: (content.x, content.y + free * running.block_align),
        }))
    }

    /// [`Self::margin_box_running_element`], laid out anew rather than
    /// borrowed from the document layout's cache, so that it can outlive
    /// the layout.
    pub(crate) fn owned_margin_box_running_element(
        &self,
        margin_box: &MarginBox,
    ) -> Result<Option<OwnedRunningElement>, RenderError> {
        let (Some(source), Some(running)) = (self.running, margin_box.running.as_ref()) else {
            return Ok(None);
        };
        let Some(node) = source
            .index
            .select(&running.name, running.fetch, self.slice.page_index)
        else {
            return Ok(None);
        };
        let content = running.content_box;
        let Some(layout) = super::running::layout_running_element(
            source.document,
            source.cascade,
            node,
            super::running::used_width(content.width),
            source.layouts.faces().clone(),
            source.svg,
        )?
        else {
            return Ok(None); // cov:ignore: a selected node is always a running element.
        };
        let free = (content.height - layout.height()).max(0.0);
        let origin = (content.x, content.y + free * running.block_align);
        Ok(Some(OwnedRunningElement { layout, origin }))
    }

    /// Structure and attributes of the document.
    pub fn dom(&self) -> DomView<'a> {
        DomView::new(self.document)
    }

    /// Computed values from the cascade used for layout. `None` for an
    /// out-of-range node.
    pub fn computed(&self, node: NodeId) -> Option<&'a ComputedValues> {
        self.cascade.computed.get(usize::try_from(node.0).ok()?)
    }

    /// The `@page` cascade for this page's context.
    pub fn page_style(&self) -> &'a PageCascadeResult {
        self.style
    }
    /// Links on this page, with their clickable areas both in layout space
    /// and where the built-in painter draws them.
    pub fn links(&self) -> impl Iterator<Item = super::Link<'a>> + 'a + use<'a> {
        let paint_quads = self
            .document
            .page_link_paint_quads(self.cascade, self.slice.page_index);
        self.document
            .page_links(self.slice.page_index)
            .zip(paint_quads)
            .map(|((owner, target, quads), paint_quads)| super::Link {
                owner,
                target,
                quads,
                paint_quads,
            })
    }

    /// The transform the built-in painter draws the box of `node` on this
    /// page through: the [`PaintEvent::PushTransform`] events around the
    /// node's events in [`Self::paint_order`], composed, or the identity
    /// when there are none. It maps [`Fragment::rect`] and the other layout
    /// space positions of the node to the page. Text takes the transform of
    /// the element it is in and, inside columns, the offsets that move its
    /// lines.
    ///
    /// Like the paint events, the transform is inside the root element's
    /// own transform, which [`Self::root_box`] reports and the painter
    /// applies around the whole page.
    ///
    /// `None` when the painter draws nothing of `node` because a transform
    /// on it or an ancestor is singular, and for an out-of-range node.
    pub fn paint_transform(&self, node: NodeId) -> Option<PaintTransform> {
        self.document
            .page_paint_transforms(self.cascade, self.slice.page_index, [node])
            .pop()
            .flatten()
    }
}

/// Whether every coordinate of `rect` is finite.
fn is_finite_rect(rect: &PaintRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .into_iter()
        .all(f32::is_finite)
}

/// Whether `rect` is finite and has a positive area.
fn is_drawable_rect(rect: &PaintRect) -> bool {
    is_finite_rect(rect) && rect.width > 0.0 && rect.height > 0.0
}

/// The raster size that paints an image at `rect`.
fn raster_size(rect: &PaintRect) -> raikiri_traits::ImageRasterSize {
    raikiri_traits::ImageRasterSize {
        width: rect.width,
        height: rect.height,
    }
}

/// The cached pixels of `url` at `size`, when they are non-empty and their
/// RGBA buffer matches their dimensions.
fn checked_pixels(
    source: &dyn raikiri_traits::ImagePixelSource,
    url: &url::Url,
    size: raikiri_traits::ImageRasterSize,
) -> Option<std::sync::Arc<raikiri_traits::DecodedImage>> {
    let pixels = source.get_decoded_at_size(url, size, None)?;
    let bytes = (pixels.width as usize)
        .checked_mul(pixels.height as usize)?
        .checked_mul(4)?;
    (pixels.width != 0 && pixels.height != 0 && pixels.rgba.len() == bytes).then_some(pixels)
}
