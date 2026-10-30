//! Append-only animation effects and their per-target creation order.

use std::collections::HashMap;

use super::AnimationEffect;

/// Structural updates stay here so every stable effect ID is also indexed
/// by target. No garbage-collected script values are retained by this index.
#[derive(Debug, Default)]
pub(crate) struct AnimationEffects {
    effects: Vec<AnimationEffect>,
    by_target: HashMap<usize, Vec<usize>>,
}

impl AnimationEffects {
    pub(super) fn push(&mut self, effect: AnimationEffect) -> usize {
        let id = self.effects.len();
        self.by_target.entry(effect.target).or_default().push(id);
        self.effects.push(effect);
        id
    }

    pub(super) fn get(&self, id: usize) -> Option<&AnimationEffect> {
        self.effects.get(id)
    }

    pub(super) fn get_mut(&mut self, id: usize) -> Option<&mut AnimationEffect> {
        self.effects.get_mut(id)
    }

    pub(super) fn for_target(&self, target: usize) -> impl Iterator<Item = &AnimationEffect> {
        self.by_target
            .get(&target)
            .into_iter()
            .flatten()
            .map(|&id| &self.effects[id])
    }
}
