//! Pagination of a document whose root element lays out its blocks on
//! vertical axes.
//!
//! The pages of such a document follow the root's block axis, the physical
//! x axis (CSS Writing Modes 4 §8; CSS Fragmentation 3 §2): `vertical-rl`
//! content continues to the left onto later pages and `vertical-lr` content
//! to the right. The paginator breaks a flow along y. Rather than teach it
//! a second axis, the document is handed to it on transposed axes, where
//! the root's block axis runs down the page, and the result is mapped back:
//!
//! - every box's layout is reflected so that its block-start edge is its
//!   top edge, with the inline axis across;
//! - the computed values the paginator reads and the page geometry are
//!   reflected the same way;
//! - after pagination, the boxes are mapped back to physical axes. A box
//!   keeps its distance from the block-start edge of its parent, which for
//!   `vertical-rl` is the parent's right edge, so content that pagination
//!   moved to later pages, and the boxes that grew to hold it, extend to the
//!   left.
//!
//! The line positions of a paragraph are logical block offsets already, so
//! the paginator splits the lines of a vertical paragraph as it does those
//! of a horizontal one. A box that lays out its content on horizontal axes
//! inside such a document (a flex, grid or table box, a multicol container,
//! or an orthogonal flow) is not broken: pagination treats it as monolithic.

use super::*;
use crate::layout::context::LayoutSession;

/// How the physical sides of a box map onto the transposed view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FlowAxes {
    /// The block flow runs right to left (`vertical-rl`), so the block-start
    /// edge is the right edge; otherwise it is the left edge (`vertical-lr`).
    right_to_left: bool,
}

impl FlowAxes {
    /// The axes pagination breaks the document along, when its root element
    /// lays out its blocks on vertical axes.
    pub(super) fn of_root(document: &Document, cascade: &CascadeResult) -> Option<Self> {
        let root = super::super::initial_containing_block::root_element(document)?;
        if !crate::layout::bridge::lays_out_vertically(cascade, root) {
            return None;
        }
        crate::taffy_impl::block_right_to_left(document, root)
            .map(|right_to_left| Self { right_to_left })
    }

    /// Whether later pages continue to the left.
    pub(super) fn right_to_left(self) -> bool {
        self.right_to_left
    }

    /// The transposed `(top, right, bottom, left)` of physical sides.
    fn sides<T>(self, top: T, right: T, bottom: T, left: T) -> (T, T, T, T) {
        if self.right_to_left {
            (right, bottom, left, top)
        } else {
            (left, bottom, right, top)
        }
    }

    /// The physical `(top, right, bottom, left)` of transposed sides.
    fn physical_sides<T>(self, top: T, right: T, bottom: T, left: T) -> (T, T, T, T) {
        if self.right_to_left {
            (left, top, right, bottom)
        } else {
            (left, bottom, right, top)
        }
    }

    fn rect<T: Copy>(self, rect: Rect<T>) -> Rect<T> {
        let (top, right, bottom, left) = self.sides(rect.top, rect.right, rect.bottom, rect.left);
        Rect {
            left,
            right,
            top,
            bottom,
        }
    }

    fn physical_rect<T: Copy>(self, rect: Rect<T>) -> Rect<T> {
        let (top, right, bottom, left) =
            self.physical_sides(rect.top, rect.right, rect.bottom, rect.left);
        Rect {
            left,
            right,
            top,
            bottom,
        }
    }

    fn style_sides<T: Copy>(self, sides: &mut raikiri_style::property::Sides<T>) {
        let (top, right, bottom, left) =
            self.sides(sides.top, sides.right, sides.bottom, sides.left);
        *sides = raikiri_style::property::Sides {
            top,
            right,
            bottom,
            left,
        };
    }

    /// The layout of a box on transposed axes. `parent_width` is the
    /// physical width of the box its location is relative to.
    fn layout(self, layout: TaffyLayout, parent_width: f32) -> TaffyLayout {
        let block_start = if self.right_to_left {
            parent_width - layout.location.x - layout.size.width
        } else {
            layout.location.x
        };
        TaffyLayout {
            order: layout.order,
            location: Point {
                x: layout.location.y,
                y: block_start,
            },
            size: Size {
                width: layout.size.height,
                height: layout.size.width,
            },
            scrollable_overflow_rect: self.rect(layout.scrollable_overflow_rect),
            scrollbar_size: Size {
                width: layout.scrollbar_size.height,
                height: layout.scrollbar_size.width,
            },
            border: self.rect(layout.border),
            padding: self.rect(layout.padding),
            margin: self.rect(layout.margin),
        }
    }

