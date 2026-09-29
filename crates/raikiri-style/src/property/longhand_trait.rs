//! Computed-value behavior for `longhands!` table entries declared with
//! `computed: via <Type>`.
//!
//! A table entry that needs more than "computed as specified" names a marker
//! type implementing [`Longhand`]. The marker's [`Longhand::compute`] usually
//! delegates to a plain function through [`run_hook`]: the function's first
//! parameter is the specified value, and any further parameters are
//! extracted from the [`AbsolutizeCx`] by their type (see [`FromCx`]), so a
//! hook states exactly which inputs it depends on:
//!
//! ```ignore
//! fn clamp_opacity(value: f32) -> f32 { value.clamp(0.0, 1.0) }
//! fn scale_by_font(value: f32, font: FontSize) -> f32 { value * font.0.px() }
//! ```

use crate::resolve::{ComputedLength, ResolveContext};

/// Inputs available to a property's specified-to-computed step: the
/// element's (or page context's) own computed `font-size`, its own used
/// line height for `lh` (`None` for `line-height: normal`), and the
/// tree-global [`ResolveContext`].
// The length inputs are read only through extractors; no table hook asks
// for them outside tests yet.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct AbsolutizeCx<'a> {
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

/// Computed-value behavior of one table property.
///
/// `Specified` is the table entry's `value:` type and `Computed` the type of
/// its `ComputedTable` field.
pub(crate) trait Longhand {
    /// The specified value type (the `PropertyValue` payload).
    type Specified;
    /// The computed value type.
    type Computed;

    /// Element cascade: specified to computed.
    fn compute(specified: Self::Specified, cx: &AbsolutizeCx<'_>) -> Self::Computed;

    /// Computed back to the specified form, for contexts that carry computed
    /// values in `PropertyValue` (page-context absolutization) or seed a
    /// child's specified state from its parent's computed value.
    fn lift(computed: Self::Computed) -> Self::Specified;
}

/// A value a hook can ask for by naming its type as a parameter.
pub(crate) trait FromCx<'a>: Sized {
    fn from_cx(cx: &AbsolutizeCx<'a>) -> Self;
}

/// The element's own computed `font-size`.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct FontSize(pub(crate) ComputedLength);

/// The element's own used line height (`None` for `line-height: normal`).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct OwnLineHeight(pub(crate) Option<ComputedLength>);

impl<'a> FromCx<'a> for FontSize {
    fn from_cx(cx: &AbsolutizeCx<'a>) -> Self {
        FontSize(cx.font_size)
    }
}

impl<'a> FromCx<'a> for OwnLineHeight {
    fn from_cx(cx: &AbsolutizeCx<'a>) -> Self {
        OwnLineHeight(cx.own_line_height)
    }
}

impl<'a> FromCx<'a> for &'a ResolveContext {
    fn from_cx(cx: &AbsolutizeCx<'a>) -> Self {
        cx.ctx
    }
}

/// A function usable as a computed-value hook: `Fn(S, P1, ..) -> C` where
/// every `Pn` implements [`FromCx`]. `Marker` only keeps the per-arity impls
/// from overlapping; callers never name it.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a computed-value hook",
    label = "not a hook for this specified value",
    note = "a hook is `fn(Specified, P1, ..) -> Computed` with at most three extra parameters",
    note = "each extra parameter must implement `FromCx`: `FontSize`, `OwnLineHeight` or `&ResolveContext`"
)]
pub(crate) trait HookFn<'a, S, Marker> {
    /// The computed value type the hook returns.
    type Out;
    fn run(&self, specified: S, cx: &AbsolutizeCx<'a>) -> Self::Out;
}

macro_rules! impl_hook_fn {
    ($($P:ident),*) => {
        impl<'a, S, C, F, $($P),*> HookFn<'a, S, fn(S, $($P),*) -> C> for F
        where
            F: Fn(S, $($P),*) -> C,
            $($P: FromCx<'a>,)*
        {
            type Out = C;
            fn run(&self, specified: S, #[allow(unused_variables)] cx: &AbsolutizeCx<'a>) -> C {
                self(specified, $($P::from_cx(cx)),*)
            }
        }
    };
}

impl_hook_fn!();
impl_hook_fn!(P1);
impl_hook_fn!(P1, P2);
impl_hook_fn!(P1, P2, P3);

/// Runs `hook` on `specified`, extracting its extra parameters from `cx`.
pub(crate) fn run_hook<'a, S, M, H: HookFn<'a, S, M>>(
    hook: H,
    specified: S,
    cx: &AbsolutizeCx<'a>,
) -> H::Out {
    hook.run(specified, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(v: f32) -> f32 {
        v
    }

    fn times_font_size(v: f32, font: FontSize) -> f32 {
        v * font.0.px()
    }

    fn lh_or_font(v: f32, font: FontSize, lh: OwnLineHeight) -> f32 {
        v * lh.0.unwrap_or(font.0).px()
    }

    fn with_root(v: f32, lh: OwnLineHeight, font: FontSize, ctx: &ResolveContext) -> f32 {
        v + lh.0.map_or(0.0, |l| l.px()) + font.0.px() + ctx.root_font_size.px()
    }

    #[test]
    fn hooks_extract_parameters_by_type() {
        let rc = ResolveContext::initial();
        let cx = AbsolutizeCx::new(ComputedLength(10.0), Some(ComputedLength(12.0)), &rc);
        assert_eq!(run_hook(identity, 2.0, &cx), 2.0);
        assert_eq!(run_hook(times_font_size, 2.0, &cx), 20.0);
        assert_eq!(run_hook(lh_or_font, 2.0, &cx), 24.0);
        assert_eq!(run_hook(with_root, 1.0, &cx), 1.0 + 12.0 + 10.0 + 16.0);
    }

    #[test]
    fn closures_are_hooks_too() {
        let rc = ResolveContext::initial();
        let cx = AbsolutizeCx::initial(&rc);
        assert_eq!(
            run_hook(|v: i32, f: FontSize| v + f.0.px() as i32, 1, &cx),
            17
        );
    }
}
