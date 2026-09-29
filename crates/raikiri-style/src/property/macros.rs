//! Declarative macros shared by the property modules.

/// Implements keyword serialization, and optionally parsing, for a CSS
/// keyword enum from one variant-to-keyword table, so the two directions
/// cannot drift apart.
///
/// `css_keywords!(Type { Variant => "keyword", ... })` adds `as_css_str`
/// and an ASCII case-insensitive `from_css_ident`. The `@serialize` form adds
/// only `as_css_str`, for enums whose grammar spans several tokens.
macro_rules! css_keywords {
    (@serialize $ty:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        impl $ty {
            /// Every variant, in table order.
            #[cfg(test)]
            pub(crate) const ALL: &'static [Self] = &[$(Self::$variant),+];

            /// This variant's own keyword spelling. Computed-value remaps, such as
            /// a legacy keyword computing to another one, are the caller's job.
            pub const fn as_css_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $css,)+
                }
            }
        }
    };
    ($ty:ident { $($variant:ident => $css:literal),+ $(,)? }) => {
        css_keywords!(@serialize $ty { $($variant => $css),+ });

        impl $ty {
            /// Parses one keyword, ASCII case-insensitively.
            pub(crate) fn from_css_ident(ident: &str) -> Option<Self> {
                cssparser::match_ignore_ascii_case! { ident,
                    $($css => Some(Self::$variant),)+
                    _ => None,
                }
            }
        }
    };
}
