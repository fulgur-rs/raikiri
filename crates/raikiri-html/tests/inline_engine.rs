//! Which engine lays out the paragraphs of a document laid out through the
//! public entry points.
//!
//! The fixtures use an input the two engines lay out differently: two
//! consecutive `<br>` leave an empty line on the inline engine (30px at
//! `line-height:10px`) and none on the parley path (20px).

use std::sync::Mutex;

use raikiri_html::{
    FontContextBuilder, FragmentKind, LayoutConfig, LayoutOptions, LayoutStatus, PageDefaults,
    RenderFonts, RenderResources, ResourceLimits, layout, parse_html_with_resources,
};
use raikiri_traits::{FetchOutcome, FetchedResource, NetworkError, NetworkProvider, Request};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const BREAKS: &str = r#"<html><body style="margin:0"><div id="box" style="font-family:Ahem;font-size:10px;line-height:10px;width:200px">aaaa<br><br>bbbb</div></body></html>"#;

const FACE_URL: &str = "https://example.test/ahem.ttf";

/// A document whose only font is an `@font-face` family served by
/// [`FontServer`]. `#box` tells the engines apart; `#wrap` wraps into four
/// lines only with Ahem's 1em advances ("aa aa" is 50px in a 40px box), so
/// it also tells whether the face was used.
const FONT_FACE_PAGE: &str = r#"<html><head><style>@font-face{font-family:Boxy;src:url(https://example.test/ahem.ttf)}</style></head><body style="margin:0"><div id="box" style="font-family:Boxy;font-size:10px;line-height:10px;width:200px">aaaa<br><br>bbbb</div><div id="wrap" style="font-family:Boxy;font-size:10px;line-height:10px;width:40px">aa aa aa aa</div></body></html>"#;

fn ahem_fonts() -> RenderFonts {
    FontContextBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build_fonts()
        .expect("fonts")
}

/// Height of the border box of the element with `id` on the first page.
fn box_height(html: &str, resources: &RenderResources<'_>, id: &str) -> f32 {
    box_heights(html, resources, &[id])[0]
}

/// Heights of the border boxes of the elements with `ids` on the first page
/// of one layout.
fn box_heights(html: &str, resources: &RenderResources<'_>, ids: &[&str]) -> Vec<f32> {
    let document = parse_html_with_resources(html.as_bytes(), resources).expect("parse");
    let status = layout(
        &document,
        PageDefaults::default(),
        LayoutConfig::default(),
        LayoutOptions::new().resources(resources),
    )
    .expect("layout");
    let LayoutStatus::Completed(document_layout) = status else {
        panic!("expected a complete layout");
    };
    let page = document_layout.page(0).expect("a first page");
    let dom = page.dom();
    ids.iter()
        .map(|id| {
            let mut stack = vec![dom.root()];
            let mut target = None;
            while let Some(node) = stack.pop() {
                if dom.attr(node, "id") == Some(*id) {
                    target = Some(node);
                    break;
                }
                stack.extend(dom.children(node));
            }
            let target = target.unwrap_or_else(|| panic!("an element with id {id}"));
            page.fragments()
                .find(|fragment| fragment.kind() == FragmentKind::Box && fragment.node() == target)
                .map(|fragment| fragment.rect().height)
                .unwrap_or_else(|| panic!("a box fragment for #{id}"))
        })
        .collect()
}

#[test]
fn the_engine_lays_out_consecutive_breaks_with_the_empty_line() {
    let resources = RenderResources::new().fonts(ahem_fonts());
    assert_eq!(box_height(BREAKS, &resources, "box"), 30.0);
}

#[test]
fn switching_the_engine_off_gives_the_parley_height() {
    let resources = RenderResources::new()
        .fonts(ahem_fonts())
        .inline_formatting(false);
    assert_eq!(box_height(BREAKS, &resources, "box"), 20.0);
}

#[test]
fn a_caller_built_font_context_keeps_the_parley_path() {
    let context = FontContextBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("context");
    let resources = RenderResources::new().font_context(context);
    assert_eq!(box_height(BREAKS, &resources, "box"), 20.0);
}

#[test]
fn the_default_resources_use_the_engine() {
    // The installed fonts give the line its height from `line-height`, so
    // the empty line shows whichever face is chosen.
    let resources = RenderResources::new();
    assert_eq!(box_height(BREAKS, &resources, "box"), 30.0);
}

/// Serves Ahem for [`FACE_URL`] and counts the requests per URL.
#[derive(Default)]
struct FontServer {
    requests: Mutex<Vec<String>>,
}

impl FontServer {
    fn fetch_count(&self, url: &str) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|requested| requested.as_str() == url)
            .count()
    }
}

impl NetworkProvider for FontServer {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        self.requests.lock().unwrap().push(request.url.to_string());
        if request.url.as_str() != FACE_URL {
            return Err(NetworkError::Aborted);
        }
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: AHEM.to_vec().into(),
            content_type: Some("font/ttf".into()),
            final_url: request.url,
            encoding: None,
        }))
    }
}

#[test]
fn an_at_font_face_family_is_available_to_the_engine() {
    let server = FontServer::default();
    let resources = RenderResources::new().network_provider(&server);
    assert_eq!(
        box_heights(FONT_FACE_PAGE, &resources, &["box", "wrap"]),
        [30.0, 40.0]
    );
}

#[test]
fn an_at_font_face_source_is_fetched_once_per_layout() {
    let server = FontServer::default();
    let resources = RenderResources::new().network_provider(&server);
    let _ = box_height(FONT_FACE_PAGE, &resources, "box");
    // One layout: the parley path and the engine share one fetch.
    assert_eq!(server.fetch_count(FACE_URL), 1);
}

#[test]
fn a_cached_font_fetch_is_not_charged_to_the_budget_twice() {
    // The aggregate budget is sized for exactly one copy of the face.
    let server = FontServer::default();
    let resources = RenderResources::new()
        .network_provider(&server)
        .resource_limits(
            ResourceLimits::new().max_aggregate_resource_bytes(Some(AHEM.len() as u64)),
        );
    // One layout, so the budget is spent once.
    assert_eq!(
        box_heights(FONT_FACE_PAGE, &resources, &["box", "wrap"]),
        [30.0, 40.0]
    );
}
