//! The painting of the border image of a box (CSS Backgrounds 3 §6).

use super::{
    ImageBrushCache, PixelCrop, averaged_part_brush, css_color, gradient_first_color, pixel_crop,
    redacted_image_url,
};
use anyrender::PaintScene;
use kurbo::{Affine, Rect};
use peniko::Fill;
use raikiri_dom::image_geometry::{
    BorderImageGeometry, BorderImagePart, DeviceGrid, VisibleArea, border_image_geometry_within,
};
use raikiri_style::property::{BackgroundImage, CssColor, Gradient, Sides};
use raikiri_style::{ComputedBorderImage, ComputedValues};
use raikiri_traits::{
    DecodedImage, ImageIntrinsicSize, ImagePixelSource, ImageRasterSize, RenderWarning, WarningKind,
};
use std::sync::Arc;

/// The border image of one box, painted through a transform inside the root
/// element's transform. It emits only the parts and tiles crossing the
/// horizontal and vertical ranges of its visible area, for a caller that
/// clips the image to that area anyway.
///
/// When it is drawn, it replaces the box's border styles; when the source is
/// `none` or cannot be displayed, the caller paints the borders as usual
/// (§6.1). Border radii do not clip a border image. A gradient source paints
/// its first color stop, like gradient backgrounds in this painter.
pub(super) struct BorderImage<'a> {
    image_cache: &'a ImageBrushCache,
    cv: &'a ComputedValues,
    image: &'a ComputedBorderImage,
    /// The border box as x, y, width and height.
    border_box: (f64, f64, f64, f64),
    /// The computed border widths in pixels.
    widths: Sides<f64>,
    grid: Option<DeviceGrid>,
    visible: VisibleArea,
}

impl<'a> BorderImage<'a> {
    /// The border image `image` of a box whose border box is `width` ×
    /// `height` at (`abs_x`, `abs_y`), shown whole.
    pub(super) fn new(
        image_cache: &'a ImageBrushCache,
        (width, height): (f32, f32),
        (abs_x, abs_y): (f32, f32),
        cv: &'a ComputedValues,
        image: &'a ComputedBorderImage,
        border: &Sides<raikiri_style::resolve::ComputedBorder>,
        transform: Affine,
    ) -> Self {
        Self {
            image_cache,
            cv,
            image,
            border_box: (
                f64::from(abs_x),
                f64::from(abs_y),
                f64::from(width),
                f64::from(height),
            ),
            widths: Sides {
                top: f64::from(border.top.width().px()),
                right: f64::from(border.right.width().px()),
                bottom: f64::from(border.bottom.width().px()),
                left: f64::from(border.left.width().px()),
            },
            grid: image_cache.device_grid(transform),
            visible: VisibleArea::UNBOUNDED,
        }
    }

    /// The same border image, showing only what crosses `visible`.
    pub(super) fn within(self, visible: VisibleArea) -> Self {
        Self { visible, ..self }
    }

    /// Paint the border image. Returns whether it was drawn.
    pub(super) fn paint(
        &self,
        scene: &mut impl PaintScene,
        pixel_source: Option<&dyn ImagePixelSource>,
        warnings: &mut Vec<RenderWarning>,
    ) -> bool {
        match &self.image.source {
            BackgroundImage::Url(raw_url) => self.paint_url(scene, raw_url, pixel_source, warnings),
            BackgroundImage::Gradient(gradient) => self.paint_gradient(scene, gradient),
            _ => false,
        }
    }

    fn geometry(&self, natural: Option<ImageIntrinsicSize>) -> Option<BorderImageGeometry> {
        border_image_geometry_within(
            self.border_box,
            self.widths,
            self.image,
            natural,
            self.grid,
            self.visible,
        )
    }

    /// Whether the tiles of `part` can cross the visible area.
    fn crosses(&self, part: &BorderImagePart) -> bool {
        let (area_x, area_y, area_w, area_h) = part.area;
        area_x <= self.visible.x.1
            && area_x + area_w >= self.visible.x.0
            && area_y <= self.visible.y.1
            && area_y + area_h >= self.visible.y.0
    }

