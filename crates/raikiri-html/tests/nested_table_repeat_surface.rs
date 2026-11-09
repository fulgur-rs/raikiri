//! A table nested in a block, in a single-line column flex container, or as
//! the only item of a single-line row flex container, repeats its header and
//! footer groups on every page it spans, however many pages that is (CSS 2
//! §17.2, CSS Fragmentation 3 §5.4).
//!
//! The expected boxes are those Chromium prints for the block and column
//! flex holders; the row flex holder lays out as the block one does.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// The border boxes of the rows and boxes with an `id`, page by page, for a
/// table of `rows` body rows in a holder styled `holder`, followed in the
/// holder by `extra`, on 5in x 3in pages with 0.5in margins.
fn pages(holder: &str, rows: usize, extra: &str) -> Vec<String> {
    let table = table(rows, true, "");
    laid_out(&format!(".holder{{{holder}}}"), &format!("{table}{extra}"))
}

/// A table of `rows` body rows, with a one-row header and footer when
/// `groups`, and `style` on the table element.
fn table(rows: usize, groups: bool, style: &str) -> String {
    let body: String = (1..=rows)
        .map(|row| format!("<tr id=r{row}><td>{row}</td></tr>"))
        .collect();
    let groups = if groups {
        "<thead><tr id=h><td>Header</td></tr></thead><tfoot><tr id=f><td>Footer</td></tr></tfoot>"
    } else {
        ""
    };
    format!("<table id=t style=\"{style}\">{groups}<tbody>{body}</tbody></table>")
}

/// The pages of [`pages`] for a holder with `content`, under the extra
/// style sheet `style`.
fn laid_out(style: &str, content: &str) -> Vec<String> {
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:5in 3in;margin:.5in}}\
         @page wide{{size:6in 3in}}body{{margin:0;font:10px/10px Ahem}}\
         .holder{{border:.25in solid black}}table{{border-spacing:0}}\
         .text,tr{{block-size:.25in}}td{{padding:0}}{style}</style></head>\
         <body><div id=before class=text>Before</div><div id=holder class=holder>{content}</div>\
         <div id=after class=text>After</div></body></html>"
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
        panic!("layout did not complete");
    };
    result
        .pages()
        .map(|page| {
            let dom = page.dom();
            let mut items: Vec<String> = page
                .fragments()
                .filter_map(|fragment| {
                    let id = dom.attr(fragment.node(), "id")?;
                    let r = fragment.rect();
                    Some(format!("{id}=({}, {}, {})", r.y, r.width, r.height))
                })
                .collect();
            items.sort();
            items.join(" ")
        })
        .collect()
}

#[test]
fn a_nested_table_longer_than_a_page_repeats_its_groups_on_each_page() {
    for holder in ["display:block", "display:flex"] {
        assert_eq!(
            pages(holder, 7, ""),
            [
                "before=(48, 384, 24) f=(216, 60, 24) h=(96, 60, 24) holder=(72, 384, 168) \
                 r1=(120, 60, 24) r2=(144, 60, 24) r3=(168, 60, 24) r4=(192, 60, 24) \
                 t=(96, 60, 144)",
                "after=(192, 384, 24) f=(144, 60, 24) h=(48, 60, 24) holder=(48, 384, 144) \
                 r5=(72, 60, 24) r6=(96, 60, 24) r7=(120, 60, 24) t=(48, 60, 120)",
            ],
            "{holder}"
        );
        // Over three pages, the middle page holds a full page of rows
        // between the copies.
        assert_eq!(
            pages(holder, 14, ""),
            [
                "before=(48, 384, 24) f=(216, 60, 24) h=(96, 60, 24) holder=(72, 384, 168) \
                 r1=(120, 60, 24) r2=(144, 60, 24) r3=(168, 60, 24) r4=(192, 60, 24) \
                 t=(96, 60, 144)",
                "f=(216, 60, 24) h=(48, 60, 24) holder=(48, 384, 192) r10=(192, 60, 24) \
                 r5=(72, 60, 24) r6=(96, 60, 24) r7=(120, 60, 24) r8=(144, 60, 24) \
                 r9=(168, 60, 24) t=(48, 60, 192)",
                "after=(216, 384, 24) f=(168, 60, 24) h=(48, 60, 24) holder=(48, 384, 168) \
                 r11=(72, 60, 24) r12=(96, 60, 24) r13=(120, 60, 24) r14=(144, 60, 24) \
                 t=(48, 60, 144)",
            ],
            "{holder}"
        );
    }
}

