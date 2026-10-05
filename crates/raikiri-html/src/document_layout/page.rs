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

    /// Positioned glyph runs of the body text on this page, in document
    /// order, at the positions the built-in painter draws them.
    ///
    /// A line belongs to the page that holds its center, the same rule as
    /// [`Fragment::line_range`], so every line appears on exactly one page;
    /// a paragraph repeated on every page (inside `position: fixed`) appears
    /// on each. Margin boxes, decorations and shadows are not included yet.
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
