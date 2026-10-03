//! Checked raster dimensions and per-document raster memory budgets.

use raikiri_traits::{LimitKind, PageBox, RenderError};

/// Largest accepted raster edge in pixels.
///
/// This also keeps the dimensions below the CPU renderer's `u16` limit.
pub const MAX_RASTER_EDGE: u32 = 16_384;

/// Largest RGBA8 buffer accepted for one page (64 MiB).
pub const MAX_PAGE_RASTER_BYTES: u64 = 64 * 1024 * 1024;

/// Largest cumulative RGBA8 page data retained for one document (256 MiB).
pub const MAX_DOCUMENT_RASTER_BYTES: u64 = 256 * 1024 * 1024;

/// A validated raster buffer size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterBufferSize {
    width: u32,
    height: u32,
    pixels: u64,
    bytes: u64,
}

impl RasterBufferSize {
    /// Return the raster width in pixels.
    pub const fn width(self) -> u32 {
        self.width
    }

    /// Return the raster height in pixels.
    pub const fn height(self) -> u32 {
        self.height
    }

    /// Return the total number of pixels.
    pub const fn pixels(self) -> u64 {
        self.pixels
    }

    /// Return the RGBA8 buffer size in bytes.
    pub const fn bytes(self) -> u64 {
        self.bytes
    }
}

/// Tracks the cumulative RGBA8 buffers retained while rasterizing one document.
#[derive(Debug, Default)]
pub struct RasterBufferBudget {
    reserved_bytes: u64,
}

impl RasterBufferBudget {
    /// Start a fresh per-document raster budget.
    pub const fn new() -> Self {
        Self { reserved_bytes: 0 }
    }

    /// Validate and reserve one page described in CSS pixels.
    pub fn reserve_page_box(&mut self, page_box: PageBox) -> Result<RasterBufferSize, RenderError> {
        let width = checked_page_edge(page_box.width)?;
        let height = checked_page_edge(page_box.height)?;
        self.reserve_dimensions(width, height)
    }

    /// Validate and reserve one page with integer pixel dimensions.
    pub fn reserve_pixels(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<RasterBufferSize, RenderError> {
        self.reserve_dimensions(width, height)
    }

    /// Return the RGBA8 bytes successfully reserved so far.
    pub const fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    fn reserve_dimensions(
        &mut self,
        width: u32,
        height: u32,
    ) -> Result<RasterBufferSize, RenderError> {
        if width == 0 || height == 0 {
            return Err(RenderError::Configuration(
                "raster dimensions must be positive".into(),
            ));
        }

        for edge in [width, height] {
            if edge > MAX_RASTER_EDGE {
                return Err(RenderError::LimitExceeded {
                    kind: LimitKind::RasterEdge,
                    limit: u64::from(MAX_RASTER_EDGE),
                    actual: u64::from(edge),
                });
            }
        }

        let (pixels, bytes) = checked_pixel_bytes(u64::from(width), u64::from(height))?;
        if bytes > MAX_PAGE_RASTER_BYTES {
            return Err(RenderError::LimitExceeded {
                kind: LimitKind::RasterPageBytes,
                limit: MAX_PAGE_RASTER_BYTES,
                actual: bytes,
            });
        }

        let total_bytes = self
            .reserved_bytes
            .checked_add(bytes)
            .ok_or_else(|| raster_byte_limit(LimitKind::RasterDocumentBytes, u64::MAX))?;
        if total_bytes > MAX_DOCUMENT_RASTER_BYTES {
            return Err(raster_byte_limit(
                LimitKind::RasterDocumentBytes,
                total_bytes,
            ));
        }

        self.reserved_bytes = total_bytes;
        Ok(RasterBufferSize {
            width,
            height,
            pixels,
            bytes,
        })
    }
}

fn checked_page_edge(css_pixels: f32) -> Result<u32, RenderError> {
    if !css_pixels.is_finite() || css_pixels <= 0.0 {
        return Err(RenderError::Configuration(
            "page raster dimensions must be finite and positive".into(),
        ));
    }

    let edge = css_pixels.ceil();
    if edge > MAX_RASTER_EDGE as f32 {
        return Err(RenderError::LimitExceeded {
            kind: LimitKind::RasterEdge,
            limit: u64::from(MAX_RASTER_EDGE),
            actual: edge as u64,
        });
    }
    Ok(edge as u32)
}

fn checked_pixel_bytes(width: u64, height: u64) -> Result<(u64, u64), RenderError> {
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| raster_byte_limit(LimitKind::RasterPageBytes, u64::MAX))?;
    let bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| raster_byte_limit(LimitKind::RasterPageBytes, u64::MAX))?;
    Ok((pixels, bytes))
}

fn raster_byte_limit(kind: LimitKind, actual: u64) -> RenderError {
    let limit = match kind {
        LimitKind::RasterDocumentBytes => MAX_DOCUMENT_RASTER_BYTES,
        _ => MAX_PAGE_RASTER_BYTES,
    };
    RenderError::LimitExceeded {
        kind,
        limit,
        actual,
    }
}

#[cfg(test)]
mod tests;
