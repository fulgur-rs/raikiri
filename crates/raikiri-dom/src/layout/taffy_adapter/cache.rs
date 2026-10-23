use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::protocol::{LayoutInput, LayoutOutput};

/// The layout results of one node, kept for inputs that come again.
///
/// It holds one entry for the final layout of the node and several for sizes
/// measured under different constraints, and it drops its entries when the
/// sizing mode or the margin collapsing of an input differs from that of the
/// entries it holds. The cache is the library's, wrapped: lookups and stores
/// take the protocol's types and convert them on the way in and out.
#[derive(Debug, Clone, Default)]
pub(crate) struct LayoutCache(taffy::Cache);

impl LayoutCache {
    /// An empty cache.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self(taffy::Cache::new())
    }

    /// The stored result for `input`, if there is one.
    #[inline]
    pub(crate) fn get(&mut self, input: &LayoutInput) -> Option<LayoutOutput> {
        self.0
            .get(&(*input).into_taffy())
            .map(LayoutOutput::from_taffy)
    }

    /// Store the `output` computed for `input`.
    #[inline]
    pub(crate) fn store(&mut self, input: &LayoutInput, output: LayoutOutput) {
        self.0.store(&(*input).into_taffy(), output.into_taffy());
    }

    /// Drop all entries.
    #[inline]
    pub(crate) fn clear(&mut self) {
        self.0.clear();
    }
}
