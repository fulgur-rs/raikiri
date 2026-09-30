use super::*;
use crate::layout::ifc::test_support::ahem_fonts;
use raikiri_style::property::FontStyle as CssFontStyle;
use raikiri_style::{ComputedLength, FontFamilyName};
use shodo::font::{FontCollection, FontOptions};
use shodo::limits::Limits;
use std::sync::Arc;

const CANVAS_TEST: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/text-autospace/CanvasTest-nospace.ttf"
));

fn key(family: &[&str], size: f32) -> ChFontKey {
    ChFontKey {
        family: Arc::new(
            family
                .iter()
                .map(|name| FontFamilyName::named(*name))
                .collect(),
        ),
        size: ComputedLength(size),
        weight: 400.0,
        style: CssFontStyle::Normal,
    }
}

fn empty_collection() -> FontCollection {
    FontCollection::with_options(
        &Limits::default(),
        FontOptions {
            system_fonts: false,
            ..Default::default()
        },
    )
}

#[test]
fn ch_is_the_zero_advance_of_the_selected_face() {
    // Ahem draws every glyph one em wide.
    assert_eq!(ch_advance(&ahem_fonts(), &key(&["Ahem"], 10.0)), 10.0);
    assert_eq!(ch_advance(&ahem_fonts(), &key(&["Ahem"], 24.0)), 24.0);
}

#[test]
fn ch_takes_the_first_family_that_has_a_face() {
    // The first family is not registered, so the second decides.
    assert_eq!(
        ch_advance(&ahem_fonts(), &key(&["NoSuchFace", "Ahem"], 10.0)),
        10.0
    );
}

#[test]
fn ch_falls_back_to_half_an_em_without_a_face() {
    assert_eq!(ch_advance(&empty_collection(), &key(&["Ahem"], 10.0)), 5.0);
}

#[test]
fn ch_falls_back_to_half_an_em_when_the_face_has_no_zero() {
    // CanvasTestNoSpace is registered and is the selected face, but it has no
    // `0`. The style layer of the parley path also gives half an em then.
    let canvas = empty_collection();
    canvas
        .register(CANVAS_TEST.to_vec())
        .expect("register the canvas font");
    assert_eq!(ch_advance(&canvas, &key(&["CanvasTestNoSpace"], 10.0)), 5.0);
}
