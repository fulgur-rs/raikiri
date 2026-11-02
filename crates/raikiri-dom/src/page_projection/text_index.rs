use std::collections::HashMap;
use std::sync::Arc;

use super::flow_pages::FlowPages;
use super::line_ownership::LineOwnership;
use super::records::{FlowAxis, PageFragment, ProjectedTextRoot};
use super::text_runs::{MarkerText, text_source};
use crate::Document;
use crate::layout::ProjectionWork;
use crate::layout::ifc::positioned_runs::geometry::{
    PositionedLineGeometry, PreparedPositionedLines,
};
use raikiri_style::CascadeResult;
use raikiri_traits::LayoutError;
use std::collections::BTreeMap;

pub(super) type GeometryCache = HashMap<usize, Arc<PositionedLineGeometry>>;

#[derive(Clone, Debug)]
pub(super) enum PageLines {
    All,
    Selected(Vec<usize>),
}

impl PageLines {
    pub(super) fn indices<'a>(
        &'a self,
        view: &'a PreparedPositionedLines<'_>,
    ) -> impl Iterator<Item = usize> + 'a {
        let all = matches!(self, Self::All)
            .then_some(view)
            .into_iter()
            .flat_map(|view| view.indices());
        let selected = match self {
            Self::Selected(lines) => lines.as_slice(),
            Self::All => &[],
        };
        all.chain(selected.iter().copied())
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct PageTextIndex {
    geometry: GeometryCache,
    lines: Vec<HashMap<usize, PageLines>>,
    positions: HashMap<u32, Vec<usize>>,
    clip_placements: HashMap<usize, super::clip_index::PlacementRanges>,
}

