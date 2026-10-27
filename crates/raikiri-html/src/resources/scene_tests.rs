//! SVG images as vector scenes: what a resource set answers to
//! `ImagePixelSource::get_vector`, and how it keeps the scenes it built.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use raikiri_traits::vector::{Node, Path, VectorImage};
use raikiri_traits::{
    ImagePixelSource, ImageRasterSize, ImageVectorRequest, ResourceKind, ResourcePolicy,
};
use url::Url;

use super::image_cache::{DecodedImageCache, ProcessImageBudget};
use super::{CachedBackgroundSource, RenderResources};
use crate::fetch::url_without_fragment;

/// A policy that allows everything but one host, with its own decode time.
struct Policy {
    denied_host: Option<&'static str>,
    decode_timeout: Duration,
}

impl ResourcePolicy for Policy {
    fn is_scheme_allowed(&self, _scheme: &str, _kind: ResourceKind) -> bool {
        true
    }
    fn is_host_allowed(&self, host: &str, _kind: ResourceKind) -> bool {
        self.denied_host != Some(host)
    }
    fn allow_redirect(&self, _from: &Url, _to: &Url, _hop: u32) -> bool {
        true
    }
    fn max_redirect_hops(&self, _kind: ResourceKind) -> u32 {
        10
    }
    fn max_fetch_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn max_decoded_bytes(&self, _kind: ResourceKind) -> Option<u64> {
        None
    }
    fn fetch_timeout(&self, _kind: ResourceKind) -> Duration {
        Duration::from_secs(10)
    }
    fn decode_timeout(&self, _kind: ResourceKind) -> Duration {
        self.decode_timeout
    }
    fn allowed_mime_types(&self, _kind: ResourceKind) -> Vec<String> {
        Vec::new()
    }
    fn max_import_depth(&self) -> u32 {
        10
    }
    fn max_svg_recursion_depth(&self) -> u32 {
        10
    }
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

const RED_SQUARE: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1' viewBox='0 0 2 1'><rect width='1' height='1' fill='red'/></svg>";

/// A mask is outside the vector subset, so the picture has to be pixels.
const MASKED_SQUARE: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1'><mask id='m'><rect width='1' height='1' fill='white'/></mask><rect width='2' height='1' fill='red' mask='url(#m)'/></svg>";

fn size(width: f32, height: f32) -> ImageRasterSize {
    ImageRasterSize { width, height }
}

fn request(width: f32, height: f32) -> ImageVectorRequest {
    ImageVectorRequest::new(size(width, height))
}

/// The percent-encoded `data:` URL of an SVG document.
fn svg_url(svg: &str) -> Url {
    let mut encoded = String::from("data:image/svg+xml,");
    for byte in svg.bytes() {
        match byte {
            b'<' | b'>' | b'#' | b' ' | b'"' => encoded.push_str(&format!("%{byte:02X}")),
            _ => encoded.push(char::from(byte)),
        }
    }
    Url::parse(&encoded).expect("a data URL")
}

/// The URLs of `images` after the page's backgrounds have been preloaded into
/// `resources`, as layout does.
fn preload(resources: &RenderResources<'_>, images: &[Url]) {
    let rules: String = images
        .iter()
        .enumerate()
        .map(|(at, url)| format!(".i{at}{{background-image:url(\"{url}\")}}"))
        .collect();
    let divs: String = (0..images.len())
        .map(|at| format!("<div class=i{at}></div>"))
        .collect();
    let html = format!("<!doctype html><style>{rules}</style><body>{divs}</body>");
    let options = crate::types::ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = crate::parse(html.as_bytes(), &options).expect("HTML parses");
    let cascade = crate::build_cascaded(&uncascaded).expect("the cascade succeeds");
    let warnings = Arc::new(Mutex::new(Vec::new()));
    let mut seen = HashSet::new();
    let mut attempts = 0;
    resources.preload_background_images(&cascade, &warnings, &mut seen, &mut attempts, None);
    assert!(warnings.lock().unwrap().is_empty(), "the images preload");
}

fn text_runs(scene: &VectorImage) -> usize {
    scene
        .root
        .children
        .iter()
        .filter(|node| matches!(node, Node::Text(_)))
        .count()
}

#[test]
fn an_svg_image_is_a_scene_built_for_the_box_it_is_asked_for() {
    let url = svg_url(RED_SQUARE);
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));

