//! Consumer-facing construction of deterministic font sets from bundled bytes.

use std::fmt;
use std::sync::Arc;

/// Maximum size of one bundled font accepted by [`FontCollectionBuilder`].
pub const MAX_BUNDLED_FONT_BYTES: u64 = 100 * 1024 * 1024;

/// One bundled OpenType font face source.
///
/// The bytes are owned and shared. The family name is registered as the
/// authored family, so consumers do not need to depend on the text engine to
/// build a font set.
#[derive(Clone)]
#[non_exhaustive]
pub struct BundledFont {
    family: String,
    bytes: Arc<[u8]>,
}

impl BundledFont {
    /// Construct a bundled font from an explicit family name and encoded bytes.
    pub fn new(family: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            family: family.into(),
            bytes: bytes.into(),
        }
    }

    /// The CSS family name assigned to the registered font.
    pub fn family(&self) -> &str {
        &self.family
    }

    /// Encoded font bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for BundledFont {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BundledFont")
            .field("family", &self.family)
            .field("byte_len", &self.bytes.len())
            .finish()
    }
}

/// Builds a [`RenderFonts`] from consumer-supplied font bytes.
///
/// System font discovery is disabled by default. Registration order is stable
/// and also defines the fallback order for generic families. Enable system
/// fonts only when host-dependent fallback is acceptable.
#[derive(Debug, Clone, Default)]
pub struct FontCollectionBuilder {
    fonts: Vec<BundledFont>,
    system_fonts: bool,
}

impl FontCollectionBuilder {
    /// Create a deterministic builder with system font discovery disabled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one font in fallback order.
    pub fn font(mut self, font: BundledFont) -> Self {
        self.fonts.push(font);
        self
    }

    /// Add one font from bytes in fallback order.
    pub fn font_bytes(self, family: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
        self.font(BundledFont::new(family, bytes))
    }

    /// Enable platform font discovery. This makes fallback host-dependent.
    pub fn system_fonts(mut self, enabled: bool) -> Self {
        self.system_fonts = enabled;
        self
    }

    /// Register all bundled fonts and return the prepared font set.
    ///
    /// Every family name resolves to its fonts, and every generic family
    /// (`serif`, `sans-serif`, `monospace`, ...) to the bundle in
    /// registration order.
    ///
    /// # Errors
    /// Returns a structured error for an empty font list, invalid family name,
    /// oversized input, or bytes rejected by the font collection.
    pub fn build(self) -> Result<RenderFonts, FontCollectionBuildError> {
        if self.fonts.is_empty() {
            return Err(FontCollectionBuildError::NoFonts);
        }
        let mut faces = Vec::with_capacity(self.fonts.len());
        for font in &self.fonts {
            let family = font.family.trim();
            if family.is_empty() {
                return Err(FontCollectionBuildError::EmptyFamily);
            }
            if font.bytes.is_empty() {
                return Err(FontCollectionBuildError::EmptyFont {
                    family: family.to_owned(),
                });
            }
            if font.bytes.len() as u64 > MAX_BUNDLED_FONT_BYTES {
                return Err(FontCollectionBuildError::FontTooLarge {
                    family: family.to_owned(),
                    limit: MAX_BUNDLED_FONT_BYTES,
                    actual: font.bytes.len() as u64,
                });
            }
            faces.push(raikiri_dom::BundledFace {
                family: family.to_owned(),
                bytes: font.bytes.to_vec(),
            });
        }
        let system_fonts = self.system_fonts;
        let collection = raikiri_dom::build_bundled_font_collection(faces.clone(), system_fonts)
            .map_err(|_| {
                // Name the first font the collection refuses on its own.
                let family = faces
                    .iter()
                    .find(|face| {
                        raikiri_dom::build_bundled_font_collection(vec![(*face).clone()], false)
                            .is_err()
                    })
                    .unwrap_or(&faces[0])
                    .family
                    .clone();
                FontCollectionBuildError::FontRejected { family }
            })?;
        Ok(RenderFonts { collection })
    }
}

/// A font set for layout and paint, built by [`FontCollectionBuilder`].
///
/// Cloning shares the font set.
#[derive(Clone)]
pub struct RenderFonts {
    collection: shodo::font::FontCollection,
}

impl RenderFonts {
    /// Whether the set was built without the installed fonts. Only then does
    /// the answer to a font lookup not depend on which installed face was
    /// loaded first.
    pub fn is_bundled_only(&self) -> bool {
        self.collection.is_bundled_only()
    }

    /// The font layer of the inline layout engine.
    pub fn collection(&self) -> &shodo::font::FontCollection {
        &self.collection
    }

    /// The font layer of the inline layout engine, by value.
    pub fn into_collection(self) -> shodo::font::FontCollection {
        self.collection
    }
}

impl fmt::Debug for RenderFonts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RenderFonts")
            .field("bundled_only", &self.is_bundled_only())
            .finish_non_exhaustive()
    }
}

/// Failure while building a font set from bundled fonts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontCollectionBuildError {
    /// No bundled fonts were supplied.
    NoFonts,
    /// A font family name was empty or whitespace-only.
    EmptyFamily,
    /// The named font had no bytes.
    EmptyFont {
        /// CSS family name.
        family: String,
    },
    /// A font exceeded [`MAX_BUNDLED_FONT_BYTES`].
    FontTooLarge {
        /// CSS family name.
        family: String,
        /// Maximum accepted byte count.
        limit: u64,
        /// Actual byte count.
        actual: u64,
    },
    /// The font collection rejected the supplied bytes.
    FontRejected {
        /// CSS family name.
        family: String,
    },
}

impl fmt::Display for FontCollectionBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFonts => write!(f, "no bundled fonts were supplied"),
            Self::EmptyFamily => write!(f, "bundled font family name must not be empty"),
            Self::EmptyFont { family } => write!(f, "bundled font {family:?} has no bytes"),
            Self::FontTooLarge {
                family,
                limit,
                actual,
            } => write!(
                f,
                "bundled font {family:?} exceeds its byte limit ({actual} > {limit})"
            ),
            Self::FontRejected { family } => {
                write!(f, "font collection rejected bundled font {family:?}")
            }
        }
    }
}

impl std::error::Error for FontCollectionBuildError {}

#[cfg(test)]
mod tests;
