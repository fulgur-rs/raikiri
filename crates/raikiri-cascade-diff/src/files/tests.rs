use super::*;
use raikiri_traits::{Body, Method, ResourceKind};

/// A directory that is removed when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "raikiri-cascade-diff-{}-{name}",
            std::process::id()
        ));
        std::fs::create_dir_all(path.join("css")).expect("create temporary directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(url: &str) -> Request {
    Request {
        url: Url::parse(url).expect("valid URL"),
        method: Method::Get,
        content_type: None,
        headers: Vec::new(),
        body: Body::Empty,
        signal: None,
        kind: ResourceKind::ExternalStylesheet,
    }
}

fn body(outcome: Result<FetchOutcome, NetworkError>) -> FetchedResource {
    match outcome {
        Ok(FetchOutcome::Body(resource)) => resource,
        other => panic!("expected a body, got {other:?}"),
    }
}

#[test]
fn rooted_files_serve_paths_under_the_root() {
    let dir = TempDir::new("rooted");
    std::fs::write(dir.0.join("css/a.css"), "p {}").unwrap();
    std::fs::write(dir.0.join("css/a.txt"), "text").unwrap();
    let files = RootedFiles {
        root: dir.0.clone(),
    };
    let css = body(files.fetch_one_hop(request("file:///css/a.css")));
    assert_eq!(&css.bytes[..], b"p {}");
    assert_eq!(css.content_type.as_deref(), Some("text/css"));
    // `..` cannot leave the root: URL parsing resolves it first.
    let css = body(files.fetch_one_hop(request("file:///../../css/a.css")));
    assert_eq!(&css.bytes[..], b"p {}");
    // Nor can a `..` that only decoding the path makes, and separators that
    // decoding makes keep the path under the root.
    std::fs::write(dir.0.join("outside.css"), "q {}").unwrap();
    let inner = RootedFiles {
        root: dir.0.join("css"),
    };
    assert!(matches!(
        inner.fetch_one_hop(request("file:///%2e%2e%2foutside.css")),
        Err(NetworkError::Other(_))
    ));
    let css = body(inner.fetch_one_hop(request("file:///%2f%2fa.css")));
    assert_eq!(&css.bytes[..], b"p {}");
    // A symbolic link is followed within the root, and refused out of it.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("a.css", dir.0.join("css/within.css")).unwrap();
        std::os::unix::fs::symlink("../outside.css", dir.0.join("css/out.css")).unwrap();
        let css = body(inner.fetch_one_hop(request("file:///within.css")));
        assert_eq!(&css.bytes[..], b"p {}");
        assert!(matches!(
            inner.fetch_one_hop(request("file:///out.css")),
            Err(NetworkError::Other(_))
        ));
        // The type follows the name requested, not the link's target.
        std::os::unix::fs::symlink("a.txt", dir.0.join("css/alias.css")).unwrap();
        std::os::unix::fs::symlink("a.css", dir.0.join("css/alias.txt")).unwrap();
        let css = body(inner.fetch_one_hop(request("file:///alias.css")));
        assert_eq!(&css.bytes[..], b"text");
        assert_eq!(css.content_type.as_deref(), Some("text/css"));
        let text = body(inner.fetch_one_hop(request("file:///alias.txt")));
        assert_eq!(&text.bytes[..], b"p {}");
        assert_eq!(
            text.content_type.as_deref(),
            Some("application/octet-stream")
        );
    }
    // Any other file is not CSS, so a stylesheet import does not inline it.
    let text = body(files.fetch_one_hop(request("file:///css/a.txt")));
    assert_eq!(
        text.content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert!(matches!(
        files.fetch_one_hop(request("file:///css/missing.css")),
        Err(NetworkError::Io(_))
    ));
    assert!(matches!(
        files.fetch_one_hop(request("https://example.com/a.css")),
        Err(NetworkError::Other(_))
    ));
}

#[test]
fn documents_get_their_path_under_the_root_as_their_url() {
    let root = Path::new("/srv/wpt");
    assert_eq!(
        document_url(root, Path::new("/srv/wpt/css/x/a b.html")).map(String::from),
        Some("file:///css/x/a%20b.html".to_owned())
    );
    assert_eq!(document_url(root, Path::new("/elsewhere/a.html")), None);
    // Relative paths are taken from the current directory, with `.` and
    // `..` resolved by name, so either path may be written either way.
    let here = std::env::current_dir().expect("current directory");
    for (root, path) in [
        (Path::new("."), Path::new("css/a.html")),
        (Path::new("./css/.."), Path::new("css/./a.html")),
        (here.as_path(), Path::new("css/a.html")),
        (Path::new("."), &here.join("css/a.html")),
    ] {
        assert_eq!(
            document_url(root, path).map(String::from),
            Some("file:///css/a.html".to_owned()),
            "{root:?} {path:?}"
        );
    }
}
