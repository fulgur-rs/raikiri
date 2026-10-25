use super::*;

fn url() -> Url {
    Url::parse("https://example.test/a").unwrap()
}

#[test]
fn a_request_starts_as_a_plain_get_and_takes_each_other_part_from_a_setter() {
    let request = Request::new(url(), ResourceKind::Image);
    assert_eq!(request.url, url());
    assert_eq!(request.method, Method::Get);
    assert_eq!(request.content_type, None);
    assert!(request.headers.is_empty());
    assert!(matches!(request.body, Body::Empty));
    assert!(request.signal.is_none());
    assert!(matches!(request.kind, ResourceKind::Image));

    let controller = AbortController::new();
    let request = request
        .method(Method::Post)
        .content_type(Some("text/plain".to_owned()))
        .headers(vec![("X-Test".to_owned(), "yes".to_owned())])
        .body(Body::Bytes(Bytes::from_static(b"a=1")))
        .signal(Some(controller.signal.clone()));
    assert_eq!(request.method, Method::Post);
    assert_eq!(request.content_type.as_deref(), Some("text/plain"));
    assert_eq!(request.headers, [("X-Test".to_owned(), "yes".to_owned())]);
    assert!(matches!(&request.body, Body::Bytes(bytes) if bytes.as_ref() == b"a=1"));
    controller.abort();
    assert!(request.signal.is_some_and(|signal| signal.is_aborted()));
}

#[test]
fn a_fetched_resource_starts_without_a_type_or_an_encoding() {
    let resource = FetchedResource::new(Bytes::from_static(b"body"), url());
    assert_eq!(resource.bytes.as_ref(), b"body");
    assert_eq!(resource.final_url, url());
    assert_eq!(resource.content_type, None);
    assert_eq!(resource.encoding, None);

    let resource = resource
        .content_type(Some("text/css".to_owned()))
        .encoding(Some("utf-8".to_owned()));
    assert_eq!(resource.content_type.as_deref(), Some("text/css"));
    assert_eq!(resource.encoding.as_deref(), Some("utf-8"));
}
