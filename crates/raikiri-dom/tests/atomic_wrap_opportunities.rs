//! There is a soft wrap opportunity before and after an atomic inline, even
//! next to a character that Unicode line breaking keeps it with (CSS Text 3
//! §5.1): U+00A0 NO-BREAK SPACE and punctuation. Glue, joiners and white space
//! next to an atomic keep their own rules. The markup is laid out in a
//! container of no width, with a 10px Ahem font and images of 20x50 pixels.
//! For no-break spaces, punctuation and white space the expected lines are the
//! ones Chromium lays out; for glue and joiners they follow the specification,
//! where Chromium breaks.

use raikiri_html::{
    FontCollectionBuilder, LayoutOptions, LayoutStatus, RenderResources, layout,
    parse_html_with_resources,
};
use raikiri_traits::{
    IntrinsicBox, LayoutConfig, PageDefaults, ReplacedResolver, ResolveDisposition,
    ResolvedIntrinsic, ResolverError, ResolverRequest,
};
use url::Url;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

/// A line with an image on the baseline: the image is 50px high, and the
/// descent of the font adds 2px below the baseline.
const IMAGE_LINE: f32 = 52.0;
/// A line with text only.
const TEXT_LINE: f32 = 10.0;

struct Portrait;

impl ReplacedResolver for Portrait {
    fn resolve(&self, _request: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic::new(
            IntrinsicBox::new(20.0, 50.0),
            ResolveDisposition::Ok,
        ))
    }
}

/// The border box `(x, y, width, height)` of every element with an `id` in
/// `body`, laid out with an 8px body margin.
fn boxes(body: &str) -> Vec<(String, [f32; 4])> {
    let html = format!(
        "<!DOCTYPE html><style>@page {{ size: 800px 600px; margin: 0 }} \
         body {{ margin: 8px; font: 10px/10px Ahem }} img {{ width: 20px; height: 50px }}</style>{body}"
    );
    let fonts = FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts");
    let resources = RenderResources::new()
        .fonts(fonts)
        .base_url(Url::parse("https://wrap.test/document.html").expect("url"))
        .replaced_resolver(&Portrait);
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
    page.fragments()
        .filter_map(|fragment| {
            let id = dom.attr(fragment.node(), "id")?;
            let rect = fragment.rect();
            Some((id.to_owned(), [rect.x, rect.y, rect.width, rect.height]))
        })
        .collect()
}

/// The height of a container with no width that holds `content`.
fn container_height(content: &str) -> f32 {
    let found = boxes(&format!("<div id=d style='width: 0'>{content}</div>"));
    let (_, rect) = found
        .iter()
        .find(|(id, _)| id == "d")
        .expect("the container");
    rect[3]
}

/// The top of the element `id` in `body`.
fn top_of(body: &str, id: &str) -> f32 {
    let found = boxes(body);
    let (_, rect) = found
        .iter()
        .find(|(found, _)| found == id)
        .unwrap_or_else(|| panic!("no box {id}"));
    rect[1]
}

#[test]
fn a_no_break_space_between_two_images_gives_each_its_own_line() {
    let body = "<div style='width: 0'><img id=a>&nbsp;<img id=c></div>";
    assert_eq!(top_of(body, "a"), 8.0);
    // The first image, the line of the space, then the second image.
    assert_eq!(top_of(body, "c"), 8.0 + IMAGE_LINE + TEXT_LINE);
    assert_eq!(
        container_height("<img>&nbsp;<img>"),
        2.0 * IMAGE_LINE + TEXT_LINE
    );
}

#[test]
fn a_no_break_space_after_an_image_goes_to_the_next_line() {
    assert_eq!(container_height("<img>&nbsp;"), IMAGE_LINE + TEXT_LINE);
}

#[test]
fn a_no_break_space_before_an_image_stays_on_the_line_above() {
    let body = "<div style='width: 0'>&nbsp;<img id=a></div>";
    assert_eq!(top_of(body, "a"), 8.0 + TEXT_LINE);
    assert_eq!(container_height("&nbsp;<img>"), IMAGE_LINE + TEXT_LINE);
}

#[test]
fn punctuation_after_an_image_goes_to_the_next_line() {
    // `:` takes no line start (IS) and `)` no line start either (CP).
    for text in [":", ")", "!"] {
        assert_eq!(
            container_height(&format!("<img>{text}")),
            IMAGE_LINE + TEXT_LINE,
            "{text}"
        );
    }
}

#[test]
fn punctuation_before_an_image_stays_on_the_line_above() {
    // `(` takes no line end (OP) and `:` no line start (IS).
    for text in ["(", ":"] {
        assert_eq!(
            container_height(&format!("{text}<img>")),
            IMAGE_LINE + TEXT_LINE,
            "{text}"
        );
    }
}

#[test]
fn an_isolate_around_the_punctuation_does_not_hide_it_from_the_image() {
    // The engine puts bidi controls around the isolate; the colon beyond
    // them is still the neighbour of the image.
    for markup in [
        "<img><span dir=rtl>:</span>",
        "<img><bdi>:</bdi>",
        "<span dir=rtl>:</span><img>",
        "<bdi>:</bdi><img>",
    ] {
        assert_eq!(container_height(markup), IMAGE_LINE + TEXT_LINE, "{markup}");
    }
}

#[test]
fn a_forced_break_after_an_image_stays_with_it() {
    // The break the author wrote is the only one: the image does not end its
    // line before it as well, which would leave a line of nothing.
    for (markup, lines) in [
        ("<img><br>X", IMAGE_LINE + TEXT_LINE),
        ("<img><br>", IMAGE_LINE),
        ("X<br><img>", TEXT_LINE + IMAGE_LINE),
    ] {
        assert_eq!(container_height(markup), lines, "{markup}");
    }
    // The same for a preserved newline.
    for white_space in ["pre-line", "pre-wrap"] {
        let body = format!("<div id=d style='width: 0; white-space: {white_space}'><img>\nX</div>");
        let found = boxes(&body);
        let (_, rect) = found
            .iter()
            .find(|(id, _)| id == "d")
            .expect("the container");
        assert_eq!(rect[3], IMAGE_LINE + TEXT_LINE, "{white_space}");
    }
}

#[test]
fn glue_and_joiners_keep_an_image_with_them() {
    // The images do not break from the narrow no-break space, the word joiner
    // or the zero width joiner, so a line holds both and overflows.
    for text in ["&#8239;", "&#8288;", "&#8205;"] {
        let body = format!("<div style='width: 0'><img id=a>{text}<img id=c></div>");
        assert_eq!(top_of(&body, "c"), top_of(&body, "a"), "{text}");
    }
}

#[test]
fn a_space_after_an_image_hangs_on_the_line_of_the_image() {
    let body = "<div style='width: 0'><img id=a> <img id=c></div>";
    assert_eq!(top_of(body, "c"), 8.0 + IMAGE_LINE);
}

#[test]
fn an_out_of_flow_box_beside_text_does_not_open_a_break_in_it() {
    // An empty inline-block is an atomic, so the paragraph takes the new
    // opportunities; it sits alone on the first line. The other box takes no
    // room on the line and is no atomic, so `AB` stays with the colon after
    // it: the second line is the only other line.
    for style in ["position: absolute", "float: left"] {
        let body = format!(
            "<div id=d style='width: 20px'><span style='display: inline-block'></span><br>\
             AB<span style='{style}'></span>:</div>"
        );
        let found = boxes(&body);
        let (_, rect) = found
            .iter()
            .find(|(id, _)| id == "d")
            .expect("the container");
        assert_eq!(rect[3], 2.0 * TEXT_LINE, "{style}");
    }
}
