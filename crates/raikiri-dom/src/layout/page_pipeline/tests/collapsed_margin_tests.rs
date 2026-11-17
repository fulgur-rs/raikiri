use super::*;

#[test]
fn negative_margin_on_an_empty_previous_sibling_keeps_the_first_break_after_content() {
    let mut next = 0;
    let (doc, pages) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:250px"));
        doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:0;margin-top:-100px;margin-bottom:300px"),
        );
        next = doc.append_element(
            Some(body),
            "div",
            Some("display:block;height:20px;margin-top:-200px"),
        );
    });
    // Before pagination, the empty border is at 150 and the next box at 350.
    // Their combined margin starts at 250, so its first page edge is 300.
    assert_eq!(page_flow_y(&doc, next), 300.0);
    assert_eq!(pages, 4);
}

/// A box that avoids a break inside it, the first in the document below a
/// block whose margin collapses through `body`, with `wrapper_css` on that
/// block. Returns the box's top and the page count.
fn avoided_box_below_a_collapsed_margin(wrapper_css: &str) -> (f32, usize) {
    let mut avoided = 0;
    let (doc, pages) = paged_body(|doc, body| {
        let wrapper = doc.append_element(Some(body), "div", Some(wrapper_css));
        avoided = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;padding-bottom:40px;break-inside:avoid"),
        );
        for _ in 0..2 {
            doc.append_element(Some(avoided), "div", Some("display:block;height:30px"));
        }
        doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    });
    (page_flow_y(&doc, avoided), pages)
}

#[test]
fn a_margin_that_collapses_through_body_starts_the_page_for_an_avoided_box() {
    // The margin of the wrapper collapses through `body`, so the margin box of
    // the wrapper, and the avoided box with it, starts the first page. The box
    // stays there and breaks inside, as in Chromium.
    for wrapper_css in [
        "display:block;margin-top:5px",
        "display:block;margin-top:5px;padding-top:5px",
        "display:block;margin-top:5px;border-top:5px solid",
        "display:flow-root;margin-top:5px",
    ] {
        let (top, pages) = avoided_box_below_a_collapsed_margin(wrapper_css);
        assert!(
            top < 100.0,
            "{wrapper_css}: the box stays on the first page at {top}"
        );
        assert_eq!(pages, 2, "{wrapper_css}");
    }
    // Content above the wrapper leaves room to move into: the box moves
    // whole, with the wrapper's margin truncated at the break.
    let mut avoided = 0;
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:5px"));
        let wrapper = doc.append_element(Some(body), "div", Some("display:block;margin-top:5px"));
        avoided = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;padding-bottom:40px;break-inside:avoid"),
        );
        for _ in 0..2 {
            doc.append_element(Some(avoided), "div", Some("display:block;height:30px"));
        }
    });
    assert_eq!(page_flow_y(&doc, avoided), 100.0);
}

/// A paragraph of one line, or a box of two that avoids a break inside it
/// (`avoided`), with an 8px margin-top as the first child of a box with
/// `root_css`, after a 70px block. Returns the tops of the root and of the
/// paragraph or box.
fn first_child_of_a_moved_root(root_css: &str, avoided: bool) -> (f32, f32) {
    let (mut root, mut child) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:70px"));
        root = doc.append_element(Some(body), "div", Some(root_css));
        if avoided {
            child = doc.append_element(
                Some(root),
                "div",
                Some("display:block;margin-top:8px;break-inside:avoid"),
            );
            for _ in 0..2 {
                doc.append_element(Some(child), "div", Some("display:block;height:15px"));
            }
        } else {
            child = doc.append_element(
                Some(root),
                "div",
                Some(
                    "display:block;margin-top:8px;font-family:Ahem;font-size:20px;line-height:30px",
                ),
            );
            doc.append_text(child, "A");
        }
        doc.append_element(Some(body), "div", Some("display:block;height:10px"));
    });
    (page_flow_y(&doc, root), page_flow_y(&doc, child))
}

