//! The clips that cut the events of a page: the overflow clips of the boxes
//! whose fragments are cut by a page break, and the clips of the columns the
//! fragments fall in.

use super::{ClipKind, PageItems, PaintEvent, fragment_hides_box, generated_image_line};
use crate::{Document, TextLineId, paint_rules};
use raikiri_style::{CascadeResult, PseudoElem};
use raikiri_traits::{NodeId, PaintClip};
use std::collections::{HashMap, HashSet};

/// The clips that cut the lines of the paragraphs of a page, found once for
/// all the events that list them.
#[derive(Default)]
struct LineClips {
    /// The clip of the column or fragmentainer each line is cut by, in page
    /// coordinates.
    clips: HashMap<TextLineId, PaintClip>,
    /// The placement overflow clip around each line, as an index into the
    /// placement overflow clips of the page projection.
    overflow_chains: HashMap<TextLineId, usize>,
}

impl LineClips {
    /// The clips of the lines that `events` list.
    fn new(
        document: &Document,
        cascade: &CascadeResult,
        items: &PageItems<'_>,
        events: &[PaintEvent<'_>],
    ) -> Self {
        let mut line_clips = Self::default();
        let listed = listed_roots(items, events);
        let page = items.page;
        for root in &document.page_projection.text_roots {
            if (root.fragment_clip.is_none() && root.overflow_chain.is_none())
                || !listed.contains(&root.node)
            {
                continue;
            }
            let indices = lines_of_root(document, cascade, root.node, root.fragmentainer);
            if let Some(chain) = root.overflow_chain {
                for &index in &indices {
                    line_clips.overflow_chains.insert(
                        TextLineId {
                            root: NodeId::new(root.node as u64),
                            index,
                        },
                        chain,
                    );
                }
            }
            let Some(mut clip) = root.fragment_clip else {
                continue;
            };
            let source = crate::generated_content::generated_origin(root.node)
                .map_or_else(|| document.ifc_source_owner(root.node), |(owner, _)| owner);
            let mut repeat = root.is_repeat;
            if let Some(shift) = document
                .table_objects
                .headers
                .shift(source, page.content_origin_y)
            {
                let Some(shift) = shift else { continue };
                clip.y += shift - page.content_origin_y;
                repeat = true;
            }
            let (dx, dy) = page.flow_shift(repeat);
            clip.x += page.content_box.x + dx;
            clip.y += page.content_box.y + dy;
            for &index in &indices {
                line_clips.clips.insert(
                    TextLineId {
                        root: NodeId::new(root.node as u64),
                        index,
                    },
                    PaintClip::new(clip),
                );
            }
        }
        line_clips
    }
}

/// The paragraphs, by root node, with lines, generated boxes or line pieces
/// among `events`; only those need their line clips.
fn listed_roots(items: &PageItems<'_>, events: &[PaintEvent<'_>]) -> HashSet<usize> {
    events
        .iter()
        .filter_map(|event| match event {
            PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => Some(line.root),
            PaintEvent::GeneratedBox(fragment) => Some(fragment.line.root),
            PaintEvent::LineBox(piece) => Some(piece.line.root),
            PaintEvent::GeneratedImage(image) => {
                let placement = items.generated_images?.get(&image.key()?)?;
                Some(NodeId::new(placement.root as u64))
            }
            _ => None,
        })
        .filter_map(|root| usize::try_from(root.0).ok())
        .collect()
}

/// The indices of the lines of the text root `node` that are positioned on
/// the page, in the column `fragmentainer` when it is in columns.
fn lines_of_root(
    document: &Document,
    cascade: &CascadeResult,
    node: usize,
    fragmentainer: Option<usize>,
) -> Vec<usize> {
    if let Some((owner, PseudoElem::Marker)) = crate::generated_content::generated_origin(node) {
        document
            .page_projection
            .markers
            .get(&owner)
            .map_or_else(Vec::new, |marker| (0..marker.line_count()).collect())
    } else {
        crate::PositionedLines::new(document, cascade, node, fragmentainer).map_or_else(
            Vec::new,
            |positioned| {
                (0..positioned.all_lines().len())
                    .filter(|&index| positioned.line_offset(index).is_some())
                    .collect()
            },
        )
    }
}