    /// The physical layout of a box laid out on transposed axes.
    /// `parent_width` is the physical width of the box its location is
    /// relative to: the transposed height of that box.
    fn physical_layout(self, layout: TaffyLayout, parent_width: f32) -> TaffyLayout {
        let width = layout.size.height;
        let x = if self.right_to_left {
            parent_width - layout.location.y - width
        } else {
            layout.location.y
        };
        TaffyLayout {
            order: layout.order,
            location: Point {
                x,
                y: layout.location.x,
            },
            size: Size {
                width,
                height: layout.size.width,
            },
            scrollable_overflow_rect: self.physical_rect(layout.scrollable_overflow_rect),
            scrollbar_size: Size {
                width: layout.scrollbar_size.height,
                height: layout.scrollbar_size.width,
            },
            border: self.physical_rect(layout.border),
            padding: self.physical_rect(layout.padding),
            margin: self.physical_rect(layout.margin),
        }
    }

    /// A taffy style with its physical sizes and sides on transposed axes,
    /// for the properties the paginator reads.
    fn style(self, style: &crate::LayoutStyle) -> crate::LayoutStyle {
        let mut style = style.clone();
        style.size = Size {
            width: style.size.height,
            height: style.size.width,
        };
        style.min_size = Size {
            width: style.min_size.height,
            height: style.min_size.width,
        };
        style.max_size = Size {
            width: style.max_size.height,
            height: style.max_size.width,
        };
        style.overflow = Point {
            x: style.overflow.y,
            y: style.overflow.x,
        };
        style.aspect_ratio = style.aspect_ratio.map(f32::recip);
        style.margin = self.rect(style.margin);
        style.padding = self.rect(style.padding);
        style.border = self.rect(style.border);
        style.inset = self.rect(style.inset);
        style
    }

    /// The computed values of `idx` with their physical sizes and sides on
    /// transposed axes.
    fn computed(self, cascade: &CascadeResult, idx: usize) -> ComputedValues {
        let mut values = cascade.computed[idx].clone();
        // A box on vertical axes has its physical sizes in its vertical
        // mapping; every other box has them in `width` and `height`.
        if let Some(size) = crate::layout::bridge::vertical_logical_size(cascade, idx) {
            values.width = size.width;
            values.height = size.height;
        }
        values.vertical_logical_size = None;
        std::mem::swap(&mut values.width, &mut values.height);
        std::mem::swap(&mut values.min_width, &mut values.min_height);
        std::mem::swap(&mut values.max_width, &mut values.max_height);
        self.style_sides(&mut values.margin);
        self.style_sides(&mut values.padding);
        self.style_sides(&mut values.border);
        let (top, right, bottom, left) =
            self.sides(values.top, values.right, values.bottom, values.left);
        values.top = top;
        values.right = right;
        values.bottom = bottom;
        values.left = left;
        values
    }

    /// The page box on transposed axes.
    pub(super) fn page_box(self, page_box: PageBox) -> PageBox {
        let mut transposed = page_box;
        transposed.width = page_box.height;
        transposed.height = page_box.width;
        transposed
    }

    /// Page margins on transposed axes.
    pub(super) fn margins(self, margins: PageMargins) -> PageMargins {
        let (top, right, bottom, left) =
            self.sides(margins.top, margins.right, margins.bottom, margins.left);
        PageMargins {
            top,
            right,
            bottom,
            left,
        }
    }

