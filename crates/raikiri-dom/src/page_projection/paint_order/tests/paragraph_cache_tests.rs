use super::*;
use crate::layout::ifc::test_support::{ahem_fonts, sheet_fixture};

#[test]
fn sparse_paragraph_slots_preserve_full_listing_with_different_supplied_lines() {
    let fixture = sheet_fixture(
        "div::first-line{opacity:.5} div::first-letter{opacity:.6;background:blue}\
         span::before{content:'B';opacity:.7} span::after{content:'A';background:red}",
        "width:30px;line-height:10px",
        |doc, root| {
            for _ in 0..20 {
                let span = doc.append_element(
                    Some(root),
                    "span",
                    taffy::Style::default(),
                    Some("opacity:.8"),
                );
                let inner = doc.append_element(
                    Some(span),
                    "em",
                    taffy::Style::default(),
                    Some("opacity:.9"),
                );
                doc.append_text(inner, "X X ");
            }
        },
    );
    let (mut doc, cascade) = (fixture.doc, fixture.cascade);
    doc.set_font_collection(ahem_fonts());
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).unwrap();
    assert!(slices.len() > 1);
    doc.project_pages(&cascade, page, &slices, &[]).unwrap();
    for slice in slices.iter().rev() {
        let runs = doc.page_text_runs(&cascade, slice.page_index);
        let mut lines = TextLines::default();
        for run in &runs {
            let group = doc.text_run_opacity_group(&cascade, run);
            if let Some(group) = group {
                let owned = lines.groups.entry(group).or_default();
                if !owned.contains(&run.line) {
                    owned.push(run.line);
                }
                if run.letter_box.is_some() {
                    lines.letters.insert(
                        (run.line.root.0 as usize, group.0 as usize),
                        doc.cached_letter_groups(&cascade, run.line.root.0 as usize)
                            .around(group.0 as usize),
                    );
                }
            } else {
                let owned = lines.paragraphs.entry(run.line.root).or_default();
                if !owned.contains(&run.line) {
                    owned.push(run.line);
                }
            }
        }
        let empty = TextLines::default();
        for supplied in [Some(&lines), Some(&empty), None, Some(&lines)] {
            let full = doc.page_paint_order_impl(&cascade, slice.page_index, supplied, false);
            let sparse = doc.page_paint_order_impl(&cascade, slice.page_index, supplied, true);
            assert_eq!(
                format!("{sparse:?}"),
                format!("{full:?}"),
                "page {}",
                slice.page_index
            );
        }
    }
}

#[test]
fn merging_groups_preserves_insertion_order_with_nonmonotonic_box_positions() {
    let mut boxes = Vec::new();
    let mut groups = Vec::new();
    let fixture = crate::layout::ifc::test_support::block_fixture_with_ahem(
        "width:100px",
        |doc, root| {
            for _ in 0..3 {
                boxes.push(doc.append_element(
                    Some(root),
                    "i",
                    taffy::Style::default(),
                    Some("display:inline-block;opacity:.5;width:1px;height:1px;position:relative;z-index:0"),
                ));
                let group = doc.append_element(
                    Some(root),
                    "span",
                    taffy::Style::default(),
                    Some("opacity:.5;position:relative;z-index:0"),
                );
                doc.append_text(group, "X");
                groups.push(group);
            }
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    crate::layout_single_page(
        &mut doc,
        &cascade,
        crate::layout::test_support::page_box_800x600(),
    )
    .unwrap();
    let frames = doc.paragraph_frames(
        &cascade,
        root,
        vec![boxes[2], boxes[0], boxes[1]],
        groups.clone(),
        LinePart::All,
        false,
    );
    let actual: Vec<_> = frames
        .below
        .into_iter()
        .chain(frames.above)
        .map(|frame| match frame {
            Frame::Visit(id) | Frame::Paragraph(id) | Frame::InlineGroup(_, id, _) => id,
            _ => panic!("unexpected paragraph frame"),
        })
        .collect();
    assert_eq!(
        actual,
        [
            groups[0], groups[1], boxes[2], boxes[0], boxes[1], groups[2]
        ]
    );
}

#[test]
fn a_first_letter_alone_keeps_its_holding_dom_group_reachable() {
    let fixture = sheet_fixture(
        "div::first-line{opacity:.5} div::first-letter{opacity:.6;background:blue}",
        "width:30px;line-height:10px",
        |doc, root| {
            let span = doc.append_element(
                Some(root),
                "span",
                taffy::Style::default(),
                Some("opacity:.8"),
            );
            doc.append_text(span, "X X X");
        },
    );
    let (mut doc, cascade, root) = (fixture.doc, fixture.cascade, fixture.root);
    doc.set_font_collection(ahem_fonts());
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    let slices = layout_pages(&mut doc, &cascade, page).unwrap();
    doc.project_pages(&cascade, page, &slices, &[]).unwrap();
    // Retain the independently projected letter decoration while removing
    // the holder's regular box and text candidates.
    doc.page_projection.pages[0]
        .items
        .retain(|item| item.node_id.0 == root as u64);
    let empty = TextLines::default();
    let full = doc.page_paint_order_impl(&cascade, 0, Some(&empty), false);
    let sparse = doc.page_paint_order_impl(&cascade, 0, Some(&empty), true);
    assert!(full.iter().any(|event| matches!(event, PaintEvent::GeneratedBox(generated) if generated.kind == GeneratedKind::FirstLetter)));
    assert_eq!(format!("{sparse:?}"), format!("{full:?}"));
}