/// Whether `event` is a box that is not visible, and paints no decoration of
/// its own wherever its fragments were listed. Its descendants decide for
/// themselves.
fn hides_decoration(cascade: &CascadeResult, event: PaintEvent<'_>) -> bool {
    match event {
        PaintEvent::Box(fragment) => fragment_hides_box(cascade, &fragment),
        PaintEvent::LineBox(piece) => fragment_hides_box(cascade, &piece.fragment),
        PaintEvent::GeneratedBox(generated) => paint_rules::hides_own_box(generated.style),
        _ => false,
    }
}

/// The placement overflow clip that cuts `event`, as an index into the
/// placement overflow clips of the page projection.
fn overflow_chain(
    cascade: &CascadeResult,
    items: &PageItems<'_>,
    lines: &LineClips,
    event: PaintEvent<'_>,
) -> Option<usize> {
    match event {
        PaintEvent::Box(fragment)
        | PaintEvent::OuterBoxShadow(fragment)
        | PaintEvent::Text(fragment)
        | PaintEvent::Replaced(fragment) => fragment.overflow_chain(),
        PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => {
            lines.overflow_chains.get(&line).copied()
        }
        PaintEvent::GeneratedBox(fragment) => lines.overflow_chains.get(&fragment.line).copied(),
        PaintEvent::LineBox(piece) => lines.overflow_chains.get(&piece.line).copied(),
        PaintEvent::GeneratedImage(image) => generated_image_line(items, image)
            .and_then(|line| lines.overflow_chains.get(&line).copied()),
        PaintEvent::ColumnRule(rule) => rule.overflow_chain,
        PaintEvent::MarkerImage(owner) => items.of(owner.0 as usize).first().and_then(|item| {
            if crate::generated_content::inside_marker_in_flow(cascade, owner.0 as usize) {
                item.own_overflow_source.or(item.overflow_chain)
            } else {
                item.overflow_chain
            }
        }),
        _ => None,
    }
}

/// The clip of the column that `event` falls in.
fn fragmentainer_clip(
    items: &PageItems<'_>,
    lines: &LineClips,
    event: PaintEvent<'_>,
) -> Option<PaintClip> {
    match event {
        PaintEvent::Box(fragment)
        | PaintEvent::OuterBoxShadow(fragment)
        | PaintEvent::Text(fragment)
        | PaintEvent::Replaced(fragment) => fragment.fragmentainer_clip(),
        PaintEvent::TextLine(line) | PaintEvent::GroupTextLine(line, _) => {
            lines.clips.get(&line).copied()
        }
        PaintEvent::GeneratedBox(fragment) => lines.clips.get(&fragment.line).copied(),
        PaintEvent::LineBox(piece) => lines.clips.get(&piece.line).copied(),
        PaintEvent::GeneratedImage(image) => {
            generated_image_line(items, image).and_then(|line| lines.clips.get(&line).copied())
        }
        PaintEvent::ColumnRule(rule) => rule.fragmentainer_clip,
        PaintEvent::MarkerImage(owner) => items
            .of(owner.0 as usize)
            .first()
            .and_then(|item| items.fragment(item).fragmentainer_clip()),
        _ => None,
    }
}

/// `events` with each one inside the clips that cut it: the placement
/// overflow clips around it, outermost first, and the clip of its column. An
/// event that paints a box that is not visible is dropped.
pub(super) fn clip_events<'a>(
    document: &Document,
    cascade: &CascadeResult,
    items: &PageItems<'a>,
    events: Vec<PaintEvent<'a>>,
) -> Vec<PaintEvent<'a>> {
    let lines = LineClips::new(document, cascade, items, &events);
    let mut clipped_events = Vec::with_capacity(events.len());
    for event in events {
        if hides_decoration(cascade, event) {
            continue;
        }
        let mut chain = overflow_chain(cascade, items, &lines, event);
        let mut overflow = Vec::new();
        while let Some(index) = chain {
            overflow.push(
                document
                    .placement_overflow_clip_on_page(index, items.page)
                    .clip,
            );
            chain = document.page_projection.placement_overflow_clips[index].parent;
        }
        for &clip in overflow.iter().rev() {
            clipped_events.push(PaintEvent::PushClip(clip, ClipKind::Overflow));
        }
        if let Some(clip) = fragmentainer_clip(items, &lines, event) {
            clipped_events.push(PaintEvent::PushClip(clip, ClipKind::Fragmentainer));
            clipped_events.push(event);
            clipped_events.push(PaintEvent::PopClip);
        } else {
            clipped_events.push(event);
        }
        clipped_events.extend(std::iter::repeat_n(PaintEvent::PopClip, overflow.len()));
    }
    clipped_events
}
