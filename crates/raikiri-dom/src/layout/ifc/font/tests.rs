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

/// Ahem sits 0.8em above and 0.2em below the baseline: 8.4px and 2.1px at
/// 10.5px, which Blink rounds to 8px and 2px.
fn assert_whole_pixel_ahem(fonts: &FontCollection) {
    let matched = fonts
        .match_cluster(&named("Ahem"), "a")
        .expect("ahem matches");
    let metrics = fonts.metrics(matched.id, 10.5);
    assert_eq!((metrics.ascent, metrics.descent), (8.0, 2.0));
}

#[test]
fn bundled_collections_report_whole_pixel_metrics() {
    let fonts = bundled_collection(&Limits::default(), vec![face("Ahem", AHEM)], false)
        .expect("bundle registers");
    assert_whole_pixel_ahem(&fonts);
}

#[test]
fn wpt_collections_report_whole_pixel_metrics() {
    let dir = font_dir(&[("Ahem.ttf", AHEM)]);
    let fonts = wpt_collection(dir.path(), &Limits::default(), None).expect("wpt fonts");
    assert_whole_pixel_ahem(&fonts);
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

/// A collection of `fonts`, each laid out after the header with its table
/// offsets moved from the start of its own file to the start of the collection.
fn ttc(fonts: &[&[u8]]) -> Vec<u8> {
    let header = 12 + 4 * fonts.len();
    let mut out = Vec::new();
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&1u16.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(fonts.len() as u32).to_be_bytes());
    let mut start = header;
    for font in fonts {
        out.extend_from_slice(&(start as u32).to_be_bytes());
        start += font.len().next_multiple_of(4);
    }
    let mut start = header as u32;
    for font in fonts {
        let mut moved = font.to_vec();
        let tables = usize::from(u16::from_be_bytes([moved[4], moved[5]]));
        for table in 0..tables {
            let at = 12 + 16 * table + 8;
            let offset = u32::from_be_bytes(moved[at..at + 4].try_into().unwrap());
            moved[at..at + 4].copy_from_slice(&(offset + start).to_be_bytes());
        }
        moved.resize(moved.len().next_multiple_of(4), 0);
        start += moved.len() as u32;
        out.extend_from_slice(&moved);
    }
    out
}

#[test]
fn the_collection_helper_builds_a_font_the_text_engine_reads_face_by_face() {
    let bytes = ttc(&[AHEM, CANVAS_TEST]);
    assert_eq!(declared_face_count(&bytes), 2);
    assert_eq!(face_family_name(&bytes, 0).as_deref(), Some("Ahem"));
    assert_eq!(
        face_family_name(&bytes, 1).as_deref(),
        Some("CanvasTestNoSpace")
    );
    assert_eq!(face_family_name(&bytes, 2), None);
}

#[test]
fn recorded_families_lists_each_face_in_order() {
    let limits = Limits::default();
    assert_eq!(
        recorded_families(&ttc(&[AHEM, CANVAS_TEST]), &limits),
        ["Ahem", "CanvasTestNoSpace"]
    );
    assert_eq!(
        recorded_families(&ttc(&[CANVAS_TEST, AHEM, CANVAS_TEST]), &limits),
        ["CanvasTestNoSpace", "Ahem", "CanvasTestNoSpace"]
    );
    assert_eq!(recorded_families(AHEM, &limits), ["Ahem"]);
    assert!(recorded_families(b"not a font", &limits).is_empty());
    assert!(recorded_families(b"", &limits).is_empty());
}

#[test]
fn recorded_families_stops_at_the_declared_count_the_file_can_hold() {
    // The header declares four billion faces in a file of 16 bytes.
    let mut hostile = b"ttcf\0\x01\0\0".to_vec();
    hostile.extend_from_slice(&u32::MAX.to_be_bytes());
    hostile.extend_from_slice(&[0; 4]);
    let unlimited = Limits {
        max_ttc_faces: None,
        ..Limits::default()
    };
    assert!(recorded_families(&hostile, &unlimited).is_empty());
    // The face limit bounds the walk of a real collection too.
    let one = Limits {
        max_ttc_faces: Some(1),
        ..Limits::default()
    };
    assert_eq!(
        recorded_families(&ttc(&[AHEM, CANVAS_TEST]), &one),
        ["Ahem"]
    );
}

