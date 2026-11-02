use super::PixelCrop;
use kurbo::Affine;
use raikiri_dom::image_geometry::DeviceGrid;
use raikiri_traits::DecodedImage;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;

/// Shares decoded pixels and isolated border slices during one paint call,
/// resolves the image URLs of that call against the document base URL, and
/// knows the raster output's device pixels, which border images snap their
/// seams to.
#[derive(Default)]
pub(crate) struct ImageBrushCache {
    conic_budget: super::gradient::ConicPaintBudget,
    brushes: RefCell<HashMap<(usize, PixelCrop), CachedBrush>>,
    averaged: RefCell<HashMap<AveragedKey, CachedBrush>>,
    base_url: Option<url::Url>,
    device_scale: Option<f64>,
    /// The root element's transform, which the document paints through
    /// once the page's own decorations are drawn.
    root_transform: Cell<Affine>,
}

struct CachedBrush {
    // Keep the allocation alive so its address cannot be reused for another image.
    _owner: Arc<DecodedImage>,
    brush: peniko::ImageBrush,
}

/// An image, the bits of the region of it that is averaged, and the size the
/// region is averaged to.
type AveragedKey = (usize, [u64; 4], u32, u32);

struct SharedPixels(Arc<DecodedImage>);

impl AsRef<[u8]> for SharedPixels {
    fn as_ref(&self) -> &[u8] {
        &self.0.rgba
    }
}

