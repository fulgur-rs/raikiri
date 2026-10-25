//! How an SVG picture is drawn: as vectors or as pixels.

use raikiri_style::ComputedValues;

/// The custom property that picks the rendering of one SVG picture.
const PROPERTY: &str = "--raikiri-svg-rendering";

/// How a consumer draws an SVG picture.
///
/// The choice is the library's; the drawing is the consumer's. Raikiri reports
/// the choice for each picture through [`crate::Page::svg_rendering`], and a
/// consumer that draws vector scenes and pixel images follows it.
///
/// An element picks its rendering with the custom property
/// `--raikiri-svg-rendering: vector | raster`. The property follows the
/// cascade and inherits like any custom property, so one declaration covers a
/// subtree or the whole page. The element that owns the picture decides: the
/// `<img>`, the element that has the `background-image`, the `<svg>` root, or
/// the `::before` or `::after` pseudo-element whose `content` is the picture.
/// Without a valid value there, the default of
/// [`crate::LayoutOptions::svg_rendering`] applies. A value that is neither
/// keyword is ignored, as if it were not declared on that element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
#[non_exhaustive]
pub enum SvgRendering {
    /// Draw the picture as vector paths, text and pictures. It stays sharp at
    /// any size, and a picture that needs more than the vector subset is
    /// drawn as pixels all the same.
    #[default]
    Vector,
    /// Draw the picture as an image at the resolution of
    /// [`crate::LayoutOptions::svg_raster_scale`].
    Raster,
}

impl SvgRendering {
    /// The rendering a value of `--raikiri-svg-rendering` names, `None` for
    /// anything but the two keywords.
    fn from_value(value: &str) -> Option<Self> {
        let value = value.trim();
        if value.eq_ignore_ascii_case("vector") {
            Some(Self::Vector)
        } else if value.eq_ignore_ascii_case("raster") {
            Some(Self::Raster)
        } else {
            None
        }
    }

    /// The rendering `style` declares or inherits, `None` when it names none.
    pub(crate) fn declared_by(style: &ComputedValues) -> Option<Self> {
        style
            .resolved_custom_property(PROPERTY)
            .and_then(|value| Self::from_value(&value))
    }
}

/// What a layout was asked for about SVG pictures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SvgSettings {
    /// The rendering an element that declares none gets.
    pub(crate) rendering: SvgRendering,
    /// Device pixels per CSS pixel of an SVG drawn as pixels.
    pub(crate) raster_scale: f32,
}

impl SvgSettings {
    /// These settings with `scale` as the raster scale, or unchanged when
    /// `scale` is not finite and greater than zero.
    pub(crate) fn with_raster_scale(mut self, scale: f32) -> Self {
        if scale.is_finite() && scale > 0.0 {
            self.raster_scale = scale;
        }
        self
    }
}

impl Default for SvgSettings {
    fn default() -> Self {
        Self {
            rendering: SvgRendering::default(),
            raster_scale: 1.0,
        }
    }
}
