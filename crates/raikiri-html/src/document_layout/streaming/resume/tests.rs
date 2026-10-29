use std::cell::RefCell;
use std::rc::Rc;

use super::super::*;
use crate::document_layout::Page;
use crate::document_layout::layout_resumed;
use crate::layout;
use crate::render::ResumePoint;

/// What a page shows: its name, every fragment with its geometry and
/// position among the node's fragments, its text runs and its margin
/// boxes, and the paint transforms of the boxes it places. A box that
/// continues on later pages is left open on a delivered page, so only the
/// top of its content box is compared.
fn digest(page: &Page<'_>) -> String {
    let mut out = format!("page {} {:?}\n", page.index(), page.name());
    for fragment in page.fragments() {
        let rect = fragment.rect();
        out.push_str(&format!(
            "{:?} {:?} {} last={:?} lines={:?} [{} {} {} {}] content={:?}\n",
            fragment.node(),
            fragment.kind(),
            fragment.fragment_index(),
            fragment.is_last_fragment(),
            fragment.line_range(),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            fragment
                .content_rect()
                .map(|content| (content.x, content.y, content.width)),
        ));
    }
    for fragment in page.fragments() {
        if let Some(transform) = page
            .paint_transform(fragment.node())
            .filter(|transform| *transform != raikiri_traits::PaintTransform::IDENTITY)
        {
            out.push_str(&format!("transform {:?} {transform:?}\n", fragment.node()));
        }
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
            let numeric = |c: char| c.is_ascii_digit() || c == '.';
            // A sign belongs to the number after it: noise around zero can
            // flip it.
            let signed = rest
                .strip_prefix('-')
                .is_some_and(|tail| tail.starts_with(|c: char| c.is_ascii_digit()));
            let end = if signed || rest.starts_with(|c: char| c.is_ascii_digit()) {
                let digits = usize::from(signed);
                rest[digits..]
                    .find(|c: char| !numeric(c))
                    .map_or(rest.len(), |end| end + digits)
            } else {
                rest.char_indices()
                    .find(|&(index, c)| {
                        c.is_ascii_digit()
                            || (c == '-'
                                && rest[index + 1..].starts_with(|c: char| c.is_ascii_digit()))
                    })
                    .map_or(rest.len(), |(index, _)| index)
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
    let resumed = assert_streams_like_batch(html);
    assert!(resumed > 0, "no layout resumed");
}

/// Streamed pages and anchors equal the batch layout's. Returns how many
/// layouts resumed.
fn assert_streams_like_batch(html: &str) -> usize {
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
    let streamed = paragraph_anchors(&summary.anchors);
    assert!(
        streamed
            .iter()
            .zip(&anchors)
            .all(|(left, right)| same(left, right)),
        "streamed {streamed:?}, batch {anchors:?}"
    );
    resumed
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
    let html = format!("<!doctype html><body>{}</body>", paragraphs(90, 60));
    assert_resumes_like_batch(&html);
}

#[test]
fn nested_sections_resume_like_batch() {
    let mut html = String::from(
        "<!doctype html><style>section { border: 2px solid; padding: 4px; min-height: 0 } \
         p::before { content: '> ' } \
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

/// A box turned about its transform origin, by default its center, is not
/// delivered before its whole height is known.
#[test]
fn boxes_turned_about_their_height_stream_like_batch() {
    let html = format!(
        "<!doctype html><body>{}<section style='transform: rotate(1deg)'>{}</section>{}</body>",
        paragraphs(15, 60),
        paragraphs(45, 60),
        paragraphs(30, 60)
    );
    assert_streams_like_batch(&html);
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

#[test]
fn layouts_resume_after_a_late_stamp_is_left_out() {
    let head = format!(
        "<!doctype html><body style='position: relative'>{}",
        paragraphs(60, 60)
    );
    let stamp = "<div style='position: absolute; top: 0'>stamp</div>";
    let tail = format!("{}</body>", paragraphs(40, 60));
    let resources = RenderResources::new();
    let pages = Digests::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        pages,
    )
    .checkpoint_bytes(2048);
    for piece in head.as_bytes().chunks(2048) {
        stream.feed(piece).expect("feed");
    }
    stream.feed(stamp.as_bytes()).expect("feed");
    // The stamp lands on the first page, already delivered, and is left out.
    let before = stream.settings.resumed.get();
    for piece in tail.as_bytes().chunks(2048) {
        stream.feed(piece).expect("feed");
    }
    assert!(
        !stream.progress.hidden_late.is_empty(),
        "the stamp is left out"
    );
    assert!(
        stream.settings.resumed.get() > before,
        "layouts after the stamp resume"
    );
}

#[test]
fn one_long_paragraph_lays_out_the_whole_document() {
    // Every page starts inside the one paragraph, so nothing lies wholly
    // before a page start to leave out.
    let mut html = String::from("<!doctype html><body><p>");
    for word in 0..6000 {
        html.push_str(&format!("w{word} "));
    }
    html.push_str("</p></body>");
    let (expected, _) = batch(&html);
    let (pages, _, resumed) = stream(&html, 2048);
    assert_eq!(pages, expected);
    assert_eq!(resumed, 0);
}

/// The root element's border box on every page, in the order the sink saw
/// the pages.
#[derive(Clone, Default)]
struct RootBoxes(Rc<RefCell<Vec<raikiri_traits::PaintRect>>>);

impl PageSink for RootBoxes {
    type Output = StreamSummary;

    fn page(
        &mut self,
        page: StreamPage<'_>,
        _events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        let root = page.page().root_box().expect("a root box");
        self.0.borrow_mut().push(root.border_box);
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        Ok(summary)
    }
}

/// What streaming `html` in `chunk`-byte pieces delivered of the root box.
struct StreamedRoots {
    /// The root box on each page.
    boxes: Vec<raikiri_traits::PaintRect>,
    /// How many layouts resumed after delivered pages.
    resumed: usize,
    /// How many pages were delivered before the input ended.
    early: u32,
}

fn stream_root_boxes(html: &str, chunk: usize) -> StreamedRoots {
    let resources = RenderResources::new();
    let roots = RootBoxes::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        roots.clone(),
    )
    .checkpoint_bytes(chunk);
    for piece in html.as_bytes().chunks(chunk) {
        stream.feed(piece).expect("feed");
    }
    let early = stream.progress.delivered;
    let resumed = stream.settings.resumed.get();
    let StreamStatus::Completed(_) = stream.finish().expect("finish") else {
        panic!("aborted");
    };
    let boxes = roots.0.borrow().clone();
    StreamedRoots {
        boxes,
        resumed,
        early,
    }
}

/// The root element's border box on every page of a batch layout.
fn batch_root_boxes(html: &str) -> Vec<raikiri_traits::PaintRect> {
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
    laid_out
        .pages()
        .map(|page| page.root_box().expect("a root box").border_box)
        .collect()
}

/// Several pages of paragraphs under `canvas` as the style of the document.
fn long_document(canvas: &str) -> String {
    format!(
        "<!doctype html><style>{canvas}</style><body>{}</body>",
        paragraphs(90, 60)
    )
}

/// The streamed root boxes equal the batch layout's: their start and
/// width, and with `whole_height` their height too.
fn assert_root_boxes_like_batch(html: &str, whole_height: bool) -> StreamedRoots {
    let expected = batch_root_boxes(html);
    assert!(expected.len() > 4, "the fixture spans several pages");
    let streamed = stream_root_boxes(html, 2048);
    assert_eq!(streamed.boxes.len(), expected.len());
    for (index, (streamed, batch)) in streamed.boxes.iter().zip(&expected).enumerate() {
        let shown = |rect: &raikiri_traits::PaintRect| {
            let height = if whole_height { rect.height } else { 0.0 };
            (rect.x, rect.y, rect.width, height)
        };
        assert_eq!(shown(streamed), shown(batch), "root box on page {index}");
    }
    streamed
}

/// A canvas image the size of the root's box is not delivered before the
/// whole height of the root is known.
#[test]
fn a_canvas_gradient_over_the_root_streams_like_batch() {
    let html = long_document("html { background: linear-gradient(red, blue) }");
    let streamed = assert_root_boxes_like_batch(&html, true);
    assert_eq!(streamed.early, 0);
}

/// The body's background reaches the canvas when the root has none.
#[test]
fn a_canvas_gradient_from_the_body_streams_like_batch() {
    let html = long_document("body { background: linear-gradient(red, blue) }");
    let streamed = assert_root_boxes_like_batch(&html, true);
    assert_eq!(streamed.early, 0);
}

/// Tiles start from the top of the root's box, which a layout resumed after
/// the delivered pages would place at its own start.
#[test]
fn a_tiled_canvas_image_streams_like_batch_without_resuming() {
    let html = long_document(
        "html { background: linear-gradient(red 50%, blue 50%); background-size: 100px 30px }",
    );
    // The tiles do not take the root's height, which the early pages only
    // know in part.
    let streamed = assert_root_boxes_like_batch(&html, false);
    assert_eq!(streamed.resumed, 0);
    assert!(streamed.early > 0, "the pages are delivered as they end");
}

/// A canvas image fixed to the page area, and a canvas that shows only a
/// color, do not use the root's box.
#[test]
fn a_canvas_that_ignores_the_root_box_still_resumes() {
    for canvas in [
        "html { background: linear-gradient(red, blue) fixed }",
        "html { background: #def }",
    ] {
        let streamed = stream_root_boxes(&long_document(canvas), 2048);
        assert!(streamed.resumed > 0, "{canvas}");
        assert!(streamed.early > 0, "{canvas}");
    }
}

/// The parsed `html`, and the first page start at or after page `page`
/// that its batch layout could resume at.
fn resume_point_of(html: &str, page: u32) -> (HtmlDocument, ResumePoint) {
    let resources = RenderResources::new();
    let document = crate::parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    let point = (1..laid_out.page_count())
        .filter(|&index| index >= page)
        .find_map(|index| {
            let names: Vec<_> = laid_out
                .pages()
                .take(index as usize)
                .map(|page| page.name().map(str::to_owned))
                .collect();
            resume_point(&document, &laid_out, index, &names)
        })
        .expect("a page start to resume at");
    (document, point)
}

fn resumed_layout(
    document: &HtmlDocument,
    point: &ResumePoint,
    config: LayoutConfig,
) -> Result<Option<LayoutStatus>, RenderError> {
    let resources = RenderResources::new();
    layout_resumed(
        document,
        PageDefaults::default(),
        config,
        LayoutOptions::new().resources(&resources),
        Some(point),
    )
}

#[test]
fn a_page_start_the_relayout_misses_is_not_resumed() {
    let html = format!("<!doctype html><body>{}</body>", paragraphs(40, 60));
    let (document, mut point) = resume_point_of(&html, 1);
    point.start.offset += 37.0;
    let status = resumed_layout(&document, &point, LayoutConfig::default()).expect("layout");
    assert!(status.is_none());
}

#[test]
fn a_resumed_layout_reports_the_page_limit() {
    let html = format!("<!doctype html><body>{}</body>", paragraphs(40, 60));
    let (document, point) = resume_point_of(&html, 1);
    let config = LayoutConfig::builder()
        .limits(
            raikiri_traits::RenderLimits::builder()
                .max_document_pages(Some(point.first_page + 1))
                .build(),
        )
        .build();
    let result = resumed_layout(&document, &point, config);
    assert!(
        matches!(
            result,
            Err(RenderError::LimitExceeded {
                kind: raikiri_traits::LimitKind::Pages,
                ..
            })
        ),
        "{:?}",
        result.err()
    );
}

/// The elements a layout of `html` can leave out before the element with
/// id `target`.
fn skipped_before_target(html: &str) -> Option<Vec<usize>> {
    let resources = RenderResources::new();
    let document = crate::parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    let dom = &document.uncascaded.dom;
    let target = (0..dom.node_count())
        .find(|&id| {
            dom.get_node(id)
                .and_then(|node| node.attribute("id"))
                .is_some_and(|id| id == "target")
        })
        .expect("target");
    let (layout_document, _) = laid_out.out.layout_for_page(0);
    super::skipped_before(dom, layout_document, &document.cascade, target)
}

#[test]
fn only_ancestors_sized_by_their_content_let_content_be_left_out() {
    let wrapped = |style: &str| {
        format!(
            "<!doctype html><body><p>before</p><div style='{style}'><p>a</p><p id=target>b</p></div></body>"
        )
    };
    let skipped = skipped_before_target(&wrapped("")).expect("left out");
    assert!(skipped.len() >= 2, "{skipped:?}");
    assert!(skipped_before_target(&wrapped("min-height: 0")).is_some());
    assert!(skipped_before_target(&wrapped("height: 300px")).is_none());
    assert!(skipped_before_target(&wrapped("min-height: 300px")).is_none());
    assert!(skipped_before_target(&wrapped("display: flex")).is_none());
}

/// The elements whose consumer property events a sink saw, in order.
#[derive(Clone, Default)]
struct Events(Rc<RefCell<Vec<NodeId>>>);

impl PageSink for Events {
    type Output = StreamSummary;

    fn page(
        &mut self,
        _page: StreamPage<'_>,
        events: Vec<ConsumerPropertyEvent>,
    ) -> std::io::Result<()> {
        self.0
            .borrow_mut()
            .extend(events.into_iter().map(|event| event.node_id));
        Ok(())
    }

    fn finish(self, summary: StreamSummary) -> std::io::Result<StreamSummary> {
        Ok(summary)
    }
}

#[test]
fn events_of_elements_spanning_delivered_pages_arrive_once() {
    let mut html = String::from("<!doctype html><body>");
    for _ in 0..3 {
        html.push_str("<section style='bookmark-level: 1'>");
        html.push_str(&paragraphs(40, 50));
        html.push_str("</section>");
    }
    html.push_str("</body>");
    let resources = RenderResources::new();
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let events = Events::default();
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        events.clone(),
    )
    .consumer_properties(&registrations)
    .checkpoint_bytes(2048);
    for piece in html.as_bytes().chunks(2048) {
        stream.feed(piece).expect("feed");
    }
    assert!(stream.settings.resumed.get() > 0, "no layout resumed");
    let StreamStatus::Completed(summary) = stream.finish().expect("finish") else {
        panic!("aborted");
    };
    assert!(summary.page_count > 6, "sections span several pages");
    let seen = events.0.borrow();
    let mut distinct = seen.clone();
    distinct.sort();
    distinct.dedup();
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert_eq!(distinct.len(), 3, "{seen:?}");
    assert!(summary.unplaced_events.is_empty());
}

#[test]
fn a_box_that_begins_in_columns_on_a_page_does_not_cross_it() {
    let mut words = String::new();
    for word in 0..400 {
        words.push_str(&format!("w{word} "));
    }
    let html = format!(
        "<!doctype html><body><p>before</p>\
         <div style='columns: 2; break-before: page'><p id=target>{words}</p></div></body>"
    );
    let resources = RenderResources::new();
    let document = crate::parse_html_with_resources(html.as_bytes(), &resources).expect("parse");
    let LayoutStatus::Completed(laid_out) = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(&resources),
    )
    .expect("layout") else {
        panic!("aborted");
    };
    let dom = &document.uncascaded.dom;
    let target = (0..dom.node_count())
        .find(|&id| {
            dom.get_node(id)
                .and_then(|node| node.attribute("id"))
                .is_some_and(|id| id == "target")
        })
        .expect("target");
    let page = laid_out.page_at(1);
    let pieces: Vec<u32> = page
        .fragments()
        .filter(|fragment| fragment.node().0 as usize == target)
        .map(|fragment| fragment.fragment_index())
        .collect();
    assert!(
        pieces.len() > 1,
        "the paragraph fills both columns: {pieces:?}"
    );
    let (layout_document, _) = laid_out.out.layout_for_page(1);
    assert!(
        layout_document
            .crossing_boxes(1)
            .iter()
            .all(|crossing| crossing.node.0 as usize != target)
    );
}

/// Loads every image but `broken.png`, and `flaky.png` only once `flaky`
/// is cleared.
struct FlakyResolver {
    flaky: std::sync::atomic::AtomicBool,
}

impl raikiri_traits::ReplacedResolver for FlakyResolver {
    fn resolve(
        &self,
        request: raikiri_traits::ResolverRequest<'_>,
    ) -> Result<raikiri_traits::ResolvedIntrinsic, raikiri_traits::ResolverError> {
        let path = request.url().path();
        let fails = path.ends_with("broken.png")
            || (path.ends_with("flaky.png")
                && self.flaky.load(std::sync::atomic::Ordering::SeqCst));
        let disposition = if fails {
            raikiri_traits::ResolveDisposition::Fallback {
                reason: "unavailable".to_owned(),
            }
        } else {
            raikiri_traits::ResolveDisposition::Ok
        };
        Ok(raikiri_traits::ResolvedIntrinsic::new(
            raikiri_traits::IntrinsicBox::new(20.0, 20.0),
            disposition,
        ))
    }
}

/// The image URLs whose fallback a summary reports.
fn image_fallbacks(summary: &StreamSummary) -> Vec<String> {
    summary
        .warnings
        .iter()
        .filter_map(|warning| match &warning.kind {
            raikiri_traits::WarningKind::ResourceFallback {
                kind: raikiri_traits::ResourceKind::Image,
                url: Some(url),
            } => Some(url.path().to_owned()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_resumed_layout_reports_the_resources_of_delivered_pages_as_last_loaded() {
    let image = |name: &str| {
        format!("<img style='width: 20px; height: 20px' src='https://example.invalid/{name}'>")
    };
    let mut html = String::from("<!doctype html><body>");
    html.push_str(&format!("<p>{}</p>", image("broken.png")));
    html.push_str(&format!("<p>{}</p>", image("flaky.png")));
    // Guarantee multiple pages even when fallback fonts fit each paragraph on one line.
    html.push_str(&paragraphs(120, 60).replace("<p ", "<p style='line-height:24px' "));
    html.push_str("</body>");
    let resolver = FlakyResolver {
        flaky: std::sync::atomic::AtomicBool::new(true),
    };
    let resources = RenderResources::new().replaced_resolver(&resolver);
    let mut stream = StreamingLayout::new(
        &resources,
        PageDefaults::default(),
        LayoutConfig::default(),
        Digests::default(),
    )
    .checkpoint_bytes(2048);
    let mut pieces = html.as_bytes().chunks(2048);
    // The first layout fails to load the flaky image before its page is
    // final; every later one loads it.
    stream.feed(pieces.next().expect("input")).expect("feed");
    assert_eq!(
        stream.progress.delivered, 0,
        "the images' page is not delivered yet"
    );
    resolver
        .flaky
        .store(false, std::sync::atomic::Ordering::SeqCst);
    for piece in pieces {
        stream.feed(piece).expect("feed");
    }
    assert!(stream.settings.resumed.get() > 0, "no layout resumed");
    let StreamStatus::Completed(summary) = stream.finish().expect("finish") else {
        panic!("aborted");
    };
    assert_eq!(image_fallbacks(&summary), ["/broken.png"]);
}

/// `count` boxes `height` tall, each ended by a page break, in a document
/// whose pages are 100px tall, under the rules `style`, and with enough
/// comments between them for the document to arrive in several chunks.
fn boxes_ending_pages(style: &str, height: u32, count: usize) -> String {
    let mut boxes = String::new();
    for index in 0..count {
        boxes.push_str(&format!(
            "<div id=p{index}></div><!-- {} -->\n",
            "x".repeat(900)
        ));
    }
    format!(
        "<!doctype html><style>@page {{ size: 100px 100px; margin: 0 }} body {{ margin: 0 }} \
         div {{ height: {height}px; break-after: page }} {style}</style><body>{boxes}</body>"
    )
}

#[test]
fn space_above_the_first_box_costs_no_streamed_pages() {
    // A relayout after delivered pages begins with a short page that stands
    // in for the space above the first box. The break after that box has to
    // open the page after the one it is on, or each relayout loses a page.
    for style in [
        "html { margin-top: 5px }",
        "html { border-top: 5px solid }",
        "body { padding-top: 5px }",
    ] {
        for height in [90, 95, 100] {
            let html = boxes_ending_pages(style, height, 40);
            let resumed = assert_streams_like_batch(&html);
            assert!(resumed > 0, "no layout resumed: {style} {height}");
        }
    }
}
