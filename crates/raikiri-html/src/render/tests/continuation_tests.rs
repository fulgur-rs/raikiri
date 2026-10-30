use super::*;
use raikiri_traits::{LayoutConfig, PageDefaults};

/// Page 0 is 160px wide and every later page 360px wide; both are 160px
/// tall, which holds sixteen 10px Ahem lines.
const PAGES: &str = "<style>\
    @page { size: 400px 200px; margin: 20px }\
    @page :first { margin-left: 220px }\
    body { margin: 0 }\
    p, div { margin: 0; font: 10px/10px Ahem }\
    </style>";

fn words(count: usize) -> Vec<String> {
    (0..count).map(|index| format!("w{index:03}")).collect()
}

fn run(html: &str) -> PipelineOutput {
    let resources = RenderResources::new().fonts(super::ahem_fonts());
    match run_pipeline(
        &parse(html),
        PageDefaults::default(),
        &LayoutConfig::default(),
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: false,
            resume: None,
        },
    )
    .expect("layout")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted | PipelineRun::NotResumed => panic!("completed layout"),
    }
}

/// Lay out `html` with at most `max_pages` document pages.
fn run_with_page_limit(html: &str, max_pages: u32) -> PipelineOutput {
    let resources = RenderResources::new().fonts(super::ahem_fonts());
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(max_pages))
                .build(),
        )
        .build();
    match run_pipeline(
        &parse(html),
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: false,
            resume: None,
        },
    )
    .expect("layout")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted | PipelineRun::NotResumed => panic!("completed layout"),
    }
}

fn parse(html: &str) -> HtmlDocument {
    crate::parse_html_with_resources(html.as_bytes(), &RenderResources::new()).expect("parse")
}

/// The words of each line of page `page_index`, top to bottom.
fn page_lines(out: &PipelineOutput, page_index: u32) -> Vec<Vec<String>> {
    let (document, cascade) = out.layout_for_page(page_index);
    let mut runs: Vec<_> = document
        .page_text_runs(cascade, page_index)
        .into_iter()
        .map(|run| (run.origin.1, run.origin.0, run.text.to_owned()))
        .collect();
    runs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut lines: Vec<(f32, Vec<String>)> = Vec::new();
    for (y, _, text) in runs {
        let words = text.split_whitespace().map(str::to_owned);
        match lines.last_mut() {
            Some((line_y, line)) if (*line_y - y).abs() < 0.5 => line.extend(words),
            _ => lines.push((y, words.collect())),
        }
    }
    lines.into_iter().map(|(_, line)| line).collect()
}

#[test]
fn a_paragraph_continues_at_the_width_of_later_pages() {
    let expected = words(100);
    let out = run(&format!("{PAGES}<p>{}</p>", expected.join(" ")));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);

    let first = page_lines(&out, 0);
    let second = page_lines(&out, 1);
    // Three 40px words fit in 160px, seven in 360px.
    assert_eq!(first.len(), 16);
    assert!(first.iter().all(|line| line.len() == 3));
    assert_eq!(second[0].len(), 7);
    // Every word appears once, in order, across the two layouts.
    let all: Vec<String> = first.into_iter().chain(second).flatten().collect();
    assert_eq!(all, expected);
}

/// Pages of a `vertical-rl` root: lines run down the page, so the page
/// area's height is their length, 160px on page 0 and 360px on later pages.
/// Every page area is 160px wide, which holds sixteen 10px Ahem lines.
const VERTICAL_PAGES: &str = "<style>\
    @page { size: 200px 400px; margin: 20px }\
    @page :first { margin-top: 220px }\
    :root { writing-mode: vertical-rl }\
    body { margin: 0 }\
    p { margin: 0; font: 10px/10px Ahem }\
    </style>";

/// The `(width, height)` of the text fragments on page `page_index`.
fn text_sizes(out: &PipelineOutput, page_index: u32) -> Vec<(f32, f32)> {
    let (document, _) = out.layout_for_page(page_index);
    document
        .page_fragments(page_index)
        .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Text)
        .map(|fragment| {
            let rect = fragment.rect();
            (rect.width, rect.height)
        })
        .collect()
}

