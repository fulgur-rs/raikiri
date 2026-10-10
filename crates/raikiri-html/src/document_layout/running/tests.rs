use super::*;

fn index(placements: &[(u64, u32, bool)]) -> RunningIndex {
    let mut index = RunningIndex::default();
    index.pools.insert(
        SmolStr::new("hdr"),
        placements
            .iter()
            .map(|&(node, page, starts_page)| Placement {
                node: NodeId(node),
                page,
                starts_page,
            })
            .collect(),
    );
    index
}

const ALL: [StringFetchMode; 4] = [
    StringFetchMode::First,
    StringFetchMode::Start,
    StringFetchMode::Last,
    StringFetchMode::FirstExcept,
];

fn selections(index: &RunningIndex, page: u32) -> Vec<Option<u64>> {
    ALL.iter()
        .map(|&fetch| index.select("hdr", fetch, page).map(|node| node.0))
        .collect()
}

#[test]
fn a_page_with_no_assignment_shows_the_entry_value_for_every_keyword() {
    let index = index(&[(1, 0, true), (2, 0, false)]);
    // Page 1 has no assignment: every keyword falls back to the last
    // element assigned on an earlier page.
    assert_eq!(selections(&index, 1), vec![Some(2); 4]);
}

#[test]
fn keywords_pick_from_the_assignments_on_the_page() {
    let index = index(&[(1, 0, true), (2, 1, false), (3, 1, false)]);
    // first, start (2 does not start page 1), last, first-except.
    assert_eq!(selections(&index, 1), vec![Some(2), Some(1), Some(3), None]);
}

#[test]
fn start_takes_an_assignment_that_starts_the_page() {
    let index = index(&[(1, 0, true), (2, 1, true), (3, 1, false)]);
    assert_eq!(
        index.select("hdr", StringFetchMode::Start, 1),
        Some(NodeId(2))
    );
}

#[test]
fn nothing_applies_before_the_first_assignment() {
    let index = index(&[(1, 2, true)]);
    assert_eq!(selections(&index, 0), vec![None; 4]);
    assert_eq!(selections(&index, 2), vec![Some(1), Some(1), Some(1), None]);
    assert_eq!(selections(&index, 3), vec![Some(1); 4]);
}

#[test]
fn unknown_names_select_nothing() {
    let index = index(&[(1, 0, true)]);
    assert_eq!(index.select("ftr", StringFetchMode::First, 0), None);
    assert!(index.contains(NodeId(1)));
    assert!(!index.contains(NodeId(2)));
}
