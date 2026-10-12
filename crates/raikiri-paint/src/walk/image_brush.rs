use super::PixelCrop;
use raikiri_traits::DecodedImage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Shares decoded pixels and isolated border slices during one paint call,
/// and resolves the image URLs of that call against the document base URL.
#[derive(Default)]
pub(crate) struct ImageBrushCache {
    brushes: RefCell<HashMap<(usize, PixelCrop), CachedBrush>>,
    base_url: Option<url::Url>,
}

struct CachedBrush {
    // Keep the allocation alive so its address cannot be reused for another image.
    _owner: Arc<DecodedImage>,
    brush: peniko::ImageBrush,
}

struct SharedPixels(Arc<DecodedImage>);

impl AsRef<[u8]> for SharedPixels {
    fn as_ref(&self) -> &[u8] {
        &self.0.rgba
    }
}

impl ImageBrushCache {
    /// A cache for one paint call over `document`.
    pub(crate) fn for_document(document: &raikiri_dom::Document) -> Self {
        Self {
            brushes: RefCell::default(),
            base_url: document.base_url().cloned(),
        }
    }

    /// The URL a computed `url()` value names.
    ///
    /// Stylesheet URLs are made absolute when the sheet is parsed, but a
    /// `style` attribute keeps its authored text, so a relative URL still
    /// left here resolves against the document base URL (CSS Values 4
    /// §4.5.1). An empty or fragment-only URL names no external image, so
    /// it resolves to nothing rather than to the document itself.
    pub(crate) fn resolve_url(&self, raw_url: &str) -> Option<url::Url> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            return None;
        }
        match url::Url::parse(raw_url) {
            Ok(url) => Some(url),
            Err(url::ParseError::RelativeUrlWithoutBase) => {
                self.base_url.as_ref()?.join(raw_url).ok()
            }
            Err(_) => None,
        }
    }

    /// [`Self::resolve_url`] without the fragment, the key background and
    /// border images are fetched under.
    pub(crate) fn image_url(&self, raw_url: &str) -> Option<url::Url> {
        let mut url = self.resolve_url(raw_url)?;
        url.set_fragment(None);
        Some(url)
    }

    pub(crate) fn whole(&self, decoded: &Arc<DecodedImage>) -> peniko::ImageBrush {
        let crop = PixelCrop::whole(decoded);
        let key = (Arc::as_ptr(decoded) as usize, crop);
        let mut brushes = self.brushes.borrow_mut();
        brushes
            .entry(key)
            .or_insert_with(|| CachedBrush {
                _owner: Arc::clone(decoded),
                brush: peniko::ImageBrush::new(peniko::ImageData {
                    data: peniko::Blob::new(Arc::new(SharedPixels(Arc::clone(decoded)))),
                    format: peniko::ImageFormat::Rgba8,
                    alpha_type: peniko::ImageAlphaType::Alpha,
                    width: decoded.width,
                    height: decoded.height,
                }),
            })
            .brush
            .clone()
    }

    pub(super) fn crop(
        &self,
        decoded: &Arc<DecodedImage>,
        crop: PixelCrop,
    ) -> Option<peniko::ImageBrush> {
        if crop == PixelCrop::whole(decoded) {
            let len = (decoded.width as usize)
                .checked_mul(decoded.height as usize)?
                .checked_mul(4)?;
            decoded.rgba.get(..len)?;
            return Some(self.whole(decoded));
        }
        let key = (Arc::as_ptr(decoded) as usize, crop);
        let mut brushes = self.brushes.borrow_mut();
        if let Some(cached) = brushes.get(&key) {
            return Some(cached.brush.clone());
        }
        let brush = crop.brush(decoded)?;
        brushes.insert(
            key,
            CachedBrush {
                _owner: Arc::clone(decoded),
                brush: brush.clone(),
            },
        );
        Some(brush)
    }
}

#[cfg(test)]
mod tests;