#[test]
fn a_vertical_paragraph_continues_at_the_height_of_later_pages() {
    let out = run(&format!("{VERTICAL_PAGES}<p>{}</p>", words(100).join(" ")));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    // Three 40px words fit on a 160px line: sixteen lines hold 48 words.
    // Seven fit on a 360px line, so the other 52 take eight lines.
    assert_eq!(text_sizes(&out, 0), vec![(160.0, 160.0)]);
    assert_eq!(text_sizes(&out, 1), vec![(80.0, 360.0)]);
}

#[test]
fn a_vertical_page_start_after_a_percentage_width_box_is_measured_in_the_page_area() {
    // A definite 160px body makes the box 80px wide, leaving eight lines
    // on page 0 for 24 words. The other 76 take eleven 360px lines. The
    // ordinary auto-width body is no longer a stand-in for the page area.
    let out = run(&format!(
        "{VERTICAL_PAGES}<style>body{{width:160px}}</style><div style='width:50%'></div><p>{}</p>",
        words(100).join(" ")
    ));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(text_sizes(&out, 0), vec![(80.0, 160.0)]);
    assert_eq!(text_sizes(&out, 1), vec![(110.0, 360.0)]);
}

#[test]
fn blocks_continue_at_the_width_of_later_pages() {
    let blocks = "<div style='height:40px'>x</div>".repeat(8);
    let out = run(&format!("{PAGES}{blocks}"));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    let width_on = |page_index: u32| {
        let (document, _) = out.layout_for_page(page_index);
        document
            .page_fragments(page_index)
            .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Box)
            .filter(|fragment| {
                document
                    .get_node(fragment.node().0 as usize)
                    .and_then(|node| node.tag_name())
                    == Some("div")
            })
            .map(|fragment| fragment.rect().width)
            .collect::<Vec<_>>()
    };
    assert_eq!(width_on(0), vec![160.0; 4]);
    assert_eq!(width_on(1), vec![360.0; 4]);
}

#[test]
fn pages_of_one_width_keep_a_single_layout() {
    let expected = words(100);
    let out = run(&format!(
        "<style>@page {{ size: 400px 200px; margin: 20px }}\
         @page :first {{ margin-top: 60px }}\
         body {{ margin: 0 }} p {{ margin: 0; font: 10px/10px Ahem }}</style><p>{}</p>",
        expected.join(" ")
    ));
    assert!(out.continuations.is_empty());
    let all: Vec<String> = (0..out.slices.len() as u32)
        .flat_map(|page| page_lines(&out, page))
        .flatten()
        .collect();
    assert_eq!(all, expected);
}

#[test]
fn alternating_widths_continue_once_per_change() {
    let expected = words(200);
    let out = run(&format!(
        "<style>@page {{ size: 400px 200px; margin: 20px }}\
         @page :left {{ margin-right: 220px }}\
         body {{ margin: 0 }} p {{ margin: 0; font: 10px/10px Ahem }}</style><p>{}</p>",
        expected.join(" ")
    ));
    assert!(out.slices.len() >= 3);
    let firsts: Vec<u32> = out
        .continuations
        .iter()
        .map(|continuation| continuation.first_page)
        .collect();
    assert_eq!(firsts, (1..out.slices.len() as u32).collect::<Vec<_>>());
    let all: Vec<String> = (0..out.slices.len() as u32)
        .flat_map(|page| page_lines(&out, page))
        .flatten()
        .collect();
    assert_eq!(all, expected);
}

#[test]
fn a_page_that_only_continues_a_box_keeps_the_first_layout() {
    let out = run(&format!("{PAGES}<div style='height:500px'></div>"));
    assert!(out.slices.len() >= 2);
    assert!(out.continuations.is_empty());
}

#[test]
fn content_after_a_page_that_only_continues_a_box_is_laid_out_at_its_own_width() {
    // Page 1 holds only the middle of the tall box, so the relayout resumes
    // at page 2, where the paragraph starts below the box's tail.
    let expected = words(30);
    let out = run(&format!(
        "{PAGES}<div style='height:400px'></div><p>{}</p>",
        expected.join(" ")
    ));
    let firsts: Vec<u32> = out
        .continuations
        .iter()
        .map(|continuation| continuation.first_page)
        .collect();
    assert_eq!(firsts, vec![2]);
    assert!(page_lines(&out, 1).is_empty());
    let lines = page_lines(&out, 2);
    assert_eq!(lines[0].len(), 7);
    assert_eq!(lines.into_iter().flatten().collect::<Vec<_>>(), expected);
}

