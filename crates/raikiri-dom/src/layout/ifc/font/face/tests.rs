use super::*;
use crate::layout::ifc::font::{BundledFace, bundled_collection};
use shodo::font::{FontOptions, FontQuery};
use shodo::style::FontFamily;
use std::collections::HashMap;

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/Ahem.ttf"
));

struct MapLoader(HashMap<&'static str, Vec<u8>>);

impl FontFaceLoader for MapLoader {
    fn load(&self, url: &str) -> Option<Vec<u8>> {
        self.0.get(url).cloned()
    }
}

fn named(family: &str) -> FontQuery {
    FontQuery {
        families: vec![FontFamily::Named(family.to_owned())],
        ..FontQuery::default()
    }
}

/// A shared layer with Ahem installed (so `local(Ahem)` can resolve).
fn shared_with_ahem() -> FontCollection {
    let shared = FontCollection::with_options(
        &Limits::default(),
        FontOptions {
            system_fonts: false,
            ..FontOptions::default()
        },
    );
    shared.register(AHEM.to_vec()).expect("register ahem");
    shared
}

fn registry(css: &str) -> FontFaceRegistry {
    FontFaceRegistry::from_source(css)
}

#[test]
fn a_document_layer_is_invisible_to_other_documents() {
    let shared = shared_with_ahem();
    let limits = Limits::default();
    let first = document_layer(&shared, &limits);
    let second = document_layer(&shared, &limits);
    let loader = MapLoader(HashMap::from([("face.ttf", AHEM.to_vec())]));
    let report = register_font_faces(
        &first,
        &registry("@font-face { font-family: Face; src: url(face.ttf); }"),
        &loader,
    );
    assert_eq!(report.applied, vec!["Face"]);

    let in_first = first.match_cluster(&named("Face"), "a").expect("first");
    let in_second = second.match_cluster(&named("Face"), "a").map(|m| m.id);
    assert_eq!(in_first.id.layer(), first.layer_handle().id());
    assert_ne!(in_second, Some(in_first.id));
    assert_eq!(second.generation(), 0);
}

#[test]
fn a_url_source_registers_the_face_under_the_authored_family() {
    let shared = shared_with_ahem();
    let doc = document_layer(&shared, &Limits::default());
    let loader = MapLoader(HashMap::from([("a.ttf", AHEM.to_vec())]));
    let report = register_font_faces(
        &doc,
        &registry("@font-face { font-family: Aliased; src: url(a.ttf); }"),
        &loader,
    );
    assert_eq!(report.applied, vec!["Aliased"]);
    assert!(report.aliased.is_empty() && report.skipped.is_empty());
    assert!(doc.match_cluster(&named("Aliased"), "a").is_some());
}

#[test]
fn a_local_source_resolves_by_full_or_postscript_name() {
    let shared = shared_with_ahem();
    let doc = document_layer(&shared, &Limits::default());
    let loader = MapLoader(HashMap::new());
    let report = register_font_faces(
        &doc,
        &registry("@font-face { font-family: Face; src: local(Ahem); }"),
        &loader,
    );
    assert_eq!(report.aliased, vec![("Face".to_owned(), "Ahem".to_owned())]);
    let matched = doc.match_cluster(&named("Face"), "a").expect("alias face");
    assert_eq!(matched.id.layer(), doc.layer_handle().id());
}

#[test]
fn a_local_source_with_an_unknown_name_is_skipped() {
    let shared = shared_with_ahem();
    let doc = document_layer(&shared, &Limits::default());
    let report = register_font_faces(
        &doc,
        &registry("@font-face { font-family: Face; src: local(NoSuchFont); }"),
        &MapLoader(HashMap::new()),
    );
    assert_eq!(report.skipped, vec!["Face"]);
    assert_eq!(doc.generation(), 0);
}

#[test]
fn sources_are_tried_in_order_and_the_first_success_wins() {
    let shared = shared_with_ahem();
    let doc = document_layer(&shared, &Limits::default());
    let loader = MapLoader(HashMap::from([("second.ttf", AHEM.to_vec())]));
    let report = register_font_faces(
        &doc,
        &registry(
            "@font-face { font-family: Face; src: url(missing.ttf), url(second.ttf), local(Ahem); }",
        ),
        &loader,
    );
    assert_eq!(report.applied, vec!["Face"]);
    assert!(report.aliased.is_empty());
    assert_eq!(doc.generation(), 1);
}

#[test]
fn undecodable_bytes_fall_through() {
    let shared = shared_with_ahem();
    let doc = document_layer(&shared, &Limits::default());
    let loader = MapLoader(HashMap::from([("bad.ttf", b"not a font".to_vec())]));
    let report = register_font_faces(
        &doc,
        &registry("@font-face { font-family: Face; src: url(bad.ttf); }"),
        &loader,
    );
    assert_eq!(report.skipped, vec!["Face"]);
    assert!(report.applied.is_empty());
}

#[test]
fn an_empty_registry_is_a_no_op() {
    let shared = shared_with_ahem();
    let doc = document_layer(&shared, &Limits::default());
    let report = register_font_faces(&doc, &FontFaceRegistry::new(), &MapLoader(HashMap::new()));
    assert_eq!(report, FontFaceApplyReport::default());
    assert_eq!(doc.generation(), 0);
}

#[test]
fn a_bundled_shared_layer_can_back_a_document_layer() {
    let shared = bundled_collection(
        &Limits::default(),
        vec![BundledFace {
            family: "Base".to_owned(),
            bytes: AHEM.to_vec(),
        }],
        false,
    )
    .expect("shared layer");
    let doc = document_layer(&shared, &Limits::default());
    assert!(doc.match_cluster(&named("Base"), "a").is_some());
}

#[test]
fn a_local_source_does_not_see_faces_registered_with_a_descriptor() {
    let shared = bundled_collection(
        &Limits::default(),
        vec![BundledFace {
            family: "Ahem".to_owned(),
            bytes: AHEM.to_vec(),
        }],
        false,
    )
    .expect("shared layer");
    let doc = document_layer(&shared, &Limits::default());
    let report = register_font_faces(
        &doc,
        &registry("@font-face { font-family: Face; src: local(Ahem); }"),
        &MapLoader(HashMap::new()),
    );
    assert_eq!(report.skipped, vec!["Face"]);
}
