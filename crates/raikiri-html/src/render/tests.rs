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
fn bundled_fonts_and_the_system_layer_both_build_paragraphs_in_parallel() {
    let mut dom = raikiri_dom::Document::new();
    enable_inline_engine(
        &mut dom,
        &RenderResources::new(),
        &FontFaceRegistry::default(),
        &NoLoader,
    );
    assert!(dom.has_font_collection());
    assert!(dom.ifc_parallel_build());
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

#[test]
fn table_column_measurement_preserves_cancellation_limits_and_invalid_targets() {
    let resources = RenderResources::new();
    let doc = crate::parse_html_with_resources(b"<table><tr><td>x</table>".as_slice(), &resources)
        .unwrap();
    let table = doc
        .dom()
        .preorder(doc.dom().root_index())
        .find(|&id| doc.dom().get_node(id).unwrap().tag_name() == Some("table"))
        .unwrap();
    let measure = |config: &LayoutConfig, id| {
        measure_table_columns(&doc, Default::default(), config, &resources, &[], id)
    };
    let mut config = LayoutConfig::default();
    assert!(matches!(
        measure(&config, usize::MAX),
        Err(RenderError::Layout(
            raikiri_traits::LayoutError::Internal { .. }
        ))
    ));
    config.limits.max_document_pages = Some(0);
    assert!(matches!(
        measure(&config, table),
        Err(RenderError::LimitExceeded {
            kind: raikiri_traits::LimitKind::Pages,
            limit: 0,
            actual: 1
        })
    ));
    let controller = raikiri_traits::AbortController::new();
    config.signal = Some(controller.signal.clone());
    controller.abort();
    assert!(matches!(
        measure(&config, table),
        Err(RenderError::Layout(raikiri_traits::LayoutError::Aborted))
    ));
}
