use crate::ifc_text::IfcPosition;
use crate::text::DecorationContext;
use anyrender::Scene;
use kurbo::Affine;
use raikiri_html::{
    DocumentLayout, FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{LayoutConfig, PageDefaults};
use std::cell::Cell;

/// What the preparations of a paragraph do.
#[derive(Clone, Copy)]
pub(super) enum Work {
    /// The pieces of a paragraph's lines are made.
    Pieces,
    /// The shape of the groups of a paragraph's lines is made.
    Shapes,
    /// A line is read by making the pieces.
    Lines,
    /// The content of a paragraph is searched for a negative stack level.
    NegativeWalks,
}

thread_local! {
    static WORK: Cell<[usize; 4]> = const { Cell::new([0; 4]) };
    static UNSHARED: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn note_work(kind: Work, amount: usize) {
    WORK.with(|counts| {
        let mut now = counts.get();
        now[kind as usize] += amount;
        counts.set(now);
    });
}

pub(super) fn sharing_is_off() -> bool {
    UNSHARED.with(Cell::get)
}

/// The work done since the last call, in the order of [`Work`].
fn take_work() -> [usize; 4] {
    WORK.with(|counts| counts.replace([0; 4]))
}

/// Makes every paint prepare each part of a paragraph on its own, as paints
/// did before the parts shared their preparations.
struct Unshared;

impl Unshared {
    fn new() -> Self {
        UNSHARED.with(|off| off.set(true));
        Self
    }
}

impl Drop for Unshared {
    fn drop(&mut self) {
        UNSHARED.with(|off| off.set(false));
    }
}

fn laid_out(html: &str) -> DocumentLayout {
    let fonts = FontCollectionBuilder::new()
        .font_bytes(
            "Ahem",
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
            ))
            .to_vec(),
        )
        .build()
        .unwrap();
    let resources = RenderResources::new().fonts(fonts);
    let parsed = parse_html_with_resources(html.as_bytes(), &resources).unwrap();
    match layout(
        &parsed,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .unwrap()
    {
        LayoutStatus::Completed(document) => document,
        _ => panic!("the layout completes"),
    }
}

/// The commands of every page of `document`, as text.
fn painted_pages(document: &DocumentLayout) -> Vec<String> {
    let pages = document.page_count();
    (0..pages)
        .map(|index| {
            let (doc, cascade, page_box, origin) = document.page(index).unwrap().paint_inputs();
            let mut scene = Scene::new();
            crate::paint_single_page_with_origin_and_page(
                &mut scene,
                doc,
                cascade,
                page_box,
                origin,
                index,
                pages,
                index % 2 == 1,
            )
            .unwrap();
            format!("{:?}", scene.commands)
        })
        .collect()
}

const DECORATED: &str = "background:#ddd;padding:0 1px;border:1px solid #888;border-radius:2px";

fn spans(count: usize, style: &str) -> String {
    (0..count)
        .map(|_| format!("<span style=\"{style}\">ab cd </span>"))
        .collect()
}

fn page(css: &str, size: &str, body: &str) -> String {
    format!(
        "<!doctype html><style>@page{{size:{size};margin:5px}}body{{margin:0;font:10px/10px Ahem}}p{{margin:0}}{css}</style>{body}"
    )
}

/// `count` decorated inline elements in a paragraph that `columns` balanced
/// columns share, on one page.
fn in_columns(columns: usize, count: usize) -> String {
    page(
        &format!(
            ".mc{{width:{}px;column-count:{columns};column-gap:0}}",
            columns * 100
        ),
        &format!("{}px 600px", columns * 100 + 10),
        &format!("<div class=mc><p>{}</p></div>", spans(count, DECORATED)),
    )
}

#[test]
fn a_paragraph_in_columns_is_prepared_once_however_many_columns_hold_it() {
    for columns in [1, 2, 4, 8] {
        // The same paragraph in more columns, and a paragraph that grows by
        // the same amount for each column.
        for count in [48, 12 * columns] {
            let document = laid_out(&in_columns(columns, count));
            assert_eq!(document.page_count(), 1, "{columns} columns of {count}");
            let _ = take_work();
            let shared = painted_pages(&document);
            let [pieces, shapes, lines, walks] = take_work();
            assert_eq!(
                (pieces, shapes, walks),
                (1, 1, 1),
                "{columns} columns of {count}"
            );
            assert!(lines > 0, "{columns} columns of {count}");
            let unshared = {
                let _unshared = Unshared::new();
                painted_pages(&document)
            };
            // Prepared for each column, every line is read once for each.
            assert_eq!(
                take_work(),
                [columns, columns, columns * lines, columns],
                "{columns} columns of {count}"
            );
            assert_eq!(shared, unshared, "{columns} columns of {count}");
        }
    }
}

#[test]
fn a_paragraph_in_columns_paints_as_it_does_when_each_column_prepares_it() {
    let box_decoration_clone = format!("{DECORATED};box-decoration-break:clone");
    let flows = [
        // One page holds every column.
        (
            "fit",
            ".mc{width:300px;height:60px;column-count:3;column-gap:0;column-fill:auto}",
            "320px 80px",
            17,
        ),
        // The columns continue on the pages that follow.
        (
            "flow",
            ".mc{width:300px;column-count:3;column-gap:0;column-fill:auto}",
            "320px 55px",
            40,
        ),
    ];
    for (flow, mc, size, count) in flows {
        let fixtures = [
            ("sliced boxes", mc.to_string(), spans(count, DECORATED)),
            ("cloned boxes", mc.to_string(), spans(count, &box_decoration_clone)),
            (
                "two paragraphs",
                mc.to_string(),
                format!(
                    "{}</p><p>{}",
                    spans(count * 3 / 4, DECORATED),
                    spans(count * 3 / 4, "background:#fc0;padding:0 3px")
                ),
            ),
            (
                "right to left",
                format!("{mc}.mc{{direction:rtl}}"),
                spans(count, DECORATED),
            ),
            (
                "translucent boxes",
                mc.to_string(),
                format!(
                    "<span style=\"opacity:.5;background:#c00\">{0}</span> tail <span style=\"opacity:.7\">{0}<span style=\"opacity:.5;background:#0c0\">{0}</span></span>",
                    "ab cd ".repeat(count / 3)
                ),
            ),
            (
                "atomic boxes in translucent boxes",
                mc.to_string(),
                (0..count / 2)
                    .map(|i| {
                        format!(
                            "<span style=\"opacity:.6\">ab cd <span style=\"display:inline-block;width:20px;height:10px;background:#00c;{}\">x</span></span>",
                            if i % 3 == 0 { "position:relative;z-index:1;" } else { "" }
                        )
                    })
                    .collect(),
            ),
            (
                "translucent first line and first letter",
                format!("{mc}p::first-line{{opacity:.5;color:#800}}p::first-letter{{opacity:.6;font-size:20px}}"),
                spans(count, DECORATED),
            ),
            (
                "generated boxes",
                format!("{mc}span::before{{content:'x';opacity:.5;display:inline-block;transform:rotate(10deg);background:#f90}}span::after{{content:'y';opacity:.4;background:#09f}}"),
                spans(count / 2, DECORATED),
            ),
            (
                "generated blocks",
                format!("{mc}p::before{{content:'';display:block;height:10px;background:#f00}}p::after{{content:'';display:block;height:10px;background:#00f}}"),
                spans(count, DECORATED),
            ),
            (
                "a box below the blocks",
                mc.to_string(),
                format!(
                    "{}<span style=\"position:relative;z-index:-1;background:#0c0\">below</span>",
                    spans(count, DECORATED)
                ),
            ),
            (
                "relative boxes with shadows",
                mc.to_string(),
                spans(
                    count,
                    &format!("{DECORATED};position:relative;left:2px;top:1px;text-shadow:1px 1px #888"),
                ),
            ),
        ];
        for (name, css, inline) in fixtures {
            let document = laid_out(&page(
                &css,
                size,
                &format!("<div class=mc><p>{inline}</p></div>"),
            ));
            let _ = take_work();
            let shared = painted_pages(&document);
            let shared_work = take_work();
            let unshared = {
                let _unshared = Unshared::new();
                painted_pages(&document)
            };
            let unshared_work = take_work();
            assert!(
                shared.iter().any(|page| page.len() > 400),
                "{flow} {name}: nothing is painted"
            );
            assert_eq!(shared, unshared, "{flow} {name}");
            // Where the columns of a page hold lines of the paragraph, they
            // share one preparation. A paragraph with atomic boxes, and one
            // with generated blocks that continues on later pages, has a
            // single column on each page.
            let split = name != "atomic boxes in translucent boxes"
                && !(flow == "flow" && name == "generated blocks");
            let (shared_pieces, unshared_pieces) = (
                shared_work[Work::Pieces as usize],
                unshared_work[Work::Pieces as usize],
            );
            if split {
                assert!(shared_pieces < unshared_pieces, "{flow} {name}");
            } else {
                assert_eq!(shared_pieces, unshared_pieces, "{flow} {name}");
            }
        }
    }
}

#[test]
fn a_vertical_paragraph_paints_as_it_does_when_each_column_prepares_it() {
    let document = laid_out(&page(
        ".mc{writing-mode:vertical-rl;width:80px;height:300px;column-count:3;column-gap:0;column-fill:auto}",
        "100px 320px",
        &format!("<div class=mc><p>{}</p></div>", spans(20, DECORATED)),
    ));
    let shared = painted_pages(&document);
    let unshared = {
        let _unshared = Unshared::new();
        painted_pages(&document)
    };
    assert!(shared.iter().any(|page| page.len() > 400));
    assert_eq!(shared, unshared);
}

#[test]
fn a_paragraph_painted_whole_is_not_kept() {
    let document = laid_out(&page(
        "",
        "120px 600px",
        &format!("<p>{}</p>", spans(30, DECORATED)),
    ));
    let (doc, cascade, page_box, _) = document.page(0).unwrap().paint_inputs();
    let cache = crate::walk::ImageBrushCache::for_document(doc);
    let root = paragraph_roots(doc)[0];
    let _ = take_work();
    for _ in 0..2 {
        draw(&cache, doc, cascade, page_box, root, None);
    }
    assert_eq!(take_work()[Work::Pieces as usize], 2);
    assert!(cache.paragraphs().entries.borrow().is_empty());
}

fn draw(
    cache: &crate::walk::ImageBrushCache,
    doc: &raikiri_dom::Document,
    cascade: &raikiri_style::CascadeResult,
    page_box: raikiri_traits::PageBox,
    root: usize,
    fragmentainer: Option<usize>,
) -> String {
    let mut scene = Scene::new();
    crate::ifc_text::draw_ifc_lines_with_resources(
        cache,
        &mut scene,
        doc,
        cascade,
        root,
        IfcPosition {
            x: 0.0,
            y: 0.0,
            shift_y: 0.0,
        },
        &DecorationContext::default(),
        fragmentainer,
        &[],
        None,
        &mut Vec::new(),
        page_box,
        None,
        Affine::IDENTITY,
        None,
    );
    format!("{:?}", scene.commands)
}

/// The paragraphs of `doc`, in order.
fn paragraph_roots(doc: &raikiri_dom::Document) -> Vec<usize> {
    (0..doc.node_count())
        .filter(|&id| doc.get_node(id).is_some_and(|node| node.is_ifc_root()))
        .collect()
}

/// The columns that hold lines of the paragraph `root`.
fn columns_of(doc: &raikiri_dom::Document, root: usize) -> Vec<usize> {
    let mut columns: Vec<_> = doc
        .get_node(root)
        .and_then(|node| node.ifc_multicol_fragments())
        .expect("columns")
        .iter()
        .map(|fragment| fragment.fragmentainer)
        .collect();
    columns.sort_unstable();
    columns.dedup();
    columns
}

#[test]
fn paragraphs_painted_by_turns_keep_their_own_preparations() {
    // Each paragraph fills a container of its own, so the paints of their
    // columns can be told apart whatever order they come in.
    let container = |decoration: &str| {
        format!(
            "<div class=mc><p>{}</p></div>",
            spans(20, &format!("background:{decoration};padding:0 1px"))
        )
    };
    let document = laid_out(&page(
        ".mc{width:200px;column-count:2;column-gap:0}",
        "210px 600px",
        &format!("{}{}", container("#c00"), container("#00c")),
    ));
    let (doc, cascade, page_box, _) = document.page(0).unwrap().paint_inputs();
    let roots = paragraph_roots(doc);
    assert_eq!(roots.len(), 2);
    let alone = |root: usize, column: usize| {
        draw(
            &crate::walk::ImageBrushCache::for_document(doc),
            doc,
            cascade,
            page_box,
            root,
            Some(column),
        )
    };
    let cache = crate::walk::ImageBrushCache::for_document(doc);
    let (first, second) = (columns_of(doc, roots[0]), columns_of(doc, roots[1]));
    assert_eq!((first.len(), second.len()), (2, 2));
    assert_ne!(alone(roots[0], first[0]), alone(roots[1], second[0]));
    for (&a, &b) in first.iter().zip(&second) {
        for (root, column) in [(roots[0], a), (roots[1], b)] {
            assert_eq!(
                draw(&cache, doc, cascade, page_box, root, Some(column)),
                alone(root, column)
            );
        }
    }
}

#[test]
fn the_negative_stack_level_of_a_paragraph_is_searched_for_once_for_all_its_columns() {
    let document = laid_out(&page(
        ".mc{width:200px;column-count:2;column-gap:0}",
        "210px 600px",
        &format!(
            "<div class=mc><p>{}<span style=\"position:relative;z-index:-1\">below</span></p></div><div class=mc><p>{}</p></div>",
            spans(20, DECORATED),
            spans(20, DECORATED)
        ),
    ));
    let (doc, cascade, _, _) = document.page(0).unwrap().paint_inputs();
    let roots = paragraph_roots(doc);
    let cache = crate::walk::ImageBrushCache::for_document(doc);
    let answers: Vec<_> = roots
        .iter()
        .map(|&root| raikiri_dom::paint_rules::holds_negative_stack_level(doc, cascade, root))
        .collect();
    assert_eq!(answers, [true, false]);
    let _ = take_work();
    for _ in 0..3 {
        for (&root, &answer) in roots.iter().zip(&answers) {
            let asked = cache
                .paragraphs()
                .holds_negative_stack_level(doc, cascade, root, true);
            assert_eq!(asked, answer);
        }
    }
    assert_eq!(take_work()[Work::NegativeWalks as usize], 2);
    // A paragraph painted whole is asked about once, and searched each time.
    for _ in 0..3 {
        assert!(
            cache
                .paragraphs()
                .holds_negative_stack_level(doc, cascade, roots[0], false)
        );
    }
    assert_eq!(take_work()[Work::NegativeWalks as usize], 3);
}

#[test]
fn a_paragraph_stays_prepared_until_its_last_column_is_painted() {
    let document = laid_out(&in_columns(4, 48));
    let (doc, cascade, page_box, _) = document.page(0).unwrap().paint_inputs();
    let cache = crate::walk::ImageBrushCache::for_document(doc);
    let root = paragraph_roots(doc)[0];
    let columns = columns_of(doc, root);
    assert_eq!(columns.len(), 4);
    let _ = take_work();
    for (painted, &column) in columns.iter().enumerate() {
        draw(&cache, doc, cascade, page_box, root, Some(column));
        let kept = cache.paragraphs().entries.borrow().len();
        assert_eq!(kept, usize::from(painted + 1 < columns.len()), "{painted}");
    }
    assert_eq!(take_work()[Work::Pieces as usize], 1);
    // A column painted again after the last one is prepared again.
    draw(&cache, doc, cascade, page_box, root, Some(columns[0]));
    assert_eq!(take_work()[Work::Pieces as usize], 1);
    assert_eq!(cache.paragraphs().entries.borrow().len(), 1);
}