#[test]
fn a_collection_registers_every_face_under_its_own_family() {
    let fonts = bundled_sources(
        &Limits::default(),
        vec![BundledSource::Collection(ttc(&[AHEM, CANVAS_TEST]))],
        false,
    )
    .expect("collection registers");
    let ahem = fonts
        .match_cluster(&named("Ahem"), "A")
        .expect("the first face answers to its family");
    let canvas = fonts
        .match_cluster(&named("CanvasTestNoSpace"), "A")
        .expect("the second face answers to its family");
    assert_ne!(ahem.id, canvas.id);
    assert_eq!(fonts.family_name(ahem.id).as_deref(), Some("Ahem"));
    assert_eq!(
        fonts.family_name(canvas.id).as_deref(),
        Some("CanvasTestNoSpace")
    );
}

#[test]
fn the_families_of_a_collection_map_onto_the_generic_families_in_face_order() {
    let fonts = bundled_sources(
        &Limits::default(),
        vec![BundledSource::Collection(ttc(&[CANVAS_TEST, AHEM]))],
        false,
    )
    .expect("collection registers");
    let canvas = fonts
        .match_cluster(&named("CanvasTestNoSpace"), "A")
        .expect("canvas");
    for generic_family in [GenericFamily::Serif, GenericFamily::Monospace] {
        let matched = fonts
            .match_cluster(&generic(generic_family), "A")
            .expect("generic family");
        assert_eq!(
            matched.id, canvas.id,
            "the first face of the collection leads {generic_family:?}"
        );
    }
}

#[test]
fn sources_register_in_order_and_a_collection_follows_what_came_before() {
    let fonts = bundled_sources(
        &Limits::default(),
        vec![
            BundledSource::Face(face("Authored", AHEM)),
            BundledSource::Collection(ttc(&[CANVAS_TEST, AHEM])),
        ],
        false,
    )
    .expect("sources register");
    let authored = fonts.match_cluster(&named("Authored"), "A").expect("face");
    let recorded = fonts
        .match_cluster(&named("Ahem"), "A")
        .expect("collection");
    assert_ne!(authored.id, recorded.id);
    let sans = fonts
        .match_cluster(&generic(GenericFamily::SansSerif), "A")
        .expect("generic family");
    assert_eq!(sans.id, authored.id, "the face bundled first leads");
}

#[test]
fn a_plain_font_as_a_collection_source_is_registered_under_its_recorded_family() {
    let fonts = bundled_sources(
        &Limits::default(),
        vec![BundledSource::Collection(AHEM.to_vec())],
        false,
    )
    .expect("font registers");
    assert!(fonts.match_cluster(&named("Ahem"), "A").is_some());
}

#[test]
fn a_collection_counts_once_against_the_blob_budget() {
    let bytes = ttc(&[AHEM, CANVAS_TEST]);
    let exactly = Limits {
        max_layer_blob_bytes: Some(bytes.len() as u64),
        ..Limits::default()
    };
    // Registering each face on its own would charge the file twice.
    bundled_sources(
        &exactly,
        vec![BundledSource::Collection(bytes.clone())],
        false,
    )
    .expect("the file is charged once");
    let short = Limits {
        max_layer_blob_bytes: Some(bytes.len() as u64 - 1),
        ..Limits::default()
    };
    let error = bundled_sources(&short, vec![BundledSource::Collection(bytes)], false)
        .expect_err("one byte short");
    assert!(matches!(
        error,
        shodo::font::FontError::Limit(exceeded)
            if exceeded.kind == shodo::limits::LimitKind::LayerBlobBytes
    ));
}

#[test]
fn a_collection_over_the_face_limit_is_refused_whole() {
    let two_faces = Limits {
        max_ttc_faces: Some(1),
        ..Limits::default()
    };
    let error = bundled_sources(
        &two_faces,
        vec![BundledSource::Collection(ttc(&[AHEM, CANVAS_TEST]))],
        false,
    )
    .expect_err("two faces over a limit of one");
    assert!(matches!(
        error,
        shodo::font::FontError::Limit(exceeded)
            if exceeded.kind == shodo::limits::LimitKind::TtcFaces
    ));
}

