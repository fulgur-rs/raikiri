//! Consumer-facing construction of deterministic font sets from bundled bytes.

use std::fmt;
use std::sync::Arc;

/// Maximum size of one bundled font accepted by [`FontCollectionBuilder`].
pub const MAX_BUNDLED_FONT_BYTES: u64 = 100 * 1024 * 1024;

/// One bundled OpenType font source.
///
/// The bytes are owned and shared. The family name is registered as the
/// authored family, so consumers do not need to depend on the text engine to
/// build a font set. A font collection (`.ttc` / `.otc`) is bundled with
/// [`BundledFont::from_ttc`] instead, because its faces belong to different
/// families that only the font knows.
#[derive(Clone)]
#[non_exhaustive]
pub struct BundledFont {
    family: String,
    bytes: Arc<[u8]>,
    recorded_families: bool,
}

impl BundledFont {
    /// Construct a bundled font from an explicit family name and encoded bytes.
    ///
    /// Only the first face of a collection is registered; bundle a collection
    /// with [`BundledFont::from_ttc`].
    pub fn new(family: impl Into<String>, bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            family: family.into(),
            bytes: bytes.into(),
            recorded_families: false,
        }
    }

    /// Construct a bundled font from the bytes of a TrueType/OpenType
    /// collection (`.ttc` / `.otc`), or of a single font.
    ///
    /// Every face is registered under the family name its own name table
    /// records (the typographic family where there is one), with the weight,
    /// width and style the face states, so a face is selected by its family
    /// name in CSS as it is from an installed collection. A face without a
    /// family name is kept as a last resort only. The families join the generic
    /// families in face order, after the fonts bundled before the collection.
    ///
    /// The file counts once against the size limits: [`MAX_BUNDLED_FONT_BYTES`]
    /// and the limits of the font library, which apply to the whole file and
    /// not to each face. [`BundledFont::family`] is empty for a font made this
    /// way.
    pub fn from_ttc(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            family: String::new(),
            bytes: bytes.into(),
            recorded_families: true,
        }
    }

    /// The CSS family name assigned to the registered font; empty for a font
    /// made with [`BundledFont::from_ttc`], whose faces are registered under
    /// the names the font records.
    pub fn family(&self) -> &str {
        &self.family
    }

    /// Encoded font bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl BundledFont {
    /// The face handed to the font layer, after the checks that need no font
    /// parsing.
    fn face(&self) -> Result<raikiri_dom::BundledFace, FontCollectionBuildError> {
        let family = self.family.trim();
        if !self.recorded_families && family.is_empty() {
            return Err(FontCollectionBuildError::EmptyFamily);
        }
        if self.bytes.is_empty() {
            return Err(FontCollectionBuildError::EmptyFont {
                family: family.to_owned(),
            });
        }
        if self.bytes.len() as u64 > MAX_BUNDLED_FONT_BYTES {
            return Err(FontCollectionBuildError::FontTooLarge {
                family: family.to_owned(),
                limit: MAX_BUNDLED_FONT_BYTES,
                actual: self.bytes.len() as u64,
            });
        }
        // An empty family registers the font under the names it records.
        Ok(raikiri_dom::BundledFace {
            family: family.to_owned(),
            bytes: self.bytes.to_vec(),
        })
    }
}

impl fmt::Debug for BundledFont {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BundledFont")
            .field("family", &self.family)
            .field("recorded_families", &self.recorded_families)
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

    /// Add a TrueType/OpenType collection (`.ttc` / `.otc`) from bytes in
    /// fallback order; see [`BundledFont::from_ttc`].
    pub fn ttc_bytes(self, bytes: impl Into<Arc<[u8]>>) -> Self {
        self.font(BundledFont::from_ttc(bytes))
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
            faces.push(font.face()?);
        }
        let system_fonts = self.system_fonts;
        let collection =
            raikiri_dom::build_bundled_font_collection(faces, system_fonts).map_err(|_| {
                // Name the first font the collection refuses on its own.
                // Recreate owned bytes only on failure, from the shared sources.
                let font = self
                    .fonts
                    .iter()
                    .find(|font| {
                        font.face().is_ok_and(|face| {
                            raikiri_dom::build_bundled_font_collection(vec![face], false).is_err()
                        })
                    })
                    .unwrap_or(&self.fonts[0]);
                FontCollectionBuildError::FontRejected {
                    family: font.family.trim().to_owned(),
                }
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
        /// CSS family name; empty for a font made with
        /// [`BundledFont::from_ttc`].
        family: String,
    },
    /// A font exceeded [`MAX_BUNDLED_FONT_BYTES`].
    FontTooLarge {
        /// CSS family name; empty for a font made with
        /// [`BundledFont::from_ttc`].
        family: String,
        /// Maximum accepted byte count.
        limit: u64,
        /// Actual byte count.
        actual: u64,
    },
    /// The font collection rejected the supplied bytes.
    FontRejected {
        /// CSS family name; empty for a font made with
        /// [`BundledFont::from_ttc`].
        family: String,
    },
}

impl fmt::Display for FontCollectionBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A font made with `BundledFont::from_ttc` has no authored family name.
        let label = |family: &str| {
            if family.is_empty() {
                "bundled TTC/OTC font".to_owned()
            } else {
                format!("bundled font {family:?}")
            }
        };
        match self {
            Self::NoFonts => write!(f, "no bundled fonts were supplied"),
            Self::EmptyFamily => write!(f, "bundled font family name must not be empty"),
            Self::EmptyFont { family } => write!(f, "{} has no bytes", label(family)),
            Self::FontTooLarge {
                family,
                limit,
                actual,
            } => write!(
                f,
                "{} exceeds its byte limit ({actual} > {limit})",
                label(family)
            ),
            Self::FontRejected { family } => {
                write!(f, "font collection rejected {}", label(family))
            }
        }
    }
}

impl std::error::Error for FontCollectionBuildError {}

#[cfg(test)]
mod tests;
