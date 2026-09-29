//! The [`Longhand`] trait that `#[derive(Longhand)]` implements, and the
//! [`AbsolutizeCx`] a computed-value hook receives.
//!
//! A hook is a plain function with a fixed signature, named by
//! `compute = ..` in the declaration; it reads whatever it depends on from
//! the context:
//!
//! ```ignore
//! fn clamp_opacity(value: f32, _cx: &AbsolutizeCx<'_>) -> f32 { value.clamp(0.0, 1.0) }
//! fn scale_by_font(value: f32, cx: &AbsolutizeCx<'_>) -> f32 { value * cx.font_size.px() }
//! ```

use cssparser::Parser;

use crate::resolve::{ComputedLength, ResolveContext};

/// Inputs available to a property's specified-to-computed step: the
/// element's (or page context's) own computed `font-size`, its own used
/// line height for `lh` (`None` for `line-height: normal`), and the
/// tree-global [`ResolveContext`].
///
/// Public only because [`Longhand::compute`] names it; it cannot be named
/// or built outside this crate.
// No declared hook reads the length inputs yet.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct AbsolutizeCx<'a> {
    pub(crate) font_size: ComputedLength,
    pub(crate) own_line_height: Option<ComputedLength>,
    pub(crate) ctx: &'a ResolveContext,
}

impl<'a> AbsolutizeCx<'a> {
    pub(crate) fn new(
        font_size: ComputedLength,
        own_line_height: Option<ComputedLength>,
        ctx: &'a ResolveContext,
    ) -> Self {
        Self {
            font_size,
            own_line_height,
            ctx,
        }
    }

    /// The context an initial value is computed in: the initial font size,
    /// no own line height, and `ctx` (normally [`ResolveContext::initial`]).
    pub(crate) fn initial(ctx: &'a ResolveContext) -> Self {
        Self::new(ctx.root_font_size, None, ctx)
    }
}

/// One declared CSS longhand: its name, value types, and how its value is
/// parsed, computed and inherited. Implemented by `#[derive(Longhand)]`.
///
/// The trait is public because generated `PropertyValue` variants and table
/// fields spell their payload types as `<P as Longhand>::Specified` and
/// `<P as Longhand>::Computed`, but its module is private: other crates see
/// the projected types and can neither name nor implement the trait.
pub trait Longhand {
    /// The property name, lowercase.
    const NAME: &'static str;
    /// Whether the property is inherited.
    const INHERITED: bool;
    /// The specified value type (the `PropertyValue` payload).
    type Specified: Clone + core::fmt::Debug + PartialEq;
    /// The computed value type.
    type Computed: Clone + core::fmt::Debug + PartialEq;

    /// The initial value, in specified form.
    fn initial() -> Self::Specified;

    /// Parses a value. CSS-wide keywords and `var()` are handled by the
    /// caller.
    fn parse(input: &mut Parser<'_, '_>) -> Option<Self::Specified>;

    /// Element cascade: specified to computed.
    fn compute(specified: Self::Specified, cx: &AbsolutizeCx<'_>) -> Self::Computed;

    /// Computed back to the specified form, for contexts that carry computed
    /// values in `PropertyValue` (page-context absolutization) or seed a
    /// child's specified state from its parent's computed value.
    fn lift(computed: Self::Computed) -> Self::Specified;

    /// A non-initial worst-case value for the page-cascade test corpus.
    #[cfg(test)]
    fn sample() -> Self::Specified;
}
