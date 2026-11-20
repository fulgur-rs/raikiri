//! The first formatted line of a box whose `::before` or `::after` is
//! block-level can be one of that pseudo-element's lines, and its
//! `::first-letter` then styles the first letter of the generated text
//! (CSS Pseudo-Elements 4 §2.4). As in Chromium, an empty block-level
//! `::before` takes the first formatted line, so the text after it has no
//! first letter.
//!
//! The expected border boxes `[x, y, width, height]` of the elements with an
//! `id` are the ones Chromium computes for the same markup in an 800px by
//! 600px viewport with no body margin.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes `[x, y, width, height]` of the elements with an `id` on
/// the first page of an 800x600 document that holds `body`.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 0; font: 10px/10px Ahem }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new().fonts(fonts);
    let doc = parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let status = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(result) = status else {
        panic!("expected a complete layout");
    };
    let page = result.pages().next().expect("a page");
    let dom = page.dom();
    let mut boxes: Vec<(String, [f32; 4])> = Vec::new();
    for fragment in page.fragments() {
        if let Some(id) = dom.attr(fragment.node(), "id")
            && !boxes.iter().any(|(seen, _)| seen == id)
        {
            let rect = fragment.rect();
            boxes.push((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]));
        }
    }
    boxes
}

/// Markup and the border boxes Chromium gives the elements with an `id` in it.
type Row<'a> = (&'a str, &'a [(&'a str, [f32; 4])]);

fn assert_rows(rows: &[Row<'_>]) {
    let failures: Vec<_> = rows
        .iter()
        .filter_map(|(body, want)| {
            let got = boxes(body);
            let matches = want.iter().all(|(id, want)| {
                got.iter().any(|(got_id, got)| {
                    got_id == id
                        && got
                            .iter()
                            .zip(want)
                            .all(|(got, want)| (got - want).abs() <= 0.5)
                })
            });
            (!matches).then(|| format!("{body}\n  expected {want:?}, got {got:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

const LETTER: &str = "#s::first-letter{font-size:20px;line-height:40px}";

#[test]
fn the_first_letter_of_a_block_before_takes_its_style() {
    let markup = |before: &str, body: &str| {
        format!("<style>{LETTER} #s::before{{{before}}}</style><div id=s>{body}</div>")
    };
    let rows = [
        markup(r#"content:"xx";display:block"#, ""),
        markup(r#"content:"xxxxxxxx";display:block"#, "abc"),
        markup(r#"content:"xx yy zz";display:block;width:30px"#, "abc"),
        // The letter is the same as without the block: on an inline
        // `::before` it is styled already.
        markup(r#"content:"xx""#, "abc"),
    ];
    let wants: [&[(&str, [f32; 4])]; 4] = [
        &[("s", [0.0, 0.0, 800.0, 40.0])],
        &[("s", [0.0, 0.0, 800.0, 50.0])],
        &[("s", [0.0, 0.0, 800.0, 70.0])],
        &[("s", [0.0, 0.0, 800.0, 40.0])],
    ];
    let rows: Vec<Row<'_>> = rows.iter().map(String::as_str).zip(wants).collect();
    assert_rows(&rows);
}

#[test]
fn the_first_letter_sizes_a_shrink_to_fit_box() {
    assert_rows(&[
        (
            r#"<style>#s::first-letter{font-size:20px} #s::before{content:"xxxxxxxx";display:block;width:40px}</style><div id=s style="float:left">abc</div>"#,
            &[("s", [0.0, 0.0, 40.0, 23.0])],
        ),
        (
            r#"<style>#s::first-letter{font-size:20px} #s::before{content:"xx";display:block}</style><div id=s style="position:absolute"></div>"#,
            &[("s", [0.0, 0.0, 30.0, 13.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:"xx";display:block}}</style><div id=s style="float:left">abc</div>"#
            ),
            &[("s", [0.0, 0.0, 30.0, 50.0])],
        ),
    ]);
}

#[test]
fn a_wrapped_first_letter_keeps_its_style_in_a_block_before() {
    // As in Chromium, the part of the letter's unit that wraps to the
    // block's second line still takes the letter's line height.
    assert_rows(&[(
        &format!(
            r#"<style>{LETTER} #s::before{{content:"! A";display:block;width:20px}}</style><div id=s style="position:absolute">abc</div>"#
        ),
        &[("s", [0.0, 0.0, 30.0, 90.0])],
    )]);
}

#[test]
fn a_block_after_holds_the_first_letter_when_nothing_comes_before_it() {
    assert_rows(&[
        (
            &format!(
                r#"<style>{LETTER} #s::after{{content:"xx";display:block}}</style><div id=s></div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 40.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::after{{content:"xx";display:block}}</style><div id=s>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 50.0])],
        ),
    ]);
}

#[test]
fn an_empty_block_before_leaves_no_first_letter() {
    assert_rows(&[
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:"";display:block}}</style><div id=s>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 10.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:"";display:block;height:20px}}</style><div id=s>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 30.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:"";display:block;padding:5px}}</style><div id=s>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 20.0])],
        ),
    ]);
}

#[test]
fn a_block_before_of_white_space_leaves_no_first_letter() {
    assert_rows(&[
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:" ";display:block}}</style><div id=s>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 10.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:" ";display:block;white-space:pre}}</style><div id=s>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 20.0])],
        ),
    ]);
}

#[test]
fn content_before_a_block_after_keeps_the_first_letter_from_it() {
    assert_rows(&[
        (
            &format!(
                r#"<style>{LETTER} #s::after{{content:"xx";display:block}}</style><div id=s> </div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 40.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::after{{content:"xx";display:block}}</style><div id=s> <!-- c --> </div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 40.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::after{{content:"xx";display:block}}</style><div id=s><span></span></div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 10.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} #s::after{{content:"xx";display:block}}</style><div id=s><span style="padding-left:5px"></span></div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 20.0])],
        ),
    ]);
}

#[test]
fn a_block_before_after_an_empty_inline_still_holds_the_first_letter() {
    assert_rows(&[
        (
            &format!(
                r#"<style>{LETTER} #s::before{{content:"xx";display:block}}</style><div id=s><span style="padding-left:5px"></span>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 50.0])],
        ),
        (
            &format!(
                r#"<style>{LETTER} .c::before{{content:"xx";display:block}}</style><div id=s><span style="display:contents" class=c></span>abc</div>"#
            ),
            &[("s", [0.0, 0.0, 800.0, 50.0])],
        ),
    ]);
}
