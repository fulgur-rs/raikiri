//! The opacity groups of the `::first-letter` fragments of a paragraph.

use crate::{Document, GeneratedKind, generated_content, paint_rules};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::NodeKind;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// The opacity groups the `::first-letter` fragments of one paragraph
/// composite in.
///
/// Each fragment keeps the style it was given for the text it wraps, so the
/// fragments of one `::first-letter` can have different opacities, or lie
/// inside different groups, where its letters come from text with different
/// inherited values. Fragments of one box share a group when their opacity
/// and the group around them agree, as in the built-in painter. Each group
/// is named by an id of [`generated_content::first_letter_group_id`].
#[derive(Debug, Default)]
pub(super) struct LetterGroups {
    /// Each fragment's own group, when it is translucent, and the innermost
    /// group around it, by box and the node its letters come from.
    pub(super) fragments: HashMap<(usize, usize), (Option<usize>, Option<usize>)>,
    /// Each group's opacity and the innermost group around it.
    groups: HashMap<usize, (f32, Option<usize>)>,
    /// Whether a fragment needed a group past the ones an id can name, so
    /// it composites with the opacity of another group, or none of its own.
    pub(super) approximated: bool,
}

impl LetterGroups {
    /// The own group and the innermost group around it of the fragment of
    /// box `letter` wrapping letters of `owner`; `None` for a fragment the
    /// paragraph does not retain.
    pub(super) fn fragment(
        &self,
        letter: usize,
        owner: usize,
    ) -> Option<(Option<usize>, Option<usize>)> {
        self.fragments.get(&(letter, owner)).copied()
    }

    /// The innermost group that paints the letters of `owner` in box
    /// `letter`; `None` for a fragment the paragraph does not retain.
    pub(super) fn nearest(&self, letter: usize, owner: usize) -> Option<Option<usize>> {
        self.fragment(letter, owner).map(|(own, held)| own.or(held))
    }

    /// The opacity of group `group`.
    pub(super) fn opacity(&self, group: usize) -> Option<f32> {
        self.groups.get(&group).map(|&(alpha, _)| alpha)
    }

    /// The innermost group around group `group`.
    pub(super) fn around(&self, group: usize) -> Option<usize> {
        self.groups.get(&group).and_then(|&(_, around)| around)
    }
}

type ParagraphLetterGroups = HashMap<(usize, bool), Arc<LetterGroups>>;

/// The [`LetterGroups`] of the paragraphs built so far, by root, with the
/// element generation of the cascade they were built for.
///
/// Every run of a paragraph is classified against the same table, so it is
/// built once per paragraph rather than once per run.
#[derive(Debug, Default, Clone)]
pub(crate) struct LetterGroupCache(pub(super) RefCell<(u64, ParagraphLetterGroups)>);

impl LetterGroupCache {
    /// Forgets every table, as for a new projection.
    pub(crate) fn clear(&mut self) {
        *self.0.get_mut() = Default::default();
    }
}

impl Document {
    /// The [`LetterGroups`] of paragraph `root`, built on first use for
    /// `cascade` and shared by later calls.
    pub(super) fn cached_letter_groups(
        &self,
        cascade: &CascadeResult,
        root: usize,
    ) -> Arc<LetterGroups> {
        self.cached_letter_groups_mode(cascade, root, true)
    }

    pub(super) fn cached_letter_groups_mode(
        &self,
        cascade: &CascadeResult,
        root: usize,
        separate_first_line: bool,
    ) -> Arc<LetterGroups> {
        let separate_first_line =
            separate_first_line && paint_rules::first_line_opacity_layer(self, root).is_some();
        let generation = cascade.element_generation();
        let mut cache = self.page_projection.letter_groups.0.borrow_mut();
        if cache.0 != generation {
            *cache = (generation, HashMap::new());
        }
        Arc::clone(
            cache
                .1
                .entry((root, separate_first_line))
                .or_insert_with(|| {
                    Arc::new(self.letter_groups_mode(cascade, root, separate_first_line))
                }),
        )
    }

    /// The opacity groups of the `::first-letter` fragments of paragraph
    /// `root`, numbered in the order the paragraph retains the fragments.
    #[cfg(test)]
    pub(super) fn letter_groups(&self, cascade: &CascadeResult, root: usize) -> LetterGroups {
        self.letter_groups_mode(cascade, root, true)
    }

