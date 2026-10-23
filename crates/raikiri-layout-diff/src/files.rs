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
        // URL parsing resolves `..` segments, but decoding the path can make
        // new ones, and separators, from escapes such as `%2e%2e%2f`. So the
        // file is found from the path's names alone, and a `..` that is left
        // is refused rather than followed out of the root.
        let mut file = self.root.clone();
        for component in path.components() {
            match component {
                Component::Normal(name) => file.push(name),
                Component::RootDir | Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) => {
                    return Err(NetworkError::Other(format!(
                        "outside the root: {}",
                        request.url
                    )));
                }
            }
        }
        // A symbolic link under the root may still lead out of it.
        let target = std::fs::canonicalize(&file).map_err(NetworkError::Io)?;
        if !target.starts_with(std::fs::canonicalize(&self.root).map_err(NetworkError::Io)?) {
            return Err(NetworkError::Other(format!(
                "outside the root: {}",
                request.url
            )));
        }
        let bytes = std::fs::read(&target).map_err(NetworkError::Io)?;
        // Local files have no headers, so the type follows the extension of
        // the name requested, not of a link's target, as a web server would
        // give it. A stylesheet import is only inlined from a CSS response,
        // and any other file is served as one that is not CSS.
        let is_css = file
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