impl PageTextIndex {
    pub(super) fn prepare(
        document: &Document,
        cascade: &CascadeResult,
        roots: &[ProjectedTextRoot],
        markers: &BTreeMap<usize, MarkerText>,
        pages: &[PageFragment],
        work: &mut ProjectionWork<'_, '_>,
    ) -> Result<(Self, Vec<Vec<usize>>), LayoutError> {
        let mut positions = HashMap::<u32, Vec<usize>>::new();
        for (position, page) in pages.iter().enumerate() {
            positions.entry(page.page_index).or_default().push(position);
        }
        let is_marker = |root: &ProjectedTextRoot| {
            matches!(
                crate::generated_content::generated_origin(root.node),
                Some((_, raikiri_style::PseudoElem::Marker))
            )
        };
        let mut geometry = GeometryCache::new();
        for root in roots {
            if is_marker(root) || geometry.contains_key(&root.node) {
                continue;
            }
            if let Some(node) = document.ifc_layout_node(root.node)
                && node.ifc_lines().is_some()
            {
                // Reading the existing paragraph records does not create
                // new fragments. Additional page associations are charged
                // below; every traversal still checks cancellation.
                work.check()?;
                if let Some(prepared) = PositionedLineGeometry::new(document, root.node) {
                    geometry.insert(root.node, Arc::new(prepared));
                }
            }
        }
        let mut selected = vec![HashMap::new(); pages.len()];
        let flow = FlowPages::new(pages);
        let mut line_ownership = LineOwnership::new(pages);
        let axis = pages
            .first()
            .map_or(FlowAxis::default(), |page| page.flow_axis);
        let same_axis = pages.iter().all(|page| page.flow_axis == axis);
        let mut clip_placements: HashMap<usize, super::clip_index::PlacementRanges> =
            HashMap::new();
        for (placement, root) in roots.iter().enumerate() {
            let has_clip = root.fragment_clip.is_some() || root.overflow_chain.is_some();
            if let Some((owner, raikiri_style::PseudoElem::Marker)) =
                crate::generated_content::generated_origin(root.node)
            {
                if has_clip && let Some(marker) = markers.get(&owner) {
                    clip_placements
                        .entry(root.node)
                        .or_default()
                        .push(0..marker.line_count(), placement);
                }
                continue;
            }
            let Some(view) = geometry
                .get(&root.node)
                .and_then(|data| data.view(document, cascade, root.fragmentainer))
            else {
                continue;
            };
            if has_clip {
                for range in view.line_ranges() {
                    work.check()?;
                    clip_placements
                        .entry(root.node)
                        .or_default()
                        .push(range, placement);
                }
            }
            let repeated = root.is_repeat
                || document
                    .table_objects
                    .headers
                    .owner(text_source(document, root.node))
                    .is_some();
            if repeated {
                for page in &mut selected {
                    work.check()?;
                    page.insert(placement, PageLines::All);
                }
                continue;
            }
            for index in view.indices() {
                work.check()?;
                #[cfg(test)]
                super::work_tests::line_candidate();
                let line = &view.all_lines()[index];
                let offset = view
                    .line_offset(index)
                    .expect("selected line has an offset");
                let top = root.y + offset.1 + line.block_offset();
                let bottom = top + line.block_size();
                if !same_axis {
                    for (position, page) in pages.iter().enumerate() {
                        work.charge(1)?;
                        let (low, high) = page.flow_axis.line_extent(root, top, bottom);
                        if line_ownership.owns(page, low, high) {
                            push_line(&mut selected[position], placement, index);
                        }
                    }
                    continue;
                }
                let (low, high) = axis.line_extent(root, top, bottom);
                if !low.is_finite() || !high.is_finite() {
                    continue;
                }
                let center = (low + high) * 0.5;
                let mut first_center: Option<(usize, f32)> = None;
                for (position, page) in flow.around_slots(center, center) {
                    if flow.ends_are_ordered() {
                        work.check()?;
                    } else {
                        work.charge(1)?;
                    }
                    let (start, end) = page.flow_range.expect("flow index contains ranged pages");
                    if crate::layout::line_center_on_page(low, high, start, end)
                        && first_center.is_none_or(|(first, _)| position < first)
                    {
                        first_center = Some((position, end - start));
                    }
                }
                let center_height = first_center.map(|(_, height)| height);
                let tall = matches!(axis, FlowAxis::Y)
                    && center_height.is_some_and(|height| high - low > height);
                let (from, to) = if matches!(axis, FlowAxis::Y) {
                    (low, high)
                } else {
                    (center, center)
                };
                let mut associations = 0usize;
                for (position, page) in flow.around_slots(from, to) {
                    if flow.ends_are_ordered() {
                        work.check()?;
                    } else {
                        work.charge(1)?;
                    }
                    let (start, end) = page.flow_range.expect("flow index contains ranged pages");
                    if crate::layout::line_center_on_page(low, high, start, end)
                        || (tall && low < end - 0.01 && high > start + 0.01)
                    {
                        if associations > 0 {
                            work.charge(1)?;
                        }
                        associations += 1;
                        push_line(&mut selected[position], placement, index);
                    }
                }
            }
        }
        let mut by_page: Vec<Vec<usize>> = selected
            .iter()
            .map(|page| page.keys().copied().collect())
            .collect();
        for (placement, root) in roots.iter().enumerate().filter(|(_, root)| is_marker(root)) {
            let repeated = root.is_repeat
                || document
                    .table_objects
                    .headers
                    .owner(text_source(document, root.node))
                    .is_some();
            if repeated {
                for page in &mut by_page {
                    work.check()?;
                    page.push(placement);
                }
            } else if let Some((owner, _)) = crate::generated_content::generated_origin(root.node)
                && let Some(first) = markers.get(&owner).and_then(MarkerText::first_page)
            {
                for &position in positions.get(&first).into_iter().flatten() {
                    work.check()?;
                    by_page[position].push(placement);
                }
            }
        }
        for page in &mut by_page {
            page.sort_unstable();
        }
        for ranges in clip_placements.values_mut() {
            work.check()?;
            ranges.finish();
        }
        Ok((
            Self {
                geometry,
                lines: selected,
                positions,
                clip_placements,
            },
            by_page,
        ))
    }

    pub(super) fn view<'a>(
        &'a self,
        document: &'a Document,
        cascade: &'a CascadeResult,
        root: &ProjectedTextRoot,
    ) -> Option<PreparedPositionedLines<'a>> {
        self.geometry
            .get(&root.node)?
            .view(document, cascade, root.fragmentainer)
    }

    pub(super) fn lines(&self, page: usize, placement: usize) -> Option<&PageLines> {
        self.lines.get(page)?.get(&placement)
    }

    pub(super) fn clip_placements(&self, root: usize, line: usize) -> Vec<usize> {
        self.clip_placements
            .get(&root)
            .map_or_else(Vec::new, |ranges| ranges.at(line))
    }

    #[cfg(test)]
    pub(super) fn clip_range_count(&self) -> usize {
        self.clip_placements
            .values()
            .map(|ranges| ranges.len())
            .sum()
    }

    pub(super) fn positions(&self, page: u32) -> &[usize] {
        self.positions.get(&page).map_or(&[], Vec::as_slice)
    }
}

fn push_line(page: &mut HashMap<usize, PageLines>, placement: usize, index: usize) {
    if let PageLines::Selected(lines) = page
        .entry(placement)
        .or_insert_with(|| PageLines::Selected(Vec::new()))
    {
        lines.push(index);
    }
}

#[cfg(test)]
mod tests;
