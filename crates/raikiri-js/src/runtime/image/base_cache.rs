//! The latest base URL result, including an absent result.
use raikiri_dom::{BaseUrlVersion, Document};

#[derive(Default)]
pub(crate) struct BaseUrlCache {
    entry: Option<Entry>,
}
struct Entry {
    version: BaseUrlVersion,
    host_url: Option<String>,
    base: Option<url::Url>,
}
impl BaseUrlCache {
    pub(super) fn resolve(
        &mut self,
        document: &Document,
        host_url: Option<String>,
        src: &str,
    ) -> Option<String> {
        let stale = self.entry.as_ref().is_none_or(|entry| {
            &entry.version != document.base_url_version() || entry.host_url != host_url
        });
        if stale {
            let base = match host_url
                .as_deref()
                .and_then(|text| url::Url::parse(text).ok())
            {
                Some(url) => document.live_base_url(Some(&url)),
                None => document.base_url().cloned(),
            };
            self.entry = Some(Entry {
                version: document.base_url_version().clone(),
                host_url,
                base,
            });
        }
        // Parsing with the cached base preserves same-scheme and file URL
        // behavior. A successful standalone parse is not a safe shortcut.
        let base = self.entry.as_ref().and_then(|entry| entry.base.as_ref());
        url::Url::options()
            .base_url(base)
            .parse(src)
            .ok()
            .map(String::from)
    }
}
#[cfg(test)]
mod tests;