#[test]
fn content_after_columns_is_laid_out_at_its_own_width() {
    // Pages that start inside the columns keep the first layout; the first
    // later page that starts in the paragraph after them is resumed at the
    // wider width, with no word of the paragraph lost or repeated.
    let expected: Vec<String> = (0..30).map(|index| format!("x{index:03}")).collect();
    let out = run(&format!(
        "{PAGES}<div style='column-count:2;column-gap:0'><p>{}</p></div><p>{}</p>",
        words(400).join(" "),
        expected.join(" ")
    ));
    assert_eq!(out.continuations.len(), 1);
    let first = out.continuations[0].first_page;
    assert!(first > 1);
    let lines = page_lines(&out, first);
    assert!(lines[0].len() > 3, "{lines:?}");
    let paragraph: Vec<String> = (0..out.slices.len() as u32)
        .flat_map(|page| page_lines(&out, page))
        .flatten()
        .filter(|word| expected.contains(word))
        .collect();
    assert_eq!(paragraph, expected);
}

#[test]
fn content_after_an_empty_first_page_is_laid_out_at_its_own_width() {
    let expected = words(30);
    let out = run(&format!(
        "{PAGES}<div style='break-after:page'></div><p>{}</p>",
        expected.join(" ")
    ));
    assert_eq!(out.continuations.len(), 1);
    assert!(page_lines(&out, 0).is_empty());
    let lines = page_lines(&out, 1);
    assert_eq!(lines[0].len(), 7);
    assert_eq!(lines.into_iter().flatten().collect::<Vec<_>>(), expected);
}

#[test]
fn a_cover_page_keeps_its_forced_break() {
    let expected = words(30);
    let out = run(&format!(
        "{PAGES}<div style='height:30px;break-after:page'>cover</div><p>{}</p>",
        expected.join(" ")
    ));
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(page_lines(&out, 0), vec![vec!["cover".to_owned()]]);
    let lines = page_lines(&out, 1);
    assert_eq!(lines[0].len(), 7);
    assert_eq!(lines.into_iter().flatten().collect::<Vec<_>>(), expected);
}

#[test]
fn a_page_that_starts_inside_columns_keeps_the_first_layout() {
    let out = run(&format!(
        "{PAGES}<div style='column-count:2;column-gap:0'><p>{}</p></div>",
        words(400).join(" ")
    ));
    assert!(out.slices.len() >= 2);
    assert!(out.continuations.is_empty());
}

#[test]
fn narrower_later_pages_count_toward_the_page_limit() {
    // Page 0 is 360px wide and later pages 160px: the first layout needs
    // two pages, the narrower continuation a third.
    let html = format!(
        "<style>\
        @page {{ size: 400px 200px; margin: 20px 20px 20px 220px }}\
        @page :first {{ margin-left: 20px }}\
        body {{ margin: 0 }}\
        p {{ margin: 0; font: 10px/10px Ahem }}\
        </style><p>{}</p>",
        words(200).join(" ")
    );
    let resources = RenderResources::new().fonts(super::ahem_fonts());
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(2))
                .build(),
        )
        .build();
    let result = run_pipeline(
        &parse(&html),
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: false,
            resume: None,
        },
    );
    assert!(matches!(
        result,
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 2,
            actual: 3,
        })
    ));
}

#[test]
fn a_named_page_of_another_width_is_laid_out_at_its_own_width() {
    // The change to the named page forces a break before the wide box, so
    // the relayout starts the box on a page of its own.
    let out = run("<style>\
        @page { size: 200px 200px; margin: 0 }\
        @page wide { size: 400px 200px }\
        body { margin: 0 } div { font: 10px/10px Ahem }\
        </style><div>first</div><div id='wide' style='page:wide'>second</div>");
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.slices[1].page_name.as_deref(), Some("wide"));
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    let (document, _) = out.layout_for_page(1);
    let widths: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Box)
        .filter(|fragment| {
            document
                .get_node(fragment.node().0 as usize)
                .and_then(|node| node.tag_name())
                == Some("div")
        })
        .map(|fragment| fragment.rect().width)
        .collect();
    assert_eq!(widths, vec![400.0]);
}

/// The widths of the `div` boxes on page `page_index`.
fn div_widths(out: &PipelineOutput, page_index: u32) -> Vec<f32> {
    let (document, _) = out.layout_for_page(page_index);
    document
        .page_fragments(page_index)
        .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Box)
        .filter(|fragment| {
            document
                .get_node(fragment.node().0 as usize)
                .and_then(|node| node.tag_name())
                == Some("div")
        })
        .map(|fragment| fragment.rect().width)
        .collect()
}