    let scene = resources
        .get_vector(&url, &request(8.0, 4.0))
        .expect("the SVG has a scene");
    assert_eq!(scene.size, (8.0, 4.0));
    assert!(scene.node_count() > 1, "the square is in the scene");
}

#[test]
fn an_svg_image_keeps_the_root_opacity_of_its_document() {
    let url = svg_url(
        "<svg xmlns='http://www.w3.org/2000/svg' width='2' height='1' opacity='0.5'><rect width='1' height='1' fill='red'/></svg>",
    );
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));

    let scene = resources.get_vector(&url, &request(8.0, 4.0)).unwrap();
    let dimmed = scene
        .root
        .children
        .iter()
        .any(|node| matches!(node, Node::Group(group) if group.opacity == 0.5));
    assert!(dimmed, "the picture is half transparent: {scene:?}");
}

#[test]
fn a_box_gets_one_scene_however_often_it_is_asked_for() {
    let url = svg_url(RED_SQUARE);
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));

    let first = resources.get_vector(&url, &request(8.0, 4.0)).unwrap();
    let again = resources.get_vector(&url, &request(8.0, 4.0)).unwrap();
    let other = resources.get_vector(&url, &request(4.0, 2.0)).unwrap();
    assert!(Arc::ptr_eq(&first, &again));
    assert!(!Arc::ptr_eq(&first, &other));
    assert_eq!(other.size, (4.0, 2.0));
    // The first box is still kept next to the second.
    assert!(Arc::ptr_eq(
        &first,
        &resources.get_vector(&url, &request(8.0, 4.0)).unwrap()
    ));
}

#[test]
fn a_scene_built_while_this_thread_waited_for_a_slot_is_used() {
    let url = svg_url(RED_SQUARE);
    // One decode slot, held while the scene is being asked for.
    let budget = Arc::new(ProcessImageBudget::new(1 << 30, 1));
    let resources = RenderResources::new().with_test_process_budget(Arc::clone(&budget));
    preload(&resources, std::slice::from_ref(&url));
    let held = budget.try_acquire().expect("the only slot");

    let (got, built) = std::thread::scope(|scope| {
        let waiter = scope.spawn(|| resources.get_vector(&url, &request(8.0, 4.0)));
        // Nothing is cached yet, so the thread waits for the slot.
        std::thread::sleep(Duration::from_millis(100));
        let built = scene_of(1);
        resources.cache_image_scene(
            url_without_fragment(&url),
            size(8.0, 4.0),
            Arc::clone(&built),
        );
        drop(held);
        (waiter.join().expect("the waiter finishes"), built)
    });
    assert!(Arc::ptr_eq(&got.expect("a scene"), &built));
}

#[test]
fn a_fragment_does_not_name_another_image() {
    let url = svg_url(RED_SQUARE);
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));

    let mut with_fragment = url.clone();
    with_fragment.set_fragment(Some("view"));
    let plain = resources.get_vector(&url, &request(8.0, 4.0)).unwrap();
    let viewed = resources
        .get_vector(&with_fragment, &request(8.0, 4.0))
        .unwrap();
    assert!(Arc::ptr_eq(&plain, &viewed));
}

#[test]
fn a_box_that_is_no_size_has_no_scene() {
    let url = svg_url(RED_SQUARE);
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));

    for (width, height) in [
        (0.0, 4.0),
        (4.0, -1.0),
        (f32::NAN, 4.0),
        (4.0, f32::INFINITY),
    ] {
        assert!(
            resources
                .get_vector(&url, &request(width, height))
                .is_none(),
            "{width} x {height}"
        );
    }
}

#[test]
fn a_raster_image_has_no_scene() {
    let mut png = Vec::new();
    image::RgbaImage::from_raw(1, 1, vec![255, 0, 0, 255])
        .expect("one pixel")
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("PNG encodes");
    let encoded: String = png.iter().map(|byte| format!("%{byte:02X}")).collect();
    let url = Url::parse(&format!("data:image/png,{encoded}")).unwrap();
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));

    assert!(
        resources.get_decoded(&url).is_some(),
        "the PNG was preloaded"
    );
    assert!(resources.get_vector(&url, &request(8.0, 4.0)).is_none());
}

