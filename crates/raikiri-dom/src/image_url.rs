//! The URL an image named in authored CSS stands for.
//!
//! The images of `content: url()` in generated content and in margin boxes,
//! and the image of `list-style-image`, all resolve their authored text here,
//! so that the three cannot disagree about which image a `url()` names.

use url::Url;

/// The absolute URL of the image that the authored `url()` text `raw` names.
///
/// An absolute URL stands for itself, a relative one resolves against
/// `base_url`, and one that needs a base when there is none names no image. An
/// empty or fragment-only URL names no external image either: joining it
/// against the base would turn it into the document itself.
///
/// The fragment is dropped. It selects a part of the resource and is not part
/// of the identity an image is fetched and cached under, so it must not make
/// two spellings of one image into two requests.
pub(crate) fn authored_image_url(raw: &str, base_url: Option<&Url>) -> Option<Url> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let mut url = Url::parse(raw)
        .ok()
        .or_else(|| base_url.and_then(|base| base.join(raw).ok()))?;
    url.set_fragment(None);
    Some(url)
}

#[cfg(test)]
mod tests;