/// The widths and block positions of the `div` boxes on page `page_index`.
fn div_boxes(out: &PipelineOutput, page_index: u32) -> Vec<(f32, f32)> {
    let (document, _) = out.layout_for_page(page_index);
    document
        .page_fragments(page_index)
        .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Box)
        .filter(|fragment| {
            document
                .get_node(fragment.node().0 as usize)
                .and_then(|node| node.tag_name())
                == Some("div")
        })
        .map(|fragment| (fragment.rect().width, fragment.rect().y))
        .collect()
}

const PAGE_WIDTHS: &str = "<style>\
    @page { size: 200px 200px; margin: 0 }\
    @page wide { size: 400px 200px }\
    body { margin: 0 } div { font: 10px/10px Ahem }\
    </style>";

#[test]
fn a_box_after_a_forced_break_keeps_its_margin_at_the_width_of_its_page() {
    // The relayout breaks before the box where the forced break was, and an
    // unforced break would cut the margin the forced break kept.
    let out = run(&format!(
        "{PAGE_WIDTHS}<div style='page:wide'>first</div>\
         <div style='break-before:page;margin-top:30px'>second</div>"
    ));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(div_boxes(&out, 1), vec![(200.0, 30.0)]);
}

#[test]
fn a_box_after_a_page_name_change_keeps_the_margin_of_its_first_child() {
    // The margin of the inner box collapses through the outer one, which
    // starts the unnamed page after the wide one.
    let out = run(&format!(
        "{PAGE_WIDTHS}<div style='page:wide'>first</div>\
         <div><div style='margin-top:30px'>second</div></div>"
    ));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(div_boxes(&out, 1), vec![(200.0, 30.0), (200.0, 30.0)]);
}

#[test]
fn a_named_page_keeps_the_margin_of_the_first_child_of_its_box() {
    let out = run(&format!(
        "{PAGE_WIDTHS}<div>first</div>\
         <div style='page:wide'><div style='margin-top:30px'>second</div></div>"
    ));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(div_boxes(&out, 1), vec![(400.0, 30.0), (400.0, 30.0)]);
}

#[test]
fn a_named_page_after_a_tall_first_page_is_laid_out_at_its_own_width_within_the_page_limit() {
    // The content before the named page is taller than the measuring room
    // alone, and the document has exactly as many pages as its limit, so
    // the measuring page has to hold the content before the page start.
    let html = "<style>\
        @page { size: 200px 2200000px; margin: 0 }\
        @page wide { size: 400px 200px }\
        body { margin: 0 } div { font: 10px/10px Ahem }\
        </style><div style='height:2100000px'></div>\
        <div style='page:wide'>second</div>";
    let out = run_with_page_limit(html, 2);
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.slices[1].page_name.as_deref(), Some("wide"));
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(div_widths(&out, 1), vec![400.0]);
    assert_eq!(page_lines(&out, 1), vec![vec!["second".to_owned()]]);
}

/// A paragraph of `count` words that takes one or two lines on the wide
/// first page and a line for every four words at the narrow named page's
/// width, followed by a box on the named page.
fn growing_paragraph(count: usize) -> String {
    format!(
        "<style>\
        @page {{ size: 4000px 300000px; margin: 0 }}\
        @page narrow {{ size: 200px 200px }}\
        body {{ margin: 0 }} p, div {{ margin: 0; font: 10px/10px Ahem }}\
        </style><p style='line-height:100000px'>{}</p>\
        <div style='page:narrow'>second</div>",
        words(count).join(" ")
    )
}

#[test]
fn a_named_page_after_content_that_grows_past_the_measuring_page_is_laid_out_at_its_own_width() {
    // In the relayout the paragraph grows past the room of the measuring
    // page, so the measurement finds the page start on a later measuring
    // page.
    let out = run(&growing_paragraph(60));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.slices[1].page_name.as_deref(), Some("narrow"));
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(div_widths(&out, 1), vec![200.0]);
    assert_eq!(page_lines(&out, 1), vec![vec!["second".to_owned()]]);
}