#[test]
fn an_svg_outside_the_vector_subset_is_left_to_the_raster_path() {
    let document = raikiri_svg::SvgDocument::parse(MASKED_SQUARE.as_bytes()).unwrap();
    let error = document
        .scene(
            raikiri_svg::SvgViewport {
                width: 2.0,
                height: 1.0,
            },
            raikiri_svg::SvgRootStyle::default(),
        )
        .unwrap_err();
    assert!(error.falls_back_to_raster(), "the premise of this test");

    let url = svg_url(MASKED_SQUARE);
    let resources = RenderResources::new();
    preload(&resources, std::slice::from_ref(&url));
    assert!(resources.get_vector(&url, &request(2.0, 1.0)).is_none());
    assert!(
        resources
            .get_decoded_at_size(&url, size(2.0, 1.0), None)
            .is_some(),
        "the same image is still drawn as pixels"
    );
    // It is remembered, so no box asks the builder again.
    let canonical = url_without_fragment(&url);
    assert!(resources.is_raster_only(&canonical));
    assert!(resources.get_vector(&url, &request(4.0, 2.0)).is_none());
}

#[test]
fn an_unknown_image_is_asked_of_the_source_the_resources_wrap() {
    struct Wrapped;
    impl ImagePixelSource for Wrapped {
        fn get_decoded(&self, _url: &Url) -> Option<Arc<raikiri_traits::DecodedImage>> {
            None
        }

        fn get_vector(&self, _url: &Url, request: &ImageVectorRequest) -> Option<Arc<VectorImage>> {
            Some(Arc::new(VectorImage::new((
                request.size.width,
                request.size.height,
            ))))
        }
    }

    let url = Url::parse("https://pictures.test/wrapped.svg").unwrap();
    let wrapped = Wrapped;
    let resources = RenderResources::new().image_pixel_source(&wrapped);
    assert_eq!(
        resources.get_vector(&url, &request(3.0, 2.0)).unwrap().size,
        (3.0, 2.0)
    );

    // The policy of the resources still decides which URLs may be shown.
    let policy = Policy {
        denied_host: Some("pictures.test"),
        decode_timeout: Duration::from_secs(10),
    };
    let resources = RenderResources::new()
        .image_pixel_source(&wrapped)
        .network_policy(&policy);
    assert!(resources.get_vector(&url, &request(3.0, 2.0)).is_none());
}

#[test]
fn a_zero_decode_budget_builds_no_scene() {
    let policy = Policy {
        denied_host: None,
        decode_timeout: Duration::ZERO,
    };
    // The policy is set first: setting it later would drop what was cached.
    let resources = RenderResources::new().network_policy(&policy);
    let url = svg_url(RED_SQUARE);
    let document = raikiri_svg::SvgDocument::parse(RED_SQUARE.as_bytes()).unwrap();
    resources
        .cache_image_source(url.clone(), CachedBackgroundSource::Svg(Arc::new(document)))
        .unwrap();

    assert!(resources.get_vector(&url, &request(8.0, 4.0)).is_none());
    assert!(
        resources.cached_scene(&url, size(8.0, 4.0)).is_none(),
        "a scene that took too long is not kept"
    );
}

#[test]
fn svg_text_is_set_in_the_fonts_of_the_resources() {
    let resources = RenderResources::new().fonts(ahem_fonts());
    let url = svg_url(
        "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><text x='0' y='8' font-family='Ahem' font-size='8'>A</text></svg>",
    );
    preload(&resources, std::slice::from_ref(&url));

    let small = resources.get_vector(&url, &request(16.0, 16.0)).unwrap();
    let large = resources.get_vector(&url, &request(32.0, 32.0)).unwrap();
    assert_eq!(text_runs(&small), 1);
    // Every scene of a resource set shares the face it was handed.
    assert!(first_face(&small).is_same_face(&first_face(&large)));
}

fn ahem_fonts() -> crate::RenderFonts {
    crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts")
}

