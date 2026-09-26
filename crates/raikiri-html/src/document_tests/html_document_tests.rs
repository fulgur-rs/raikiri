use super::*;

fn hello_world_doc() -> HtmlDocument {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded = crate::parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
    let cascade = build_cascaded(&uncascaded);
    // 内部 field 直接 construct (crate-internal test なので pub(crate) field OK)
    HtmlDocument {
        uncascaded,
        cascade,
        font_faces: raikiri_style::FontFaceRegistry::new(),
        effective_base_url: None,
    }
}

#[test]
fn consumer_property_cascade_wrapper_accepts_registration() {
    let opts = ParseOptions {
        extra_stylesheets: &[],
        network: None,
        base_url: None,
    };
    let uncascaded =
        crate::parse(&b"<p style=\"bookmark-level: 4\">Hi</p>"[..], &opts).expect("parse");
    let registrations = [ConsumerPropertyRegistration::integer("bookmark-level")];
    let cascade = build_cascaded_with_consumer_properties(&uncascaded, &registrations);
    assert!(!cascade.computed.is_empty());
}

#[test]
fn html_document_accessors_expose_underlying_types() {
    let doc = hello_world_doc();
    // accessor が inner field と identity 一致 (別 heap 割当てなし)
    let dom_ref: &raikiri_dom::Document = doc.dom();
    let cascade_ref: &CascadeResult = doc.cascade();
    let sources_ref: &[String] = doc.stylesheet_sources();

    assert!(
        std::ptr::eq(dom_ref, &doc.uncascaded.dom),
        "dom() must return &doc.uncascaded.dom"
    );
    assert!(
        std::ptr::eq(cascade_ref, &doc.cascade),
        "cascade() must return &doc.cascade"
    );
    assert!(
        std::ptr::eq(
            sources_ref.as_ptr(),
            doc.uncascaded.stylesheet_sources.as_ptr()
        ) || (sources_ref.is_empty() && doc.uncascaded.stylesheet_sources.is_empty()),
        "stylesheet_sources() must alias inner Vec"
    );
}

#[test]
fn html_document_into_parts_returns_owned_halves() {
    let doc = hello_world_doc();
    let expected_nodes = doc.dom().node_count();
    let expected_computed = doc.cascade().computed.len();
    let (uncascaded, cascade) = doc.into_parts();
    assert_eq!(uncascaded.dom.node_count(), expected_nodes);
    assert_eq!(cascade.computed.len(), expected_computed);
}

#[test]
fn html_document_cascade_populated_after_construct() {
    let doc = hello_world_doc();
    assert!(
        !doc.cascade().computed.is_empty(),
        "cascade must be populated (build_cascaded produces per-node ComputedValues)"
    );
    // node_count と cascade.computed.len() 契約
    assert_eq!(
        doc.cascade().computed.len(),
        doc.dom().node_count(),
        "cascade.computed.len() must equal document.node_count()"
    );
}
