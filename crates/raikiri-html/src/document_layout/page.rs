use super::{DomView, Fragment};
use raikiri_style::{ComputedValues, PageCascadeResult};
use raikiri_traits::{NodeId, PageFragment, PaintInsets, PaintRect};

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
    pub(super) fragment: &'a PageFragment,
    pub(super) style: &'a PageCascadeResult,
    pub(super) document: &'a raikiri_dom::Document,
    pub(super) cascade: &'a raikiri_style::CascadeResult,
}

impl<'a> Page<'a> {
    /// Zero-based page index.
    pub fn index(&self) -> u32 {
        self.fragment.page_index
    }

    /// Named page, if the page context selected one.
    pub fn name(&self) -> Option<&'a str> {
        self.fragment.page_name.as_deref()
    }

    /// Page geometry.
    pub fn geometry(&self) -> PageGeometry {
        let f = self.fragment;
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
    pub fn fragments(&self) -> impl Iterator<Item = Fragment<'a>> + 'a {
        let content_box = self.fragment.content_box;
        self.fragment
            .items
            .iter()
            .map(move |item| Fragment::new(item, content_box))
    }

    /// Structure and attributes of the document.
    pub fn dom(&self) -> DomView<'a> {
        DomView::new(self.document)
    }

    /// Computed values from the cascade used for layout. `None` for an
    /// out-of-range node.
    pub fn computed(&self, node: NodeId) -> Option<&'a ComputedValues> {
        self.cascade.computed.get(node.0 as usize)
    }

    /// The `@page` cascade for this page's context.
    pub fn page_style(&self) -> &'a PageCascadeResult {
        self.style
    }
}