#[test]
fn a_clone_given_other_fonts_shares_no_scene_with_the_original() {
    let original = RenderResources::new().fonts(ahem_fonts());
    let url = svg_url(
        "<svg xmlns='http://www.w3.org/2000/svg' width='16' height='16'><text x='0' y='8' font-family='Ahem' font-size='8'>A</text></svg>",
    );
    preload(&original, std::slice::from_ref(&url));
    let built = original.get_vector(&url, &request(16.0, 16.0)).unwrap();

    let clone = original.clone().fonts(ahem_fonts());
    // The clone starts from nothing: it does not see the scene built in the
    // fonts it replaced.
    assert!(clone.get_vector(&url, &request(16.0, 16.0)).is_none());
    preload(&clone, std::slice::from_ref(&url));
    let rebuilt = clone.get_vector(&url, &request(16.0, 16.0)).unwrap();
    assert!(!Arc::ptr_eq(&built, &rebuilt));
    assert!(!first_face(&built).is_same_face(&first_face(&rebuilt)));
    // The original keeps its scene and its faces.
    let again = original.get_vector(&url, &request(16.0, 16.0)).unwrap();
    assert!(Arc::ptr_eq(&built, &again));
}

/// The face of the first text run of `scene`.
fn first_face(scene: &VectorImage) -> raikiri_traits::vector::FontFace {
    match scene.root.children.iter().find_map(|node| match node {
        Node::Text(run) => run.glyphs.first().map(|glyph| glyph.font.clone()),
        _ => None,
    }) {
        Some(face) => face,
        None => panic!("the scene holds no text: {scene:?}"),
    }
}

// The scene cache.

/// A scene with `children` empty paths, whose heap size grows with them.
fn scene_of(children: usize) -> Arc<VectorImage> {
    let mut scene = VectorImage::new((1.0, 1.0));
    scene
        .root
        .children
        .extend((0..children).map(|_| Node::Path(Path::new(Vec::new()))));
    Arc::new(scene)
}

fn bytes_of(scene: &VectorImage) -> u64 {
    DecodedImageCache::scene_bytes(&url("a"), scene)
}

fn url(name: &str) -> Url {
    Url::parse(&format!("https://pictures.test/{name}.svg")).unwrap()
}

fn cache_with(max_retained: u64) -> (DecodedImageCache, Arc<ProcessImageBudget>) {
    let budget = Arc::new(ProcessImageBudget::new(max_retained, 1));
    (DecodedImageCache::new(Arc::clone(&budget)), budget)
}

#[test]
fn the_cache_keeps_a_scene_per_image_and_box() {
    let (mut cache, budget) = cache_with(u64::MAX);
    let scene = scene_of(2);
    cache.insert_scene(url("a"), size(1.0, 1.0), Arc::clone(&scene));
    assert!(Arc::ptr_eq(
        &cache.scene(&url("a"), size(1.0, 1.0)).unwrap(),
        &scene
    ));
    assert!(cache.scene(&url("a"), size(2.0, 1.0)).is_none());
    assert!(cache.scene(&url("b"), size(1.0, 1.0)).is_none());
    assert_eq!(budget.retained(), bytes_of(&scene));
}

#[test]
fn a_scene_built_again_replaces_the_one_before_without_being_charged_twice() {
    let (mut cache, budget) = cache_with(u64::MAX);
    let first = scene_of(2);
    let second = scene_of(2);
    cache.insert_scene(url("a"), size(1.0, 1.0), Arc::clone(&first));
    cache.insert_scene(url("a"), size(1.0, 1.0), Arc::clone(&second));
    assert!(Arc::ptr_eq(
        &cache.scene(&url("a"), size(1.0, 1.0)).unwrap(),
        &second
    ));
    assert_eq!(budget.retained(), bytes_of(&second));
}

#[test]
fn the_oldest_scenes_make_room_when_the_set_is_out_of_bytes() {
    let (mut cache, budget) = cache_with(u64::MAX);
    let each = bytes_of(&scene_of(1));
    cache.max_bytes = each * 2;
    cache.insert_scene(url("a"), size(1.0, 1.0), scene_of(1));
    cache.insert_scene(url("b"), size(1.0, 1.0), scene_of(1));
    cache.insert_scene(url("c"), size(1.0, 1.0), scene_of(1));
    assert!(cache.scene(&url("a"), size(1.0, 1.0)).is_none());
    assert!(cache.scene(&url("b"), size(1.0, 1.0)).is_some());
    assert!(cache.scene(&url("c"), size(1.0, 1.0)).is_some());
    assert_eq!(budget.retained(), each * 2);
}

