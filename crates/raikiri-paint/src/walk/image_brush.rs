use super::PixelCrop;
use raikiri_traits::DecodedImage;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Shares decoded pixels and isolated border slices during one paint call.
#[derive(Default)]
pub(crate) struct ImageBrushCache {
    brushes: RefCell<HashMap<(usize, PixelCrop), CachedBrush>>,
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
