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
//! - the style facts the paginator reads from a box's style are read on
//!   those axes through `LayoutSession::block_axis`, from a style view whose
//!   layout writing mode is the root's, so the styles of the boxes
//!   themselves stay on physical axes;
//! - after pagination, the boxes are mapped back to physical axes. A box
//!   keeps its distance from the block-start edge of its parent, which for
//!   `vertical-rl` is the parent's right edge, so content that pagination
//!   moved to later pages, and the boxes that grew to hold it, extend to the
//!   left.
//!
//! The line positions of a paragraph are logical block offsets already, so
//! the paginator splits the lines of a vertical paragraph as it does those
//! of a horizontal one. A box that lays out its content on horizontal axes
//! inside such a document (a table box, a multicol container, or an
//! orthogonal flow) is not broken: pagination treats it as monolithic. A flex
//! or grid container runs its algorithm on the axes of its own writing mode,
//! so it is not monolithic on that account.

use super::*;
use crate::layout::block_axis_style::BlockAxisStyle;
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
        crate::layout::dispatch::block_right_to_left(document, root)
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

    fn rect<T: Copy>(self, rect: Edges<T>) -> Edges<T> {
        let (top, right, bottom, left) = self.sides(rect.top, rect.right, rect.bottom, rect.left);
        Edges {
            left,
            right,
            top,
            bottom,
        }
    }

    fn physical_rect<T: Copy>(self, rect: Edges<T>) -> Edges<T> {
        let (top, right, bottom, left) =
            self.physical_sides(rect.top, rect.right, rect.bottom, rect.left);
        Edges {
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
    fn layout(self, layout: BoxLayout, parent_width: f32) -> BoxLayout {
        let block_start = if self.right_to_left {
            parent_width - layout.location.x - layout.size.width
        } else {
            layout.location.x
        };
        BoxLayout {
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
    fn physical_layout(self, layout: BoxLayout, parent_width: f32) -> BoxLayout {
        let width = layout.size.height;
        let x = if self.right_to_left {
            parent_width - layout.location.y - width
        } else {
            layout.location.y
        };
        BoxLayout {
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

    /// The writing mode whose axes a style view returns its sizes and sides
    /// on, so that the root's block axis is the vertical one: the one the
    /// root has, as far as the axes go.
    pub(super) fn layout_wm(self) -> raikiri_style::property::WritingMode {
        if self.right_to_left {
            raikiri_style::property::WritingMode::VerticalRl
        } else {
            raikiri_style::property::WritingMode::VerticalLr
        }
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
    physical: Vec<BoxLayout>,
    /// The layout of each key as it was handed to the paginator.
    transposed: Vec<BoxLayout>,
    live_keys: Vec<(usize, crate::node::arena::BoxKey)>,
    generated_generation: u64,
    prepared_stamp: crate::layout::box_style_data::PreparedStamp,
}

impl TransposedDocument {
    pub(super) fn with_transposed<R>(
        document: &mut LayoutSession<'_>,
        flow_root: usize,
        axes: FlowAxes,
        page_area_width: f32,
        run: impl FnOnce(&mut LayoutSession<'_>, &Self) -> R,
    ) -> R {
        let transposed = Self::transpose(document, flow_root, axes, page_area_width);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(document, &transposed)));
        let unchanged = transposed.restore_inner(document);
        match result {
            Ok(value) => {
                assert!(
                    unchanged,
                    "pagination must preserve box identities and generations"
                );
                value
            }
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Put `document` on transposed axes. [`Self::with_transposed`] restores
    /// the retained geometry after the pagination body exits; the retained
    /// styles are not touched.
    pub(super) fn transpose(
        document: &mut LayoutSession<'_>,
        flow_root: usize,
        axes: FlowAxes,
        page_area_width: f32,
    ) -> Self {
        document.assert_style_queries_allowed();
        document.scoped_overrides.assert_inactive();
        assert!(
            document.context.transposed_blocks.is_empty()
                && document.context.resolved_edge_bases.is_empty(),
            "pagination starts outside native style scopes"
        );
        let live_keys = document
            .nodes
            .layout_keys()
            .map(|index| {
                (
                    index,
                    document.nodes.box_key(index).expect("live layout key"),
                )
            })
            .collect();
        let generated_generation = document.nodes.generated_generation();
        let prepared_stamp = document.box_style_data.stamp();
        let keys = document.nodes.len()
            + document.table_objects.tables.len()
            + document.table_objects.cells.len();
        // Only the virtual document node is outside the flow. The html
        // box and its children all have real layouts and local frames.
        let outside: Vec<usize> = document.ancestors(flow_root).collect();
        let frames: Vec<Option<usize>> = (0..keys)
            .map(|key| layout_frame(document, key).filter(|frame| !outside.contains(frame)))
            .collect();
        let physical: Vec<BoxLayout> = (0..keys)
            .map(|key| document.table_layout_node_mut(key).layout)
            .collect();
        let frame_width =
            |key: usize| frames[key].map_or(page_area_width, |frame| physical[frame].size.width);
        // The boxes around the body stay where they are, so that offsets
        // summed up to the page area are those of the body's coordinates.
        let transposed: Vec<BoxLayout> = (0..keys)
            .map(|key| {
                if outside.contains(&key) {
                    physical[key]
                } else {
                    axes.layout(physical[key], frame_width(key))
                }
            })
            .collect();
        // The styles are read from the physical computed values on the axes
        // of the root's writing mode. Reading them off the transposed computed
        // values below instead would swap them a second time.
        let block_styles: Vec<BlockAxisStyle> = (0..keys)
            .map(|key| {
                let view = document.style_view(key);
                let context = raikiri_layout_style::ViewContext {
                    layout_wm: axes.layout_wm(),
                    ..view.context()
                };
                BlockAxisStyle::of_view(&view.with_context(context))
            })
            .collect();
        document.document.layout_cache_context.invalidate();
        for (key, &layout) in transposed.iter().enumerate() {
            document.table_layout_node_mut(key).layout = layout;
        }
        document.document.pagination_block_styles = block_styles;
        document.context.table_cache.clear();
        document.context.pagination_on_transposed_axes = true;
        Self {
            axes,
            page_area_width,
            outside,
            frames,
            physical,
            transposed,
            live_keys,
            generated_generation,
            prepared_stamp,
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
    fn restore_inner(self, document: &mut LayoutSession<'_>) -> bool {
        let unchanged = self.generated_generation == document.nodes.generated_generation()
            && self.prepared_stamp == document.box_style_data.stamp()
            && self
                .live_keys
                .iter()
                .copied()
                .eq(document.nodes.layout_keys().map(|index| {
                    (
                        index,
                        document.nodes.box_key(index).expect("live layout key"),
                    )
                }));
        document.document.layout_cache_context.invalidate();
        document.document.pagination_block_styles = Vec::new();
        document.context.pagination_on_transposed_axes = false;
        document.context.table_cache.clear();
        if !unchanged {
            // Identity violations cannot use the old numeric coordinate frames.
            // Restore surviving typed keys before reporting the boundary failure.
            for (old_index, physical) in self.physical.iter().enumerate() {
                let key = self
                    .live_keys
                    .iter()
                    .find(|(index, _)| *index == old_index)
                    .map(|(_, key)| *key);
                if let Some(index) = key.and_then(|key| document.nodes.layout_index(key)) {
                    document.nodes[index].layout = *physical;
                }
            }
            return false;
        }
        let keys = self.physical.len();
        let current: Vec<BoxLayout> = (0..keys)
            .map(|key| document.table_layout_node_mut(key).layout)
            .collect();
        for key in 0..keys {
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
            document.table_layout_node_mut(key).layout = layout;
        }
        document.context.table_cache.clear();
        document.context.pagination_on_transposed_axes = false;
        true
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