#[test]
fn a_scene_larger_than_the_set_is_not_kept_and_costs_nothing() {
    let (mut cache, budget) = cache_with(u64::MAX);
    let small = scene_of(1);
    let large = scene_of(64);
    cache.max_bytes = bytes_of(&small) + 1;
    cache.insert_scene(url("a"), size(1.0, 1.0), Arc::clone(&small));
    cache.insert_scene(url("b"), size(1.0, 1.0), large);
    assert!(cache.scene(&url("b"), size(1.0, 1.0)).is_none());
    // It did not push out the one that fits.
    assert!(cache.scene(&url("a"), size(1.0, 1.0)).is_some());
    assert_eq!(budget.retained(), bytes_of(&small));
}

#[test]
fn a_scene_the_process_has_no_room_for_is_not_kept() {
    let (mut cache, budget) = cache_with(0);
    cache.insert_scene(url("a"), size(1.0, 1.0), scene_of(1));
    assert!(cache.scene(&url("a"), size(1.0, 1.0)).is_none());
    assert_eq!(budget.retained(), 0);
}

#[test]
fn scenes_count_against_the_room_left_for_rasters() {
    let (mut cache, _budget) = cache_with(u64::MAX);
    let scene = scene_of(2);
    let before = cache.remaining_for(&url("a"));
    cache.insert_scene(url("b"), size(1.0, 1.0), Arc::clone(&scene));
    assert_eq!(cache.remaining_for(&url("a")), before - bytes_of(&scene));
}

#[test]
fn a_source_cached_again_drops_the_scenes_of_the_one_before() {
    let (mut cache, budget) = cache_with(u64::MAX);
    cache.insert_scene(url("a"), size(1.0, 1.0), scene_of(1));
    cache.insert_scene(url("b"), size(1.0, 1.0), scene_of(1));
    let document = raikiri_svg::SvgDocument::parse(RED_SQUARE.as_bytes()).unwrap();
    cache
        .insert_source(url("a"), CachedBackgroundSource::Svg(Arc::new(document)))
        .unwrap();
    assert!(cache.scene(&url("a"), size(1.0, 1.0)).is_none());
    assert!(cache.scene(&url("b"), size(1.0, 1.0)).is_some());
    assert_eq!(
        budget.retained(),
        cache.decoded_bytes + bytes_of(&scene_of(1))
    );
}

#[test]
fn what_is_learned_of_an_image_is_forgotten_when_its_source_is_cached_again() {
    let (mut cache, _budget) = cache_with(u64::MAX);
    cache.mark_raster_only(url("a"));
    assert!(cache.is_raster_only(&url("a")));
    assert!(!cache.is_raster_only(&url("b")));
    let document = raikiri_svg::SvgDocument::parse(RED_SQUARE.as_bytes()).unwrap();
    cache
        .insert_source(url("a"), CachedBackgroundSource::Svg(Arc::new(document)))
        .unwrap();
    assert!(!cache.is_raster_only(&url("a")));
}

#[test]
fn dropping_the_cache_gives_the_scene_bytes_back() {
    let (mut cache, budget) = cache_with(u64::MAX);
    cache.insert_scene(url("a"), size(1.0, 1.0), scene_of(3));
    assert!(budget.retained() > 0);
    drop(cache);
    assert_eq!(budget.retained(), 0);
}

#[test]
fn svg_sources_charge_the_document_and_process_cache_budgets() {
    let budget = Arc::new(ProcessImageBudget::new(u64::MAX, 4));
    let resources = RenderResources::new().with_test_process_budget(Arc::clone(&budget));
    let warning = resources.decode_svg_background(
        url("source.svg"),
        RED_SQUARE.as_bytes(),
        std::time::Instant::now(),
    );
    assert!(warning.is_none());
    let bytes = resources.image_cache.lock().unwrap().decoded_bytes;
    assert!(bytes >= RED_SQUARE.len() as u64);
    assert_eq!(budget.retained(), bytes);
    drop(resources);
    assert_eq!(budget.retained(), 0);
}

#[test]
fn svg_source_admission_respects_both_cache_limits() {
    for process_limited in [false, true] {
        let budget = Arc::new(ProcessImageBudget::new(
            if process_limited { 1 } else { u64::MAX },
            4,
        ));
        let resources = RenderResources::new().with_test_process_budget(Arc::clone(&budget));
        if !process_limited {
            resources.image_cache.lock().unwrap().max_bytes = 1;
        }
        let warning = resources.decode_svg_background(
            url("denied.svg"),
            RED_SQUARE.as_bytes(),
            std::time::Instant::now(),
        );
        assert!(warning.is_some());
        assert!(
            resources
                .cached_background_image(&url("denied.svg"))
                .is_none()
        );
        assert_eq!(budget.retained(), 0);
    }
}

