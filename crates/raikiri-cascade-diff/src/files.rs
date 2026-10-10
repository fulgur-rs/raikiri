//! Serving a document's linked stylesheets from a directory.

use std::path::{Component, Path, PathBuf};

use raikiri_traits::{FetchOutcome, FetchedResource, NetworkError, NetworkProvider, Request};
use url::Url;

/// Serves `file:///<path>` from `root`, so that a document parsed with the
/// base URL [`document_url`] resolves its relative and root-relative links
/// (`/css/support/…` in WPT) against the directory it was found in.
pub(crate) struct RootedFiles {
    pub(crate) root: PathBuf,
}

impl NetworkProvider for RootedFiles {
    fn fetch_one_hop(&self, request: Request) -> Result<FetchOutcome, NetworkError> {
        let path = request
            .url
            .to_file_path()
            .map_err(|()| NetworkError::Other(format!("not a local file: {}", request.url)))?;
        // URL parsing has already resolved `..` segments, so the path stays
        // under the root.
        let path = self.root.join(path.strip_prefix("/").unwrap_or(&path));
        let bytes = std::fs::read(&path).map_err(NetworkError::Io)?;
        // Local files have no headers, so the type follows the extension. A
        // stylesheet import is only inlined from a CSS response, and any
        // other file is served as one that is not CSS, as a web server would
        // serve it.
        let is_css = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("css"));
        let content_type = if is_css {
            "text/css"
        } else {
            "application/octet-stream"
        };
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: bytes.into(),
            content_type: Some(content_type.to_owned()),
            final_url: request.url,
            encoding: None,
        }))
    }
}

/// The base URL of the document at `path` under `root`: `file:///` and the
/// path relative to the root, or `None` when the document is not under it.
/// Both paths are made absolute first, so a relative root such as `.` holds
/// the paths under it however either is written.
pub(crate) fn document_url(root: &Path, path: &Path) -> Option<Url> {
    let (root, path) = (absolute(root)?, absolute(path)?);
    let relative = path.strip_prefix(&root).ok()?;
    Url::from_file_path(Path::new("/").join(relative)).ok()
}

/// `path` against the current directory, with its `..` segments resolved by
/// name, as a URL path resolves them, not through the file system. The
/// components of an absolute path hold no `.` segments.
fn absolute(path: &Path) -> Option<PathBuf> {
    let mut resolved = PathBuf::new();
    for component in std::path::absolute(path).ok()?.components() {
        if component == Component::ParentDir {
            resolved.pop();
        } else {
            resolved.push(component);
        }
    }
    Some(resolved)
}

#[cfg(test)]
mod tests;