#[test]
fn content_that_grows_past_more_measuring_pages_than_the_page_limit_is_measured_again() {
    // The paragraph grows past more measuring pages than the document's
    // two-page limit, so the measurement stops before the page start and is
    // repeated on a taller page.
    let out = run_with_page_limit(&growing_paragraph(120), 2);
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(div_widths(&out, 1), vec![200.0]);
    assert_eq!(page_lines(&out, 1), vec![vec!["second".to_owned()]]);
}

#[test]
fn a_float_below_its_block_does_not_spill_onto_a_relaid_out_page() {
    // The float hangs below its block on the wide page. A relayout of the
    // last page without the forced breaks before it would carry the float
    // onto that page, so the page keeps the earlier layout instead.
    let out = run("<style>\
        @page { size: 200px 200px; margin: 0 }\
        @page wide { size: 400px 200px }\
        body { margin: 0 } div { font: 10px/10px Ahem }\
        </style><div>first</div>\
        <div style='page:wide'><div style='float:right;margin-top:150px;width:50px;height:50px'></div>x</div>\
        <div>last</div>");
    assert_eq!(out.slices.len(), 3);
    let (document, _) = out.layout_for_page(2);
    let floats = document
        .page_fragments(2)
        .filter(|fragment| fragment.rect().width == 50.0)
        .count();
    assert_eq!(floats, 0);
    assert_eq!(page_lines(&out, 2), vec![vec!["last".to_owned()]]);
}

#[test]
fn a_named_page_after_text_beside_a_float_is_laid_out_at_its_own_width() {
    // The named page's box clears the tall float beside the text before it.
    // The measurement keeps that clearance, so the box still continues at
    // the named page's width.
    let out = run("<style>\
        @page { size: 200px 200px; margin: 0 }\
        @page wide { size: 400px 200px }\
        body { margin: 0; font: 10px/10px Ahem }\
        </style><div style='float:right;width:50px;height:150px'></div>first\
        <div style='page:wide'>second</div>");
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.slices[1].page_name.as_deref(), Some("wide"));
    assert_eq!(out.continuations.len(), 1);
    let (document, _) = out.layout_for_page(1);
    let widths: Vec<f32> = document
        .page_fragments(1)
        .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Box)
        .filter(|fragment| {
            document
                .get_node(fragment.node().0 as usize)
                .and_then(|node| node.tag_name())
                == Some("div")
        })
        .map(|fragment| fragment.rect().width)
        .collect();
    assert_eq!(widths, vec![400.0]);
}

#[test]
fn a_float_continuing_onto_a_page_counts_as_content_before_its_start() {
    // The float starts on the first page and continues onto the second,
    // which starts with the paragraph's later lines.
    let out = run(&format!(
        "<style>@page {{ size: 200px 200px; margin: 0 }} body {{ margin: 0 }}\
         p {{ margin: 0; font: 10px/10px Ahem }}</style>\
         <div style='float:right;width:50px;height:300px'></div><p>{}</p>",
        words(120).join(" ")
    ));
    assert!(out.slices.len() >= 2);
    let (document, cascade) = out.layout_for_page(1);
    let start = document.page_start(cascade, 1).expect("a page start");
    let before = document.page_content_before(cascade, 1, start.token);
    let floats: Vec<_> = before
        .iter()
        .filter(|&&node| document.get_node(node).and_then(|node| node.tag_name()) == Some("div"))
        .collect();
    assert_eq!(floats.len(), 1, "{before:?}");
}

/// The body's box fragment on each page, as (top, height).
fn body_on_pages(out: &PipelineOutput) -> Vec<Option<(f32, f32)>> {
    (0..out.slices.len() as u32)
        .map(|page| {
            let (document, _) = out.layout_for_page(page);
            document
                .page_fragments(page)
                .filter(|fragment| fragment.kind() == raikiri_dom::FragmentKind::Box)
                .find(|fragment| {
                    document
                        .get_node(fragment.node().0 as usize)
                        .and_then(|node| node.tag_name())
                        == Some("body")
                })
                .map(|fragment| (fragment.rect().y, fragment.rect().height))
        })
        .collect()
}

#[test]
fn a_bordered_body_reaches_every_relaid_out_page() {
    // 14 blocks of 40px below a 1px border: the later pages start at the
    // top of their first block, so the body's bottom border ends 81px down
    // the fourth page.
    let blocks = "<div style='height:40px'>x</div>".repeat(14);
    let out = run(&format!(
        "{PAGES}<style>body {{ border: 1px solid }}</style>{blocks}"
    ));
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(
        body_on_pages(&out),
        vec![
            Some((20.0, 160.0)),
            Some((20.0, 160.0)),
            Some((20.0, 160.0)),
            Some((20.0, 81.0)),
        ]
    );
}

