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
        },
    )
    .expect("layout")
    {
        PipelineRun::Completed(out) => *out,
        PipelineRun::Aborted => panic!("completed layout"),
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
