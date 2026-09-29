//! The [`Longhand`] trait that `#[longhands]` implements for each longhand
//! declared in a `properties!` table, and the [`AbsolutizeCx`] its
//! specified-to-computed step receives.
//!
//! A `compute:` hook named in the table is a plain function with a fixed
//! signature; it reads whatever it depends on from the context:
//!
//! ```ignore
//! fn clamp_opacity(value: f32, _cx: &AbsolutizeCx<'_>) -> f32 { value.clamp(0.0, 1.0) }
//! ```

use cssparser::Parser;

use crate::resolve::{ComputedLength, ResolveContext};

/// Inputs of a longhand's specified-to-computed step: the element's (or
/// page context's) own computed `font-size`, its own used line height for
/// `lh` (`None` for `line-height: normal`), and the tree-global
/// [`ResolveContext`]. These are the inputs
/// [`SpecifiedValues::finalize`](crate::specified::SpecifiedValues::finalize)
/// and the page-context absolutization already have for their own length
/// resolution.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AbsolutizeCx<'a> {
    #[expect(dead_code, reason = "no declared longhand's compute hook reads it yet")]
    pub(crate) font_size: ComputedLength,
    #[expect(dead_code, reason = "no declared longhand's compute hook reads it yet")]
    pub(crate) own_line_height: Option<ComputedLength>,
    #[expect(dead_code, reason = "no declared longhand's compute hook reads it yet")]
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

    /// The context an initial value is computed in: the root font size of
    /// `ctx` (normally [`ResolveContext::initial`]) and no own line height.
    pub(crate) fn initial(ctx: &'a ResolveContext) -> Self {
        Self::new(ctx.root_font_size, None, ctx)
    }
}

/// One CSS longhand declared in a `properties!` table: its name, value
/// types, and how its value is parsed, computed and inherited.
///
/// Implemented by `#[longhands]` on the uninhabited marker type
/// `<field>::Property` of each entry; nothing implements it by hand.
pub(crate) trait Longhand {
    /// The property name, lowercase.
    // The generated code spells names and inheritance as literals per entry.
    // `NAME` is read only by the generated test-only `longhand_samples()`,
    // which nothing calls yet, and `INHERITED` by nothing; both stay as part
    // of the declared longhand's description.
    #[expect(
        dead_code,
        reason = "only the uncalled test-only `longhand_samples()` reads it"
    )]
    const NAME: &'static str;
    /// Whether the property is inherited.
    #[expect(dead_code, reason = "the generated code writes inheritance per entry")]
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

    /// Specified to computed.
    fn compute(specified: Self::Specified, cx: &AbsolutizeCx<'_>) -> Self::Computed;

    /// Computed back to the specified form, for page-context absolutization
    /// (which carries computed values in `PropertyValue`) and for seeding a
    /// child's specified state from its parent's computed value.
    fn lift(computed: Self::Computed) -> Self::Specified;

    /// A non-initial worst-case value for the page-cascade test corpus.
    #[cfg(test)]
    fn sample() -> Self::Specified;
}
