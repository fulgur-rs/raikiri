use super::*;
use raikiri_traits::{Body, Method, Request, ResourceKind};

fn get(url: raikiri::Url) -> Request {
    Request {
        url,
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::Image,
    }
}

fn checkout() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("fonts")).unwrap();
    std::fs::write(root.path().join("fonts/Ahem.ttf"), b"").unwrap();
    std::fs::create_dir_all(root.path().join("images")).unwrap();
    std::fs::write(root.path().join("images/green.png"), b"root").unwrap();
    std::fs::create_dir_all(root.path().join("css/support")).unwrap();
    std::fs::write(root.path().join("css/support/a.png"), b"page").unwrap();
    root
}

#[test]
fn the_checkout_root_is_the_ancestor_that_ships_ahem() {
    let root = checkout();
    let root_path = root.path().to_path_buf();
    assert_eq!(
        wpt_root_above(&root_path.join("css/support")),
        Some(root_path.clone())
    );
    assert_eq!(wpt_root_above(root_path.parent().unwrap()), None);
}

#[test]
fn a_root_relative_url_is_served_from_the_checkout() {
    let root = checkout();
    let provider = WptFileNetworkProvider::new(Some(root.path().to_path_buf()));
    let url = raikiri::Url::parse("file:///images/green.png").unwrap();
    let fetched = provider.fetch(get(url)).unwrap();
    assert_eq!(&fetched.bytes[..], b"root");
    assert_eq!(
        fetched.final_url,
        raikiri::Url::from_file_path(root.path().join("images/green.png")).unwrap()
    );
}

#[test]
fn a_url_inside_the_checkout_is_left_alone() {
    let root = checkout();
    let provider = WptFileNetworkProvider::new(Some(root.path().to_path_buf()));
    let url = raikiri::Url::from_file_path(root.path().join("css/support/a.png")).unwrap();
    let fetched = provider.fetch(get(url.clone())).unwrap();
    assert_eq!(&fetched.bytes[..], b"page");
    assert_eq!(fetched.final_url, url);
}

#[test]
fn without_a_checkout_root_urls_are_not_rebased() {
    let provider = WptFileNetworkProvider::new(None);
    let url = raikiri::Url::parse("file:///images/green.png").unwrap();
    assert!(provider.fetch(get(url)).is_err());
}

struct Failing;

impl ReplacedResolver for Failing {
    fn resolve(&self, _: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Err(ResolverError::Decode("not an image".to_owned()))
    }
}

struct Sized;

impl ReplacedResolver for Sized {
    fn resolve(&self, _: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        Ok(ResolvedIntrinsic {
            intrinsic: IntrinsicBox::new(4.0, 2.0),
            disposition: ResolveDisposition::Ok,
        })
    }
}

#[test]
fn a_failed_image_load_becomes_a_broken_image() {
    let url = raikiri::Url::parse("about:invalid").unwrap();
    let resolved = BrokenImageFallback::new(&Failing)
        .resolve(ResolverRequest::new(&url))
        .unwrap();
    assert_eq!(resolved.intrinsic, IntrinsicBox::default());
    assert!(matches!(
        resolved.disposition,
        ResolveDisposition::Fallback { ref reason } if reason.contains("not an image")
    ));
}

#[test]
fn a_loaded_image_passes_through() {
    let url = raikiri::Url::parse("file:///a.png").unwrap();
    let resolved = BrokenImageFallback::new(&Sized)
        .resolve(ResolverRequest::new(&url))
        .unwrap();
    assert_eq!(resolved.intrinsic, IntrinsicBox::new(4.0, 2.0));
    assert!(matches!(resolved.disposition, ResolveDisposition::Ok));
}

#[test]
fn a_non_file_url_is_left_alone() {
    let root = checkout();
    let provider = WptFileNetworkProvider::new(Some(root.path().to_path_buf()));
    let url = raikiri::Url::parse("about:invalid").unwrap();
    assert_eq!(provider.rebase(get(url.clone())).url, url);
}

#[test]
fn a_rebased_url_keeps_its_query_and_fragment() {
    let root = checkout();
    let provider = WptFileNetworkProvider::new(Some(root.path().to_path_buf()));
    let url = raikiri::Url::parse("file:///images/green.png?a=1#b").unwrap();
    let rebased = provider.rebase(get(url)).url;
    assert_eq!(rebased.query(), Some("a=1"));
    assert_eq!(rebased.fragment(), Some("b"));
    assert!(rebased.path().ends_with("/images/green.png"));
}
