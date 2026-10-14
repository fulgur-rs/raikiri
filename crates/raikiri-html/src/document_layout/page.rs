use super::{
    DomView, Fragment, GeneratedImageId, MarginBox, OverflowClip, PaintEvent, PositionedGlyphRun,
    TextLineId,
};
use crate::RenderError;
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

/// Prepared vector source and resolved placement of an inline SVG root.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct InlineSvg {
    /// Standalone XML with the viewport, root color, and root font resolved.
    pub source: String,
    /// Whole content box in CSS page coordinates, before pagination cuts.
    pub viewport: PaintRect,
    /// Host root opacity represented by the page's paint events.
    ///
    /// When present, resolve SVG inheritance first, then remove only the
    /// parsed root group's opacity. Descendant opacity remains in the SVG;
    /// ancestor opacity remains in the surrounding page paint events.
    pub host_opacity: Option<f32>,
}

/// Prepared inline SVG sources of one laid-out document, keyed by SVG root
/// node and viewport size.
///
/// The prepared source depends only on the root's subtree, its computed
/// styles and its viewport size, so a root painted on several pages (a
/// running header, or an SVG split across a page break) is serialized,
/// parsed and restyled once. Placement is taken from each fragment.
#[derive(Debug, Default)]
pub(crate) struct InlineSvgCache {
    sources: std::cell::RefCell<rustc_hash::FxHashMap<(usize, u32, u32), PreparedSvg>>,
}

#[cfg(test)]
impl InlineSvgCache {
    pub(super) fn len(&self) -> usize {
        self.sources.borrow().len()
    }
}

#[derive(Debug, Clone)]
struct PreparedSvg {
    source: String,
    host_opacity: Option<f32>,
}