    fn paint_url(
        &self,
        scene: &mut impl PaintScene,
        raw_url: &str,
        pixel_source: Option<&dyn ImagePixelSource>,
        warnings: &mut Vec<RenderWarning>,
    ) -> bool {
        let Some((url, pixel_source)) = self.image_cache.image_url(raw_url).zip(pixel_source)
        else {
            return false;
        };
        let Some(natural) = pixel_source.intrinsic_size(&url) else {
            return false;
        };
        let Some(geometry) = self.geometry(Some(natural)) else {
            // A displayable image that draws nothing still replaces the
            // border styles.
            return true;
        };
        let Some(decoded) = decoded_at(pixel_source, &url, raster_size(&geometry)) else {
            warnings.push(RenderWarning::new(
                WarningKind::ResourceFallback {
                    kind: raikiri_traits::ResourceKind::Image,
                    url: Some(redacted_image_url(&url)),
                },
                "CSS border image could not be rasterized; the border styles were painted instead"
                    .into(),
            ));
            return false;
        };
        // Decoded pixels per image unit.
        let (image_w, image_h) = geometry.image_size;
        let pixels_per_unit = (
            f64::from(decoded.width) / image_w,
            f64::from(decoded.height) / image_h,
        );
        let crops = part_crops(&geometry.parts, &decoded, pixels_per_unit);
        let shared = self.shared_copy(&decoded, &crops);
        for (part, crop) in geometry.parts.iter().zip(crops) {
            let painted = crop.filter(|_| self.crosses(part)).and_then(|crop| {
                self.part_brush(part, crop, &decoded, shared.as_ref(), pixels_per_unit)
            });
            if let Some((brush, pixels)) = painted {
                paint_tiles(scene, part, &brush, pixels);
            }
        }
        true
    }

    /// One copy of the whole image that every part samples, when the slices
    /// overlap. Each part normally samples its own pixels, so filtering at
    /// the slice lines does not bleed in the neighbouring slice. When the
    /// slices overlap, the copies would hold more pixels than the image, so
    /// every part samples one shared copy instead.
    fn shared_copy(
        &self,
        decoded: &Arc<DecodedImage>,
        crops: &[Option<PixelCrop>],
    ) -> Option<peniko::ImageBrush> {
        let mut unique: Vec<PixelCrop> = crops.iter().flatten().copied().collect();
        unique.sort_unstable();
        unique.dedup();
        let cropped_pixels: u64 = unique.iter().map(|crop| crop.pixels()).sum();
        (cropped_pixels > u64::from(decoded.width) * u64::from(decoded.height))
            .then(|| self.image_cache.whole(decoded))
    }

    /// The brush that paints `part`, with the transform from its pixels to
    /// image coordinates, or `None` when the part has no pixels to sample.
    fn part_brush(
        &self,
        part: &BorderImagePart,
        crop: PixelCrop,
        decoded: &Arc<DecodedImage>,
        shared: Option<&peniko::ImageBrush>,
        pixels_per_unit: (f64, f64),
    ) -> Option<(peniko::ImageBrush, Affine)> {
        let (src_x, src_y, src_w, src_h) = part.source;
        let (scale_x, scale_y) = pixels_per_unit;
        averaged_part_brush(
            self.image_cache,
            decoded,
            part,
            crop,
            pixels_per_unit,
            self.grid,
        )
        .or_else(|| {
            shared.map(|brush| {
                // Whole decoded pixels to image coordinates.
                (
                    brush.clone(),
                    Affine::scale_non_uniform(1.0 / scale_x, 1.0 / scale_y),
                )
            })
        })
        .or_else(|| {
            self.image_cache.crop(decoded, crop).map(|brush| {
                // Cropped pixels to image coordinates.
                let pixels = Affine::translate((src_x, src_y))
                    * Affine::scale_non_uniform(
                        src_w / f64::from(crop.width()),
                        src_h / f64::from(crop.height()),
                    );
                (brush, pixels)
            })
        })
    }

    fn paint_gradient(&self, scene: &mut impl PaintScene, gradient: &Gradient) -> bool {
        // A gradient with nothing to draw still replaces the border styles.
        let drawable = self
            .geometry(None)
            .zip(gradient_first_color(gradient, self.cv.color));
        if let Some((geometry, color)) = drawable {
            // Fill each tile, so `space` keeps its gaps transparent.
            for part in geometry.parts.iter().filter(|part| self.crosses(part)) {
                paint_solid_tiles(scene, part, color);
            }
        }
        true
    }
}