#[test]
fn a_bordered_body_reaches_every_page_of_one_width() {
    let blocks = "<div style='height:40px'>x</div>".repeat(14);
    let out = run(&format!(
        "{}<style>body {{ border: 1px solid }}</style>{blocks}",
        PAGES.replace("margin-left: 220px", "margin-top: 20px")
    ));
    assert!(out.continuations.is_empty());
    assert_eq!(
        body_on_pages(&out),
        vec![
            Some((20.0, 160.0)),
            Some((20.0, 160.0)),
            Some((20.0, 160.0)),
            Some((20.0, 82.0)),
        ]
    );
}

#[test]
fn a_bordered_paragraph_body_reaches_every_page() {
    // Seven 50px words fit in 360px, so the body's 200 words take 29 10px
    // lines: with its 1px borders it ends 132px down the second page.
    let out = run(&format!(
        "{}<style>body {{ border: 1px solid; font: 10px/10px Ahem }}</style>{}",
        PAGES.replace("margin-left: 220px", "margin-top: 20px"),
        words(200).join(" ")
    ));
    assert_eq!(
        body_on_pages(&out),
        vec![Some((20.0, 160.0)), Some((20.0, 132.0))]
    );
}

#[test]
fn a_body_with_only_a_side_border_ends_with_its_last_block() {
    // The last block ends 130px down the third page; its block-end margin
    // collapses through the body, which has no block-end edges.
    let blocks = "<div style='height:40px;margin-bottom:5px'>x</div>".repeat(10);
    let out = run(&format!(
        "{}<style>body {{ border-left: 1px solid }}</style>{blocks}",
        PAGES.replace("margin-left: 220px", "margin-top: 20px")
    ));
    assert_eq!(
        body_on_pages(&out),
        vec![
            Some((20.0, 160.0)),
            Some((20.0, 160.0)),
            Some((20.0, 130.0)),
        ]
    );
}

#[test]
fn a_formatting_context_body_keeps_its_last_blocks_margin() {
    // A `flow-root` body keeps the last block's 5px block-end margin, so
    // it ends 135px down the third page.
    let blocks = "<div style='height:40px;margin-bottom:5px'>x</div>".repeat(10);
    let out = run(&format!(
        "{}<style>body {{ display: flow-root; border-left: 1px solid }}</style>{blocks}",
        PAGES.replace("margin-left: 220px", "margin-top: 20px")
    ));
    assert_eq!(
        body_on_pages(&out),
        vec![
            Some((20.0, 160.0)),
            Some((20.0, 160.0)),
            Some((20.0, 135.0)),
        ]
    );
}

#[test]
fn generated_overflow_leaves_a_definite_body_height() {
    // The `::after` overflows the 10px body onto the third page. The body
    // keeps its authored 10px content height plus its 1px borders, and
    // does not follow its generated overflow onto those later pages.
    let out = run(&format!(
        "{}<style>body {{ height: 10px; border: 1px solid }} \
         body::after {{ content: ''; display: block; height: 400px }}</style>",
        PAGES.replace("margin-left: 220px", "margin-top: 20px")
    ));
    assert_eq!(body_on_pages(&out), vec![Some((20.0, 12.0)), None, None]);
}

#[test]
fn generated_overflow_leaves_a_paragraph_body_at_its_max_height() {
    // Text and a tall `::after` in the body's own paragraph overflow its
    // 10px `max-height`.
    let out = run(&format!(
        "{}<style>body {{ max-height: 10px; border: 1px solid }} \
         body::after {{ content: ''; display: block; height: 400px }}</style>text",
        PAGES.replace("margin-left: 220px", "margin-top: 20px")
    ));
    let pages = body_on_pages(&out);
    assert!(pages.len() > 1);
    assert_eq!(pages[0], Some((20.0, 12.0)));
    assert!(pages[1..].iter().all(Option::is_none), "{pages:?}");
}

