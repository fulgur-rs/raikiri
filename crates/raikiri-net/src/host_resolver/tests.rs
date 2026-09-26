use super::*;
use ureq::Timeout;
use ureq::unversioned::transport::time::Duration;

fn timeout() -> NextTimeout {
    NextTimeout {
        after: Duration::from_secs(1),
        reason: Timeout::Resolve,
    }
}

#[test]
fn override_normalizes_aliases_and_returns_replaced_address() {
    let mut overrides = HostResolverOverrides::new();
    assert_eq!(overrides.get("missing.test"), None);
    assert_eq!(
        overrides.insert("WEB-PLATFORM.TEST.", "127.0.0.1".parse().unwrap()),
        None
    );
    assert_eq!(
        overrides.get("web-platform.test"),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        overrides.insert("web-platform.test", "::1".parse().unwrap()),
        Some("127.0.0.1".parse().unwrap())
    );
    assert_eq!(
        overrides.get("Web-Platform.Test."),
        Some("::1".parse().unwrap())
    );
}

#[test]
fn overrides_preserve_url_port_and_http_https_defaults() {
    let mut overrides = HostResolverOverrides::new();
    overrides.insert("web-platform.test", "::1".parse().unwrap());
    let resolver = OverrideResolver::new(overrides);
    for (url, port) in [
        ("http://web-platform.test:8000/", 8000),
        ("http://web-platform.test/", 80),
        ("https://web-platform.test/", 443),
    ] {
        let addresses = resolver
            .resolve(&url.parse().unwrap(), &Config::default(), timeout())
            .unwrap();
        assert_eq!(
            addresses.iter().copied().collect::<Vec<_>>(),
            vec![SocketAddr::new("::1".parse().unwrap(), port)]
        );
    }
    assert!(format!("{resolver:?}").contains("OverrideResolver"));
}

#[test]
fn unmapped_ip_literal_uses_inner_resolver() {
    let resolver = OverrideResolver::new(HostResolverOverrides::new());
    let addresses = resolver
        .resolve(
            &"http://127.0.0.1:9876/".parse().unwrap(),
            &Config::default(),
            timeout(),
        )
        .unwrap();
    assert_eq!(
        addresses.iter().copied().collect::<Vec<_>>(),
        vec!["127.0.0.1:9876".parse().unwrap()]
    );
}

#[derive(Debug)]
struct RejectingResolver;

impl Resolver for RejectingResolver {
    fn resolve(&self, _: &Uri, _: &Config, _: NextTimeout) -> Result<ResolvedSocketAddrs, Error> {
        Err(Error::HostNotFound)
    }
}

#[test]
fn unsupported_or_incomplete_uris_fall_back_to_inner_error() {
    let mut overrides = HostResolverOverrides::new();
    overrides.insert("web-platform.test", "127.0.0.1".parse().unwrap());
    let resolver = OverrideResolver {
        overrides,
        inner: RejectingResolver,
    };
    for url in [
        "/relative",
        "web-platform.test:8000",
        "ftp://web-platform.test/",
        "http://missing.test/",
    ] {
        assert!(matches!(
            resolver.resolve(&url.parse().unwrap(), &Config::default(), timeout()),
            Err(Error::HostNotFound)
        ));
    }
}
