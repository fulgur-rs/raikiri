//! Where one page's content sits on the paper: the offsets the paint sites
//! apply, the clips of the page area, and the lines the page owns.

use super::{FLOW_EXTENT, FlowBand, PageLines, find_html, page_area_rect};
use kurbo::{Affine, Rect};
use raikiri_dom::{Document, PageContentInsets, PageMargins};
use raikiri_style::CascadeResult;
use raikiri_traits::PageBox;

/// The geometry of one page, found once before its nodes are walked.
pub(super) struct PageGeometry {
    pub(super) margins: PageMargins,
    pub(super) insets: PageContentInsets,
    /// The page area inside the page border and padding.
    pub(super) content_width: f32,
    pub(super) content_height: f32,
    /// The page area in page-local coordinates, which `fixed` background
    /// layers are positioned in.
    pub(super) page_area: Rect,
    /// Whether the root element's direction is right to left.
    pub(super) fixed_rtl: bool,
    /// The width fixed-position containing blocks take, from the initial
    /// layout.
    pub(super) fixed_content_width: f32,
    /// Whether pages follow the x axis, and if so whether the flow runs from
    /// right to left.
    pub(super) pages_follow_x_axis: Option<bool>,
    /// The span of the flow along y that the page holds.
    pub(super) page_top: f32,
    pub(super) page_bottom: f32,
    /// The translation from the flow to the page.
    pub(super) page_offset_x: f32,
    pub(super) page_offset_y: f32,
    /// The part of the page offset that viewport-fixed boxes take back out.
    pub(super) fixed_flow_shift_x: f32,
    pub(super) fixed_origin_y: f32,
    /// The transform that undoes the root element's, for page-area clips.
    pub(super) page_clip_transform: Affine,
    /// The physical content box that element backgrounds are clipped to.
    pub(super) page_content_clip: Option<Rect>,
    /// The band of the flow a page along the x axis holds.
    pub(super) flow_band: Option<FlowBand>,
    /// The lines the page owns of a paragraph that runs over several pages.
    pub(super) page_lines: Option<PageLines>,
}

impl PageGeometry {
    pub(super) fn new(
        document: &Document,
        cascade: &CascadeResult,
        page_box: PageBox,
        content_origin_y: f32,
        fixed_page_width: f32,
        root_transform: Affine,
    ) -> Self {
        let margins = raikiri_dom::page_margins(cascade, page_box);
        let insets = raikiri_dom::page_content_insets(cascade, page_box);
        // Keep fixed-position sizing consistent with layout: the page area inside
        // the page border and padding.
        let content_width = insets.page_area_width(margins, page_box);
        let content_height = insets.page_area_height(margins, page_box);
        // The page area in page-local coordinates, which `fixed` background
        // layers are positioned in (CSS Backgrounds 3 section 2.5).
        let page_area = page_area_rect(margins, insets, page_box);
        // Fixed-position containing blocks use the page area of the initial
        // layout even when a later named page has a different paper width.
        // The page area takes the root element's direction, which decides the
        // inset that places an over-constrained fixed box.
        let fixed_rtl = find_html(document)
            .and_then(|html| cascade.computed.get(html))
            .is_some_and(|cv| cv.direction == raikiri_style::property::Direction::Rtl);
        let fixed_content_width = if fixed_page_width.is_finite() && fixed_page_width > 0.0 {
            fixed_page_width
        } else {
            content_width
        };
        // The walker keeps source coordinates in the stack.  Only the paint sites
        // receive the page translation, which makes page membership testable even
        // when an ancestor spans a page boundary.
        let pages_follow_x_axis = raikiri_dom::pages_follow_x_axis(document, cascade);
        let (page_top, page_bottom, page_offset_x, page_offset_y) = match pages_follow_x_axis {
            // Pages that follow the x axis hold a band of the flow across the
            // whole page height: every box is on the page along y, and the
            // page area decides along x which boxes and lines the page holds. Flowing right to left, the
            // band starts at the page area's right edge, so on a page whose area
            // is narrower or wider than the one this document was laid out in it
            // moves with that edge.
            Some(right_to_left) => (
                -FLOW_EXTENT,
                FLOW_EXTENT,
                margins.left
                    + insets.left
                    + if right_to_left {
                        let layout_width = document
                            .laid_out_page_area_width()
                            .unwrap_or(fixed_content_width);
                        content_origin_y + content_width - layout_width
                    } else {
                        -content_origin_y
                    },
                margins.top + insets.top,
            ),
            None => (
                content_origin_y,
                content_origin_y + content_height,
                margins.left + insets.left,
                margins.top + insets.top - content_origin_y,
            ),
        };
        // Viewport-fixed boxes repeat at the same place on every page, so they
        // are placed against the page, not against the band of the flow that
        // the page offsets above carry.
        // The whole band offset, including a right-to-left page's edge shift,
        // is taken back out so those boxes stay page-local.
        let (fixed_flow_shift_x, fixed_origin_y) = if pages_follow_x_axis.is_some() {
            (page_offset_x - (margins.left + insets.left), 0.0)
        } else {
            (0.0, content_origin_y)
        };
        // Element backgrounds can extend past the page content box, for example
        // through negative margins. Clip those backgrounds to the physical
        // content box without clipping text ink, which may legitimately overflow
        // its box.
        // `scene` paints through the root element's `root_transform`, which
        // moves the document but not the page: page-area clips undo it.
        let page_clip_transform = root_transform.inverse();
        let page_content_clip = if margins.left > 0.0
            || margins.right > 0.0
            || margins.top > 0.0
            || margins.bottom > 0.0
            || insets.left > 0.0
            || insets.right > 0.0
            || insets.top > 0.0
            || insets.bottom > 0.0
        {
            let left = margins.left + insets.left;
            Some(Rect::new(
                left as f64,
                (margins.top + insets.top) as f64,
                (page_box.width - margins.right - insets.right).max(left) as f64,
                (margins.top + insets.top + content_height) as f64,
            ))
        } else {
            None
        };
        // A page that follows the x axis holds the boxes and lines that lie
        // across its page area, as pages along y hold those across their page
        // height; nothing else of the flow beside it is painted.
        let flow_band = pages_follow_x_axis.map(|right_to_left| {
            let left = margins.left + insets.left;
            FlowBand {
                left,
                right: left + content_width,
                right_to_left,
                root_transform,
            }
        });
        // A paragraph that runs over several pages paints on each only the
        // lines that page owns, not the ones layout moved beside it into the
        // page margins. The first page also owns whatever lies before the flow.
        let page_lines = Some(flow_band.map_or_else(
            || PageLines::AlongY {
                start: if content_origin_y > 0.0 {
                    content_origin_y
                } else {
                    -FLOW_EXTENT
                },
                end: content_origin_y + content_height,
                paint_shift: page_offset_y,
            },
            PageLines::AlongX,
        ));
        Self {
            margins,
            insets,
            content_width,
            content_height,
            page_area,
            fixed_rtl,
            fixed_content_width,
            pages_follow_x_axis,
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
        }
    }
}
