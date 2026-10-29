//! Conservative positions of object fragments in their processed text.

use shodo::{Fragment, Line};
use std::collections::HashMap;

/// Prefix states for the text datasets retained by one line generation.
/// Dataset identity includes the address and length: first-line transforms
/// may supply another complete text dataset. The owning lines keep these
/// addresses alive. Only text facts are retained, so ellipsis can change the
/// fragments without invalidating this cache.
#[derive(Debug)]
pub(crate) struct ObjectTextPrefixes {
    datasets: HashMap<(usize, usize), Vec<bool>>,
}

impl ObjectTextPrefixes {
    pub(super) fn new(lines: &[Line]) -> Self {
        let mut datasets = HashMap::new();
        for line in lines {
            let text = line.text();
            datasets
                .entry((text.as_ptr() as usize, text.len()))
                .or_insert_with(|| prefix_states(text));
        }
        Self { datasets }
    }

    pub(super) fn closed_before(&self, line: &Line) -> Option<bool> {
        let text = line.text();
        self.datasets
            .get(&(text.as_ptr() as usize, text.len()))?
            .get(line.text_range().start)
            .copied()
    }
}

fn prefix_states(text: &str) -> Vec<bool> {
    let mut closed = vec![false; text.len() + 1];
    let mut embeddings = 0_usize;
    let mut isolates = Vec::new();
    for (offset, c) in text.char_indices() {
        #[cfg(test)]
        super::tests::image_offset_tests::note_prefix();
        closed[offset] = embeddings == 0 && isolates.is_empty();
        match c {
            '\u{202A}' | '\u{202B}' | '\u{202D}' | '\u{202E}' => {
                embeddings = embeddings.saturating_add(1);
            }
            '\u{2066}'..='\u{2068}' => {
                isolates.push(embeddings);
                embeddings = 0;
            }
            '\u{202C}' => embeddings = embeddings.saturating_sub(1),
            '\u{2069}' => {
                if let Some(saved) = isolates.pop() {
                    embeddings = saved;
                }
            }
            _ => {}
        }
    }
    closed[text.len()] = embeddings == 0 && isolates.is_empty();
    closed
}

fn uncovered_slots(
    text: &str,
    range: std::ops::Range<usize>,
    covered: Vec<std::ops::Range<usize>>,
) -> Vec<usize> {
    // A difference array unions overlapping glyph ranges without repeatedly
    // testing every range for each literal object-replacement character.
    let mut coverage = vec![0_isize; text.len() + 1];
    for run in covered {
        #[cfg(test)]
        super::tests::image_offset_tests::note_coverage();
        let start = run.start.max(range.start);
        let end = run.end.min(range.end);
        if start < end {
            coverage[start - range.start] += 1;
            coverage[end - range.start] -= 1;
        }
    }
    let mut next_byte = 0;
    let mut active = 0_isize;
    let slots: Vec<usize> = text
        .match_indices('\u{FFFC}')
        .filter_map(|(offset, _)| {
            for delta in &coverage[next_byte..=offset] {
                #[cfg(test)]
                super::tests::image_offset_tests::note_coverage_byte();
                active += delta;
            }
            next_byte = offset + 1;
            (active == 0).then_some(range.start + offset)
        })
        .collect();
    slots
}

/// The UTF-8 object positions by fragment index, when visual order can
/// conservatively determine logical order. Unknown or mixed bidi state and
/// unequal object/slot counts retain the existing `None` result.
pub(super) fn object_text_offsets(
    line: &Line,
    prefix_closed: impl FnOnce() -> Option<bool>,
) -> Option<Vec<Option<usize>>> {
    let range = line.text_range();
    let text = line.text().get(range.clone())?;
    if text.chars().any(|c| {
        #[cfg(test)]
        super::tests::image_offset_tests::note_text();
        matches!(
            c,
            '\u{200E}' | '\u{200F}' | '\u{061C}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
        )
    }) || !prefix_closed()?
    {
        return None;
    }
    let rtl = u8::from(line.used_direction() == shodo::geometry::Direction::Rtl);
    let mut covered = Vec::new();
    let mut objects = Vec::new();
    for (index, fragment) in line.fragments().enumerate() {
        match fragment {
            Fragment::GlyphRun(run) => {
                if run.bidi_level() % 2 != rtl {
                    return None;
                }
                covered.push(run.text_range());
            }
            Fragment::Atomic(atomic) => objects.push((atomic.border_rect.inline_start, index)),
            Fragment::OutOfFlowAnchor(anchor) => objects.push((anchor.inline_position, index)),
            _ => {}
        }
    }
    let slots = uncovered_slots(text, range, covered);
    if slots.len() != objects.len() {
        return None;
    }
    objects.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut offsets = vec![None; line.fragments().len()];
    for ((_, fragment), offset) in objects.into_iter().zip(slots) {
        offsets[fragment] = Some(offset);
    }
    Some(offsets)
}

#[cfg(test)]
mod tests;
