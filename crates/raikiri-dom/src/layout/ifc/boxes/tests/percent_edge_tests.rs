//! Percentage margins and padding of inline boxes, resolved against the
//! inline size of the containing block at line layout and taken as zero for
//! intrinsic sizing.

use crate::Document;
use crate::layout::ifc::projection::project_ifc;
use crate::layout::ifc::root::IfcRoot;
use crate::layout::ifc::test_support::{
    ahem_fonts, block_fixture, block_fixture_with_font_collection, span,
};
use crate::layout::layout_single_page;
use crate::layout::test_support::{ahem_paragraph_with, page_box_800x600, with_ahem};
use raikiri_traits::PageBox;
use shodo::{Fragment, LayoutContext};

/// A paragraph `A<span style=span_css>B</span>C` in a root styled by
/// `root_css`, laid out on an 800x600 page. Returns the document, the root
/// and the text node `C`.
fn padded(root_css: &str, span_css: &str) -> (Document, usize, usize) {
    let mut tail = 0;
    let fixture = block_fixture_with_font_collection(
        &format!("line-height:10px;{root_css}"),
        |doc, root| {
            doc.append_text(root, "A");
            let inner = span(doc, root, &format!("display:inline;{span_css}"));
            doc.append_text(inner, "B");
            tail = doc.append_text(root, "C");
        },
        ahem_fonts(),
    );
    let mut doc = fixture.doc;
    let cascade = fixture.cascade;
    let root = fixture.root;
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    (doc, root, tail)
}

/// The line-local inline offset of the first glyph of text node `node`, with
/// the index of its line.
fn text_start(doc: &Document, root: usize, node: usize) -> (usize, f32) {
    let lines = doc.nodes[root].ifc_lines().expect("lines");
    lines
        .iter()
        .enumerate()
        .find_map(|(index, line)| {
            line.fragments().find_map(|fragment| match fragment {
                Fragment::GlyphRun(run) if run.node().map(|id| id.0) == Some(node as u64) => {
                    run.glyph_origin(0).map(|(inline, _)| (index, inline))
                }
                _ => None,
            })
        })
        .expect("text run")
}

/// How far a span styled by `span_css` moves the text after it, against the
/// same paragraph with an unstyled span.
fn shift(root_css: &str, span_css: &str) -> f32 {
    let (doc, root, tail) = padded(root_css, span_css);
    let (plain_doc, plain_root, plain_tail) = padded(root_css, "");
    text_start(&doc, root, tail).1 - text_start(&plain_doc, plain_root, plain_tail).1
}

#[test]
fn a_percentage_padding_resolves_against_the_root_width() {
    // CSS Box 4 §4.2: padding percentages refer to the inline size of the
    // containing block, 10% of 300px here.
    assert_eq!(shift("width:300px", "padding-left:10%"), 30.0);
    assert_eq!(shift("width:300px", "padding-right:10%"), 30.0);
}

#[test]
fn a_percentage_margin_resolves_against_the_root_width() {
    assert_eq!(shift("width:300px", "margin-left:10%"), 30.0);
    assert_eq!(shift("width:300px", "margin-left:-5%"), -15.0);
}

#[test]
fn a_calc_edge_adds_its_absolute_part_to_the_percentage() {
    assert_eq!(shift("width:300px", "padding-left:calc(10% + 4px)"), 34.0);
    assert_eq!(shift("width:300px", "margin-right:calc(10% - 4px)"), 26.0);
    // A negative sum of padding is zero.
    assert_eq!(shift("width:300px", "padding-left:calc(1% - 10px)"), 0.0);
}

#[test]
fn a_percentage_edge_in_ch_calc_resolves_both_parts() {
    // `1ch` is one Ahem em, 10px.
    assert_eq!(shift("width:300px", "padding-left:calc(1ch + 10%)"), 40.0);
}

#[test]
fn a_vertical_root_resolves_against_its_inline_size() {
    // CSS Writing Modes 4 §7.2: the containing block's inline size is its
    // physical height in vertical writing modes.
    assert_eq!(
        shift("writing-mode:vertical-rl;height:200px", "padding-top:10%"),
        20.0
    );
}

#[test]
fn a_shrink_to_fit_root_takes_percentages_as_zero_for_its_size() {
    // CSS Sizing 3 §5.2.1: a cyclic percentage contributes zero to the
    // intrinsic size, then resolves against the size it produced.
    for css in ["float:left", "display:inline-block", "position:absolute"] {
        let (doc, root, tail) = padded(css, "padding-left:50%");
        assert_eq!(
            doc.nodes[root].unrounded_layout.size.width, 30.0,
            "{css} is sized without the padding"
        );
        assert_eq!(text_start(&doc, root, tail), (0, 35.0), "{css}");
    }
}

#[test]
fn a_root_without_percentage_edges_breaks_from_its_own_paragraph() {
    let (doc, root, _) = padded("width:300px", "padding-left:4px");
    let ifc = doc.nodes[root].ifc.as_ref().expect("root");
    assert!(!ifc.paragraph.has_percent_edges());
    assert_eq!(ifc.line_paragraph.id(), ifc.paragraph.id());
}

fn projected_root(css: &str) -> (IfcRoot, LayoutContext) {
    let fixture = block_fixture(&format!("line-height:10px;{css}"), |doc, root| {
        doc.append_text(root, "A");
        let inner = span(doc, root, "display:inline;padding-left:10%");
        doc.append_text(inner, "B");
    });
    let mut cx = LayoutContext::new();
    let projected = project_ifc(
        &fixture.doc,
        &fixture.cascade,
        fixture.root,
        &mut cx,
        &ahem_fonts(),
        &shodo::limits::Limits::default(),
    )
    .expect("project");
    (IfcRoot::new(projected), cx)
}

