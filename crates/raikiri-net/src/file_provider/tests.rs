use super::*;
use std::io::Write;

#[test]
fn reads_bytes_from_a_local_file() {
    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    tmp.write_all(b"hello").unwrap();
    let url = url::Url::from_file_path(tmp.path()).unwrap();

    let provider = FileNetworkProvider;
    let result = provider
        .fetch(Request::new(
            url.clone(),
            raikiri_traits::ResourceKind::Image,
        ))
        .expect("fetch should succeed");

    assert_eq!(&result.bytes[..], b"hello");
    assert_eq!(result.final_url, url);
}

#[test]
fn rejects_non_file_scheme() {
    let provider = FileNetworkProvider;
    let url = url::Url::parse("https://example.com/x.png").unwrap();
    let err = provider
        .fetch(Request::new(url, raikiri_traits::ResourceKind::Image))
        .unwrap_err();
    assert!(matches!(err, NetworkError::Other(_)));
}