/// One laid-out page, borrowed from a [`super::DocumentLayout`].
#[derive(Clone, Copy)]
pub struct Page<'a> {
    pub(super) slice: &'a raikiri_dom::PageSlice,
    pub(super) geometry: &'a crate::render::ResolvedPageGeometry,
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
    /// Prepared inline SVG sources of [`Self::document`].
    pub(super) svg_sources: &'a InlineSvgCache,
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
        if ![rect.x, rect.y, rect.width, rect.height]
            .into_iter()
            .all(f32::is_finite)
        {
            return None;
        }
        let pixels = source.get_decoded_at_size(
            &url,
            raikiri_traits::ImageRasterSize {
                width: rect.width,
                height: rect.height,
            },
            None,
        )?;
        let bytes = (pixels.width as usize)
            .checked_mul(pixels.height as usize)?
            .checked_mul(4)?;
        if pixels.width == 0 || pixels.height == 0 || pixels.rgba.len() != bytes {
            return None;
        }
        Some(RasterImage {
            node: fragment.node(),
            url,
            pixels,
            rect,
            clip: raikiri_traits::PaintClip::new(content),
            clip_owner: Some(fragment.node()),
        })
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
        let (url, rect) = self.document.page_marker_image(self.index(), owner)?;
        if ![rect.x, rect.y, rect.width, rect.height]
            .into_iter()
            .all(f32::is_finite)
            || rect.width <= 0.0
            || rect.height <= 0.0
        {
            return None;
        }
        let pixels = source.get_decoded_at_size(
            url,
            raikiri_traits::ImageRasterSize {
                width: rect.width,
                height: rect.height,
            },
            None,
        )?;
        let bytes = (pixels.width as usize)
            .checked_mul(pixels.height as usize)?
            .checked_mul(4)?;
        if pixels.width == 0 || pixels.height == 0 || pixels.rgba.len() != bytes {
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
        Some(RasterImage {
            node: owner,
            url: url.clone(),
            pixels,
            rect,
            clip: raikiri_traits::PaintClip::new(rect),
            clip_owner,
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
        if ![rect.x, rect.y, rect.width, rect.height]
            .into_iter()
            .all(f32::is_finite)
            || rect.width <= 0.0
            || rect.height <= 0.0
        {
            return None;
        }
        // The size layout prepared the pixels at, which stays bounded when
        // `object-fit` makes the object far larger than its box.
        let raster = owner
            .and_then(|owner| {
                self.document
                    .content_image_raster_size(owner, pseudo, image.index)
            })
            .unwrap_or(raikiri_traits::ImageRasterSize {
                width: rect.width,
                height: rect.height,
            });
        let pixels = source.get_decoded_at_size(url, raster, None)?;
        let bytes = (pixels.width as usize)
            .checked_mul(pixels.height as usize)?
            .checked_mul(4)?;
        if pixels.width == 0 || pixels.height == 0 || pixels.rgba.len() != bytes {
            return None;
        }
        Some(RasterImage {
            node: image.owner,
            url: url.clone(),
            pixels,
            rect,
            clip: raikiri_traits::PaintClip::new(content),
            clip_owner: Some(image.owner),
        })
    }

    /// Prepares the vector source of an SVG root placed by this page.
    ///
    /// Pass a fragment from [`Self::fragments`]. Ordinary elements, text and
    /// empty SVG viewports return `None`. Hidden roots retain source so their
    /// explicitly visible descendants can still be drawn. The host box is drawn
    /// separately; source admission and preparation errors are returned.
    /// Fixed boxes placed through other insets than `top` and `left` lengths
    /// are rejected, including when the fixed box is an ancestor. Their paint
    /// placement is not represented by the fragment projection yet.
    pub fn inline_svg(
        &self,
        fragment: &Fragment<'a>,
    ) -> Result<Option<InlineSvg>, raikiri_svg::SvgError> {
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
        if let Some(prepared) = self.svg_sources.sources.borrow().get(&key) {
            return Ok(Some(InlineSvg {
                source: prepared.source.clone(),
                viewport,
                host_opacity: prepared.host_opacity,
            }));
        }
        let Some(prepared) = self.prepare_inline_svg(fragment, id, viewport, computed)? else {
            return Ok(None); // cov:ignore: The checked SVG namespace/root predicate guarantees a serializable root.
        };
        let svg = InlineSvg {
            source: prepared.source.clone(),
            viewport,
            host_opacity: prepared.host_opacity,
        };
        self.svg_sources.sources.borrow_mut().insert(key, prepared);
        Ok(Some(svg))
    }

    /// Serializes, parses and restyles the SVG root of `fragment`.
    fn prepare_inline_svg(
        &self,
        fragment: &Fragment<'a>,
        id: usize,
        viewport: PaintRect,
        computed: &ComputedValues,
    ) -> Result<Option<PreparedSvg>, raikiri_svg::SvgError> {
        let dom = self.dom();
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
        let mut node_styles = Vec::new();
        let mut remaining_style_bytes = 32 * 1024 * 1024usize;
        let mut stack = vec![fragment.node()];
        let mut element_index = 0;
        while let Some(node) = stack.pop() {
            if dom.kind(node) == Some(raikiri_traits::NodeKind::Element) {
                if element_index != 0 {
                    let properties = self
                        .cascade
                        .svg_style_properties(raikiri_style::StyleNodeId(node.0));
                    if !properties.is_empty()
                        && let Some(style) = self.computed(node)
                    {
                        let declarations =
                            svg_node_declarations(style, properties, &mut remaining_style_bytes)?;
                        node_styles.push((element_index, declarations));
                    }
                }
                element_index += 1;
            }
            let children: Vec<_> = dom.children(node).collect();
            stack.extend(children.into_iter().rev());
        }
        let element_styles: Vec<_> = node_styles
            .iter()
            .map(
                |(element_index, declarations)| raikiri_svg::SvgElementStyle {
                    element_index: *element_index,
                    declarations,
                },
            )
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
                family: &svg_font_family(computed),
            },
            &element_styles,
        )?;
        Ok(Some(PreparedSvg {
            source,
            host_opacity,
        }))
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
    /// stacking level. A paragraph's inline boxes come before its text.
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
    /// Where the built-in painter differs: it draws the `<body>` box on every
    /// page, while this list has it only on the pages its box reaches; and it
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
    /// pseudo-element's own id instead of an element's.
    /// A `::first-line` with opacity below one, on the paragraph or on an
    /// ancestor block whose first line it holds, makes a group of the
    /// paragraph's first line, listed
    /// before the paragraph's other inline groups: a
    /// [`PaintEvent::PushOpacity`], the content of the first line in the
    /// order above, and the [`PaintEvent::PopOpacity`]. An inline element on
    /// the first line and on later ones is split by line: in place of its
    /// [`PaintEvent::Box`], its piece on the first line is a
    /// [`PaintEvent::LineBox`] in the group, and its other pieces are
    /// [`PaintEvent::LineBox`] events with the other lines. Floats and atomic
    /// inlines not on the first line stay with the other lines.
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
    /// translucent `::before` or `::after` belong to that pseudo-element's
    /// group. Its id is not a node of [`Self::dom`]; it is the group of the
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
    /// order: the top row, the bottom row, the left and right columns, then
    /// the corners. Draw them after the page background and border and
    /// before [`Self::paint_order`].
    ///
    /// Generated content is resolved for this page: `counter(page)`,
    /// `counter(pages)`, quotes, and the document-wide values of `string()`
    /// and `element()`. Each box's text is available as glyph runs through
    /// [`MarginBox::text_runs`].
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
    pub fn root_box(&self) -> Option<super::RootBox> {
        raikiri_dom::root_box(
            self.document,
            self.cascade,
            self.geometry.page_box,
            self.slice.content_origin_y,
            self.page_count,
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
    /// Draw the element's page at [`PlacedRunningElement::origin`], clipped
    /// to the margin box, in place of [`MarginBox::text`].
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
    /// `None` when the painter draws nothing of `node` because a transform
    /// on it or an ancestor is singular, and for an out-of-range node.
    pub fn paint_transform(&self, node: NodeId) -> Option<PaintTransform> {
        self.document
            .page_paint_transforms(self.cascade, self.slice.page_index, [node])
            .pop()
            .flatten()
    }
}

fn svg_font_family(style: &ComputedValues) -> String {
    style
        .font_family
        .iter()
        .map(|family| {
            if family.1 == raikiri_style::property::FontFamilyKind::Generic {
                family.as_str().to_owned()
            } else {
                let name = family
                    .as_str()
                    .replace('\\', "\\\\")
                    .replace('"', "\\\"")
                    .replace('\n', "\\a ")
                    .replace('\r', "\\d ")
                    .replace('\u{c}', "\\c ");
                format!("\"{name}\"")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn svg_node_declarations(
    style: &ComputedValues,
    properties: &[raikiri_style::cascade::SvgStyleProperty],
    remaining_bytes: &mut usize,
) -> Result<String, raikiri_svg::SvgError> {
    use raikiri_style::property::PropertyKey;
    let mut css = String::new();
    // Bound metadata before allocating repeated declarations from the host CSS.
    let mut consume = |bytes: usize| {
        *remaining_bytes = remaining_bytes.checked_sub(bytes).ok_or_else(|| {
            raikiri_svg::SvgError::InvalidDocument(
                "SVG descendant style source limit exceeded".into(),
            )
        })?;
        Ok::<(), raikiri_svg::SvgError>(())
    };
    consume(128)?;
    for property in properties {
        let size = if property.property == PropertyKey::FontFamily && !property.inherited {
            style.font_family.iter().fold(128usize, |size, name| {
                size.saturating_add(name.as_str().len().saturating_mul(8))
            })
        } else {
            128
        };
        consume(size)?;
        let (name, value) = if property.inherited {
            let name = match property.property {
                PropertyKey::Color => "color",
                PropertyKey::Display => "display",
                PropertyKey::Opacity => "opacity",
                PropertyKey::Visibility => "visibility",
                PropertyKey::FontSize => "font-size",
                PropertyKey::FontFamily => "font-family",
                PropertyKey::FontStyle => "font-style",
                PropertyKey::FontWeight => "font-weight",
                _ => continue,
            };
            // SVG renderers such as usvg copy an ancestor's raw `font-size`
            // for `inherit` and apply it again, compounding a relative size.
            // `100%` has the same computed value without that copy.
            let value = if property.property == PropertyKey::FontSize {
                "100%"
            } else {
                "inherit"
            };
            (name, value.to_owned())
        } else {
            match property.property {
                PropertyKey::Color => {
                    let color = style.color;
                    (
                        "color",
                        format!(
                            "rgba({},{},{},{:.6})",
                            color.r,
                            color.g,
                            color.b,
                            f32::from(color.a) / 255.0
                        ),
                    )
                }
                PropertyKey::Display => ("display", style.display.as_css_str().to_owned()),
                PropertyKey::Opacity => ("opacity", style.opacity.to_string()),
                PropertyKey::Visibility => ("visibility", style.visibility.as_css_str().to_owned()),
                PropertyKey::FontSize => ("font-size", format!("{}px", style.font_size.0)),
                PropertyKey::FontFamily => ("font-family", svg_font_family(style)),
                PropertyKey::FontStyle => ("font-style", style.font_style.as_css_str().to_owned()),
                PropertyKey::FontWeight => ("font-weight", style.font_weight.to_string()),
                _ => continue,
            }
        };
        let value = property
            .expression
            .as_ref()
            .filter(|_| !property.inherited)
            .unwrap_or(&value);
        css.push_str(name);
        css.push(':');
        css.push_str(value);
        css.push_str("!important;");
    }
    Ok(css)
}

#[cfg(test)]
mod tests;