impl ImageBrushCache {
    /// Work for one conic, charged to every background in this paint call.
    pub(super) fn conic_budget(&self) -> super::gradient::ConicBudget<'_> {
        self.conic_budget.for_gradient()
    }

    /// A cache for one paint call over `document`.
    pub(crate) fn for_document(document: &raikiri_dom::Document) -> Self {
        Self {
            conic_budget: super::gradient::ConicPaintBudget::default(),
            brushes: RefCell::default(),
            averaged: RefCell::default(),
            base_url: document.base_url().cloned(),
            device_scale: None,
            root_transform: Cell::new(Affine::IDENTITY),
        }
    }

    /// This cache for a raster output with `device_scale` device pixels per
    /// CSS pixel, or for a vector output with `None` or a scale that is not
    /// finite and positive.
    pub(crate) fn with_device_scale(mut self, device_scale: Option<f32>) -> Self {
        self.device_scale = device_scale
            .filter(|scale| scale.is_finite() && *scale > 0.0)
            .map(f64::from);
        self
    }

    /// Paint what follows through the root element's `transform`.
    pub(crate) fn set_root_transform(&self, transform: Affine) {
        self.root_transform.set(transform);
    }

    /// The device pixel grid of a raster output for what is painted through
    /// `transform` inside the root element's transform, or `None` for a
    /// vector output or a transform that rotates or skews, whose seams
    /// cannot meet on device pixels.
    pub(crate) fn device_grid(&self, transform: Affine) -> Option<DeviceGrid> {
        let scale = self.device_scale?;
        let [a, b, c, d, e, f] =
            (Affine::scale(scale) * self.root_transform.get() * transform).as_coeffs();
        (b == 0.0 && c == 0.0).then_some(DeviceGrid::new(a, e, d, f))
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

    /// The pixels of `region` (decoded pixel coordinates `[x0, y0, x1, y1]`,
    /// possibly fractional) of `decoded`, averaged down to `size` pixels so
    /// that each stands for the mean color of the area it covers, or `None`
    /// when the region or the size holds no pixel.
    ///
    /// Drawing an image much smaller than its pixels with a bilinear filter
    /// reads only the few pixels around each sample point and skips the
    /// rest, which turns every seam between two colors into a hard step.
    pub(super) fn averaged(
        &self,
        decoded: &Arc<DecodedImage>,
        region: [f64; 4],
        size: (u32, u32),
    ) -> Option<peniko::ImageBrush> {
        let key = (
            Arc::as_ptr(decoded) as usize,
            region.map(f64::to_bits),
            size.0,
            size.1,
        );
        let mut brushes = self.averaged.borrow_mut();
        if let Some(cached) = brushes.get(&key) {
            return Some(cached.brush.clone());
        }
        let rgba = box_average(decoded, region, size)?;
        let brush = peniko::ImageBrush::new(peniko::ImageData {
            data: peniko::Blob::from(rgba),
            format: peniko::ImageFormat::Rgba8,
            alpha_type: peniko::ImageAlphaType::Alpha,
            width: size.0,
            height: size.1,
        });
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

/// For each of `count` equal parts of `from..to`, the first pixel it overlaps
/// and how far it overlaps each pixel from there, in a row of `limit` pixels.
fn axis_overlaps(from: f64, to: f64, count: u32, limit: usize) -> Option<Vec<(usize, Vec<f32>)>> {
    let limit_f = limit as f64;
    if !(from.is_finite() && to.is_finite() && to > from && from < limit_f && to > 0.0) {
        return None;
    }
    let step = (to - from) / f64::from(count);
    Some(
        (0..count)
            .map(|index| {
                let start = (from + step * f64::from(index)).clamp(0.0, limit_f);
                let end = (from + step * f64::from(index + 1)).clamp(0.0, limit_f);
                let first = start.floor() as usize;
                let last = (end.ceil() as usize).min(limit);
                let overlaps = (first..last)
                    .map(|pixel| {
                        ((end.min(pixel as f64 + 1.0) - start.max(pixel as f64)) as f32).max(0.0)
                    })
                    .collect();
                (first, overlaps)
            })
            .collect(),
    )
}

/// The mean premultiplied color of `line`, one row of straight-alpha RGBA8
/// pixels, under each of `columns`, written to `means`.
fn average_row(line: &[u8], columns: &[(usize, Vec<f32>)], means: &mut [[f32; 4]]) {
    for ((first, overlaps), mean) in columns.iter().zip(means) {
        *mean = [0.0; 4];
        let mut total = 0.0;
        for (pixel, overlap) in line[first * 4..].chunks_exact(4).zip(overlaps) {
            let alpha = f32::from(pixel[3]) / 255.0;
            for channel in 0..3 {
                mean[channel] += f32::from(pixel[channel]) / 255.0 * alpha * overlap;
            }
            mean[3] += alpha * overlap;
            total += overlap;
        }
        if total > 0.0 {
            mean.iter_mut().for_each(|value| *value /= total);
        }
    }
}

/// `region` of `decoded` averaged to `width` × `height` straight-alpha RGBA8
/// pixels, weighing every source pixel by the area of it that a result pixel
/// covers. Colors are averaged with their alpha, so a transparent pixel adds
/// no color.
fn box_average(
    decoded: &DecodedImage,
    [x0, y0, x1, y1]: [f64; 4],
    (width, height): (u32, u32),
) -> Option<Vec<u8>> {
    let (source_w, source_h) = (decoded.width as usize, decoded.height as usize);
    let source_len = source_w.checked_mul(source_h)?.checked_mul(4)?;
    if width == 0 || height == 0 || decoded.rgba.len() < source_len {
        return None;
    }
    let columns = axis_overlaps(x0, x1, width, source_w)?;
    let rows = axis_overlaps(y0, y1, height, source_h)?;
    let width = width as usize;
    let byte = |value: f32| (value * 255.0 + 0.5).clamp(0.0, 255.0) as u8;
    let mut rgba = Vec::with_capacity(width * height as usize * 4);
    // One source row averaged across, and the rows of a result row summed down.
    let mut across = vec![[0.0f32; 4]; width];
    let mut down = vec![[0.0f32; 4]; width];
    for (first, overlaps) in &rows {
        down.fill([0.0; 4]);
        for (offset, overlap) in overlaps.iter().enumerate() {
            let row = first + offset;
            average_row(
                &decoded.rgba[row * source_w * 4..][..source_w * 4],
                &columns,
                &mut across,
            );
            for (sum, mean) in down.iter_mut().zip(&across) {
                sum.iter_mut()
                    .zip(mean)
                    .for_each(|(sum, value)| *sum += value * overlap);
            }
        }
        let total: f32 = overlaps.iter().sum();
        for sum in &down {
            let alpha = if total > 0.0 { sum[3] / total } else { 0.0 };
            if alpha > 0.0 {
                // Back to straight alpha: the premultiplied mean over the alpha mean.
                rgba.extend((0..3).map(|channel| byte(sum[channel] / total / alpha)));
            } else {
                rgba.extend([0; 3]);
            }
            rgba.push(byte(alpha));
        }
    }
    Some(rgba)
}

#[cfg(test)]
mod tests;
