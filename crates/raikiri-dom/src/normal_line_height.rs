//! The measurer of `line-height: normal` a document's cascades run with.

use std::sync::Mutex;

mod cache;
use cache::Measurements;

use raikiri_style::{ChFontKey, ComputedLength, NormalLineHeight, NormalLineHeightSource};
use shodo::font::FontCollection;

/// Measures `line-height: normal` for the `lh` unit in the fonts the inline
/// engine lays text out with, so `1lh` matches the height of a `normal` line.
///
/// Exact measurements are reused per family, size, weight, style and optical
/// sizing. Each metric retains at most 512 values and 256 KiB of charged family
/// data, evicting the oldest key buckets without changing measurement results.
pub(crate) struct EngineNormalLineHeight {
    fonts: FontCollection,
    measured: Mutex<NormalLineHeights>,
    ch_measured: Mutex<ChAdvances>,
}

type NormalLineHeights = Measurements<Option<f32>>;
type ChAdvances = Measurements<f32>;

impl EngineNormalLineHeight {
    /// A measurer in `fonts`, with an empty cache.
    pub(crate) fn source(fonts: FontCollection) -> NormalLineHeightSource {
        NormalLineHeightSource::new(Self {
            fonts,
            measured: Mutex::default(),
            ch_measured: Mutex::default(),
        })
    }
}

impl NormalLineHeight for EngineNormalLineHeight {
    fn normal_line_height(&self, font: &ChFontKey) -> Option<ComputedLength> {
        let mut measured = self.measured.lock().unwrap_or_else(|e| e.into_inner());
        measured
            .get_or_measure(font, || {
                crate::layout::measure_normal_line_height(&self.fonts, font)
            })
            .map(ComputedLength)
    }

    fn ch_advance(&self, font: &ChFontKey) -> Option<ComputedLength> {
        let mut measured = self.ch_measured.lock().unwrap_or_else(|e| e.into_inner());
        Some(ComputedLength(measured.get_or_measure(font, || {
            crate::layout::measure_ch_advance(&self.fonts, font)
        })))
    }
}

#[cfg(test)]
mod tests;
