//! The element whose background the canvas shows, and what the images of
//! that background take from the root element's box.

use super::root_box::root_element;
use crate::Document;
use raikiri_style::property::{BackgroundAttachment, BackgroundImage, BackgroundRepeatKeyword};
use raikiri_style::{
    CascadeResult, ComputedBackgroundLayer, ComputedBackgroundSize, ComputedCssPositionOffset,
    ComputedLengthPercentage, ComputedLengthPercentageOrAuto,
};

/// What the images of the canvas background take from the root element's
/// box, which a layout that has not seen the whole document only knows in
/// part.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CanvasBoxUse {
    /// Nothing: the canvas shows a color, or its images are fixed to the
    /// page area.
    None,
    /// Where the root's box starts along the flow of the pages. Tiles that
    /// are laid from that edge in lengths of their own come out the same
    /// wherever the root ends.
    Start,
    /// The size of the root's box along the flow of the pages too: images
    /// that fill it, are placed in it by a percentage or from its far edge,
    /// or are spread over it by `space` or `round`.
    Extent,
}

/// The element whose background paints the canvas (CSS Backgrounds 3
/// §2.11.2): the root element, or the `<body>` when the root's background
/// is transparent and has no image.
#[doc(hidden)]
pub fn canvas_background_owner(document: &Document, cascade: &CascadeResult) -> Option<usize> {
    let html_id = root_element(document)?;
    let html = cascade.computed.get(html_id)?;
    let html_is_transparent = !html.has_background_image() && html.background_color.a == 0;
    if !html_is_transparent {
        return Some(html_id);
    }

    // Only a `body` propagates its background, not a `frameset` (CSS
    // Backgrounds 3, 2.11.2).
    let body_id = crate::paint_rules::find_paint_root(document)
        .filter(|&id| document.get_node(id).and_then(|node| node.tag_name()) == Some("body"))?;
    let body = cascade.computed.get(body_id)?;
    let body_has_background = body.has_background_image() || body.background_color.a != 0;
    body_has_background.then_some(body_id)
}

/// What the images of the canvas background of `document` take from the
/// root element's box.
///
/// A layout of a document that is not complete, such as the one a streamed
/// page comes from, ends the root's box where the content that arrived ends,
/// and one resumed after delivered pages starts it where it resumed.
/// [`CanvasBoxUse::Start`] and [`CanvasBoxUse::Extent`] tell which of the two
/// the images of the canvas cannot do without.
///
/// An image counts as taking the root's extent unless its height is a length,
/// it starts at the top of the box in a length of its own, and it tiles by
/// repeating. An image without a size of its own, such as a gradient or a
/// vector picture, fills the box; one of an intrinsic size is not known to
/// have one before it is decoded, so it counts as filling the box too. When
/// the pages run across the page instead of down it, as for a root that
/// stacks its blocks on vertical axes, every image does.
#[doc(hidden)]
pub fn canvas_box_use(document: &Document, cascade: &CascadeResult) -> CanvasBoxUse {
    let Some(computed) =
        canvas_background_owner(document, cascade).and_then(|owner| cascade.computed.get(owner))
    else {
        return CanvasBoxUse::None;
    };
    // The pages follow the x axis when the root stacks its blocks on
    // vertical axes. Read from the cascade, as the pages are not laid out yet
    // when the streaming layout asks.
    let pages_run_across = root_element(document)
        .is_some_and(|root| super::bridge::lays_out_vertically(cascade, root));
    computed
        .background_layers()
        .iter()
        .filter(|layer| {
            !matches!(layer.image, BackgroundImage::None)
                && layer.attachment != BackgroundAttachment::Fixed
        })
        .map(|layer| {
            if pages_run_across || takes_extent(layer) {
                CanvasBoxUse::Extent
            } else {
                CanvasBoxUse::Start
            }
        })
        .max()
        .unwrap_or(CanvasBoxUse::None)
}

/// Whether the image of `layer` depends on how far the root's box reaches
/// down the pages.
fn takes_extent(layer: &ComputedBackgroundLayer) -> bool {
    let height_is_a_length = matches!(
        layer.size,
        ComputedBackgroundSize::Explicit {
            height: ComputedLengthPercentageOrAuto::Px(_),
            ..
        }
    );
    let starts_at_the_top = matches!(
        layer.position.vertical,
        ComputedCssPositionOffset::Start(ComputedLengthPercentage::Px(_))
    ) || matches!(
        layer.position.vertical,
        ComputedCssPositionOffset::Start(ComputedLengthPercentage::Percent(percent))
            if percent == 0.0
    );
    let tiles_by_repeating = !matches!(
        layer.repeat.y,
        BackgroundRepeatKeyword::Space | BackgroundRepeatKeyword::Round
    );
    !(height_is_a_length && starts_at_the_top && tiles_by_repeating)
}

#[cfg(test)]
#[path = "canvas_background/tests.rs"]
mod tests;