#[test]
fn shared_svg_sources_are_charged_once_and_failed_replacement_keeps_the_old_source() {
    let (mut cache, budget) = cache_with(u64::MAX);
    let document = Arc::new(raikiri_svg::SvgDocument::parse(RED_SQUARE.as_bytes()).unwrap())
        .into_source_only();
    let allocation = document.source_only_retained_bytes().unwrap();
    cache
        .insert_source(url("a"), CachedBackgroundSource::Svg(Arc::clone(&document)))
        .unwrap();
    let first = cache.decoded_bytes;
    cache
        .insert_source(url("b"), CachedBackgroundSource::Svg(Arc::clone(&document)))
        .unwrap();
    assert_eq!(cache.decoded_bytes, first * 2 - allocation);
    assert_eq!(budget.retained(), cache.decoded_bytes);
    cache.max_bytes = 1;
    let replacement = Arc::new(raikiri_svg::SvgDocument::parse(RED_SQUARE.as_bytes()).unwrap());
    assert!(
        cache
            .insert_source(url("a"), CachedBackgroundSource::Svg(replacement))
            .is_err()
    );
    let CachedBackgroundSource::Svg(retained) = &cache.entries[&url("a")].source else {
        panic!()
    };
    assert!(Arc::ptr_eq(retained, &document));
    assert_eq!(budget.retained(), first * 2 - allocation);
    drop(cache);
    assert_eq!(budget.retained(), 0);
    assert!(document.source_only_retained_bytes().is_some());
}

#[test]
fn sixteen_seventeen_and_thirty_two_small_svg_scenes_survive_repeated_cycles() {
    for count in [16, 17, 32] {
        let images: Vec<_> = (0..count)
            .map(|at| svg_url(&RED_SQUARE.replace("red", &format!("rgb({at},0,0)"))))
            .collect();
        let resources = RenderResources::new();
        preload(&resources, &images);
        let first: Vec<_> = images
            .iter()
            .map(|url| resources.get_vector(url, &request(8.0, 4.0)).unwrap())
            .collect();
        for _ in 0..3 {
            for (url, original) in images.iter().zip(&first) {
                assert!(
                    Arc::ptr_eq(
                        original,
                        &resources.get_vector(url, &request(8.0, 4.0)).unwrap()
                    ),
                    "scene cycle {count}"
                );
            }
        }
    }
}

#[test]
fn empty_scene_metadata_and_url_storage_are_bounded_by_the_byte_budget() {
    let (mut cache, budget) = cache_with(u64::MAX);
    let each = bytes_of(&scene_of(0));
    assert!(each > 0);
    cache.max_bytes = each * 2;
    for name in ["a", "b", "c"] {
        cache.insert_scene(url(name), size(1.0, 1.0), scene_of(0));
    }
    assert!(cache.scene(&url("a"), size(1.0, 1.0)).is_none());
    assert!(cache.scene(&url("b"), size(1.0, 1.0)).is_some());
    assert!(cache.scene(&url("c"), size(1.0, 1.0)).is_some());
    assert_eq!(budget.retained(), each * 2);
    assert!(DecodedImageCache::scene_bytes(&url("a-longer-url"), &scene_of(0)) > each);
    drop(cache);
    assert_eq!(budget.retained(), 0);
}

#[test]
fn hot_svg_scenes_survive_one_shot_images_while_the_byte_budget_has_room() {
    let images: Vec<_> = (0..40)
        .map(|at| svg_url(&RED_SQUARE.replace("red", &format!("rgb({at},0,0)"))))
        .collect();
    let resources = RenderResources::new();
    preload(&resources, &images);
    let hot: Vec<_> = images[..8]
        .iter()
        .map(|url| resources.get_vector(url, &request(8.0, 4.0)).unwrap())
        .collect();
    for one_shot in &images[8..] {
        resources.get_vector(one_shot, &request(8.0, 4.0)).unwrap();
        for (url, original) in images[..8].iter().zip(&hot) {
            assert!(Arc::ptr_eq(
                original,
                &resources.get_vector(url, &request(8.0, 4.0)).unwrap()
            ));
        }
    }
}