#[test]
fn a_cancellation_while_measuring_a_continuation_aborts_the_run() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Cancels the run on its third image, which the continuation's
    /// measurement resolves after the probe and the first layout.
    struct AbortOnThirdResolve {
        controller: raikiri_traits::AbortController,
        calls: AtomicUsize,
    }

    impl raikiri_traits::ReplacedResolver for AbortOnThirdResolve {
        fn resolve(
            &self,
            _request: raikiri_traits::ResolverRequest<'_>,
        ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) + 1 == 3 {
                self.controller.abort();
            }
            Ok(raikiri_traits::ResolvedIntrinsic::new(
                raikiri_traits::IntrinsicBox::new(1.0, 1.0),
                raikiri_traits::ResolveDisposition::Ok,
            ))
        }
    }

    let resolver = AbortOnThirdResolve {
        controller: raikiri_traits::AbortController::new(),
        calls: AtomicUsize::new(0),
    };
    let resources = RenderResources::new()
        .fonts(super::ahem_fonts())
        .replaced_resolver(&resolver);
    let html = "<style>\
        @page { size: 200px 200px; margin: 0 }\
        @page wide { size: 400px 200px }\
        body { margin: 0 } div { font: 10px/10px Ahem }\
        </style><div>first</div><div style='page:wide'>\
        <img src='https://example.invalid/image.png' style='display:block;height:30px'>\
        second</div>";
    let config = LayoutConfig::builder()
        .signal(Some(resolver.controller.signal.clone()))
        .build();
    let result = run_pipeline(
        &parse(html),
        PageDefaults::default(),
        &config,
        PipelineInputs {
            resources: Some(&resources),
            consumer_properties: &[],
            property_observer: None,
            preload_background_images: true,
            resume: None,
        },
    );
    assert!(matches!(result, Ok(PipelineRun::Aborted)));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn a_named_page_after_an_empty_box_is_laid_out_at_its_own_width() {
    // The empty box sits at the end of the first page's content. It does
    // not open the next page, so the change to the named page breaks once,
    // and the relayout reproduces that page start.
    let out = run("<style>\
        @page { size: 200px 200px; margin: 0 }\
        @page wide { size: 400px 200px }\
        body { margin: 0 } div { font: 10px/10px Ahem }\
        </style><div>first</div><div style='height:0'></div>\
        <div style='page:wide'>second</div>");
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.slices[1].page_name.as_deref(), Some("wide"));
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    assert_eq!(page_lines(&out, 1), vec![vec!["second".to_owned()]]);
}

#[test]
fn text_after_the_floats_of_named_pages_continues_at_the_width_of_the_next_page() {
    // Each 500px wide `large` page ends with a float, and the text after
    // them starts a 400px wide page: eight 40px words with their spaces fit
    // on a line there, ten on a `large` page.
    let expected = words(12);
    let float = "<div style='float:right; margin-top:300px; width:100px; height:100px'></div>";
    let out = run(&format!(
        "<style>@page {{ size: 400px; margin: 0 }} @page large {{ size: 500px 400px }}\
         body {{ margin: 0; font: 10px/10px Ahem }}</style>\
         first\
         <div style='page:large'>{float}<div style='width:400px'>a</div>\
         <div style='break-before:page'>{float}<div style='width:400px'>b</div></div></div>\
         {}",
        expected.join(" ")
    ));
    assert_eq!(out.slices.len(), 4);
    let first_pages: Vec<u32> = out.continuations.iter().map(|c| c.first_page).collect();
    assert_eq!(first_pages, vec![1, 3]);
    let last = page_lines(&out, 3);
    assert_eq!(last[0].len(), 8);
    assert_eq!(last.concat(), expected);
}

#[test]
fn content_after_a_float_and_a_forced_break_continues_at_the_width_of_the_next_page() {
    // The float ends on the 300px wide first page, before the break.
    let expected = words(12);
    let out = run(&format!(
        "<style>@page {{ size: 400px; margin: 0 }} @page :first {{ margin-left: 100px }}\
         body {{ margin: 0; font: 10px/10px Ahem }}</style>\
         <div>first</div>\
         <div style='float:right; margin-top:280px; width:100px; height:100px'></div>\
         <div style='break-before:page'>{}</div>",
        expected.join(" ")
    ));
    assert_eq!(out.slices.len(), 2);
    assert_eq!(out.continuations.len(), 1);
    assert_eq!(out.continuations[0].first_page, 1);
    let second = page_lines(&out, 1);
    assert_eq!(second[0].len(), 8);
    assert_eq!(second.concat(), expected);
}
