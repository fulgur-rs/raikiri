use super::*;

fn example_url() -> Url {
    Url::parse("https://example.com/page.html").unwrap()
}

#[test]
fn new_defaults_to_get_with_empty_body() {
    let options = NavigationOptions::new(example_url(), None, 3);
    assert_eq!(options.url.as_str(), "https://example.com/page.html");
    assert_eq!(options.content_type, None);
    assert_eq!(options.source_document, 3);
    assert_eq!(options.method, Method::GET);
    assert!(matches!(options.document_resource, Body::Empty));
}

#[test]
fn new_preserves_content_type() {
    let options = NavigationOptions::new(example_url(), Some("text/html".to_string()), 1);
    assert_eq!(options.content_type.as_deref(), Some("text/html"));
}

#[test]
fn set_document_resource_builder() {
    let options = NavigationOptions::new(example_url(), None, 0)
        .set_document_resource(Body::Bytes(bytes::Bytes::from_static(b"body")));
    assert!(matches!(options.document_resource, Body::Bytes(_)));
}

#[test]
fn set_method_builder() {
    let options = NavigationOptions::new(example_url(), None, 0).set_method(Method::POST);
    assert_eq!(options.method, Method::POST);
}

#[test]
fn into_request_maps_all_fields() {
    let url = example_url();
    let options = NavigationOptions::new(url.clone(), Some("text/html".to_string()), 9)
        .set_method(Method::POST)
        .set_document_resource(Body::Empty);
    let request = options.into_request();
    assert_eq!(request.url, url);
    assert_eq!(request.method, Method::POST);
    assert_eq!(request.content_type.as_deref(), Some("text/html"));
    assert!(request.headers.is_empty());
    assert!(matches!(request.body, Body::Empty));
    assert!(request.signal.is_none());
}

#[test]
fn dummy_provider_navigate_to_does_not_panic() {
    let provider = DummyNavigationProvider;
    provider.navigate_to(NavigationOptions::new(example_url(), None, 0));
}