/// The size to rasterize a vector image at, so that its parts are drawn at
/// the scale they are shown at, as backgrounds rasterize at their used size.
/// Each part counts with its smaller axis scale, the one across a stretched
/// edge, so stretching a thin slice along a long edge does not ask for a
/// raster of the whole page. A raster image keeps its natural pixels
/// whatever size is asked for.
fn raster_size(geometry: &BorderImageGeometry) -> ImageRasterSize {
    let (image_w, image_h) = geometry.image_size;
    let scale = geometry
        .parts
        .iter()
        .map(|part| {
            let (_, _, src_w, src_h) = part.source;
            (part.tile_width / src_w).min(part.tile_height / src_h)
        })
        .filter(|scale| scale.is_finite() && *scale > 0.0)
        .fold(None, |max: Option<f64>, scale| {
            Some(max.map_or(scale, |max| max.max(scale)))
        })
        .unwrap_or(1.0);
    ImageRasterSize {
        width: (image_w * scale) as f32,
        height: (image_h * scale) as f32,
    }
}

/// The image at `url` decoded for `size`. A buffer shorter than its size says
/// draws nothing, so it falls back like an image that could not be
/// rasterized.
fn decoded_at(
    pixel_source: &dyn ImagePixelSource,
    url: &url::Url,
    size: ImageRasterSize,
) -> Option<Arc<DecodedImage>> {
    pixel_source
        .get_decoded_at_size(url, size, None)
        .filter(|decoded| {
            decoded.width > 0
                && decoded.height > 0
                && (decoded.width as usize)
                    .checked_mul(decoded.height as usize)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .is_some_and(|len| decoded.rgba.len() >= len)
        })
}

/// The whole decoded pixels that each of `parts` shows, given the decoded
/// pixels per image unit.
fn part_crops(
    parts: &[BorderImagePart],
    decoded: &DecodedImage,
    (scale_x, scale_y): (f64, f64),
) -> Vec<Option<PixelCrop>> {
    parts
        .iter()
        .map(|part| {
            let (src_x, src_y, src_w, src_h) = part.source;
            pixel_crop(
                decoded,
                (src_x * scale_x, src_y * scale_y),
                ((src_x + src_w) * scale_x, (src_y + src_h) * scale_y),
            )
        })
        .collect()
}

/// Fill the tiles of `part`, clipped to its area, with `color`.
fn paint_solid_tiles(scene: &mut impl PaintScene, part: &BorderImagePart, color: CssColor) {
    let (area_x, area_y, area_w, area_h) = part.area;
    scene.push_clip_layer(
        Affine::IDENTITY,
        &Rect::new(area_x, area_y, area_x + area_w, area_y + area_h),
    );
    for tile_y in &part.y {
        for tile_x in &part.x {
            scene.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                css_color(color),
                None,
                &Rect::new(
                    *tile_x,
                    *tile_y,
                    tile_x + part.tile_width,
                    tile_y + part.tile_height,
                ),
            );
        }
    }
    scene.pop_layer();
}

/// Draw the tiles of `part`, clipped to its area, from `brush`, whose pixels
/// `pixels` takes to image coordinates.
fn paint_tiles(
    scene: &mut impl PaintScene,
    part: &BorderImagePart,
    brush: &peniko::ImageBrush,
    pixels: Affine,
) {
    let (src_x, src_y, src_w, src_h) = part.source;
    let (area_x, area_y, area_w, area_h) = part.area;
    let source = Rect::new(src_x, src_y, src_x + src_w, src_y + src_h);
    scene.push_clip_layer(
        Affine::IDENTITY,
        &Rect::new(area_x, area_y, area_x + area_w, area_y + area_h),
    );
    let scale = Affine::scale_non_uniform(part.tile_width / src_w, part.tile_height / src_h)
        * Affine::translate((-src_x, -src_y));
    for tile_y in &part.y {
        for tile_x in &part.x {
            scene.fill(
                Fill::NonZero,
                Affine::translate((*tile_x, *tile_y)) * scale,
                brush.as_ref(),
                Some(pixels),
                &source,
            );
        }
    }
    scene.pop_layer();
}
