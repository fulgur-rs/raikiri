use super::*;
use shodo::font::FontQuery;
use shodo::style::{FontFamily, GenericFamily};

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));
const CANVAS_TEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/CanvasTest-nospace.ttf"
));

fn named(family: &str) -> FontQuery {
    FontQuery {
        families: vec![FontFamily::Named(family.to_owned())],
        ..FontQuery::default()
    }
}

fn generic(generic: GenericFamily) -> FontQuery {
    FontQuery {
        families: vec![FontFamily::Generic(generic)],
        ..FontQuery::default()
    }
}

fn face(family: &str, bytes: &[u8]) -> BundledFace {
    BundledFace {
        family: family.to_owned(),
        bytes: bytes.to_vec(),
    }
}

fn font_dir(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temporary font directory");
    for (name, bytes) in files {
        std::fs::write(dir.path().join(name), bytes).expect("write font");
    }
    dir
}

#[test]
fn family_name_reads_the_name_table() {
    assert_eq!(family_name(AHEM).as_deref(), Some("Ahem"));
    assert_eq!(
        family_name(CANVAS_TEST).as_deref(),
        Some("CanvasTestNoSpace")
    );
    assert_eq!(family_name(b"not a font"), None);
}

#[test]
fn bundled_collection_matches_the_authored_family() {
    let fonts = bundled_collection(&Limits::default(), vec![face("Bundled Test", AHEM)], false)
        .expect("bundle registers");
    let matched = fonts
        .match_cluster(&named("Bundled Test"), "a")
        .expect("the authored family matches");
    assert!(fonts.font_data(matched.id).is_some());
}

#[test]
fn generic_families_follow_registration_order() {
    let fonts = bundled_collection(
        &Limits::default(),
        vec![face("First", AHEM), face("Second", CANVAS_TEST)],
        false,
    )
    .expect("bundle registers");
    let first = fonts.match_cluster(&named("First"), "a").expect("first");
    for kind in [
        GenericFamily::Serif,
        GenericFamily::SansSerif,
        GenericFamily::Monospace,
        GenericFamily::SystemUi,
        GenericFamily::Cursive,
        GenericFamily::Fantasy,
    ] {
        let matched = fonts.match_cluster(&generic(kind), "a").expect("generic");
        assert_eq!(matched.id, first.id, "{kind:?}");
    }
}

#[test]
fn registering_the_same_family_twice_keeps_registration_and_generics_working() {
    let fonts = bundled_collection(
        &Limits::default(),
        vec![face("Same", AHEM), face("Same", CANVAS_TEST)],
        false,
    )
    .expect("bundle registers");
    assert!(fonts.match_cluster(&named("Same"), "a").is_some());
    assert!(
        fonts
            .match_cluster(&generic(GenericFamily::Serif), "a")
            .is_some()
    );
}

#[test]
fn an_empty_bundle_is_rejected() {
    let error = bundled_collection(&Limits::default(), Vec::new(), false).expect_err("no faces");
    assert!(matches!(error, shodo::font::FontError::Malformed(_)));
}

#[test]
fn garbage_bytes_are_rejected_without_registering() {
    let error = bundled_collection(
        &Limits::default(),
        vec![face("Bad", b"definitely not a font")],
        false,
    )
    .expect_err("garbage");
    assert!(matches!(error, shodo::font::FontError::Malformed(_)));
}

#[test]
fn wpt_collection_puts_ahem_first_and_maps_generics_onto_the_bundle() {
    let dir = font_dir(&[("CanvasTest-nospace.ttf", CANVAS_TEST), ("Ahem.ttf", AHEM)]);
    let fonts = wpt_collection(dir.path(), &Limits::default(), None).expect("wpt fonts");
    let ahem = fonts.match_cluster(&named("Ahem"), "a").expect("ahem");
    let serif = fonts
        .match_cluster(&generic(GenericFamily::Serif), "a")
        .expect("serif");
    assert_eq!(serif.id, ahem.id);
}

#[test]
fn wpt_collection_skips_a_broken_font_but_keeps_the_rest() {
    let dir = font_dir(&[("Ahem.ttf", AHEM), ("Broken.ttf", b"not a font")]);
    let fonts = wpt_collection(dir.path(), &Limits::default(), None).expect("wpt fonts");
    assert!(fonts.match_cluster(&named("Ahem"), "a").is_some());
}

#[test]
fn wpt_collection_reports_a_missing_directory() {
    let dir = tempfile::tempdir().expect("temporary directory");
    let missing = dir.path().join("does-not-exist");
    let error = wpt_collection(&missing, &Limits::default(), None).expect_err("missing");
    assert!(matches!(error, crate::fonts::FontError::DirNotFound(_)));
}

#[test]
fn wpt_collection_requires_ahem() {
    let dir = font_dir(&[("CanvasTest-nospace.ttf", CANVAS_TEST)]);
    let error = wpt_collection(dir.path(), &Limits::default(), None).expect_err("no ahem");
    assert!(matches!(
        error,
        crate::fonts::FontError::PreferredFontUnavailable { .. }
    ));
}
