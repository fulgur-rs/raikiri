use std::cell::RefCell;
use std::rc::Rc;

use super::super::*;
use crate::document_layout::Page;
use crate::layout;

/// What a page shows: its name, every fragment with its geometry and
/// position among the node's fragments, its text runs and its margin
/// boxes.
fn digest(page: &Page<'_>) -> String {
    let mut out = format!("page {} {:?}\n", page.index(), page.name());
    for fragment in page.fragments() {
        let rect = fragment.rect();
        out.push_str(&format!(
            "{:?} {:?} {} last={:?} lines={:?} [{} {} {} {}]\n",
            fragment.node(),
            fragment.kind(),
            fragment.fragment_index(),
            fragment.is_last_fragment(),
            fragment.line_range(),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
        ));
    }
    for run in page.text_runs() {
        out.push_str(&format!(
            "run {:?} {:?} ({} {})\n",
            run.source, run.text, run.origin.0, run.origin.1
        ));
    }
    out.push_str(&format!("margins {:?}\n", page.margin_boxes()));
    out
}

/// Pages a sink saw, as digests.
#[derive(Clone, Default)]
struct Digests(Rc<RefCell<Vec<String>>>);

impl PageSink for Digests {
    type Output = StreamSummary;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        self.0.borrow_mut().push(digest(&page.page()));
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        Ok(summary)
    }
}

/// The anchors of paragraphs `p0` to `p59`.
fn paragraph_anchors(anchors: &AnchorIndex) -> Vec<String> {
    (0..60)
        .map(|index| format!("{:?}", anchors.get(&format!("p{index}"))))
        .collect()
}

/// Page digests and paragraph anchors of a batch layout.
fn batch(html: &str) -> (Vec<String>, Vec<String>) {
    let resources = RenderResources::new();
    let doc = crate::parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &doc,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    (
        laid_out.pages().map(|page| digest(&page)).collect(),
        paragraph_anchors(laid_out.anchors()),
    )
}

/// Stream `html` in `chunk`-byte pieces with a checkpoint after each.
/// Returns the page digests, the summary and how many layouts resumed.
fn stream(html: &str, chunk: usize) -> (Vec<String>, StreamSummary, usize) {
    let resources = RenderResources::new();
    let pages = Digests::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        pages.clone(),
    )
    .checkpoint_bytes(chunk);
    for piece in html.as_bytes().chunks(chunk) {
        stream.feed(piece).expect("feed");
    }
    let resumed = stream.settings.resumed.get();
    let StreamStatus::Completed(summary) = stream.finish().expect("finish") else {
        panic!("aborted");
    };
    let pages = pages.0.borrow().clone();
    (pages, summary, resumed)
}

/// Whether `left` and `right` read the same, with their numbers equal up to
/// the noise a relayout from another origin adds to coordinates.
fn same(left: &str, right: &str) -> bool {
    fn parts(text: &str) -> Vec<Result<f32, &str>> {
        let mut parts = Vec::new();
        let mut rest = text;
        while !rest.is_empty() {
            let numeric = |c: char| c.is_ascii_digit() || c == '.' || c == '-';
            let end = if rest.starts_with(|c: char| c.is_ascii_digit()) {
                rest.find(|c: char| !numeric(c)).unwrap_or(rest.len())
            } else {
                rest.find(|c: char| c.is_ascii_digit())
                    .unwrap_or(rest.len())
            };
            let (part, tail) = rest.split_at(end);
            parts.push(part.parse::<f32>().map_err(|_| part));
            rest = tail;
        }
        parts
    }
    let (left, right) = (parts(left), parts(right));
    left.len() == right.len()
        && left.iter().zip(&right).all(|pair| match pair {
            (Ok(left), Ok(right)) => (left - right).abs() <= 0.01,
            (left, right) => left == right,
        })
}

