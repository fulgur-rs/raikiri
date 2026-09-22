//! Consumer-facing construction of deterministic font contexts from bundled bytes.

use std::fmt;
use std::sync::Arc;

use parley::FontContext;
use parley::fontique::{
    Blob, Collection, CollectionOptions, FontInfoOverride, GenericFamily, SourceCache,
};

/// Maximum size of one bundled font accepted by [`FontContextBuilder`].
pub const MAX_BUNDLED_FONT_BYTES: u64 = 100 * 1024 * 1024;

/// One bundled OpenType font face source.
///
/// The bytes are owned and shared. The family name is registered as the
/// authored family, so consumers do not need to depend on `parley` or
/// `fontique` to build a font context.
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

struct SharedFontBytes(Arc<[u8]>);

impl AsRef<[u8]> for SharedFontBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Builds a [`FontContext`] from consumer-supplied font bytes.
///
/// System font discovery is disabled by default. Registration order is stable
/// and also defines the fallback order for generic families. Enable system
/// fonts only when host-dependent fallback is acceptable.
#[derive(Debug, Clone, Default)]
pub struct FontContextBuilder {
    fonts: Vec<BundledFont>,
    system_fonts: bool,
}

impl FontContextBuilder {
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

    /// Register all bundled fonts and return the prepared context.
    ///
    /// # Errors
    /// Returns a structured error for an empty font list, invalid family name,
    /// oversized input, or bytes rejected by the font collection.
    pub fn build(self) -> Result<FontContext, FontContextBuildError> {
        if self.fonts.is_empty() {
            return Err(FontContextBuildError::NoFonts);
        }

        let mut context = FontContext {
            source_cache: SourceCache::new_shared(),
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: self.system_fonts,
            }),
        };
        let mut family_ids = Vec::new();

        for font in self.fonts {
            let family = font.family.trim();
            if family.is_empty() {
                return Err(FontContextBuildError::EmptyFamily);
            }
            if font.bytes.is_empty() {
                return Err(FontContextBuildError::EmptyFont {
                    family: family.to_owned(),
                });
            }
            if font.bytes.len() as u64 > MAX_BUNDLED_FONT_BYTES {
                return Err(FontContextBuildError::FontTooLarge {
                    family: family.to_owned(),
                    limit: MAX_BUNDLED_FONT_BYTES,
                    actual: font.bytes.len() as u64,
                });
            }

            let registered = context.collection.register_fonts(
                Blob::new(Arc::new(SharedFontBytes(Arc::clone(&font.bytes))) as _),
                Some(FontInfoOverride {
                    family_name: Some(family),
                    width: None,
                    style: None,
                    weight: None,
                    axes: None,
                }),
            );
            if registered.is_empty() {
                return Err(FontContextBuildError::FontRejected {
                    family: family.to_owned(),
                });
            }
            family_ids.extend(registered.iter().map(|(id, _)| *id));
        }

        // Match the stable fallback behavior used by the WPT font context:
        // all generic families select from the explicitly ordered bundle.
        for generic in [
            GenericFamily::Serif,
            GenericFamily::SansSerif,
            GenericFamily::Monospace,
            GenericFamily::SystemUi,
            GenericFamily::Cursive,
            GenericFamily::Fantasy,
        ] {
            context
                .collection
                .append_generic_families(generic, family_ids.iter().copied());
        }

        Ok(context)
    }
}

/// Failure while building a context from bundled fonts.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FontContextBuildError {
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

impl fmt::Display for FontContextBuildError {
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

impl std::error::Error for FontContextBuildError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_requires_explicit_font_bytes() {
        assert!(matches!(
            FontContextBuilder::new().build(),
            Err(FontContextBuildError::NoFonts)
        ));
    }

    #[test]
    fn builder_rejects_empty_and_invalid_font_sources() {
        assert!(matches!(
            FontContextBuilder::new()
                .font_bytes("   ", b"not a font".as_slice())
                .build(),
            Err(FontContextBuildError::EmptyFamily)
        ));
        assert!(matches!(
            FontContextBuilder::new()
                .font_bytes("Example", b"not a font".as_slice())
                .build(),
            Err(FontContextBuildError::FontRejected { family }) if family == "Example"
        ));
    }

    #[test]
    fn builder_registers_consumer_supplied_font_bytes_without_system_fonts() {
        // Use a small valid test font so the registration path is independent
        // of platform font discovery.
        let mut context = FontContextBuilder::new()
            .font_bytes(
                "Bundled Test",
                include_bytes!("../tests/data/NotoSansTest-Regular.ttf").as_slice(),
            )
            .build()
            .expect("bundled bytes register");
        assert!(context.collection.family_id("Bundled Test").is_some());
    }
}
