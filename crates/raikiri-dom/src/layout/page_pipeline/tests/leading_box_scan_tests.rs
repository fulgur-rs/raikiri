use super::*;

fn reset() {
    PARENT_CHILD_STEPS.with(|count| count.set(0));
}

fn steps() -> usize {
    PARENT_CHILD_STEPS.with(|count| count.get())
}

/// `count` blocks of `css` under one section on 100px pages, each holding
/// `lines` lines of Ahem text that are as wide as the page. Returns the top of
/// each below the section, the page count and the children of parents that
/// were read to place them.
fn blocks_in_a_section(count: usize, lines: usize, css: &str) -> (Vec<f32>, usize, usize) {
    use crate::layout::test_support::with_ahem;
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px;font:10px/10px Ahem"),
    );
    let section = doc.append_element(Some(body), "section", Some("display:block"));
    let blocks: Vec<usize> = (0..count)
        .map(|_| {
            let block = doc.append_element(Some(section), "div", Some(css));
            doc.append_text(block, "aaaaaaaaaa ".repeat(lines));
            block
        })
        .collect();
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    reset();
    let pages = layout_pages(with_ahem(&mut doc), &cascade, page).unwrap();
    let work = steps();
    let tops = blocks
        .iter()
        .map(|&id| doc.get_node(id).unwrap().layout.location.y)
        .collect();
    (tops, pages.len(), work)
}

/// The work of placing 64 and 256 of the blocks, which must grow with their
/// number (4 times) and not with its square (16 times).
fn work_for_four_times_the_blocks(lines: usize, css: &str, expected_pages: [usize; 2]) {
    let mut work = [0, 0];
    for (at, count) in [64, 256].into_iter().enumerate() {
        let (tops, pages, steps) = blocks_in_a_section(count, lines, css);
        assert_eq!(pages, expected_pages[at], "{css}: pages for {count}");
        // Every block starts a page of its own.
        let expected: Vec<f32> = (0..count).map(|index| index as f32 * 100.0).collect();
        assert_eq!(tops, expected, "{css}: tops for {count}");
        work[at] = steps;
    }
    assert!(work[0] > 0, "{css}: the lookups are reached");
    assert!(
        work[1] <= 6 * work[0],
        "{css}: {} children read for 64 blocks and {} for 256",
        work[0],
        work[1]
    );
}

#[test]
fn a_paragraph_that_moves_whole_to_the_next_page_is_found_without_counting_its_place() {
    // Nine 10px lines, so that the second paragraph's first line is the only
    // one of it that fits below the first and its orphans move it whole.
    work_for_four_times_the_blocks(9, "display:block", [64, 256]);
}

#[test]
fn a_box_that_breaks_before_itself_is_placed_without_counting_its_place() {
    work_for_four_times_the_blocks(1, "display:block;height:30px;break-before:page", [64, 256]);
}

/// The result `renders_before_child` had when it looked for the place of
/// `child` first and then for a sibling that decides.
fn renders_before_child_by_place(document: &Document, parent: usize, child: usize) -> bool {
    let out_of_flow = |node: usize| {
        document.nodes[node].kind() == NodeKind::Element
            && matches!(
                document.nodes[node].position(),
                PositionValue::Absolute | PositionValue::Fixed
            )
    };
    if sibling_renders(document, child) && out_of_flow(child) {
        return true;
    }
    let siblings = document.nodes[parent].layout_children();
    let child_index = siblings.iter().position(|&sibling| sibling == child);
    siblings.iter().enumerate().any(|(index, &sibling)| {
        sibling_renders(document, sibling)
            && !out_of_flow(sibling)
            && child_index.is_none_or(|child_index| index < child_index)
    })
}

/// A wrapper with the given kinds of children, and a box outside it.
fn wrapper_with(kinds: &[usize]) -> (Document, usize, Vec<usize>, usize) {
    const BOXES: [&str; 6] = [
        "display:block;height:5px",
        "display:none",
        "display:contents",
        "position:absolute;width:5px;height:5px",
        "position:fixed;width:5px;height:5px",
        "position:absolute;display:none",
    ];
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(Some(html), "body", Some("display:block"));
    // Text among the children of a block would make it hold lines, so the
    // wrapper lists its children as a flex container does.
    let wrapper = doc.append_element(
        Some(body),
        "div",
        Some("display:flex;flex-direction:column"),
    );
    let children: Vec<usize> = kinds
        .iter()
        .map(|&kind| match kind {
            0..=5 => doc.append_element(Some(wrapper), "div", Some(BOXES[kind])),
            6 => doc.append_text(wrapper, "  \n "),
            _ => doc.append_text(wrapper, "x"),
        })
        .collect();
    let outside = doc.append_element(Some(body), "div", Some("display:block"));
    doc.mark_in_document_flags();
    let rules = raikiri_style::build_rule_tree(&doc);
    let cascade = raikiri_style::cascade(&doc, &rules).expect("cascade");
    layout_single_page(&mut doc, &cascade, page_box_800x600()).unwrap();
    (doc, wrapper, children, outside)
}

#[test]
fn what_renders_before_a_child_is_decided_as_it_was_by_its_place() {
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |below: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % below as u64) as usize
    };
    let (mut yes, mut no) = (0, 0);
    for _ in 0..600 {
        let kinds: Vec<usize> = (0..next(8)).map(|_| next(8)).collect();
        let (doc, wrapper, children, outside) = wrapper_with(&kinds);
        // Each child, and a box that the wrapper does not hold.
        for child in children.iter().copied().chain([outside]) {
            let decided = renders_before_child(&doc, wrapper, child);
            assert_eq!(
                decided,
                renders_before_child_by_place(&doc, wrapper, child),
                "{kinds:?} at {child}"
            );
            if decided {
                yes += 1;
            } else {
                no += 1;
            }
        }
    }
    assert!(yes > 200 && no > 200, "{yes} true, {no} false");
}

#[test]
fn only_boxes_in_the_flow_before_a_child_count() {
    // (kinds of the children, index of the child, whether something renders
    // before it)
    let cases: &[(&[usize], usize, bool)] = &[
        // A box before it.
        (&[0, 0], 1, true),
        // Boxes that render nothing, and blank text, before it.
        (&[1, 6, 5], 2, false),
        // Text before it.
        (&[7, 0], 1, true),
        // A box after it does not count.
        (&[0, 0], 0, false),
        // Nor does an absolutely or fixed positioned one, wherever it is.
        (&[0, 3], 0, false),
        (&[0, 4], 0, false),
        (&[3, 4, 0], 2, false),
        (&[3, 0, 0], 2, true),
        // `display: contents` renders.
        (&[2, 0], 1, true),
        // The child itself, placed out of the flow.
        (&[3], 0, true),
    ];
    for &(kinds, at, expected) in cases {
        let (doc, wrapper, children, _) = wrapper_with(kinds);
        assert_eq!(
            renders_before_child(&doc, wrapper, children[at]),
            expected,
            "{kinds:?} at {at}"
        );
    }
}
