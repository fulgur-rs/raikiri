use super::*;
use crate::layout::context::LayoutSession;
use crate::layout::page_pipeline::layout_single_page_in_session;
use crate::layout::test_support::{absolute_rect, cascaded, page_box_800x600, with_ahem};
use raikiri_style::CascadeResult;

/// `html > body > div > div > …`: `levels` boxes with the style `css`, each in
/// the one before. Returns the document, its cascade and the innermost box.
fn chain(css: &str, levels: usize) -> (Document, CascadeResult, usize) {
    let mut doc = Document::new();
    let html = doc.append_element(Some(0), "html", Some("display:block"));
    let mut parent = doc.append_element(Some(html), "body", Some("display:block;margin:0"));
    for _ in 0..levels {
        parent = doc.append_element(Some(parent), "div", Some(css));
    }
    doc.append_text(parent, "x");
    let cascade = cascaded(&doc);
    with_ahem(&mut doc);
    (doc, cascade, parent)
}

/// Lay `doc` out with the nesting limit `limit`, if there is one. Returns the
/// result and the deepest nesting the layout reached.
fn laid_out(
    doc: &mut Document,
    cascade: &CascadeResult,
    limit: Option<usize>,
) -> (Result<(), LayoutError>, usize) {
    let mut session = LayoutSession::with_cascade(doc, cascade);
    if let Some(limit) = limit {
        session.context.recursion.limit = limit;
    }
    let result = layout_single_page_in_session(&mut session, cascade, page_box_800x600());
    (result, session.context.recursion.peak)
}

fn peak_of_chain(css: &str, levels: usize) -> usize {
    let (mut doc, cascade, _) = chain(css, levels);
    let (result, peak) = laid_out(&mut doc, &cascade, None);
    result.unwrap_or_else(|error| panic!("{css} x {levels}: {error}"));
    peak
}

/// Chains of every kind of box whose layout nests differently: the layout of a
/// box asks for the layouts of its children in a way of its own.
const KINDS: &[&str] = &[
    "display:block",
    "display:flex",
    "display:flex;flex-direction:column",
    "display:flex;flex-wrap:wrap",
    "display:grid",
    "display:inline-block",
    "display:inline-flex",
    "display:flow-root",
    "display:table",
    "display:table-row",
    "display:table-cell",
    "float:left",
    "position:relative",
    "position:absolute",
    "position:fixed",
    "writing-mode:vertical-rl",
    "writing-mode:vertical-lr;display:inline-block",
    "contain:size",
    "contain:size;writing-mode:vertical-rl;width:50px;height:50px",
    "width:min-content",
    "width:fit-content",
    "height:100%;padding:5%",
    "overflow:hidden",
    "columns:2",
    "columns:2;height:300px",
    "aspect-ratio:2/1",
];

#[test]
fn the_nesting_of_a_chain_of_boxes_grows_with_the_depth_of_the_tree_at_a_rate_the_limit_allows() {
    const SHORT: usize = 3;
    const LONG: usize = 8;
    for css in KINDS {
        let short = peak_of_chain(css, SHORT);
        let long = peak_of_chain(css, LONG);
        let per_level = (long - short).div_ceil(LONG - SHORT);
        assert!(
            per_level * MAX_LAYOUT_DEPTH + short <= MAX_LAYOUT_RECURSION,
            "{css}: {short} nested layouts for {SHORT} boxes and {long} for {LONG} would reach \
             more than {MAX_LAYOUT_RECURSION} for {MAX_LAYOUT_DEPTH} boxes"
        );
    }
}

#[test]
fn a_layout_nested_past_the_limit_ends_with_an_error() {
    let (mut doc, cascade, _) = chain("display:block", 30);
    let (result, peak) = laid_out(&mut doc, &cascade, None);
    result.expect("layout under the default limit");

    let (result, reached) = laid_out(&mut doc, &cascade, Some(peak));
    result.expect("layout under a limit that is just enough");
    assert_eq!(reached, peak);

    let (result, reached) = laid_out(&mut doc, &cascade, Some(peak - 1));
    let Err(LayoutError::Internal { message }) = result else {
        panic!("expected an internal error, got {result:?}");
    };
    assert!(
        message.contains(&format!("nested more than {} levels", peak - 1)),
        "{message}"
    );
    assert_eq!(reached, peak - 1);
}

#[test]
fn a_document_laid_out_again_after_a_pass_that_hit_the_limit_gets_its_real_boxes() {
    let css = "display:block;padding:1px;width:auto";
    let (mut reference, reference_cascade, innermost) = chain(css, 12);
    laid_out(&mut reference, &reference_cascade, None)
        .0
        .expect("reference layout");
    let expected = absolute_rect(&reference, innermost);
    assert!(expected.1 > 0.0 && expected.2 > 0.0, "{expected:?}");

    let (mut doc, cascade, innermost) = chain(css, 12);
    assert!(laid_out(&mut doc, &cascade, Some(6)).0.is_err());
    laid_out(&mut doc, &cascade, None)
        .0
        .expect("layout after the failed one");
    assert_eq!(absolute_rect(&doc, innermost), expected);
}

#[test]
fn a_session_counts_its_nesting_from_zero() {
    let (mut doc, cascade, _) = chain("display:block", 5);
    let (result, _) = laid_out(&mut doc, &cascade, Some(2));
    assert!(result.is_err());
    let session = LayoutSession::with_cascade(&mut doc, &cascade);
    assert_eq!(session.context.recursion.depth, 0);
    assert_eq!(session.context.recursion.exceeded_at, None);
    assert_eq!(session.context.recursion.limit, MAX_LAYOUT_RECURSION);
}