    /// Page border and padding insets on transposed axes.
    pub(super) fn insets(self, insets: PageContentInsets) -> PageContentInsets {
        let (top, right, bottom, left) =
            self.sides(insets.top, insets.right, insets.bottom, insets.left);
        PageContentInsets {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// A document handed to the paginator on transposed axes, with what maps
/// it back.
pub(super) struct TransposedDocument {
    axes: FlowAxes,
    /// The physical width of the page area, which the root element's box is
    /// placed in.
    page_area_width: f32,
    /// Source nodes above the flow root, which keep their physical layout.
    outside: Vec<usize>,
    /// Each layout key's frame: the key its location is relative to, or
    /// `None` for the page area.
    frames: Vec<Option<usize>>,
    /// The physical layout of each layout key before pagination.
    physical: Vec<TaffyLayout>,
    /// The layout of each key as it was handed to the paginator.
    transposed: Vec<TaffyLayout>,
    styles: Vec<crate::LayoutStyle>,
}

impl TransposedDocument {
    /// Put `document` on transposed axes. The returned value maps it back
    /// with [`Self::restore`].
    pub(super) fn transpose(
        document: &mut LayoutSession<'_>,
        flow_root: usize,
        axes: FlowAxes,
        page_area_width: f32,
    ) -> Self {
        let keys = document.nodes.len()
            + document.table_objects.tables.len()
            + document.table_objects.cells.len();
        // Only the virtual document node is outside the flow. The html
        // box and its children all have real layouts and local frames.
        let outside: Vec<usize> = document.ancestors(flow_root).collect();
        let frames: Vec<Option<usize>> = (0..keys)
            .map(|key| layout_frame(document, key).filter(|frame| !outside.contains(frame)))
            .collect();
        let physical: Vec<TaffyLayout> = (0..keys)
            .map(|key| document.table_layout_node_mut(key).unrounded_layout)
            .collect();
        let frame_width =
            |key: usize| frames[key].map_or(page_area_width, |frame| physical[frame].size.width);
        // The boxes around the body stay where they are, so that offsets
        // summed up to the page area are those of the body's coordinates.
        let transposed: Vec<TaffyLayout> = (0..keys)
            .map(|key| {
                if outside.contains(&key) {
                    physical[key]
                } else {
                    axes.layout(physical[key], frame_width(key))
                }
            })
            .collect();
        document.document.layout_cache_context.invalidate();
        let mut styles = Vec::with_capacity(keys);
        for (key, &layout) in transposed.iter().enumerate() {
            document.table_layout_node_mut(key).unrounded_layout = layout;
            let style = axes.style(
                document
                    .layout_style_for_key(key)
                    .expect("transposed box has a retained style"),
            );
            styles.push(std::mem::replace(
                document
                    .layout_style_for_key_mut(key)
                    .expect("transposed box has a retained style"),
                style,
            ));
        }
        document.context.table_cache.clear();
        document.context.pagination_on_transposed_axes = true;
        Self {
            axes,
            page_area_width,
            outside,
            frames,
            physical,
            transposed,
            styles,
        }
    }

    /// The computed values of `cascade` on transposed axes.
    pub(super) fn cascade(&self, cascade: &CascadeResult) -> CascadeResult {
        let mut transposed = cascade.clone();
        for idx in 0..transposed.computed.len() {
            transposed.computed[idx] = self.axes.computed(cascade, idx);
        }
        transposed
    }

    /// Map the paginated `document` back to physical axes. A box that
    /// pagination left alone, in a parent that kept its size, gets its
    /// original layout back exactly.
    pub(super) fn restore(self, document: &mut LayoutSession<'_>) {
        document.document.layout_cache_context.invalidate();
        let keys = self.physical.len();
        let current: Vec<TaffyLayout> = (0..keys)
            .map(|key| document.table_layout_node_mut(key).unrounded_layout)
            .collect();
        for (key, style) in self.styles.into_iter().enumerate() {
            let frame = self.frames[key];
            let frame_kept = frame.is_none_or(|frame| {
                current[frame].size.height == self.transposed[frame].size.height
            });
            let layout = if self.outside.contains(&key) {
                current[key]
            } else if current[key] == self.transposed[key] && frame_kept {
                self.physical[key]
            } else {
                let parent_width =
                    frame.map_or(self.page_area_width, |frame| current[frame].size.height);
                self.axes.physical_layout(current[key], parent_width)
            };
            document.table_layout_node_mut(key).unrounded_layout = layout;
            *document
                .layout_style_for_key_mut(key)
                .expect("transposed box has a retained style") = style;
        }
        document.context.table_cache.clear();
        document.context.pagination_on_transposed_axes = false;
    }
}

/// The layout key whose border box the location of `key` is relative to.
fn layout_frame(document: &Document, key: usize) -> Option<usize> {
    if key >= document.nodes.len() {
        // An anonymous table cell is placed relative to its source owner,
        // and an anonymous table relative to its parent.
        return Some(document.ifc_source_owner(key));
    }
    if let Some(coordinate) = document.oof_coordinates.get(&key) {
        // An out-of-flow box is placed relative to its containing-block
        // owner. The initial containing block is no layout key of the
        // transposed document, and its frame is the page area.
        return (document.nodes.initial_containing_block_key() != Some(coordinate.owner))
            .then_some(coordinate.owner);
    }
    document
        .parent_of(key)
        .and_then(|_| document.layout_parent_of(key))
}

#[cfg(test)]
mod tests;
