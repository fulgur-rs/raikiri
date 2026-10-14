//! The measurer of `line-height: normal` a document's cascades run with.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use raikiri_style::{ChFontKey, ComputedLength, NormalLineHeight, NormalLineHeightSource};
use shodo::font::FontCollection;

/// Measures `line-height: normal` for the `lh` unit in the fonts the inline
/// engine lays text out with, so `1lh` matches the height of a `normal` line.
///
/// Measures are kept per family list, size and weight, with one entry per
/// style and optical sizing, since a document uses few fonts and cascades it more than once.
pub(crate) struct EngineNormalLineHeight {
    fonts: FontCollection,
    measured: Mutex<NormalLineHeights>,
}

type NormalLineHeights = HashMap<
    (Arc<Vec<raikiri_style::FontFamilyName>>, u32, u32),
    Vec<(
        raikiri_style::property::FontStyle,
        raikiri_style::property::FontOpticalSizing,
        Option<f32>,
    )>,
>;

impl EngineNormalLineHeight {
    /// A measurer in `fonts`, with an empty cache.
    pub(crate) fn source(fonts: FontCollection) -> NormalLineHeightSource {
        NormalLineHeightSource::new(Self {
            fonts,
            measured: Mutex::default(),
        })
    }
}

impl NormalLineHeight for EngineNormalLineHeight {
    fn normal_line_height(&self, font: &ChFontKey) -> Option<ComputedLength> {
        let key = (
            font.family.clone(),
            font.size.0.to_bits(),
            font.weight.to_bits(),
        );
        let mut measured = self.measured.lock().unwrap_or_else(|e| e.into_inner());
        let styles = measured.entry(key).or_default();
        let value = match styles.iter().find(|(style, optical_sizing, _)| {
            *style == font.style && *optical_sizing == font.optical_sizing
        }) {
            Some(&(_, _, value)) => value,
            None => {
                let value = crate::layout::measure_normal_line_height(&self.fonts, font);
                styles.push((font.style, font.optical_sizing, value));
                value
            }
        };
        value.map(ComputedLength)
    }
}

#[cfg(test)]
mod tests;
