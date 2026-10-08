use super::{DomView, Fragment, PaintEvent, PositionedGlyphRun};
use raikiri_style::{ComputedValues, PageCascadeResult};
use raikiri_traits::{NodeId, PaintInsets, PaintRect};

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

/// One laid-out page, borrowed from a [`super::DocumentLayout`].
#[derive(Clone, Copy)]
pub struct Page<'a> {
    pub(super) slice: &'a raikiri_dom::PageSlice,
    pub(super) geometry: &'a crate::render::ResolvedPageGeometry,
    pub(super) style: &'a PageCascadeResult,
    pub(super) document: &'a raikiri_dom::Document,
    pub(super) cascade: &'a raikiri_style::CascadeResult,
}

impl<'a> Page<'a> {
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

    /// Prepares the vector source of an SVG root placed by this page.
    ///
    /// Pass a fragment from [`Self::fragments`]. Ordinary elements, text and
    /// empty or hidden SVG viewports return `None`. The host box is drawn
    /// separately; source admission and preparation errors are returned.
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
        if viewport.width <= 0.0
            || viewport.height <= 0.0
            || computed.visibility != raikiri_style::property::Visibility::Visible
        {
            return Ok(None);
        }
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
        let source = document.styled_source_with_root_color_and_font(
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
                visible: true,
            },
            [color.r, color.g, color.b, color.a],
            computed.font_size.0,
            &computed
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
                .join(","),
        )?;
        Ok(Some(InlineSvg {
            source,
            viewport,
            host_opacity,
        }))
    }

    /// Positioned glyph runs of the body text on this page, in document
    /// order, at the positions the built-in painter draws them.
    ///
    /// A line belongs to the page that holds its center, the same rule as
    /// [`Fragment::line_range`], so every line appears on exactly one page;
    /// a paragraph repeated on every page (inside `position: fixed`) appears
    /// on each. Propagated text decorations are included; margin boxes and
    /// shadows are not included yet.
    /// A paragraph whose text is placed by geometry the runs do not model
    /// has no runs here: a vertical writing mode, a multicol container's
    /// columns, relatively positioned inline elements, a transform or
    /// relative offset on the paragraph or an ancestor, or a fixed box not
    /// placed by `top` and `left` lengths. Each such paragraph is reported
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
    /// Generated content and markers are not listed yet, and multi-column
    /// containers are listed without column clips (reported once in
    /// [`super::DocumentLayout::warnings`] with
    /// [`raikiri_traits::WarningKind::PaintOrderApproximated`]).
    ///
    /// Where the built-in painter differs: it draws the `<body>` box on every
    /// page, while this list has it only on the pages its box reaches; and it
    /// draws each line's inline element backgrounds just before that line's
    /// text, while this list has all of a paragraph's inline element boxes
    /// before all of its text.
    pub fn paint_order(&self) -> Vec<PaintEvent<'a>> {
        self.document.page_paint_order(
            self.cascade,
            self.slice.page_index,
            self.slice.page_name.as_deref(),
        )
    }

    /// Body paint order using the supplied [`Self::text_runs`] instead of
    /// DOM text fragments. Each [`PaintEvent::TextLine`] identifies all runs
    /// of one paragraph line, including generated text and ellipses, inside
    /// the paragraph's ancestor clips and opacity groups.
    ///
    /// Pass the runs from this page. No text is extracted or shaped again.
    /// Boxes, replaced content and the approximations of [`Self::paint_order`]
    /// retain their existing order; paragraphs omitted by [`Self::text_runs`]
    /// have no text events here. Inline element opacity is not represented.
    pub fn paint_order_for_text_runs(
        &self,
        runs: &[PositionedGlyphRun<'a>],
    ) -> Vec<PaintEvent<'a>> {
        self.document
            .page_paint_order_for_text_runs(self.cascade, self.slice.page_index, runs)
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
    /// Links on this page. Quads are in layout space until paint-space
    /// positions are exposed.
    pub fn links(&self) -> impl Iterator<Item = super::Link<'a>> + 'a + use<'a> {
        self.document
            .page_links(self.slice.page_index)
            .map(|(owner, target, quads)| super::Link {
                owner,
                target,
                quads,
            })
    }
}
