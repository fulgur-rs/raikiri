//! The measurer of `line-height: normal` a document's cascades run with.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use raikiri_style::{ChFontKey, ComputedLength, NormalLineHeight, NormalLineHeightSource};
use shodo::font::FontCollection;

/// Measures `line-height: normal` for the `lh` unit in the fonts the inline
/// engine lays text out with, so `1lh` matches the height of a `normal` line.
///
/// Measures are kept per family list, size and weight, with one entry per
/// style, since a document uses few fonts and cascades it more than once.
pub(crate) struct EngineNormalLineHeight {
    fonts: Fonts,
    measured: Mutex<NormalLineHeights>,
}

/// The fonts measured in.
enum Fonts {
    /// Fonts given to the document.
    Given(FontCollection),
    /// The installed fonts, taken on the first measure, since scanning the
    /// platform is not free and most documents never measure.
    Installed(OnceLock<FontCollection>),
}

type NormalLineHeights = HashMap<
    (Arc<Vec<raikiri_style::FontFamilyName>>, u32, u32),
    Vec<(raikiri_style::property::FontStyle, Option<f32>)>,
>;

impl EngineNormalLineHeight {
    /// A measurer in `fonts`.
    pub(crate) fn source(fonts: FontCollection) -> NormalLineHeightSource {
        NormalLineHeightSource::new(Self::new(Fonts::Given(fonts)))
    }

    /// The process-wide measurer in the installed fonts
    /// ([`crate::system_font_collection`]), which a document lays out with
    /// when it was given none.
    pub(crate) fn installed() -> NormalLineHeightSource {
        static SHARED: OnceLock<NormalLineHeightSource> = OnceLock::new();
        SHARED
            .get_or_init(|| {
                NormalLineHeightSource::new(Self::new(Fonts::Installed(OnceLock::new())))
            })
            .clone()
    }

    fn new(fonts: Fonts) -> Self {
        Self {
            fonts,
            measured: Mutex::default(),
        }
    }

    fn fonts(&self) -> &FontCollection {
        match &self.fonts {
            Fonts::Given(fonts) => fonts,
            Fonts::Installed(fonts) => fonts.get_or_init(crate::fonts::system_font_collection),
        }
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
        let value = match styles.iter().find(|(style, _)| *style == font.style) {
            Some(&(_, value)) => value,
            None => {
                let value = crate::layout::measure_normal_line_height(self.fonts(), font);
                styles.push((font.style, value));
                value
            }
        };
        value.map(ComputedLength)
    }
}
