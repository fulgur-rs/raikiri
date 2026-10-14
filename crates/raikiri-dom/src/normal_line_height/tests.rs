use super::*;
use raikiri_style::property::{FontOpticalSizing, FontStyle};

#[test]
fn measures_are_kept_apart_per_optical_sizing() {
    let measurer = EngineNormalLineHeight {
        fonts: crate::layout::ifc::test_support::ahem_fonts(),
        measured: Mutex::default(),
    };
    let mut font = ChFontKey {
        family: Arc::new(vec![raikiri_style::FontFamilyName::named("Ahem")]),
        size: ComputedLength(20.0),
        weight: 400.0,
        style: FontStyle::Normal,
        optical_sizing: FontOpticalSizing::Auto,
    };
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
    let entries: Vec<_> = measured.values().flatten().map(|e| e.1).collect();
    assert_eq!(entries, [FontOpticalSizing::Auto, FontOpticalSizing::None]);
}
