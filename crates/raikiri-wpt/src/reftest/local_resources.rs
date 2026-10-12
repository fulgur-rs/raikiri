//! Local resource loading for file-backed reftests.
//!
//! WPT serves its checkout from the origin root, so a root-relative URL such
//! as `/images/green.png` names a file under the checkout, and a resource that
//! cannot be loaded leaves a broken image instead of failing the page. The
//! file-backed reftest renderer reads the checkout through `file://` URLs;
//! these adapters give those URLs the same meaning.

use std::path::{Path, PathBuf};

use raikiri_traits::{
    FetchOutcome, IntrinsicBox, NetworkError, NetworkProvider, ReplacedResolver,
    ResolveDisposition, ResolvedIntrinsic, ResolverError, ResolverRequest,
};

/// The WPT checkout root above `base`: the nearest ancestor holding the
/// bundled Ahem font, which every checkout ships under `fonts/`.
pub(super) fn wpt_root_above(base: &Path) -> Option<PathBuf> {
    base.ancestors()
        .find(|candidate| is_wpt_root(candidate))
        .map(Path::to_path_buf)
}

pub(super) fn is_wpt_root(candidate: &Path) -> bool {
    candidate.join("fonts").join("Ahem.ttf").is_file()
        || candidate.join("fonts").join("ahem.css").is_file()
}

/// `file://` provider that serves root-relative WPT URLs from the checkout.
///
/// A document under the checkout resolves `/images/x.png` to
/// `file:///images/x.png`; outside the checkout that path is rebased onto the
/// checkout root, which is where the WPT server would have served it from.
#[derive(Debug, Clone, Default)]
pub(super) struct WptFileNetworkProvider {
    root: Option<PathBuf>,
}

impl WptFileNetworkProvider {
    pub(super) fn new(root: Option<PathBuf>) -> Self {
        Self { root }
    }

    fn rebase(&self, mut request: raikiri_traits::Request) -> raikiri_traits::Request {
        // `to_file_path` also rejects every scheme other than `file`.
        if let Some(root) = self.root.as_deref()
            && let Ok(path) = request.url.to_file_path()
            && !path.starts_with(root)
            && let Ok(relative) = path.strip_prefix("/")
            && let Ok(mut url) = raikiri::Url::from_file_path(root.join(relative))
        {
            url.set_query(request.url.query());
            url.set_fragment(request.url.fragment());
            request.url = url;
        }
        request
    }
}

impl NetworkProvider for WptFileNetworkProvider {
    fn fetch_one_hop(
        &self,
        request: raikiri_traits::Request,
    ) -> Result<FetchOutcome, NetworkError> {
        raikiri_net::FileNetworkProvider.fetch_one_hop(self.rebase(request))
    }
}

/// Replaced-element resolver that turns a failed load into a broken image.
///
/// A browser renders an image it cannot fetch or decode as a broken image
/// with no natural size and goes on with the page; the reftest renderer would
/// otherwise abort the whole render on the resolver error.
pub(super) struct BrokenImageFallback<'a> {
    inner: &'a dyn ReplacedResolver,
}

impl<'a> BrokenImageFallback<'a> {
    pub(super) fn new(inner: &'a dyn ReplacedResolver) -> Self {
        Self { inner }
    }
}

impl ReplacedResolver for BrokenImageFallback<'_> {
    fn resolve(&self, req: ResolverRequest<'_>) -> Result<ResolvedIntrinsic, ResolverError> {
        match self.inner.resolve(req) {
            Ok(resolved) => Ok(resolved),
            Err(error) => Ok(ResolvedIntrinsic {
                intrinsic: IntrinsicBox::default(),
                disposition: ResolveDisposition::Fallback {
                    reason: error.to_string(),
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests;
