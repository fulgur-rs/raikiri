//! Which `<image>` references a document may carry is decided at parse time.

use crate::{SvgDocument, SvgError, SvgImageAccess};

fn parse(access: SvgImageAccess, body: &str) -> Result<SvgDocument, SvgError> {
    parse_with(access, "", body)
}

fn parse_with(
    access: SvgImageAccess,
    root_attributes: &str,
    body: &str,
) -> Result<SvgDocument, SvgError> {
    SvgDocument::parse_with_images(
        format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"
                     width="4" height="4" {root_attributes}>{body}</svg>"##
        )
        .as_bytes(),
        access,
    )
}

fn image(href: &str) -> String {
    format!(r##"<image href="{href}" width="1" height="1"/>"##)
}

const URLS: [&str; 5] = [
    "https://example.test/a.png",
    "file:///etc/passwd",
    "a.png",
    "//example.test/a.png",
    "",
];
const DATA: [&str; 3] = [
    "data:image/png;base64,AA==",
    "DATA:image/png;base64,AA==",
    "  data:,x",
];

#[test]
fn nothing_but_fragments_is_the_default() {
    for href in URLS.iter().chain(&DATA) {
        assert!(
            matches!(
                parse(SvgImageAccess::None, &image(href)),
                Err(SvgError::ExternalReference)
            ),
            "{href}"
        );
    }
    assert!(parse(SvgImageAccess::None, &image("#elsewhere")).is_ok());
    // `parse` is the same as asking for nothing.
    let document = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4">{}</svg>"##,
        image("a.png")
    );
    assert!(matches!(
        SvgDocument::parse(document.as_bytes()),
        Err(SvgError::ExternalReference)
    ));
}

#[test]
fn data_access_allows_data_urls_and_nothing_else() {
    for href in DATA {
        assert!(parse(SvgImageAccess::Data, &image(href)).is_ok(), "{href}");
    }
    for href in URLS.iter().chain(&["data", "ééé", "dat:image/png,x"]) {
        assert!(
            matches!(
                parse(SvgImageAccess::Data, &image(href)),
                Err(SvgError::ExternalReference)
            ),
            "{href}"
        );
    }
    assert!(parse(SvgImageAccess::Data, &image("#elsewhere")).is_ok());
    // The xlink spelling is held to the same rule.
    let xlink = r##"<image xlink:href="https://example.test/a.png" width="1" height="1"/>"##;
    assert!(matches!(
        parse(SvgImageAccess::Data, xlink),
        Err(SvgError::ExternalReference)
    ));
}

#[test]
fn any_access_allows_every_reference() {
    for href in URLS.iter().chain(&DATA) {
        assert!(parse(SvgImageAccess::Any, &image(href)).is_ok(), "{href}");
    }
}

#[test]
fn a_base_of_the_documents_own_is_refused_where_urls_are_allowed() {
    // The host resolves every URL against the page, so a base in the document
    // would send a request somewhere the page never named.
    for (root_attributes, body) in [
        (r#"xml:base="https://other.test/""#, String::new()),
        ("", r##"<g xml:base="sub/"/>"##.to_owned()),
    ] {
        assert!(matches!(
            parse_with(SvgImageAccess::Any, root_attributes, &body),
            Err(SvgError::ExternalReference)
        ));
        for access in [SvgImageAccess::None, SvgImageAccess::Data] {
            assert!(parse_with(access, root_attributes, &body).is_ok());
        }
    }
    // An empty base names nothing.
    assert!(parse_with(SvgImageAccess::Any, r#"xml:base="  ""#, "").is_ok());
}

#[test]
fn only_image_elements_of_the_svg_namespace_are_references() {
    let foreign = r##"<image xmlns="urn:other" href="https://example.test/a.png"/>"##;
    assert!(parse(SvgImageAccess::None, foreign).is_ok());
    let not_an_image = r##"<rect href="https://example.test/a.png" width="1" height="1"/>"##;
    assert!(parse(SvgImageAccess::None, not_an_image).is_ok());
}
