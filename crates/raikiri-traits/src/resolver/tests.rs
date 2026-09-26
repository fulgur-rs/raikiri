use super::*;

#[test]
fn resolver_request_carries_url() {
    let url = Url::parse("file:///tmp/x.png").unwrap();
    let req = ResolverRequest::new(&url);
    assert_eq!(req.url(), &url);
}

#[test]
fn intrinsic_box_carries_dimensions() {
    let b = IntrinsicBox::new(64.0, 32.0);
    assert_eq!((b.width, b.height), (64.0, 32.0));
}

#[test]
fn resolver_error_network_and_decode_display() {
    let net_err = NetworkError::Other("boom".into());
    let e = ResolverError::Network(net_err);
    assert!(format!("{e}").contains("boom"));

    let e = ResolverError::Decode("bad PNG".into());
    assert_eq!(format!("{e}"), "image decode failed: bad PNG");
}