    fn letter_groups_mode(
        &self,
        cascade: &CascadeResult,
        root: usize,
        separate_first_line: bool,
    ) -> LetterGroups {
        let mut table = LetterGroups::default();
        let Some(node) = self.ifc_layout_node(root) else {
            return table; // cov:ignore: paragraph roots come from the arena
        };
        // The group of each box so far, by box, opacity and the group
        // around, and the number of groups each box has.
        let mut numbered: HashMap<(usize, u32, Option<usize>), Option<usize>> = HashMap::new();
        let mut counts: HashMap<usize, usize> = HashMap::new();
        // The last group each box numbered inside each group around it.
        let mut last: HashMap<(usize, Option<usize>), usize> = HashMap::new();
        // The innermost group of each fragment resolved so far, its own
        // included.
        let mut nearest: HashMap<(usize, usize), Option<usize>> = HashMap::new();
        let fragments: Vec<(usize, usize)> = node
            .ifc_letter_fragments()
            .map(|(letter, owner, _)| (letter, owner))
            .collect();
        for (letter, owner) in fragments {
            // Walk out through the boxes of the ancestor blocks' first
            // letters around the same letters to one already resolved or to
            // the outermost, then resolve back in, outer groups first.
            let mut path = Vec::new();
            let mut key = (letter, owner);
            let mut group = loop {
                if let Some(&group) = nearest.get(&key) {
                    break group;
                }
                nearest.insert(key, None);
                path.push(key);
                match node.ifc_typographic_parent(key.0) {
                    Some(parent) => key = (parent, owner),
                    None => {
                        break self.letter_source_group(cascade, root, owner, separate_first_line);
                    }
                }
            };
            for key in path.into_iter().rev() {
                let retained = node.ifc_typographic_style_for_owner(key.0, key.1);
                let own =
                    paint_rules::letter_opacity_layer(cascade, key.0, retained).and_then(|alpha| {
                        *numbered
                            .entry((key.0, alpha.to_bits(), group))
                            .or_insert_with(|| {
                                let count = counts.entry(key.0).or_default();
                                let index = *count;
                                *count += 1;
                                let id = generated_content::generated_origin(key.0).map_or(
                                    Some(key.0),
                                    |(element, _)| {
                                        generated_content::first_letter_group_id(element, index)
                                    },
                                );
                                match id {
                                    Some(id) => {
                                        table.groups.entry(id).or_insert((alpha, group));
                                        last.insert((key.0, group), id);
                                        Some(id)
                                    }
                                    // Beyond the groups an id can name, fragments
                                    // share a group inside the same group around
                                    // them, so no group is opened in two places.
                                    // Their opacity is then approximated, which
                                    // the paragraph reports.
                                    None => {
                                        table.approximated = true;
                                        last.get(&(key.0, group)).copied()
                                    }
                                }
                            })
                    });
                table.fragments.entry(key).or_insert((own, group));
                group = own.or(group);
                nearest.insert(key, group);
            }
        }
        table
    }

    /// The innermost opacity group of paragraph `root` around the letters
    /// that `owner` supplies to the outermost `::first-letter` box: the
    /// group of the `::before` or `::after` whose text they are, or the
    /// inline group around their text.
    pub(super) fn letter_source_group(
        &self,
        cascade: &CascadeResult,
        root: usize,
        owner: usize,
        separate_first_line: bool,
    ) -> Option<usize> {
        if let Some((element, pseudo)) = generated_content::generated_origin(owner) {
            let kind = match pseudo {
                PseudoElem::After => GeneratedKind::After,
                _ => GeneratedKind::Before,
            };
            return super::generated_group(self, cascade, root, element, kind, separate_first_line)
                .or_else(|| {
                    self.inline_opacity_group_mode(cascade, root, element, separate_first_line)
                });
        }
        let anchor = match self.get_node(owner)?.kind() {
            NodeKind::Text => self.parent_of(owner)?,
            _ => owner, // cov:ignore: letters come from text or generated content
        };
        self.inline_opacity_group_mode(cascade, root, anchor, separate_first_line)
    }
}
