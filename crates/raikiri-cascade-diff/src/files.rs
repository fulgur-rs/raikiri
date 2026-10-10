//! Serving a document's linked stylesheets from a directory.

use std::path::{Path, PathBuf};

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
        // Stylesheet imports are only inlined from a CSS response, and local
        // files have no headers to say so.
        let content_type = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("css"))
            .then(|| "text/css".to_owned());
        Ok(FetchOutcome::Body(FetchedResource {
            bytes: bytes.into(),
            content_type,
            final_url: request.url,
            encoding: None,
        }))
    }
}

/// The base URL of the document at `path` under `root`: `file:///` and the
/// path relative to the root, or `None` when the document is not under it.
pub(crate) fn document_url(root: &Path, path: &Path) -> Option<Url> {
    let relative = path.strip_prefix(root).ok()?;
    Url::from_file_path(Path::new("/").join(relative)).ok()
}

#[cfg(test)]
mod tests;