#[test]
fn a_collection_source_that_is_not_a_font_is_refused() {
    let error = bundled_sources(
        &Limits::default(),
        vec![BundledSource::Collection(b"not a font".to_vec())],
        false,
    )
    .expect_err("garbage");
    assert!(matches!(error, shodo::font::FontError::Malformed(_)));
    let empty = bundled_sources(&Limits::default(), Vec::new(), false).expect_err("no sources");
    assert!(matches!(empty, shodo::font::FontError::Malformed(_)));
}

#[test]
fn bundled_collection_is_the_face_only_case_of_bundled_sources() {
    let fonts = bundled_collection(
        &Limits::default(),
        vec![face("First", AHEM), face("Second", CANVAS_TEST)],
        false,
    )
    .expect("faces register");
    let first = fonts.match_cluster(&named("First"), "A").expect("first");
    let second = fonts.match_cluster(&named("Second"), "A").expect("second");
    assert_ne!(first.id, second.id);
}

#[test]
fn the_public_builder_takes_an_empty_family_as_a_font_to_register_by_its_recorded_names() {
    let fonts = crate::fonts::build_bundled_font_collection(
        vec![face("First", AHEM), face("  ", &ttc(&[CANVAS_TEST]))],
        false,
    )
    .expect("faces register");
    assert!(fonts.is_bundled_only());
    assert!(fonts.match_cluster(&named("First"), "A").is_some());
    let canvas = fonts
        .match_cluster(&named("CanvasTestNoSpace"), "A")
        .expect("the collection's face answers to its family");
    assert_eq!(
        fonts.family_name(canvas.id).as_deref(),
        Some("CanvasTestNoSpace")
    );
}

/// `font` with the record count of its name table zeroed: it records no
/// family name any more.
fn without_family_names(font: &[u8]) -> Vec<u8> {
    let mut out = font.to_vec();
    let tables = usize::from(u16::from_be_bytes([out[4], out[5]]));
    let name = (0..tables)
        .map(|table| 12 + 16 * table)
        .find(|&record| &out[record..record + 4] == b"name")
        .map(|record| u32::from_be_bytes(out[record + 8..record + 12].try_into().unwrap()) as usize)
        .expect("a name table");
    out[name + 2..name + 4].copy_from_slice(&0u16.to_be_bytes());
    out
}

/// `font` whose typographic family name is present but empty, the legacy
/// family name being left as it was.
fn with_empty_typographic_family(font: &[u8]) -> Vec<u8> {
    let mut out = font.to_vec();
    let tables = usize::from(u16::from_be_bytes([out[4], out[5]]));
    let name = (0..tables)
        .map(|table| 12 + 16 * table)
        .find(|&record| &out[record..record + 4] == b"name")
        .map(|record| u32::from_be_bytes(out[record + 8..record + 12].try_into().unwrap()) as usize)
        .expect("a name table");
    let records = usize::from(u16::from_be_bytes([out[name + 2], out[name + 3]]));
    let mut emptied = 0;
    for record in (0..records).map(|record| name + 6 + 12 * record) {
        if out[record + 6..record + 8] == 16u16.to_be_bytes() {
            out[record + 8..record + 10].copy_from_slice(&0u16.to_be_bytes());
            emptied += 1;
        }
    }
    assert!(emptied > 0, "the font records a typographic family");
    out
}

#[test]
fn a_face_with_an_empty_typographic_family_has_no_recorded_family() {
    let emptied = with_empty_typographic_family(AHEM);
    // The legacy name is still there, but the font library registers a face
    // under the typographic name when it has one, and under none when that
    // name is empty.
    assert_eq!(family_name(&emptied).as_deref(), Some(""));
    assert_eq!(
        recorded_families(&ttc(&[emptied.as_slice(), CANVAS_TEST]), &Limits::default()),
        ["CanvasTestNoSpace"]
    );
}

#[test]
fn a_collection_whose_faces_record_no_family_still_answers_the_generic_families() {
    let nameless = without_family_names(AHEM);
    assert_eq!(family_name(&nameless), None);
    assert!(recorded_families(&nameless, &Limits::default()).is_empty());
    let fonts = bundled_sources(
        &Limits::default(),
        vec![BundledSource::Collection(ttc(&[nameless.as_slice()]))],
        false,
    )
    .expect("a nameless face registers");
    // The generic families map onto no family, and the bundled face is still
    // the last resort the font library falls back to.
    assert!(
        fonts
            .match_cluster(&generic(GenericFamily::SansSerif), "A")
            .is_some()
    );
}
