use super::*;

#[test]
fn initial_page_context_errors_map_to_render_errors() {
    let layout_error = map_initial_page_context_error(InitialPageContextError::Layout(
        raikiri_traits::LayoutError::Internal {
            message: "test".to_owned(),
        },
    ));
    assert!(matches!(layout_error, RenderError::Layout(_)));
    assert!(matches!(
        map_initial_page_context_error(InitialPageContextError::PageGeometryDidNotConverge {
            iterations: 3
        }),
        RenderError::PageGeometryDidNotConverge { iterations: 3 }
    ));
}

const AHEM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../raikiri-dom/tests/data/text-autospace/Ahem.ttf"
));

/// A loader for documents without `@font-face` sources.
struct NoLoader;

impl FontFaceLoader for NoLoader {
    fn load(&self, _url: &str) -> Option<Vec<u8>> {
        None
    }
}

fn ahem_fonts() -> crate::RenderFonts {
    crate::FontCollectionBuilder::new()
        .font_bytes("Ahem", AHEM.to_vec())
        .build()
        .expect("fonts")
}

#[test]
fn a_bundled_font_set_enables_parallel_builds_and_the_system_layer_does_not() {
    let mut dom = raikiri_dom::Document::new();
    enable_inline_engine(
        &mut dom,
        &RenderResources::new(),
        &FontFaceRegistry::default(),
        &NoLoader,
    );
    assert!(dom.has_font_collection());
    assert!(!dom.ifc_parallel_build());
    let mut dom = raikiri_dom::Document::new();
    enable_inline_engine(
        &mut dom,
        &RenderResources::new().fonts(ahem_fonts()),
        &FontFaceRegistry::default(),
        &NoLoader,
    );
    assert!(dom.has_font_collection());
    assert!(dom.ifc_parallel_build());
}

mod continuation_tests;
mod pipeline_tests;
mod viewport_tests;
