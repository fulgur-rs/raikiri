//! The depth of `open-quote` and `close-quote` counts through the generated
//! text of the whole document, in document order (CSS Content 3 §2.4.2), so a
//! quote inside another one takes the next pair of `quotes`.
//!
//! The expected texts are the ones Chromium prints for the same markup.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Pairs for the first four levels, so every depth shows which pair it took:
/// `<` `>` at the outermost level, `[` `]` below it, and the last pair for
/// every deeper level.
const PAIRS: &str = "quotes:'<' '>' '[' ']'";

/// The text of the first page of `body` under `css`, in reading order.
fn text_of(body: &str, css: &str) -> String {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!("../../raikiri-dom/tests/data/text-autospace/Ahem.ttf").to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let html = format!(
        "<!doctype html><style>@page{{size:800px 600px;margin:0}}\
         body{{margin:0;font:16px/16px Ahem}}\
         q::before,blockquote::before{{content:open-quote}}\
         q::after,blockquote::after{{content:close-quote}}\
         {css}</style><body>{body}"
    );
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    let LayoutStatus::Completed(result) = layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap() else {
        panic!("completed layout")
    };
    let mut runs = result.page(0).unwrap().text_runs();
    // The runs follow the arena order of their paragraphs; the page reads
    // line by line and left to right.
    runs.sort_by(|a, b| {
        a.origin
            .1
            .total_cmp(&b.origin.1)
            .then(a.origin.0.total_cmp(&b.origin.0))
    });
    runs.iter().map(|run| run.text).collect()
}

/// `body` with the explicit pairs of [`PAIRS`] on its container.
fn paired(body: &str, css: &str) -> String {
    text_of(
        &format!("<div class=qq>{body}</div>"),
        &format!(".qq{{{PAIRS}}}{css}"),
    )
}

#[test]
fn a_nested_q_takes_the_inner_pair() {
    assert_eq!(paired("<q>a<q>b</q></q>", ""), "<a[b]>");
    assert_eq!(
        paired("<q>a<q>b<q>c<q>d</q></q></q></q>", ""),
        "<a[b[c[d]]]>"
    );
}

#[test]
fn a_nested_q_takes_the_inner_auto_marks() {
    assert_eq!(
        text_of("<q>a <q>b <q>c</q></q></q>", ""),
        "\u{201c}a \u{2018}b \u{2018}c\u{2019}\u{2019}\u{201d}"
    );
}

#[test]
fn nested_blockquotes_take_the_inner_pair() {
    assert_eq!(
        paired("<blockquote>a<blockquote>b</blockquote></blockquote>", ""),
        "<a[b]>"
    );
}

#[test]
fn sibling_quotes_share_a_depth() {
    assert_eq!(paired("<q>a<q>b</q>c<q>d</q></q>", ""), "<a[b]c[d]>");
    assert_eq!(paired("<q>a</q><q>b</q>", ""), "<a><b>");
}

#[test]
fn an_unclosed_quote_deepens_the_quotes_after_it() {
    let css = ".o::before{content:open-quote}";
    assert_eq!(
        paired("<p class=o>x</p><p class=o>y</p><q>z</q>", css),
        "<x[y[z]"
    );
}

#[test]
fn an_after_comes_after_the_children_of_its_element() {
    let css = ".ao::after{content:open-quote}";
    assert_eq!(paired("<p class=ao>a<q>b</q></p><q>c</q>", css), "a<b><[c]");
}

#[test]
fn a_close_quote_with_nothing_open_inserts_nothing() {
    let css = ".c::after{content:close-quote}.o::before{content:open-quote}";
    assert_eq!(paired("<p class=c>x</p><q>y</q>", css), "x<y>");
    assert_eq!(
        paired("<p class=c>x</p><p class=o>y</p><p class=o>z</p>", css),
        "x<y[z"
    );
    // The second close finds nothing open, and stays at depth zero.
    let two = ".cc::after{content:close-quote close-quote}";
    assert_eq!(
        paired(
            "<p class=o>a<span class=cc>b</span>c</p><q>d</q>",
            &format!("{css}{two}")
        ),
        "<ab>c<d>"
    );
}

#[test]
fn no_open_quote_and_no_close_quote_still_move_the_depth() {
    let css = ".no::before{content:no-open-quote}.nc::after{content:no-close-quote}";
    assert_eq!(
        paired("<p class=no>x</p><q>y</q><p class=nc>z</p><q>w</q>", css),
        "x[y]z<w>"
    );
}

#[test]
fn a_content_that_opens_and_closes_leaves_the_depth_alone() {
    let css = ".oc::before{content:open-quote 'x' close-quote}";
    assert_eq!(paired("<p class=oc>a</p><q>w</q>", css), "<x>a<w>");
}

#[test]
fn quotes_none_still_counts_the_depth() {
    assert_eq!(
        paired("<q>a<q style='quotes:none'>b</q><q>c</q></q>", ""),
        "<ab[c]>"
    );
}

#[test]
fn hidden_quotes_do_not_count() {
    assert_eq!(
        paired("<div style='display:none'><q>z</q></div><q>w</q>", ""),
        "<w>"
    );
    let css = ".h::before{content:open-quote;display:none}";
    assert_eq!(paired("<p class=h>x</p><q>w</q>", css), "x<w>");
}

#[test]
fn a_replaced_element_and_a_line_break_have_no_pseudo_elements_to_count() {
    // Layout draws no generated content for an image or a line break.
    let css = "img::before,img::after,br::before,br::after{content:open-quote}";
    assert_eq!(paired("<q>a<img width=4 height=4><br></q>", css), "<a>");
}

#[test]
fn an_element_that_layout_leaves_out_adds_no_quotes_even_when_displayed() {
    // `template` and `noscript` are skipped with their subtrees whatever
    // their `display` says.
    let css = "template,noscript{display:block}\
               template::before,.o::before{content:open-quote}";
    assert_eq!(paired("<template></template><q>a</q>", css), "<a>");
    assert_eq!(
        paired("<noscript><p class=o>x</p></noscript><q>a</q>", css),
        "<a>"
    );
}

#[test]
fn the_elements_of_an_inline_svg_add_no_quotes() {
    // The vector path draws an inline SVG as a unit, with no generated
    // content on its elements.
    let css = ".o::before{content:open-quote}";
    assert_eq!(
        paired("<svg width=4 height=4><g class=o></g></svg><q>x</q>", css),
        "<x>"
    );
}

#[test]
fn the_pseudo_elements_of_a_display_contents_element_count() {
    assert_eq!(
        paired("<div style='display:contents'><q>a<q>b</q></q></div>", ""),
        "<a[b]>"
    );
}

#[test]
fn a_list_marker_counts_before_the_content_of_its_item() {
    let css = "li{list-style-position:inside}li::marker{content:open-quote}";
    assert_eq!(
        paired("<ul><li>a</li><li>b</li></ul><q>w</q>", css),
        "<a[b[w]"
    );
    assert_eq!(paired("<q>p<ul><li>a</li></ul></q>", css), "<p[a]");
    // An outside marker is painted apart from its item and counts the same.
    let outside = "li::marker{content:open-quote}";
    assert_eq!(
        paired("<ul><li>a</li><li>b</li></ul><q>w</q>", outside),
        "<a[b[w]"
    );
}
