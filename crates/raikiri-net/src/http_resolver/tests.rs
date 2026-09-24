use super::*;

use ureq::Timeout;
use ureq::unversioned::transport::time::Duration;

// `NextTimeout` has no `Default` impl in this `ureq` version (its
// `after`/`reason` fields use `ureq`-internal `Duration`/`Timeout`
// types, not `std::time::Duration`) and no public constructor either,
// so tests build one via its public field literal. The specific
// `reason` is irrelevant here — `SsrfSafeResolver::resolve` never reads
// it, only forwards it to the wrapped `DefaultResolver`.
fn some_timeout() -> NextTimeout {
    NextTimeout {
        after: Duration::from_secs(5),
        reason: Timeout::Resolve,
    }
}

#[test]
fn allows_a_public_ip_literal_host() {
    let resolver = SsrfSafeResolver::default();
    let uri: Uri = "http://1.1.1.1/".parse().unwrap();
    let config = Config::default();
    let result = resolver.resolve(&uri, &config, some_timeout());
    assert!(result.is_ok(), "expected Ok, got {result:?}");
    assert!(!result.unwrap().is_empty());
}

#[test]
fn blocks_a_loopback_ip_literal_host() {
    let resolver = SsrfSafeResolver::default();
    let uri: Uri = "http://127.0.0.1/".parse().unwrap();
    let config = Config::default();
    let err = resolver
        .resolve(&uri, &config, some_timeout())
        .expect_err("loopback must be rejected");
    match err {
        Error::Other(inner) => {
            let blocked = inner
                .downcast_ref::<SsrfBlocked>()
                .expect("Error::Other must carry SsrfBlocked");
            assert_eq!(blocked.uri, uri, "SsrfBlocked must name the rejected URI");
        }
        other => panic!("expected Error::Other(SsrfBlocked), got {other:?}"),
    }
}