/// Streamed pages and anchors equal the batch layout's, and some layouts
/// resumed.
fn assert_resumes_like_batch(html: &str) {
    let (expected, anchors) = batch(html);
    assert!(expected.len() > 4, "the fixture spans several pages");
    let (pages, summary, resumed) = stream(html, 2048);
    assert_eq!(pages.len(), expected.len());
    for (streamed, batch) in pages.iter().zip(&expected) {
        assert!(
            same(streamed, batch),
            "streamed:\n{streamed}\nbatch:\n{batch}"
        );
    }
    assert!(resumed > 0, "no layout resumed");
    let streamed = paragraph_anchors(&summary.anchors);
    assert!(
        streamed
            .iter()
            .zip(&anchors)
            .all(|(left, right)| same(left, right)),
        "streamed {streamed:?}, batch {anchors:?}"
    );
}

fn paragraphs(count: usize, words: usize) -> String {
    let mut html = String::new();
    for paragraph in 0..count {
        html.push_str(&format!("<p id=p{paragraph}>Paragraph {paragraph}. "));
        for word in 0..words {
            html.push_str(&format!("w{word} "));
        }
        html.push_str("</p>\n");
    }
    html
}

#[test]
fn plain_paragraphs_resume_like_batch() {
    let html = format!("<!doctype html><body>{}</body>", paragraphs(60, 60));
    assert_resumes_like_batch(&html);
}

#[test]
fn nested_sections_resume_like_batch() {
    let mut html = String::from(
        "<!doctype html><style>section { border: 2px solid; padding: 4px } \
         h2 { break-after: avoid }</style><body>",
    );
    for section in 0..16 {
        html.push_str(&format!("<section><h2>Section {section}</h2>"));
        html.push_str(&paragraphs(6, 50));
        html.push_str("</section>");
    }
    html.push_str("</body>");
    assert_resumes_like_batch(&html);
}

#[test]
fn list_numbers_and_page_counters_resume_like_batch() {
    let mut html = String::from(
        "<!doctype html><style>@page { @bottom-center { content: counter(page) } } \
         h2 { counter-increment: chapter } h2::before { content: counter(chapter) '. ' }\
         </style><body>",
    );
    for chapter in 0..6 {
        html.push_str(&format!("<h2>Chapter {chapter}</h2><ol>"));
        for item in 0..20 {
            html.push_str(&format!("<li>Item {item} of a list long enough to wrap "));
            html.push_str(&"word ".repeat(20));
            html.push_str("</li>");
        }
        html.push_str("</ol>");
    }
    html.push_str("</body>");
    assert_resumes_like_batch(&html);
}

#[test]
fn named_pages_sides_and_strings_resume_like_batch() {
    let mut html = String::from(
        "<!doctype html><style>@page :left { margin-left: 2cm } \
         @page :right { margin-right: 2cm } @page wide { size: A4 landscape } \
         .wide { page: wide } @page { @top-center { content: string(title) } } \
         h2 { string-set: title content() }</style><body>",
    );
    for chapter in 0..6 {
        let class = if chapter % 3 == 2 { " class=wide" } else { "" };
        html.push_str(&format!("<div{class}><h2>Chapter {chapter}</h2>"));
        html.push_str(&paragraphs(24, 60));
        html.push_str("</div>");
    }
    html.push_str("</body>");
    assert_resumes_like_batch(&html);
}

#[test]
fn target_counters_lay_out_the_whole_document() {
    let html = format!(
        "<!doctype html><style>a::after {{ content: target-counter('#p50', page) }}</style>\
         <body><a href='#p50'>to</a>{}</body>",
        paragraphs(60, 60)
    );
    let (expected, _) = batch(&html);
    let (pages, _, resumed) = stream(&html, 2048);
    assert_eq!(pages, expected);
    assert_eq!(resumed, 0);
}

#[test]
fn fixed_height_ancestors_lay_out_the_whole_document() {
    let html = format!(
        "<!doctype html><body><div style='height: 6000px'>{}</div></body>",
        paragraphs(60, 60)
    );
    let (expected, _) = batch(&html);
    let (pages, _, resumed) = stream(&html, 2048);
    assert_eq!(pages, expected);
    assert_eq!(resumed, 0);
}