#[test]
fn the_paragraph_is_rebuilt_only_when_the_basis_changes() {
    let fonts = ahem_fonts();
    let (mut root, mut cx) = projected_root("");
    let base = root.paragraph.id();
    assert!(root.paragraph.has_percent_edges());
    assert_eq!(root.line_paragraph.id(), base);

    root.resolve_percent_edges(300.0, &mut cx, &fonts);
    let resolved = root.line_paragraph.id();
    assert_ne!(resolved, base);
    assert_eq!(root.line_paragraph.edge_basis(), 300.0);
    // The base paragraph keeps its percentages at zero for intrinsic sizing.
    assert_eq!(root.paragraph.id(), base);
    assert_eq!(root.paragraph.edge_basis(), 0.0);

    root.resolve_percent_edges(300.0, &mut cx, &fonts);
    assert_eq!(root.line_paragraph.id(), resolved, "same width, no rebuild");
    let copy = root.without_lines();
    assert_eq!(copy.line_paragraph.id(), resolved);
    assert_eq!(copy.edge_basis, 300.0);

    root.resolve_percent_edges(200.0, &mut cx, &fonts);
    let narrower = root.line_paragraph.id();
    assert_ne!(narrower, resolved);
    assert_eq!(root.line_paragraph.edge_basis(), 200.0);

    // A width probed before is not shaped again.
    root.resolve_percent_edges(300.0, &mut cx, &fonts);
    assert_eq!(root.line_paragraph.id(), resolved, "earlier basis reused");
    root.resolve_percent_edges(200.0, &mut cx, &fonts);
    assert_eq!(root.line_paragraph.id(), narrower, "earlier basis reused");

    // A zero, negative or non-finite basis is the base paragraph.
    for basis in [0.0, -1.0, f32::NAN] {
        root.resolve_percent_edges(200.0, &mut cx, &fonts);
        root.resolve_percent_edges(basis, &mut cx, &fonts);
        assert_eq!(root.line_paragraph.id(), base, "{basis}");
    }
}

#[test]
fn columns_resolve_against_the_column_width() {
    // CSS Multicol 1 §2: the column box is the containing block of the
    // content, so 10% of a 150px column.
    assert_eq!(
        shift("width:300px;columns:2;column-gap:0", "padding-left:10%"),
        15.0
    );
}

#[test]
fn a_paginated_paragraph_keeps_its_percentage_edges_on_later_pages() {
    // Twenty lines of `A` come first, so the span starts line 20, past the
    // first 50px page.
    const PADDED_LINE: usize = 20;
    let mut tail = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with(
        "width:300px;white-space:pre;orphans:1;widows:1",
        |doc, root| {
            doc.append_text(root, "A\n".repeat(PADDED_LINE));
            let inner = span(doc, root, "display:inline;padding-left:10%");
            doc.append_text(inner, "B");
            tail = doc.append_text(root, "C");
        },
    );
    let mut page = PageBox::new();
    page.width = 400.0;
    page.height = 50.0;
    let pages = crate::layout::layout_pages(with_ahem(&mut doc), &cascade, page).expect("pages");
    assert!(pages.len() > 1, "the paragraph spans pages");
    assert_eq!(text_start(&doc, root, tail), (PADDED_LINE, 40.0));
}

#[test]
fn text_broken_again_for_a_width_resolves_against_that_width() {
    let (mut doc, cascade, root) = ahem_paragraph_with("", |doc, root| {
        doc.append_text(root, "A");
        let inner = span(doc, root, "display:inline;padding-left:10%");
        doc.append_text(inner, "B");
    });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let inner = doc.nodes[root].children[1];
    let text = doc.nodes[inner].children[0];
    assert_eq!(text_start(&doc, root, text).1, 90.0, "10% of 800px");
    crate::layout::relayout_text_for_width(&mut doc, &cascade, 400.0);
    assert_eq!(text_start(&doc, root, text).1, 50.0, "10% of 400px");
}

/// The border-box width of a span styled by `span_css` inside a ruby
/// annotation of a `width:300px` root.
fn annotation_span_width(span_css: &str) -> f32 {
    let mut target = 0;
    let (mut doc, cascade, root) = ahem_paragraph_with("width:300px", |doc, root| {
        let ruby = span(doc, root, "display:ruby");
        let base = span(doc, ruby, "display:ruby-base");
        doc.append_text(base, "AAAAAAAAAA");
        let annotation = span(doc, ruby, "display:ruby-text;font-size:5px");
        doc.append_text(annotation, "a");
        target = span(doc, annotation, &format!("display:inline;{span_css}"));
        doc.append_text(target, "b");
    });
    layout_single_page(with_ahem(&mut doc), &cascade, page_box_800x600()).expect("layout");
    let pieces = doc.nodes[root].ifc_inline_boxes().expect("pieces");
    pieces
        .iter()
        .find(|piece| piece.node == target)
        .expect("annotation piece")
        .border_box
        .width
}

#[test]
fn a_percentage_edge_inside_a_ruby_annotation_resolves_against_the_root_width() {
    // Annotation boxes take percentages from the same containing block as
    // the paragraph, 10% of 300px here, not from the annotation's own size.
    // Ruby alignment spreads the annotation over its base, so compare with
    // the same edge in px rather than with a fixed offset.
    let plain = annotation_span_width("");
    let percent = annotation_span_width("padding-left:10%");
    assert!(percent > plain, "{percent} > {plain}");
    assert_eq!(percent, annotation_span_width("padding-left:30px"));
    assert_eq!(
        annotation_span_width("padding-right:calc(10% + 2px)"),
        annotation_span_width("padding-right:32px")
    );
}
