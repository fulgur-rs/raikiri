//! Layout results kept apart for each way the style of a box is overridden.

use super::SavedLayoutCache;
use crate::{layout::context::LayoutSession, node::arena::BoxKey};
use raikiri_layout_style::OwnedPropertyOverride;
use rustc_hash::FxHashMap;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// The variant of a box laid out with the style it has: no override.
const BASE_VARIANT: u64 = 0;

/// The caches of boxes that are laid out with more than one style.
#[derive(Debug, Default)]
pub(crate) struct VariantCaches {
    /// What the cache of a box held for each override variant while another
    /// style was in place, by box.
    saved: FxHashMap<usize, Vec<(u64, SavedLayoutCache)>>,
    /// The variant of the last layout of each box.
    last: FxHashMap<usize, u64>,
}

impl VariantCaches {
    /// Drop everything kept for the boxes.
    pub(crate) fn clear(&mut self) {
        self.saved.clear();
        self.last.clear();
    }

    /// Drop what is kept for the box `idx`.
    pub(crate) fn forget(&mut self, idx: usize) {
        self.saved.remove(&idx);
        self.last.remove(&idx);
    }

    fn take(&mut self, idx: usize, variant: u64) -> Option<SavedLayoutCache> {
        let variants = self.saved.get_mut(&idx)?;
        let at = variants.iter().position(|(held, _)| *held == variant)?;
        Some(variants.swap_remove(at).1)
    }

    fn put(&mut self, idx: usize, variant: u64, cache: SavedLayoutCache) {
        self.saved.entry(idx).or_default().push((variant, cache));
    }
}

impl LayoutSession<'_> {
    /// Run `f`, a layout of the box `idx`, with `entries` overriding its style.
    /// `variant` names the style they make of it, and is not zero.
    ///
    /// A box that is laid out once for each width its parent tries, with its
    /// percentage widths replaced by what they stand for, has everything inside
    /// it laid out again by each of those layouts when its scope moves the cache
    /// generation (see [`LayoutSession::push_local_overrides`]) or drops what the
    /// box computed as it ends. A chain of such boxes then takes time
    /// exponential in its length. The scope here leaves the generation alone,
    /// and the cache of the box is swapped for the one the same variant had
    /// last time, so that what the layout computes is there for the next one.
    /// The boxes inside are laid out with the inputs the box gives them, which
    /// is all their caches depend on, so those stay as they are.
    ///
    /// A layout of the box leaves its lines and the layouts of the boxes inside
    /// as it sets them, and the final layout in a cache stands for what the
    /// last layout through that cache left. When the last layout of the box ran
    /// under another variant, or with the style the box has, the final layout of
    /// this variant no longer stands for them, and is not answered from. The
    /// sizes it measured are not affected.
    pub(crate) fn with_style_variant<R>(
        &mut self,
        idx: usize,
        variant: u64,
        entries: Vec<(BoxKey, OwnedPropertyOverride)>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        debug_assert_ne!(variant, BASE_VARIANT);
        // Pending mutations of the document drop what is kept here, so they
        // are published first.
        self.sync_layout_cache_mutations();
        let switched = self.context.variant_caches.last.insert(idx, variant) != Some(variant);
        let mut base = self.take_layout_cache(idx);
        if switched {
            base.forget_final_layout();
        }
        if let Some(mut held) = self.context.variant_caches.take(idx, variant) {
            if switched {
                held.forget_final_layout();
            }
            self.restore_layout_cache(idx, held);
        }
        let result = catch_unwind(AssertUnwindSafe(|| self.with_local_overrides(entries, f)));
        let held = self.take_layout_cache(idx);
        self.context.variant_caches.put(idx, variant, held);
        self.restore_layout_cache(idx, base);
        match result {
            Ok(value) => value,
            Err(payload) => resume_unwind(payload),
        }
    }

    /// Note that the box `idx`, which is laid out with an override at other
    /// times, is laid out with the style it has: the next layout of it under
    /// an override finds the final layout of its cache no longer valid.
    pub(crate) fn note_base_variant(&mut self, idx: usize) {
        self.context.variant_caches.last.insert(idx, BASE_VARIANT);
    }
}
