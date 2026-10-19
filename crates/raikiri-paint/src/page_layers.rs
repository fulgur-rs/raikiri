//! The layers a page paints around the document's own boxes.
//!
//! A page is painted back to front: the paper and the margin boxes behind the
//! canvas, the canvas, the page border and outline, the document, and the
//! margin boxes in front of it. The document itself is painted by the caller,
//! between [`PageLayers::paint_behind_document`] and
//! [`PageLayers::paint_in_front_of_document`].

use anyrender::PaintScene;
use raikiri_dom::{Document, MarginBox, RootBox};
use raikiri_style::CascadeResult;
use raikiri_traits::{ImagePixelSource, PageBox, RenderWarning};

use crate::walk;

/// What every layer of one page reads: the laid-out document, its cascade, the
/// page box and, when images are painted, where their pixels come from.
pub(crate) struct PageLayers<'a> {
    document: &'a Document,
    cascade: &'a CascadeResult,
    page_box: PageBox,
    pixel_source: Option<&'a dyn ImagePixelSource>,
}

impl<'a> PageLayers<'a> {
    pub(crate) fn new(
        document: &'a Document,
        cascade: &'a CascadeResult,
        page_box: PageBox,
        pixel_source: Option<&'a dyn ImagePixelSource>,
    ) -> Self {
        Self {
            document,
            cascade,
            page_box,
            pixel_source,
        }
    }

    /// Paint everything behind the document: the page background, the margin
    /// boxes `behind` the canvas, the canvas background of `root`, and the page
    /// border and outline.
    ///
    /// The canvas, the page border and the document contents are one layer at
    /// level zero. The margin boxes below it go right after the page
    /// background (CSS Page 3 §3.1).
    pub(crate) fn paint_behind_document(
        &self,
        scene: &mut impl PaintScene,
        image_cache: &walk::ImageBrushCache,
        warnings: &mut Vec<RenderWarning>,
        behind: &[MarginBox],
        root: Option<&RootBox>,
    ) {
        walk::paint_page_background(
            image_cache,
            scene,
            self.document,
            self.cascade,
            self.page_box,
            self.pixel_source,
            warnings,
        );
        walk::paint_page_margin_boxes(
            image_cache,
            scene,
            behind,
            self.page_box,
            self.pixel_source,
            warnings,
        );
        walk::paint_canvas_background(
            image_cache,
            scene,
            self.document,
            self.cascade,
            self.page_box,
            root,
            self.pixel_source,
            warnings,
        );
        walk::paint_page_border(
            image_cache,
            scene,
            self.document,
            self.cascade,
            self.page_box,
            self.pixel_source,
            warnings,
        );
        walk::paint_page_outline(scene, self.cascade, self.page_box);
    }

    /// Paint the margin boxes `in_front` of the document (CSS Page 3 §3.1).
    pub(crate) fn paint_in_front_of_document(
        &self,
        scene: &mut impl PaintScene,
        image_cache: &walk::ImageBrushCache,
        warnings: &mut Vec<RenderWarning>,
        in_front: &[MarginBox],
    ) {
        walk::paint_page_margin_boxes(
            image_cache,
            scene,
            in_front,
            self.page_box,
            self.pixel_source,
            warnings,
        );
    }
}