#[test]
fn a_box_with_its_own_formatting_context_takes_its_first_childs_margin_to_the_next_page() {
    // The margin of the first child stays inside a box that establishes an
    // independent formatting context, so it does not adjoin the break before
    // the box and is not truncated: the box starts the next page and the
    // child lands below the margin, as in Chromium.
    for root_css in ["display:flow-root", "display:block;overflow:hidden"] {
        for avoided in [false, true] {
            assert_eq!(
                first_child_of_a_moved_root(root_css, avoided),
                (100.0, 108.0),
                "{root_css}, avoided: {avoided}"
            );
        }
    }
    // A plain block lets the margin collapse through its top to the break,
    // which truncates it.
    for avoided in [false, true] {
        assert_eq!(
            first_child_of_a_moved_root("display:block", avoided),
            (100.0, 100.0),
            "avoided: {avoided}"
        );
    }
}

#[test]
fn a_box_that_avoids_breaks_takes_a_kept_margin_along_even_when_it_then_breaks() {
    // The box with the kept margin no longer fits the next page, but the
    // margin still goes along and the box breaks inside there, as in Chromium.
    let (mut root, mut avoided) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:70px"));
        root = doc.append_element(Some(body), "div", Some("display:flow-root"));
        avoided = doc.append_element(
            Some(root),
            "div",
            Some("display:block;margin-top:8px;break-inside:avoid"),
        );
        for height in [65, 30] {
            doc.append_element(
                Some(avoided),
                "div",
                Some(&format!("display:block;height:{height}px")),
            );
        }
    });
    assert_eq!(
        [root, avoided].map(|id| page_flow_y(&doc, id)),
        [100.0, 108.0]
    );
}

#[test]
fn a_negative_first_child_margin_inside_its_own_formatting_context_goes_along_too() {
    // A negative margin is kept inside the box as well: the box starts the
    // next page and the child lands above its top, as in Chromium.
    let (mut root, mut avoided) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:70px"));
        root = doc.append_element(Some(body), "div", Some("display:flow-root"));
        avoided = doc.append_element(
            Some(root),
            "div",
            Some("display:block;margin-top:-10px;break-inside:avoid"),
        );
        for _ in 0..2 {
            doc.append_element(Some(avoided), "div", Some("display:block;height:30px"));
        }
    });
    assert_eq!(
        [root, avoided].map(|id| page_flow_y(&doc, id)),
        [100.0, 90.0]
    );
}

#[test]
fn a_negative_first_child_margin_goes_along_with_a_paragraph_too() {
    // The line runs past the page end, so the box that keeps the margin of
    // the paragraph inside it starts the next page, with the paragraph above
    // its top, as in Chromium.
    let (mut root, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:95px"));
        root = doc.append_element(Some(body), "div", Some("display:flow-root"));
        paragraph = doc.append_element(
            Some(root),
            "div",
            Some("display:block;margin-top:-5px;font-family:Ahem;font-size:20px;line-height:30px"),
        );
        doc.append_text(paragraph, "A");
    });
    assert_eq!(
        [root, paragraph].map(|id| page_flow_y(&doc, id)),
        [100.0, 95.0]
    );
}

/// The tops of a box with `root_css` after a block of `above` px, and of a
/// box inside it, below `wrapper_css` blocks, that avoids a break inside it
/// with `child_css`.
fn nested_avoided_box(
    above: f32,
    root_css: &str,
    wrapper_css: &[&str],
    child_css: &str,
) -> [f32; 2] {
    let (mut root, mut child) = (0, 0);
    let (doc, _) = paged_body(|doc, body| {
        doc.append_element(
            Some(body),
            "div",
            Some(&format!("display:block;height:{above}px")),
        );
        root = doc.append_element(Some(body), "div", Some(root_css));
        let mut parent = root;
        for &css in wrapper_css {
            parent = doc.append_element(Some(parent), "div", Some(css));
        }
        child = doc.append_element(
            Some(parent),
            "div",
            Some(&format!("display:block;break-inside:avoid;{child_css}")),
        );
        for _ in 0..2 {
            doc.append_element(Some(child), "div", Some("display:block;height:15px"));
        }
    });
    [root, child].map(|id| page_flow_y(&doc, id))
}

