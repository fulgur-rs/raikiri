use super::*;
use raikiri_style::property::{FontOpticalSizing, FontStyle};
use std::sync::Arc;

#[test]
fn measures_are_kept_apart_per_optical_sizing() {
    let measurer = EngineNormalLineHeight {
        fonts: crate::layout::ifc::test_support::ahem_fonts(),
        measured: Mutex::default(),
        ch_measured: Mutex::default(),
    };
    let mut font = ChFontKey::new(
        Arc::new(vec![raikiri_style::FontFamilyName::named("Ahem")]),
        ComputedLength(20.0),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::Auto,
    );
    for optical_sizing in [FontOpticalSizing::Auto, FontOpticalSizing::None] {
        font.optical_sizing = optical_sizing;
        // The second lookup of each key is served from the cache.
        for _ in 0..2 {
            assert_eq!(
                measurer.normal_line_height(&font),
                Some(ComputedLength(20.0))
            );
        }
    }
    let measured = measurer.measured.lock().unwrap();
    let entries: Vec<_> = measured.values.values().flatten().map(|e| e.1).collect();
    assert_eq!(entries, [FontOpticalSizing::Auto, FontOpticalSizing::None]);
}

#[test]
fn ch_advances_are_cached_independently_of_normal_line_height() {
    let measurer = EngineNormalLineHeight {
        fonts: crate::layout::ifc::test_support::ahem_fonts(),
        measured: Mutex::default(),
        ch_measured: Mutex::default(),
    };
    let font = ChFontKey::new(
        Arc::new(vec![raikiri_style::FontFamilyName::named("Ahem")]),
        ComputedLength(20.0),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::Auto,
    );

    assert_eq!(measurer.ch_advance(&font), Some(ComputedLength(20.0)));
    assert_eq!(measurer.ch_advance(&font), Some(ComputedLength(20.0)));
    assert!(measurer.measured.lock().unwrap().values.is_empty());
    assert_eq!(
        measurer
            .ch_measured
            .lock()
            .unwrap()
            .values
            .values()
            .flatten()
            .count(),
        1
    );
}

#[test]
fn historical_font_sizes_have_a_bounded_retained_value_count() {
    let measurer = EngineNormalLineHeight {
        fonts: FontCollection::new(&shodo::limits::Limits::default()),
        measured: Mutex::default(),
        ch_measured: Mutex::default(),
    };
    let mut font = ChFontKey::new(
        Arc::new(vec![raikiri_style::FontFamilyName::generic(
            "invalid-cache-test",
        )]),
        ComputedLength(16.0),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::None,
    );
    for i in 0..4_000 {
        font.size = ComputedLength(17.0 + i as f32 / 16.0);
        assert_eq!(measurer.normal_line_height(&font), None);
        assert_eq!(
            measurer.ch_advance(&font),
            Some(ComputedLength(font.size.0 * 0.5))
        );
    }
    assert!(
        measurer
            .measured
            .lock()
            .unwrap()
            .values
            .values()
            .flatten()
            .count()
            <= 512
    );
    assert!(
        measurer
            .ch_measured
            .lock()
            .unwrap()
            .values
            .values()
            .flatten()
            .count()
            <= 512
    );
}

#[test]
fn real_font_values_survive_eviction_for_all_styles_and_optical_modes() {
    let measurer = EngineNormalLineHeight {
        fonts: crate::layout::ifc::test_support::ahem_fonts(),
        measured: Mutex::default(),
        ch_measured: Mutex::default(),
    };
    let mut font = ChFontKey::new(
        Arc::new(vec![raikiri_style::FontFamilyName::named("Ahem")]),
        ComputedLength(20.0),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::Auto,
    );
    for style in [FontStyle::Normal, FontStyle::Italic, FontStyle::Oblique] {
        for optical in [FontOpticalSizing::Auto, FontOpticalSizing::None] {
            font.style = style;
            font.optical_sizing = optical;
            for _ in 0..2 {
                assert_eq!(
                    measurer.normal_line_height(&font),
                    Some(ComputedLength(20.0))
                );
                assert_eq!(measurer.ch_advance(&font), Some(ComputedLength(20.0)));
            }
        }
    }
    for i in 0..600 {
        font.size = ComputedLength(30.0 + i as f32);
        assert_eq!(measurer.normal_line_height(&font), Some(font.size));
        assert_eq!(measurer.ch_advance(&font), Some(font.size));
    }
    font.size = ComputedLength(20.0);
    assert_eq!(
        measurer.normal_line_height(&font),
        Some(ComputedLength(20.0))
    );
    assert_eq!(measurer.ch_advance(&font), Some(ComputedLength(20.0)));
}

#[test]
fn font_replacement_uses_a_new_source_while_shared_old_sources_keep_their_fonts() {
    let mut doc = crate::Document::new();
    doc.set_font_collection(crate::layout::ifc::test_support::ahem_fonts());
    let old = doc.normal_line_height();
    let clone = doc.clone();
    assert!(Arc::ptr_eq(&old.0, &clone.normal_line_height().0));
    let font = ChFontKey::new(
        Arc::new(vec![raikiri_style::FontFamilyName::named("Ahem")]),
        ComputedLength(20.0),
        400.0,
        FontStyle::Normal,
        FontOpticalSizing::Auto,
    );
    assert_eq!(old.0.ch_advance(&font), Some(ComputedLength(20.0)));
    let empty = FontCollection::new(&shodo::limits::Limits::default());
    let expected = crate::layout::measure_normal_line_height(&empty, &font).map(ComputedLength);
    let expected_ch = ComputedLength(crate::layout::measure_ch_advance(&empty, &font));
    assert_ne!(expected_ch, ComputedLength(20.0));
    doc.set_font_collection(empty);
    let new = doc.normal_line_height();
    assert!(!Arc::ptr_eq(&old.0, &new.0));
    assert_eq!(new.0.ch_advance(&font), Some(expected_ch));
    assert_eq!(new.0.normal_line_height(&font), expected);
    assert_eq!(old.0.ch_advance(&font), Some(ComputedLength(20.0)));
    assert_eq!(
        clone.normal_line_height().0.ch_advance(&font),
        Some(ComputedLength(20.0))
    );
}
