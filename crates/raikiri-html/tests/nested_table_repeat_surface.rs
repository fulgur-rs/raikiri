//! A table nested in a block, or as the only item of a single-line row flex
//! container, repeats its header and footer groups on every page it spans,
//! however many pages that is (CSS 2 §17.2, CSS Fragmentation 3 §5.4).
//!
//! The expected boxes are those Chromium prints for the block holder; the
//! row flex holder lays out as the block one does.

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
    let body: String = (1..=rows)
        .map(|row| format!("<tr id=r{row}><td>{row}</td></tr>"))
        .collect();
    let html = format!(
        "<!DOCTYPE html><html><head><style>@page{{size:5in 3in;margin:.5in}}\
         body{{margin:0;font:10px/10px Ahem}}.holder{{border:.25in solid black;{holder}}}\
         table{{border-spacing:0}}.text,tr{{block-size:.25in}}td{{padding:0}}</style></head>\
         <body><div id=before class=text>Before</div><div id=holder class=holder>\
         <table id=t><thead><tr id=h><td>Header</td></tr></thead>\
         <tfoot><tr id=f><td>Footer</td></tr></tfoot><tbody>{body}</tbody></table>{extra}</div>\
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
fn other_flex_containers_keep_their_items_unrepeated() {
    // A column flex container, or a row one with another item, fragments its
    // items on its own path, which places the groups once. Chromium repeats
    // them there too; these pin the current behavior.
    for (holder, extra, width) in [
        ("display:flex;flex-direction:column", "", 336),
        ("display:flex", "<div id=x>x</div>", 60),
    ] {
        let pages = pages(holder, 7, extra);
        assert_eq!(pages.len(), 2, "{holder}");
        assert!(
            pages[0].contains(&format!("h=(96, {width}, 24)")) && !pages[0].contains(" f="),
            "{holder}: {}",
            pages[0]
        );
        assert!(
            pages[1].contains(&format!("f=(96, {width}, 24)")) && !pages[1].contains(" h="),
            "{holder}: {}",
            pages[1]
        );
    }
}