#[test]
fn a_kept_margin_goes_along_through_blocks_it_collapses_through() {
    // The margin collapses up through the plain blocks in between, so the
    // box that keeps it still takes it to the next page, as in Chromium.
    for (wrappers, top) in [
        (&["display:block"][..], 108.0),
        (&["display:block;margin-top:-4px"][..], 104.0),
        (
            &["display:block", "display:block;margin-top:-3px"][..],
            105.0,
        ),
    ] {
        assert_eq!(
            nested_avoided_box(70.0, "display:flow-root", wrappers, "margin-top:8px"),
            [100.0, top],
            "{wrappers:?}"
        );
    }
}

#[test]
fn a_kept_margin_that_runs_over_the_page_end_takes_its_box_along() {
    // The margin of the first child crosses the page end, so the child starts
    // the next page by itself; the break cannot truncate a margin kept inside
    // the box, so the box moves with it, as in Chromium.
    assert_eq!(
        nested_avoided_box(95.0, "display:flow-root", &[], "margin-top:8px"),
        [100.0, 108.0]
    );
}

#[test]
fn a_kept_margin_goes_along_below_a_relative_offset() {
    // The offset only paints the box elsewhere; the margin is kept in the
    // flow and goes along, as in Chromium.
    assert_eq!(
        nested_avoided_box(
            70.0,
            "display:flow-root",
            &[],
            "position:relative;top:4px;margin-top:8px"
        ),
        [100.0, 112.0]
    );
}

#[test]
fn a_negative_margin_inside_a_top_border_goes_along_with_a_paragraph() {
    let (mut root, mut paragraph) = (0, 0);
    let (doc, _) = paged_body_with_ifc(|doc, body| {
        doc.append_element(Some(body), "div", Some("display:block;height:95px"));
        root = doc.append_element(
            Some(body),
            "div",
            Some("display:block;border-top:1px solid"),
        );
        let wrapper = doc.append_element(Some(root), "div", Some("display:block"));
        paragraph = doc.append_element(
            Some(wrapper),
            "div",
            Some("display:block;margin-top:-10px;font-family:Ahem;font-size:20px;line-height:30px"),
        );
        doc.append_text(paragraph, "A");
    });
    assert_eq!(
        [root, paragraph].map(|id| page_flow_y(&doc, id)),
        [100.0, 91.0]
    );
}

/// Whether the box of set height zero inside a `display: contents` wrapper
/// stays with the box before the wrapper, whose bottom margin is
/// `margin_bottom`.
fn empty_box_in_contents_stays(margin_bottom: &str) -> bool {
    use raikiri_style::{build_rule_tree, cascade};
    use raikiri_traits::PageBox;

    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let body = doc.append_element(
        Some(html),
        "body",
        Some("display:block;margin:0;width:100px"),
    );
    let style = format!("display:block;height:20px;margin-bottom:{margin_bottom}");
    doc.append_element(Some(body), "div", Some(&style));
    let wrapper = doc.append_element(Some(body), "div", Some("display:contents"));
    let empty = doc.append_element(
        Some(wrapper),
        "div",
        Some("display:block;height:0;margin-bottom:30px"),
    );
    doc.mark_in_document_flags();
    let rules = build_rule_tree(&doc);
    let cascade = cascade(&doc, &rules).unwrap();
    let mut page = PageBox::new();
    page.width = 100.0;
    page.height = 100.0;
    layout_pages(&mut doc, &cascade, page).unwrap();
    let session = crate::layout::context::LayoutSession::bridged(&mut doc, &cascade);
    collapsed_empty_box(&session, &cascade, empty)
}

#[test]
fn the_margin_above_an_empty_box_is_found_past_a_contents_wrapper() {
    // The box before the wrapper adjoins the empty box: its bottom margin
    // collapses into the empty box's top, which keeps the box behind.
    assert!(empty_box_in_contents_stays("5px"));
    assert!(!empty_box_in_contents_stays("0"));
}