#[test]
fn a_table_in_a_column_flex_container_repeats_as_in_block_flow() {
    // The items before and after the table stack as blocks do, and Chromium
    // prints both holders alike.
    let column = |content: &str| laid_out(".holder{display:flex;flex-direction:column}", content);
    let block = |content: &str| laid_out(".holder{display:block}", content);
    for (pre, post) in [
        ("", ""),
        ("<div id=y class=text>Y</div>", ""),
        ("", "<div id=x class=text>X</div>"),
    ] {
        for rows in [7, 14] {
            // A full-width table is as wide in both holders.
            let content = format!("{pre}{}{post}", table(rows, true, "width:100%"));
            assert_eq!(column(&content), block(&content), "{pre}{post} {rows}");
        }
    }
    assert_eq!(
        column(&format!(
            "{}<div id=x class=text>X</div>",
            table(7, true, "")
        )),
        [
            "before=(48, 384, 24) f=(216, 336, 24) h=(96, 336, 24) holder=(72, 384, 168) \
             r1=(120, 336, 24) r2=(144, 336, 24) r3=(168, 336, 24) r4=(192, 336, 24) \
             t=(96, 336, 144)",
            "after=(216, 384, 24) f=(144, 336, 24) h=(48, 336, 24) holder=(48, 384, 168) \
             r5=(72, 336, 24) r6=(96, 336, 24) r7=(120, 336, 24) t=(48, 336, 120) \
             x=(168, 336, 24)",
        ]
    );
}

#[test]
fn a_table_beside_another_row_flex_item_keeps_the_flex_path() {
    // Items side by side fragment in parallel, so a header copy must not
    // push the other item down as block flow would. These pin the current
    // behavior, which places the groups once; Chromium repeats them.
    let pages = pages("display:flex", 7, "<div id=x>x</div>");
    assert_eq!(pages.len(), 2);
    assert!(
        pages[0].contains("h=(96, 60, 24)") && !pages[0].contains(" f="),
        "{}",
        pages[0]
    );
    assert!(
        pages[1].contains("f=(96, 60, 24)") && !pages[1].contains(" h="),
        "{}",
        pages[1]
    );
}

#[test]
fn a_sole_table_flex_item_fragments_as_in_block_flow() {
    let block = |content: &str| laid_out(".holder{display:block}", content);
    let flex = |content: &str| laid_out(".holder{display:flex}", content);
    // Without groups to repeat, the rows still break as in a block.
    let plain = table(9, false, "");
    assert_eq!(flex(&plain), block(&plain));
    // A `display: contents` wrapper generates no item of its own.
    let groups = table(7, true, "");
    assert_eq!(
        flex(&format!("<div style=display:contents>{groups}</div>")),
        block(&groups)
    );
    // A named page on the table starts the holder on that page, as Chromium
    // prints both holders.
    let named = table(3, true, "page:wide");
    assert_eq!(
        flex(&named),
        [
            "before=(48, 384, 24)",
            "f=(168, 60, 24) h=(72, 60, 24) holder=(48, 480, 168) r1=(96, 60, 24) \
             r2=(120, 60, 24) r3=(144, 60, 24) t=(72, 60, 120)",
            "after=(48, 384, 24)",
        ]
    );
    assert_eq!(flex(&named), block(&named));
}

#[test]
fn a_generated_item_beside_the_table_keeps_the_flex_path() {
    let pages = laid_out(
        ".holder{display:flex}.holder::before{content:'L'}",
        &table(7, true, ""),
    );
    assert!(!pages[0].contains(" f="), "{}", pages[0]);
    assert!(!pages[1].contains(" h="), "{}", pages[1]);
}
