use super::*;

use ureq::Timeout;
use ureq::unversioned::transport::time::Duration;

// `NextTimeout` has no `Default` impl in this `ureq` version (its
// `after`/`reason` fields use `ureq`-internal `Duration`/`Timeout`
// types, not `std::time::Duration`) and no public constructor either,
// so tests build one via its public field literal. The specific
// `reason` is irrelevant here — `SsrfSafeResolver::resolve` never reads
// it, only forwards it to the wrapped inner resolver.
fn some_timeout() -> NextTimeout {
    NextTimeout {
        after: Duration::from_secs(5),
        reason: Timeout::Resolve,
    }
}

#[test]
fn allows_a_public_ip_literal_host() {
    let resolver = SsrfSafeResolver::new();
    let uri: Uri = "http://1.1.1.1/".parse().unwrap();
    let config = Config::default();
    let result = resolver.resolve(&uri, &config, some_timeout());
    assert!(result.is_ok(), "expected Ok, got {result:?}");
    assert!(!result.unwrap().is_empty());
}

#[test]
fn blocks_a_loopback_ip_literal_host() {
    let resolver = SsrfSafeResolver::new();
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

/// Inner resolver that returns a fixed address list (or a fixed error)
/// regardless of the URI, standing in for a DNS answer with several
/// records.
#[derive(Debug)]
struct ScriptedResolver(Result<Vec<std::net::SocketAddr>, ()>);

impl Resolver for ScriptedResolver {
    fn resolve(
        &self,
        _uri: &Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, Error> {
        let addrs = self.0.as_ref().map_err(|()| Error::HostNotFound)?;
        let mut out = self.empty();
        for addr in addrs {
            out.push(*addr);
        }
        Ok(out)
    }
}

fn scripted(addrs: &[&str]) -> SsrfSafeResolver<ScriptedResolver> {
    SsrfSafeResolver {
        inner: ScriptedResolver(Ok(addrs.iter().map(|a| a.parse().unwrap()).collect())),
    }
}

#[test]
fn keeps_only_the_public_addresses_from_a_mixed_lookup() {
    // A hostname whose records mix private and public addresses must
    // resolve to just the public ones: not an error, and never a private
    // address the connector could fall back to.
    let resolver = scripted(&[
        "10.0.0.1:80",
        "1.1.1.1:80",
        "127.0.0.1:80",
        "[fd00::1]:80",
        "[2606:4700:4700::1111]:80",
        "169.254.169.254:80",
    ]);
    let uri: Uri = "http://mixed.example/".parse().unwrap();
    let resolved = resolver
        .resolve(&uri, &Config::default(), some_timeout())
        .expect("a lookup with public addresses must not be rejected");
    let got: Vec<std::net::SocketAddr> = resolved.iter().copied().collect();
    let want: Vec<std::net::SocketAddr> = vec![
        "1.1.1.1:80".parse().unwrap(),
        "[2606:4700:4700::1111]:80".parse().unwrap(),
    ];
    assert_eq!(got, want);
}

#[test]
fn rejects_a_lookup_whose_addresses_are_all_private() {
    let resolver = scripted(&["10.0.0.1:80", "[::1]:80"]);
    let uri: Uri = "http://internal.example/".parse().unwrap();
    let err = resolver
        .resolve(&uri, &Config::default(), some_timeout())
        .expect_err("all-private lookup must be rejected");
    assert!(
        matches!(&err, Error::Other(inner) if inner.downcast_ref::<SsrfBlocked>().is_some()),
        "expected Error::Other(SsrfBlocked), got {err:?}"
    );
}

#[test]
fn passes_an_inner_resolution_failure_through_unchanged() {
    let resolver = SsrfSafeResolver {
        inner: ScriptedResolver(Err(())),
    };
    let uri: Uri = "http://missing.example/".parse().unwrap();
    let err = resolver
        .resolve(&uri, &Config::default(), some_timeout())
        .expect_err("inner failure must propagate");
    assert!(
        matches!(err, Error::HostNotFound),
        "a genuine DNS failure must not be relabeled as SsrfBlocked, got {err:?}"
    );
}

#[test]
fn ssrf_blocked_display_names_the_rejected_uri() {
    let uri: Uri = "http://169.254.169.254/latest/meta-data/".parse().unwrap();
    let blocked = SsrfBlocked { uri };
    let message = blocked.to_string();
    assert!(
        message.contains("http://169.254.169.254/latest/meta-data/"),
        "SsrfBlocked's Display must name the rejected URI so diagnostics \
         can identify what was blocked, got: {message:?}"
    );
}
