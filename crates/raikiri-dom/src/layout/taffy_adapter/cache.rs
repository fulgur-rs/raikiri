use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::protocol::{LayoutInput, LayoutOutput};

/// How many intrinsic widths a node keeps besides its measured sizes.
pub(super) const MAX_WIDTHS: usize = 128;

/// The layout results of one node, kept for inputs that come again.
///
/// It holds one entry for the final layout of the node and several for sizes
/// measured under different constraints, and it drops its entries when the
/// sizing mode or the margin collapsing of an input differs from that of the
/// entries it holds. The cache is the library's, wrapped: lookups and stores
/// take the protocol's types and convert them on the way in and out.
///
/// Next to those it holds the widths that a shrink-to-fit box was measured to
/// have under its intrinsic constraints (see [`LayoutCache::width`]).
#[derive(Debug, Clone, Default)]
pub(crate) struct LayoutCache {
    cache: taffy::Cache,
    widths: Vec<(LayoutInput, f32)>,
}

impl LayoutCache {
    /// An empty cache.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self {
            cache: taffy::Cache::new(),
            widths: Vec::new(),
        }
    }

    /// The stored result for `input`, if there is one.
    #[inline]
    pub(crate) fn get(&mut self, input: &LayoutInput) -> Option<LayoutOutput> {
        self.cache
            .get(&(*input).into_taffy())
            .map(LayoutOutput::from_taffy)
    }

    /// Store the `output` computed for `input`.
    #[inline]
    pub(crate) fn store(&mut self, input: &LayoutInput, output: LayoutOutput) {
        self.cache
            .store(&(*input).into_taffy(), output.into_taffy());
    }

    /// The width stored for `input` by [`LayoutCache::store_width`].
    ///
    /// A box that shrinks to fit measures its min-content and max-content
    /// widths each time it is laid out at a width it was not laid out at
    /// before, and each measure runs its whole content. The measures do not
    /// depend on that width, so they are kept apart from the sizes above: those
    /// are a handful of slots that every width tried on the box takes turns in.
    /// An entry matches only an input equal to the one it was stored for in
    /// every field, the parent's size and the margin collapsing included.
    pub(crate) fn width(&self, input: &LayoutInput) -> Option<f32> {
        self.widths
            .iter()
            .find_map(|(stored, width)| (stored == input).then_some(*width))
    }

    /// Keep `width` as the width measured for `input`; the oldest width goes
    /// when there are too many.
    pub(crate) fn store_width(&mut self, input: &LayoutInput, width: f32) {
        if let Some((_, stored)) = self.widths.iter_mut().find(|(stored, _)| stored == input) {
            *stored = width;
            return;
        }
        if self.widths.len() == MAX_WIDTHS {
            self.widths.remove(0);
        }
        self.widths.push((*input, width));
    }

    /// Drop all entries.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.cache.clear();
        self.widths.clear();
    }
}
