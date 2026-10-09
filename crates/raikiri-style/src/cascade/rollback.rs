//! Candidate selection after cascade rollback.
use std::collections::HashSet;

use crate::layer::LayerPosition;
use crate::property::{CssWideKeyword, PropertyValue};
use crate::ruletree::Origin;

use super::collect::cascade_rank;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rollback {
    None,
    Layer,
    Origin,
}

pub(crate) fn rollback_kind(value: &PropertyValue) -> Rollback {
    let keyword = match value {
        PropertyValue::AllRevertLayer => return Rollback::Layer,
        PropertyValue::Deferred(marker) => match marker.css_wide_keyword() {
            Some(keyword) => keyword,
            None => return Rollback::None,
        },
        PropertyValue::CustomProperty(custom) => {
            return super::custom_property::custom_property_rollback(&custom.value);
        }
        PropertyValue::BorderTopWidthCssWide(keyword)
        | PropertyValue::BorderRightWidthCssWide(keyword)
        | PropertyValue::BorderBottomWidthCssWide(keyword)
        | PropertyValue::BorderLeftWidthCssWide(keyword)
        | PropertyValue::BorderTopStyleCssWide(keyword)
        | PropertyValue::BorderRightStyleCssWide(keyword)
        | PropertyValue::BorderBottomStyleCssWide(keyword)
        | PropertyValue::BorderLeftStyleCssWide(keyword)
        | PropertyValue::BorderTopColorCssWide(keyword)
        | PropertyValue::BorderRightColorCssWide(keyword)
        | PropertyValue::BorderBottomColorCssWide(keyword)
        | PropertyValue::BorderLeftColorCssWide(keyword) => *keyword,
        _ => return Rollback::None,
    };
    match keyword {
        CssWideKeyword::Revert => Rollback::Origin,
        CssWideKeyword::RevertLayer => Rollback::Layer,
        _ => Rollback::None,
    }
}

/// Inspect relevant candidates in descending cascade order. Normal rollback
/// removes its layer; important rollback also removes declarations between
/// that layer's normal and important levels. Attached important styles retain
/// stylesheet important declarations but remove the intervening animation.
/// Sorting once bounds repeated rollback chains to O(n log n).
pub(crate) fn select_layered_winner<T, P: Ord>(
    candidates: &[T],
    mut inspect: impl FnMut(usize, &T) -> Option<(P, Origin, LayerPosition, bool, Rollback)>,
) -> Option<usize> {
    let mut ordered: Vec<_> = candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            let (priority, origin, layer, important, rollback) = inspect(index, candidate)?;
            Some((priority, index, origin, layer, important, rollback))
        })
        .collect();
    ordered.sort_unstable_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut excluded_layers = HashSet::new();
    let mut excluded_origins = HashSet::new();
    let mut normal_cutoff = None;
    for (_, index, origin, layer, important, rollback) in ordered.into_iter().rev() {
        let normal_origin = cascade_rank(origin, false);
        let precedence = (cascade_rank(origin, important), layer.priority(important));
        if excluded_layers.contains(&(normal_origin, layer))
            || excluded_origins.contains(&normal_origin)
            || normal_cutoff.is_some_and(|cutoff| precedence >= cutoff)
        {
            continue;
        }
        match rollback {
            Rollback::None => return Some(index),
            Rollback::Layer => {
                excluded_layers.insert((normal_origin, layer));
                if important && layer.attached {
                    excluded_origins.insert(cascade_rank(Origin::Animation, false));
                } else if important {
                    // Remaining candidates are already below this marker's
                    // important level, so only the interval's lower bound is needed.
                    let cutoff = (normal_origin, layer.priority(false));
                    normal_cutoff =
                        Some(normal_cutoff.map_or(cutoff, |previous| previous.min(cutoff)));
                }
            }
            Rollback::Origin => {
                // Author rollback includes presentational hints and animation;
                // user rollback additionally removes author and user origins.
                let first = match origin {
                    Origin::UserAgent => 0,
                    Origin::User => 1,
                    Origin::AuthorPresentationalHint | Origin::Author | Origin::Animation => 2,
                };
                excluded_origins.extend(first..=cascade_rank(Origin::Animation, false));
            }
        }
    }
    None
}
