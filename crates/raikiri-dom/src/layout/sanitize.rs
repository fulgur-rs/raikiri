use super::*;

// ---------------------------------------------------------------------------
// Guard for non-finite f32.
// ---------------------------------------------------------------------------

/// Absolute upper limit for geometric values (px, and percentage fractions) passed to Taffy.
///
/// # Why clamping is necessary
///
/// Author CSS is untrusted input. `padding: 1e40px` becomes **+Inf** during cssparser's f64 -> f32
/// conversion, `padding: 1e40em` becomes **+Inf** due to multiplication during absolutization, and when
/// combined with `font-size: 0px`, `0.0 * inf` becomes **NaN**. Even extreme literals are not needed; just
/// nesting `font-size: 10em` 38 times will produce +Inf from `16 * 10^38 > f32::MAX`.
///
/// These were **accidentally** absorbed by the `Length::Em(_) | Length::Rem(_) => length(0.0)` arm of
/// `layout.rs` until absolutization was included in the cascade. While converting to exhaustive matching
/// is correct, that arm was also suppressing pathological numbers.
///
/// # Spec basis (§ title + anchor, `data-level` verified)
///
/// CSS Values 4 §5 "Numeric Data Types"
/// (<https://www.w3.org/TR/css-values-4/#numeric-types>) verbatim:
///
/// > The precision and supported range of numeric values in CSS is
/// > implementation-defined, and can vary based on the property or other
/// > context a value is used in. However, within the CSS specifications,
/// > infinite precision and range is assumed. When a value cannot be explicitly
/// > supported due to range/precision limitations, it must be converted to the
/// > closest value supported by the implementation, but how the implementation
/// > defines "closest" is implementation-defined as well.
///
/// That is, (a) having an upper limit itself complies with the spec, (b) **the upper limit may differ for
/// each property / context**, and (c) exceeding values are converted to "the closest value supported by
/// the implementation" = the upper limit. §5 is written in the **imperative** with `must be converted` and
/// does not say to discard values, so the declaration remains. This module having different upper limits
/// for each site is a direct application of (b).
///
/// (There is **no** explicit phrasing in §5 that says "do not invalidate declarations" — that phrasing is
/// from §3.1 / §10.12, so do not import it from there.)
///
/// # Separation of §5 and §5.1
///
/// §5.1 "Range Restrictions and Range Definition Notation"
/// Violations of the range notation (`<length-percentage [0,∞]>`, etc.) in
/// (<https://www.w3.org/TR/css-values-4/#numeric-ranges>) are about **dropping declarations at the parse
/// stage**, which raikiri handles with `parse_padding_side`, etc. This guard deals with values that are
/// **within the range of §5.1 but outside implementation capacity**, which are subject to §5. Do not
/// confuse the two, as they are different layers.
///
/// Section 10.12 "Range Checking" (<https://www.w3.org/TR/css-values-4/#calc-range>) of the same spec
/// stipulates that for the result of a math function, "the value resulting from a top-level calculation
/// must be clamped to the range allowed in the target context," and that clamping is performed on the
/// computed/used value—the same approach as this guard. However, **the input to this guard is not `calc()`
/// but a raw `em` multiplication, so it is not a direct basis**. The same wording in Section 3.1 "Range
/// Checking" (<https://www.w3.org/TR/css-values-4/#combining-range>) is a clause specifically for
/// interpolation and also does not apply to this case. The direct basis is the aforementioned Section 5.
///
/// # Value Determination (1e7 px)
///
/// The spec does not define an upper limit, so it is left to implementation discretion (see (b)(c) above).
/// The actual range held by implementations was taken from a primary source: Tab Atkins' post in CSSWG
/// issue #4552.
/// (<https://lists.w3.org/Archives/Public/public-css-archive/2019Dec/0015.html>,
/// 2019-12-02) verbatim:
///
/// > right now an s32 LayoutUnit's upper range is between 1e7px and 1e8px
/// > (exact value depends on the LU->px conversion in use)
///
/// The same post states units-per-px for **Firefox 60 / Chrome 64 / old-Edge 100**, so `2^31 / units` is
/// 3.58e7 / 3.36e7 / 2.15e7 px. This implementation adopts the **lower bound** `1e7` of the range (below
/// the upper limit of any of the three engines).
///
/// **This is not normative spec text**—it is a comment in a CSSWG issue, used as engineering evidence to
/// show "the digits that implementations actually have."
///
/// 1e7 px is approximately 2.6 km / 8900 A4 pages at 96dpi, so it does not become a practical constraint.
/// There is a 31-digit margin from the f32 upper limit (3.4e38), so the **sum** (width + padding + border
/// + margin) that taffy performs internally will not overflow and return to non-finite.
///
/// # Range of input-side bounds and resolution by output-side guards
///
/// This constant is applied via [`sanitize_taffy`] to **both px geometry and percentage fractions**. For
/// the px side, the derivation in #4552 above still holds, but **as a bound for the fraction side, no
/// matter how small the value is taken, it is insufficient.**
///
/// The resolution of percentages relative to their containing block occurs at the used value layer (taffy
/// side) and **re-applies with each nesting**, compounding exponentially with depth. Starting from A4
/// (793.7 px), the margin until f32 becomes non-finite is approximately 35.6 digits, so when the upper
/// limit of the fraction is `F` (> 1), the depth at which it first becomes non-finite is roughly
/// `35.6 / log10(F)` — **always finite**. The measured depth range before the fix matches this model:
///
/// | decl | fraction | `35.6 / log10(F)` | Measured first non-finite depth |
/// |---|---|---|---|
/// | `width: 1e9%` (This constant exactly) | 1e7 | 5.1 | 6 |
/// | `width: 100000%` | 1e3 | 11.9 | 12 |
/// | `width: 10000%` | 1e2 | 17.8 | 18 |
/// | `width: 1000%` | 1e1 | 35.6 | 36 |
///
/// (If `padding-left` is set to the same value, the test setup becomes shallower: 4 / 8 / — / 25. This is
/// because padding also contributes to the accumulation of `location` / `scrollable_overflow_rect`.)
///
/// Direct evidence of depth 1: `width: 1e9%` → `size.width = 7937008000.0` (= A4 793.7008px × fraction
/// 1e7) — already 3 orders of magnitude above "length 1e7 px". In contrast, the px path is sound, and even
/// if all properties are set to `1e7px`, it remains finite up to depth 45.
///
/// Setting it to `F <= 1.0` (= `100%`) would make it depth-independent, but this cannot be adopted as it
/// would kill spec-valid and common declarations like `width: 200%`. This means that **no matter how we
/// choose the input bound on the fraction side, this loophole cannot be closed**. CSSWG #4552 only
/// discusses px (it says nothing about multipliers for percentages), and there is no primary basis for
/// deriving a constant specifically for fractions.
///
/// **The resolution was placed on the output side** — [`sanitize_taffy_layout`] clamps taffy's
/// [`taffy::Layout`] **after resolution** with the same `[-MAX, MAX]`. This is depth-independent.
///
/// The cascade stage to which this clamp belongs is the **actual value**.
/// (CSS Cascade 5 §4.6 "Actual Values",
/// <https://www.w3.org/TR/css-cascade-5/#actual-value> verbatim:
/// "A used value is in principle ready to be used, but a user agent may not
/// be able to make use of the value in a given environment. For example, a
/// user agent may only be able to render borders with integer pixel widths
/// and may therefore have to approximate the used width."). In other words, the **used value (= the value
/// calculated by Taffy) has not been rewritten** — only the actual value, with environment-derived
/// approximations applied, has been placed in the arena. The approximation method
/// CSS Values 4 §Range Restrictions
/// of (<https://www.w3.org/TR/css-values-4/#numeric-ranges>) "must be"
/// converted to the closest value supported by the implementation, but how
/// the implementation defines "closest" is implementation-defined as well"
/// Note that the px-derived rationale in #4552 **originally applies to this output side** — because the
/// value approximated there is the used value in px, not a fraction.
///
/// The input-side guard ([`sanitize_taffy`]) **must not be removed** even after the output-side guard is
/// introduced: it still has the role of preventing ±Inf / NaN from entering Taffy's internal operations
/// (tests at sites 1-4 check this), and the output-side clamp only guarantees that "non-finite values do
/// not enter the arena".
///
/// # The range where the output-side clamp is actually effective (boundary with normal layout).
///
/// Since this constant is also the upper limit of the actual value, **values will change even if not
/// pathological for inputs where the used value exceeds 1e7 px**. Example: If `width: 200%` is nested 14
/// times, the used width = 793.7008 × 2^14 ≈ 1.30e7 px, and the actual value is approximated to 1e7
/// (before the fix, 1.30e7 was directly entered into the arena).
///
/// This is within the range allowed by CSS Values 4 §Range Restrictions, but it is an intentional choice
/// to record that this implementation is 2.2 to 3.6 times stricter than the upper limits of the three
/// engines (2.15e7 / 3.36e7 / 3.58e7 px) cited in #4552 — "should support reasonably useful ranges" in the
/// same § is a SHOULD, and 1e7 px ≈ 2.6 km / 8900 A4 pages is sufficient. "Zero impact" only holds for
/// layouts that fit within `[-1e7, 1e7]`.
///
/// Note that the previous flaw was not a regression of this guard — it was bit-identical to before the
/// guard was introduced (base), and for inputs exceeding the threshold, having the guard was a strict
/// improvement (`width: 1e40%` was depth 1 in base → depth 6 after guard). Availability impact was also
/// measured, and the complete render pipeline (`raikiri::html_to_png`) produced normal PNGs in ~200ms at
/// any depth of 1 / 3 / 4 / 6 (no hang / OOM / panic).
pub(crate) const MAX_TAFFY_MAGNITUDE: f32 = 1e7;

/// Upper limit (px) of `font-size` passed to parley.
///
/// This is separated from taffy geometry ([`MAX_TAFFY_MAGNITUDE`]) to comply with CSS Values 4 §5, which
/// states that "the supported range may differ per property / context" (the policy is not to make it
/// uniform because the target context differs per site). The target context for font-size is the parley →
/// skrifa glyph scaler, and its valid range differs from that of geometry.
///
/// # Value Determination (1e6 px)
///
/// Upper **measured** limit: The dependency chain's `skrifa` passes through
/// `Fixed::from_bits((ppem * 64.) as i32)` when converting font size to 16.16 fixed-point (for
/// compatibility with `skrifa-0.42.1/src/instance.rs`'s `Size::fixed_linear_scale` and FreeType's
/// `FT_Set_Pixel_Size`). Therefore, the conversion saturates at `i32::MAX / 64 ≈ 3.36e7` ppem where
/// `ppem * 64.0` no longer fits within `i32` (Rust's `f32 as i32` is a saturating cast, so it's not UB,
/// but the scale factor becomes a meaningless value).
///
/// This implementation adopts `1e6`, which is more than one order of magnitude lower. The difference is
/// reserved as a margin for the coefficient that parley multiplies by font-size (`line-height`'s unitless
/// multiplier, the `metric / units_per_em` ratio of ascent / descent) — `layout/data.rs` of
/// `parley-0.10.0` calculates `LineHeight::FontSizeRelative(value) * font_size` and
/// `font_size / units_per_em`.
///
/// A 1e6 px glyph is about 890 times the height of A4 and has no typographic meaning, so it does not
/// impose a practical constraint.
///
/// # The harm on this site is not "value corruption" but **hang** (measured).
///
/// The consequences for downstream sinks were initially uncharacterized as a plausible risk, but were
/// measured during the implementation of this guard: replacing `sanitize_finite` with an identity function
/// and running `nonfinite_font_size_is_clamped_before_parley` alone **does not terminate even after 25
/// seconds**. This means that non-finite font-sizes prevent parley's shaping from completing in bounded
/// time.
///
/// In contrast, the taffy-side tests for sites 1-4 **with this test input** immediately fail with an
/// assertion (only the value is corrupted). This is not a general proposition that "taffy does not hang
/// with non-finite values" — only 5 inputs were measured. The behavior of taffy's internal used values
/// remains a separate characterization task for the downstream sink.
///
/// Since it is reached by 1 element of untrusted author CSS (`<p style="font-size: 1e40px">`), **the guard
/// on this site is a requirement for availability, not correctness**. Do not delete or bypass it.
///
/// Regression detection is **bounded** by `nonfinite_font_size_is_clamped_before_parley` with worker
/// thread + `recv_timeout`. We do not rely on CI timeouts (only `timeout-minutes` per job unit of
/// `.github/workflows/ci.yml`, there is no nextest setting) — job kills cannot be distinguished from infra
/// flakes, and the results of subsequent tests in the same test binary are also lost.
pub(crate) const MAX_FONT_SIZE_PX: f32 = 1e6;

/// Unitless multiplier for `line-height` to pass to parley
/// (`ComputedLineHeight::Number` → `parley::LineHeight::FontSizeRelative`)
/// Upper limit.
///
/// # Determining the value (1e6, avoiding overflow based on actual measurements)
///
/// parley calculates `FontSizeRelative(value) * font_size` (line height calculation within `push_run` of
/// `parley-0.10.0/src/layout/data.rs`). Since `font_size` is already clamped to [`MAX_FONT_SIZE_PX`]
/// (`1e6`) or less when it is passed here, if the `value` side is also limited to the same `1e6`, the
/// product will be at most `1e12` — there is a margin of 26 digits or more from `f32::MAX` (`≈3.4e38`), so
/// multiplication of finite × finite will not overflow and become `+Inf`.
///
/// The reason this margin is necessary has been confirmed by actual measurements: if `value = f32::MAX` is
/// passed through as is, `f32::MAX * font_size` (as long as `font_size` exceeds `1.0`) will overflow and
/// become `+Inf`, and `+Inf` line height will enter the hang path mentioned in `sanitize_line_height`'s
/// doc. The specific numerical value `1e6` itself has no other basis; any "finite upper limit that is
/// confirmed not to overflow" would suffice — the reason for adopting the same value as
/// [`MAX_FONT_SIZE_PX`] is that both are similarly excessive safety margins when viewed from a
/// typographically meaningful line-height multiplier (which is at most single-digit in practice).
///
/// # Compile-time check of the combination
///
/// The above argument that overflow does not occur depends on the combination itself, namely that "both
/// constants are the same `1e6`"; if either one is rewritten, the argument breaks. The
/// `const _: () = assert!(...)` immediately below fixes the relational expression of the above paragraph
/// itself, "the product is at most `1e12`", at compile time. Since the current product itself is checked
/// as a band, rather than immediately below `f32::MAX`, if either constant is changed in a direction that
/// **increases** the product, the build will fail (changing it in a direction that decreases it only
/// widens the safe range, so it passes through). Do not simply loosen the value to pass; review both
/// constants and the overflow argument together. The valid range of [`MAX_FONT_SIZE_PX`] itself is
/// separately fixed by `clamp_limits_are_in_the_documented_range` (test) with a check of the same form.
pub(crate) const MAX_LINE_HEIGHT_NUMBER: f32 = 1e6;

// The product is taken with `f64` to prevent this check from being affected by a difference of less than
// 1 ULP, which determines which side of the `1e12` boundary the product of both constants rounded to
// `f32` rounds to (even if the product of `f32` overflows, it does not trap but merely saturates to
// `+Inf`, and `+Inf <= 1e12` is correctly evaluated as false. The concern here is not overflow but the
// precision of the rounding boundary).
const _: () = assert!((MAX_LINE_HEIGHT_NUMBER as f64) * (MAX_FONT_SIZE_PX as f64) <= 1e12);

/// Converts non-finite f32 to the finite value of `[min, max]`.
///
/// - **NaN → 0.0**. `f32::clamp` returns NaN **as NaN** (`NaN.clamp(a, b)` is NaN), so clamping alone
///   cannot eliminate it. Since NaN is not a point on the number line, §5's "closest value supported"
///   cannot be defined either.
///
///   The reason for choosing 0.0 is **not** "because it's spec initial" — initial values are geometric `0`
///   only for padding / margin (both CSS Box 3 `#propdef-padding-top` / `#propdef-margin-top` and
///   `Initial: 0`), while the initial values for `width` / `height` are
///   **`auto`** (CSS Sizing 3 §3.1.1 "Preferred Size Properties"
///   <https://www.w3.org/TR/css-sizing-3/#preferred-size-properties>,
///   `data-level="3.1.1"` (verified by actual testing), and `border-*-width` is **`medium`**.
///   (CSS Backgrounds 3 §3.3 "Line Thickness: the border-width properties"
///   <https://www.w3.org/TR/css-backgrounds-3/#border-width>,
///   `data-level="3.3"`, both TR / ED `Initial: medium`). Since neither `auto` nor `medium` are geometric
///   values, but rather **resolution rules / keywords**, they cannot be chosen as alternative values for
///   f32. Therefore, we default to **0.0 uniformly across all sites**. Since §5 states that the definition
///   of "closest" is left to implementation discretion, this choice itself is spec-compliant.
///
///   Furthermore, if we limit ourselves to the path where NaN occurs in `raikiri-style::resolve` (`0px` ×
///   `1e40em` = `0.0 * inf`), then **0.0 matches the correct answer according to the spec** — as §5 states
///   "within the CSS specifications, infinite precision and range is assumed", the computed value
///   evaluated with infinite precision is `0 × 10^40 = 0px`. NaN is merely an artifact produced by the
///   finite precision of f32.
///
///   Corroborating evidence (not direct justification): CSS Values 4 §10.9.1 "Infinities, NaN, and
///   Signed Zero" (<https://www.w3.org/TR/css-values-4/#calc-ieee>,
///   `data-level="10.9.1"` Verified. (Although it is numbered as §10.9.2 in ED, the anchor is the same) is
///   verbatim for math functions.
///   `NaN does not escape a top-level calculation; it's censored into a zero
///   value` / `Infinities do not escape a top-level calculation; they're clamped
///   to the minimum or maximum value allowed in the context …` (the latter continues with `, as defined
///   in § 10.12 Range Checking.` in the original text — omission indicated by `…`).
///   **The input to this guard is not `calc()`, so it is not a direct basis**
///   (treated as engineering evidence, similar to #4552), but the fact that CSS's censoring rule for
///   similar situations is a two-branch NaN→zero / Inf→clamp, which matches the choice here, reinforces
///   the validity of the choice.
/// - **±Inf and finite values out of range → `min` / `max`**. Application of "converted to the closest
///   value supported by the implementation" from CSS Values 4 §5.
///
/// **Clamp even huge finite values** (do not just make them finite) — `1e38%` is finite in the bridge, but
/// it becomes +Inf when multiplied by the containing block inside taffy, which negates the purpose of
/// placing a guard. Since §5 states that the implementation determines the "supported range," clamping
/// finite values outside the range to the upper limit is also an application of the same clause.
///
/// Does not panic (does not return `LayoutError` either) — the policy is to **clamp and continue**.
///
/// # Why not silent clamp?
///
/// `log` / `tracing` has no workspace dependencies (`grep` → 0 hit), but **that is not the reason** — the
/// same crate's `fonts.rs` already has a zero-dependency diagnostic mechanism (`FontWarn` enum +
/// `FontWarnObserver = Option<&mut dyn FnMut(&FontWarn)>` + `emit_warn`).
///
/// Initially, I judged that passing this observer all the way to the main site would affect the public
/// signature, so I kept it silent: `sanitize_*` is deep within `bridge_*` → `apply_computed_to_style` →
/// `layout_single_page`, and the output choke point (`sanitize_taffy_layout`) is called from
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout` — this is a fixed trait method signature
/// on the `taffy` crate side, so **an observation argument cannot be added**.
///
/// I introduced a common [`crate::diag::emit_warn_via`] mechanism and solved these two points as follows:
/// - The `bridge_*` → `apply_computed_to_style` chain consists only of private functions within the crate,
///   so passing `diag: &mut Vec<LayoutWarn>` is completed with a crate-internal signature change (the pub
///   signature remains intact).
/// - `set_unrounded_layout` can accept `self` (`&mut Document`), so by having the observer as an **owned
///   buffer** (`Document::layout_warnings`) in `self` instead of a closure, it can be pushed from the
///   choke point without changing the trait signature. `layout_single_page` drains this buffer at the end
///   of the path and flows it to observer-or-eprintln via `fonts.rs`, which is the same as
///   `emit_warn_via`.
///
/// I avoid "spreading raw `eprintln!` per-node and spamming" — [`push_layout_warn`] only accumulates an
/// event if clamping actually occurred (the value changed), so normal range layouts leave nothing in the
/// buffer. This is the same design principle as `FontWarn` passing only "warn+skip anomalies" to the
/// observer, rather than reporting every processed file each time.
///
/// **Residual risk (only partially reduced)**:
/// If a genuine arithmetic bug (e.g., `1e9px` due to a unit conversion error) were to be introduced on the
/// absolutize side in the future, there was always a risk that this guard would absorb it into 1e7 and
/// **render it as a "plausible layout"** — it would not be visualized as NaN or a breakdown. This is a
/// **residual risk of the same class** as the fail-quiet arm (`Em(_) => length(0.0)`) that was previously
/// removed.
///
/// Currently, every time a clamp occurs, [`LayoutWarn::NonFiniteClamped`] is output via
/// observer-or-eprintln, but it should be accurately described as **"demotion from silent to
/// stderr-visible" rather than "resolution"** — there is currently no way to insert an external observer
/// into `layout_single_page` (see `LayoutWarnObserver` scaffolding documentation), so this event will not
/// be read by anyone unless there is a consumer actively monitoring stderr within this crate. `fonts.rs`'s
/// `FontWarn` is in the same state (stderr only if no observer), so it has achieved the same level of
/// visibility, but it can only be said that "value pathologies are visualized" if there is a human
/// operator watching stderr.
pub(crate) fn sanitize_finite(
    v: f32,
    min: f32,
    max: f32,
    site: &'static str,
    diag: &mut Vec<LayoutWarn>,
) -> f32 {
    let clamped = if v.is_nan() { 0.0 } else { v.clamp(min, max) };
    // `v != clamped` correctly returns true even for NaN input (NaN comparison is always "other than" false
    // in IEEE 754 = `!=` is true), so both NaN → 0.0 assignment and out-of-range value clamping can be caught
    // under the same condition. Normal values within the range are `clamped == v`, so nothing is accumulated
    // (to avoid spam, see the doc above).
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site,
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// Symmetric clamp with [`MAX_TAFFY_MAGNITUDE`] as the upper limit (for taffy geometry).
///
/// It is symmetric (`[-MAX, MAX]`) because **negative values of `margin` are spec-valid**.
/// CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties"
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>, `data-level="3.1"`
/// verified in practice) is verbatim
///
/// > Negative values for margin properties are allowed,
/// > but there may be implementation-specific limits.
///
/// This is stipulated. This is the **only** place among the properties clamped by this delta where the
/// **spec explicitly acknowledges the existence of "implementation-specific limits"**, justifying both
/// symmetry and an upper bound simultaneously (stronger than the absent argument of "no non-negative
/// constraint").
///
/// Since `padding` / `width` / `height` / `border-width` are enforced to be non-negative at the parsing
/// stage, the value does not change even if they are symmetrical.
///
/// `site` is only used as a call-site label for [`LayoutWarn::NonFiniteClamped`] (it does not affect the
/// clamp arithmetic).
pub(crate) fn sanitize_taffy(v: f32, site: &'static str, diag: &mut Vec<LayoutWarn>) -> f32 {
    sanitize_finite(v, -MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE, site, diag)
}

/// Structured warn event for this module's non-finite-clamp diagnostic sites
/// (`sanitize_finite` / `sanitize_taffy` / `sanitize_taffy_layout` /
/// `sanitize_font_weight`). Sibling
/// of [`crate::fonts::FontWarn`], generalized via the
/// shared [`crate::diag::emit_warn_via`] mechanism so
/// the "silent clamp" residual risk documented on [`sanitize_finite`] gets
/// the same observability `fonts.rs` already has.
///
/// Every variant is fully owned (no borrowed `Path`, unlike `FontWarn`)
/// because these clamp sites only ever see primitive `f32` values. See
/// [`crate::diag`]'s module doc for why this owned shape — not `FontWarn`'s
/// borrowed one — is what a shared generic `Observer<W>` type could actually
/// have supported, and why a macro was used instead so both shapes share one
/// mechanism anyway.
///
/// No `#[non_exhaustive]` (unlike `FontWarn`, which is `pub`): that attribute
/// only constrains *downstream crates*, and this enum is `pub(crate)` with no
/// external consumer to protect. Add it back if this type is ever promoted
/// to a public export.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LayoutWarn {
    /// A non-finite (NaN / +-Inf) or out-of-range `f32` was clamped to a
    /// finite in-range value before being handed to one of this module's
    /// sink boundaries: `taffy::Style` or the `Node.unrounded_layout` arena
    /// field (sites 1-5, `sanitize_finite` / `sanitize_taffy` /
    /// `sanitize_taffy_layout`),
    /// `parley::FontWeight::new` (site 6, `sanitize_font_weight`), or
    /// `StyleProperty::LineHeight`'s two numeric sub-values (sites 7-8,
    /// `sanitize_line_height`). Only
    /// emitted when clamping actually changed the
    /// value (not on every call) so ordinary in-range layouts stay silent —
    /// the "warn+skip" shape `FontWarn` uses, not a per-node trace.
    NonFiniteClamped {
        /// Call-site label (e.g. `"font-size"`, `"margin"`,
        /// `"layout.size"`) — a human-readable category, not a stable
        /// machine-parseable identifier.
        site: &'static str,
        /// Pre-clamp value (may be NaN or +-Inf).
        raw: f32,
        /// Post-clamp value actually used.
        clamped: f32,
    },
    /// `suppressed` additional [`LayoutWarn::NonFiniteClamped`] events were
    /// dropped once [`LAYOUT_WARN_CAP`] was reached during a single
    /// `layout_single_page` pass, bounding memory / `eprintln!` spam under a
    /// pathological input that clamps every field of every node (e.g. deep
    /// `width: 200%` nesting — see [`MAX_TAFFY_MAGNITUDE`]'s doc). Emitted at
    /// most once per pass, after all the real events it summarizes.
    Truncated {
        /// Count of additional `NonFiniteClamped` events dropped after the
        /// cap was reached.
        suppressed: usize,
    },
    /// One or more subtrees had a broken **parent/child geometry invariant**
    /// and were reset to a deterministic zero
    /// [`taffy::Layout`] by [`enforce_layout_invariants`]. This is a
    /// different failure class than [`LayoutWarn::NonFiniteClamped`]: that
    /// variant fires when a single `f32` field was out of range, this one
    /// fires when every individual field of the (already per-field-clamped)
    /// `Layout`s involved was in range, but the *relationship* between two
    /// or more fields — possibly on different nodes — was not (e.g. a
    /// node's own content box went negative, or a child's border box did
    /// not fit inside its parent's once one of the two had its actual value
    /// approximated by [`sanitize_taffy_layout`]). See
    /// [`enforce_layout_invariants`]'s doc for exactly which two invariants
    /// are checked and why unconditionally checking cross-node containment
    /// would be spec-incorrect.
    ///
    /// Aggregated per invariant (at most one event per invariant kind per
    /// `layout_single_page` pass, each counting every subtree it reset)
    /// rather than one event per reset subtree, so a pathological input
    /// that trips the same invariant on many nodes cannot reintroduce the
    /// per-node spam [`LAYOUT_WARN_CAP`] exists to bound.
    GeometryInvariantViolated {
        /// Which invariant was violated — `"content_box_non_negative"` or
        /// `"child_within_parent_border_box"` (see
        /// [`enforce_layout_invariants`]'s doc). Like `NonFiniteClamped`'s
        /// `site`, a human-readable category, not a stable
        /// machine-parseable identifier.
        ///
        /// The `"child_within_parent_border_box"`
        /// value can no longer actually occur — [`child_within_parent_border_box`]
        /// (the predicate) now always returns `true`, so
        /// [`enforce_layout_invariants`]'s containment branch that would
        /// produce this event is unreachable for any input. It remains
        /// listed here (and the branch remains in the code) because whether
        /// to remove the dead invariant check entirely is a separate,
        /// explicitly deferred decision. Do not treat this
        /// value's continued presence in this doc as evidence the check is
        /// still live.
        invariant: &'static str,
        /// Number of distinct subtree roots this invariant caused
        /// [`enforce_layout_invariants`] to reset in this pass (not a count
        /// of individual arena nodes touched — a reset subtree may contain
        /// further descendants that also got zeroed as part of the same
        /// reset).
        subtree_count: usize,
    },
}

impl std::fmt::Display for LayoutWarn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutWarn::NonFiniteClamped { site, raw, clamped } => write!(
                f,
                "{site}: clamped non-finite/out-of-range value {raw} to {clamped}"
            ),
            LayoutWarn::Truncated { suppressed } => write!(
                f,
                "{suppressed} additional layout clamp warning(s) suppressed (buffer cap reached)"
            ),
            LayoutWarn::GeometryInvariantViolated {
                invariant,
                subtree_count,
            } => write!(
                f,
                "{invariant}: {subtree_count} subtree(s) had a broken parent/child geometry invariant and were reset to a zero layout"
            ),
        }
    }
}

/// Observer alias for [`LayoutWarn`] — sibling of `fonts.rs`'s
/// `FontWarnObserver`. Unlike that alias, this one carries no inner lifetime
/// (`LayoutWarn` is fully owned), so it is a plain, non-higher-ranked
/// `Option<&mut dyn FnMut(&LayoutWarn)>`. Not yet reachable from any public
/// entry point — see [`Document::layout_warnings`](crate::document::Document)
/// for why (dom→paint wall: `layout_single_page`'s signature is consumed by
/// `raikiri-paint` and the `raikiri` crate, so adding a parameter — or a new
/// `_with_observer` sibling — to it is a decision for that wall, not this
/// task). This scaffolding exists so a future `_with_observer` addition only
/// has to plumb one new parameter through, rather than re-deriving the whole
/// mechanism.
pub(crate) type LayoutWarnObserver<'o> = Option<&'o mut dyn FnMut(&LayoutWarn)>;

/// Emit a [`LayoutWarn`] event: call the observer if `Some`, otherwise
/// `eprintln!` (matches [`crate::fonts`] of `emit_warn`'s shape exactly, via the
/// shared [`crate::diag::emit_warn_via`] macro).
pub(crate) fn emit_layout_warn(observer: &mut LayoutWarnObserver<'_>, event: LayoutWarn) {
    crate::diag::emit_warn_via!(observer, "[raikiri-dom::layout]", event);
}

/// Cap on buffered [`LayoutWarn::NonFiniteClamped`] events per
/// `layout_single_page` pass.
///
/// A single pathological input (e.g. deep `width: 200%` nesting hitting
/// every node, [`MAX_TAFFY_MAGNITUDE`]'s doc) can clamp every `f32` field of
/// every node in the arena, which would otherwise make both the buffer and
/// the eventual `eprintln!` replay unbounded — precisely the "per-node spam"
/// concern [`sanitize_finite`]'s doc raised about threading an observer down
/// this chain in the first place. [`push_layout_warn`] collapses anything
/// past this cap into a single running [`LayoutWarn::Truncated`] counter
/// instead of dropping it silently.
pub(crate) const LAYOUT_WARN_CAP: usize = 63;

/// Push a [`LayoutWarn`] onto `diag`, respecting [`LAYOUT_WARN_CAP`].
///
/// Once the cap is reached, further events collapse into (rather than grow)
/// a single trailing [`LayoutWarn::Truncated`] counter, so the buffer size is
/// bounded (`LAYOUT_WARN_CAP + 1`) regardless of how many clamp sites fire in
/// one pass.
pub(crate) fn push_layout_warn(diag: &mut Vec<LayoutWarn>, event: LayoutWarn) {
    if diag.len() < LAYOUT_WARN_CAP {
        diag.push(event);
        return;
    }
    match diag.last_mut() {
        Some(LayoutWarn::Truncated { suppressed }) => *suppressed += 1,
        _ => diag.push(LayoutWarn::Truncated { suppressed: 1 }),
    }
}

/// An **output-side** guard that passes all f32 fields of [`taffy::Layout`] resolved by taffy to
/// [`sanitize_taffy`].
///
/// # Why on the output side (cannot be closed by an input-side bound)
///
/// As described in the "Scope of input-side bounds" section of [`MAX_TAFFY_MAGNITUDE`], percentages are
/// resolved against the containing block at the used value layer, compounding with each nest. Therefore,
/// **any fraction upper bound greater than 1 will overflow f32 at a finite depth**. Since the depth is
/// determined by untrusted input (DOM nesting), there is no way to close this except by placing a guard in
/// a depth-independent location — **after** resolution.
///
/// # Single call site (choke point)
///
/// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout`
/// (`taffy_impl.rs`) — the **only** path through which Taffy writes layout back to the arena (all block /
/// flexbox / grid / leaf algorithms in `taffy-0.12.1` pass through this single method). Therefore,
/// "`Node.unrounded_layout` never contains non-finite values" is a structural invariant and cannot be
/// broken by a forgotten call, unlike an afterthought, batch range check.
///
/// The other half of the invariant is the **initial value**: `Node::new*` places `Layout::with_order(0)`
/// (`node.rs`), which has all fields as 0 and finite. Subsequent writes pass through this guard as
/// described above, so there is no moment when the arena holds a non-finite layout.
///
/// # Do not change taffy's internal calculations (used values are immutable, only actual values are
/// approximated).
///
/// `RoundTree::get_unrounded_layout` is the only one that **reads back** `taffy::Layout` from the arena,
/// and this is exclusively for `taffy::round_layout`. Raikiri does not call `round_layout` and does not
/// implement `RoundTree` (a `grep -rn 'round_layout\|RoundTree' crates/` shows 0 hits other than this doc
/// comment, as measured). Therefore, this clamp **only constrains the observation surface** and does not
/// affect taffy's internal percentage resolution chain (`LayoutInput::parent_size`). This means that "a
/// result that internally became inf at a deep level is seen as a clamped value," and the layout
/// calculation itself is not rewritten.
///
/// # Exhaustive struct literal (without using `..`)
///
/// Explicitly list all f32 fields. If `..*layout` is used, when taffy adds f32 fields in the future, they
/// will **silently leak outside the guard**, but an exhaustive literal will cause a compile error, forcing
/// a review. `order` is `u32`, so it is not subject to the guard.
///
/// The same reason applies to not limiting to only the 4 fields that paint actually reads — "the arena
/// does not have non-finite geometry" is an invariant that can be stated and tested, but "the fields that
/// paint happens to read" is not.
///
/// # Only finiteness is guaranteed (box model containment is not preserved)
///
/// Note that this guard **only guarantees finiteness**, and does not preserve box model containment (CSS
/// Box 3's content ⊆ padding ⊆ border) — because it clamps each field independently, `size.width` and
/// `padding.{left,right}` can saturate simultaneously, causing `size.width - padding.left - padding.right`
/// to become negative. Currently, there are no consumers reading `padding` / `border` /
/// `scrollable_overflow_rect` / `scrollbar_size` (measured by grep), but if paint uses these in the
/// future, do not assume non-negativity.
///
/// `diag` collects [`LayoutWarn::NonFiniteClamped`] events for whichever
/// fields actually get clamped (site labels: `"layout.location"`,
/// `"layout.size"`, `"layout.scrollable_overflow_rect"`, `"layout.scrollbar_size"`,
/// `"layout.border"`, `"layout.padding"`, `"layout.margin"`). The sole caller
/// (`<Document as taffy::LayoutPartialTree>::set_unrounded_layout` in
/// `taffy_impl.rs`) passes `&mut self.layout_warnings` — an owned buffer on
/// `Document`, not a live observer — because that trait method's signature
/// is fixed by `taffy` and cannot receive one (see
/// `Document::layout_warnings`'s doc for why).
pub(crate) fn sanitize_taffy_layout(
    layout: &TaffyLayout,
    diag: &mut Vec<LayoutWarn>,
) -> TaffyLayout {
    fn size(s: Size<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Size<f32> {
        Size {
            width: sanitize_taffy(s.width, site, diag),
            height: sanitize_taffy(s.height, site, diag),
        }
    }
    fn rect(r: Rect<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Rect<f32> {
        Rect {
            left: sanitize_taffy(r.left, site, diag),
            right: sanitize_taffy(r.right, site, diag),
            top: sanitize_taffy(r.top, site, diag),
            bottom: sanitize_taffy(r.bottom, site, diag),
        }
    }
    TaffyLayout {
        order: layout.order,
        location: Point {
            x: sanitize_taffy(layout.location.x, "layout.location", diag),
            y: sanitize_taffy(layout.location.y, "layout.location", diag),
        },
        size: size(layout.size, "layout.size", diag),
        scrollable_overflow_rect: rect(
            layout.scrollable_overflow_rect,
            "layout.scrollable_overflow_rect",
            diag,
        ),
        scrollbar_size: size(layout.scrollbar_size, "layout.scrollbar_size", diag),
        border: rect(layout.border, "layout.border", diag),
        padding: rect(layout.padding, "layout.padding", diag),
        margin: rect(layout.margin, "layout.margin", diag),
    }
}

/// A layer above the **finiteness** guaranteed by [`sanitize_taffy_layout`] — it inspects the **semantic**
/// invariants of parent-child geometry and replaces broken subtrees with deterministic default geometry
/// (zero).
///
/// # Why `sanitize_taffy_layout` alone is not sufficient
///
/// As the `sanitize_taffy_layout` documentation explicitly states, its guard clamps **each field
/// independently**, so it does not preserve box model containment (CSS Box 3's content ⊆ padding ⊆ border)
/// — if `size.width` and `padding.{left,right}` saturate simultaneously, `content_box_width()` can become
/// negative. Also, the internal taffy operation chain returns **raw values before clamping** from child to
/// parent via `LayoutOutput` (`taffy-0.12.1`'s `compute/block.rs:947,973,981,1072` and
/// `set_unrounded_layout` are called **after** that, and the child's `Layout` is written not by the child
/// itself but by the **parent's algorithm**). Therefore, even if a node's position is calculated based on
/// "another (clamped) node," each node is only guaranteed that "its own fields are finite." This function
/// fills these two gaps — box model containment within a single node, and the positional relationship
/// between parent and child.
///
/// # Two invariants to check
///
/// 1. **Content box non-negative** — Both `Layout::content_box_width()` and `content_box_height()` are
///    `>= 0.0`. This applies **unconditionally to all nodes**.
///    During implementation, a probe with
///    `<div style="width: 10px; padding: 50px; box-sizing: border-box;">` through the normal layout path
///    (`layout_single_page`) showed that Taffy automatically stretches the border box until
///    `size.width == padding_left + padding_right` (= `100.0`) and floors the content box to exactly `0.0`
///    (internal logic similar to `maybe_max(padding_border_size)` in `taffy-0.12.1/src/compute/mod.rs`).
///    This means that **in normal CSS not involving non-finite clamps, the content box will never be
///    negative** — checking this invariant unconditionally will not falsely detect legitimate layouts.
/// 2. **The origin of the child's border box (`location`) fits within the parent's border box** — This was
///    originally designed as an invariant to check containment, but **a change was introduced to
///    unconditionally accept saturated axes regardless of sign** (details in the sign-specific section
///    below) — meaning this invariant no longer actually checks anything (taffy's coordinate system is
///    "position relative to parent border box origin", see `Layout::content_box_x/y`'s doc in
///    `taffy-0.12.1/src/tree/layout.rs`). **Do not look at `child.size`** — this invariant originally only
///    stipulated that "the child's **location** is within the parent's border box" and did not cover the
///    child's own size (see [`child_within_parent_border_box`]'s doc, "Reason not to look at `child.size`"
///    section, which records the history of how it was found during implementation that the extent
///    (`location + size`) version would falsely detect legitimate nests of `width: 200%`).
///
///    **Record of the design and its rationale before this change** — As mentioned above, since this
///    change, this invariant has actually become an unconditional accept that checks nothing. Why was this
///    not originally an unconditional check? CSS commonly allows children to overflow their parent's
///    border box (`overflow: visible` is the initial value, negative margins, fixed small container +
///    large content, declarations like `width: 200%` where "child is intentionally larger than parent") —
///    raikiri currently has `position: absolute` unimplemented, but that alone is enough to reproduce it.
///    When implementing, it was measured (probe: child of parent
///    `<div style="width: 50px; height: 50px;">` with
///    `<div style="width: 200px; height: 200px; margin-left: -30px;">`) showed parent `size=(50,50)` and
///    child `location=(-30,0)` — the origin itself is outside the parent's left edge (`x=0`), but this is
///    **correct layout and not a bug**. If checked unconditionally, legitimate layouts would be
///    incorrectly fallen back, so **only** when **that axis itself** of `child.location.x` /
///    `child.location.y` exactly reaches the saturation boundary of [`MAX_TAFFY_MAGNITUDE`], that axis is
///    checked (see implementation of [`child_within_parent_border_box`] — whether `parent.size` or
///    `child.size` is saturated is not involved in this gate). The occurrence of saturation means that the
///    "actual value" (approximated value) of that field deviates from the "used value" (actual calculation
///    result) within taffy — in the previous version, based on this, the design was "that's why we
///    explicitly check consistency again, and if it's broken, we fall back to a deterministic value."
///    **This design was later overturned**: the conclusion was uniformly reached, regardless of the sign,
///    that "the fact that the actual value is approximated" is an operation within the range allowed by
///    the specification, and whether it fits or not, this function will not be a reason for reset (details
///    in the sign-specific sections below). `saturated_but_contained_layout_is_not_reset` was originally a
///    test intended to check the conjunction of "gate AND containment violation," but after this change,
///    the assert itself still passes (because this test's fixture happens to be a "fitting" case), but the
///    claim of the conjunction no longer holds — see that test's doc and the paired regression check
///    (`saturated_child_outside_parent_is_not_reset`), which was added/renamed to show directly that even a
///    fixture clearly outside the parent is not reset.
///
///    **Actual measurements during implementation (`width: 200%` nested 45 levels, single chain)**:
///    Initially, we were checking the extent (`location + size <= parent.size`), but it was found that
///    this legitimate declaration (representing a consistent relationship where "child is twice the
///    parent" at any depth) was incorrectly reset at deep levels — the child's origin always remains
///    `(0, 0)`, so it is not reset by the current definition that only looks at the origin
///    (`nested_percentage_wide_child_chain_is_not_reset` is pinned).
///
///    **Previously discovered gate flaw (fixed)**: The version immediately after changing to only look at
///    the origin still had the gate condition as "if any one of `parent.size` / `child.size` /
///    `child.location` is saturated, inspect both axes regardless of axis distinction." In this form, even
///    if **the child's own location is not saturated** — for example, if `parent.size` of some other node
///    in the same subtree was saturated (due to an unrelated cause) — a child with a legitimate negative
///    margin (`location.x` is a negative value in the normal range, e.g., `-30`) could fall to `>= 0.0`
///    and be incorrectly reset. This was pointed out as a non-trivial finding. The current axis-unit gate
///    (inspecting only that axis if `child.location.x` / `.y` itself is saturated) structurally closes
///    this — the field to be inspected is always limited to the field "that was approximated itself," so
///    legitimate small negative values will not pass through this gate.
///    (`saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    directly checks — The history of replacing the fixture with one where the `y` axis is saturated and
///    contained, after receiving a comment that the old and new implementations could not be distinguished
///    in a fixture where "the reset can be explained by the `y` axis alone," is documented in the test's
///    doc.
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    before this change, was a check that the detection capability of the `y` axis was maintained, but
///    since that detection capability itself was lost with this change, it is now a check for a different
///    assertion (neither axis is a reason for reset), as recorded in the test's doc.
///
///    **False positive in the negative direction (initially recognized as a residual risk, but later
///    resolved)**: Even after closing the axis-unit gate, a false positive specific to the case where the
///    saturated axis itself was **negative** remained. The re-inspection formula
///    `child.location >= 0.0 && child.location <= parent.size`, introduced at the time, was **identically
///    false** while `child.location` was negative — since no negative number satisfies `>= 0.0`, for the
///    negative direction, this formula was mathematically equivalent to "unconditionally reset if
///    saturated and negative" rather than "re-validate containment." Under CSS Box 3 §3.1 (cited above,
///    negative values of `margin` are allowed without limit within "implementation-specific limits"),
///    [`MAX_TAFFY_MAGNITUDE`] is precisely that "implementation-specific limit" itself, and reaching it is
///    not evidence of failure — it merely indicates that a legitimate negative relationship has begun to
///    be approximated beyond the upper limit of this implementation. This is symmetrical to "unsaturated
///    negative values" (`-30` of `legitimate_negative_margin_overflow_is_not_reset`, or intermediate
///    stages that monotonically increase like `-30`,`-60`,... in deep nests) which the same function
///    already unconditionally trusts, and there is no reason for the handling to discontinuously reverse
///    only at the boundary (the moment `±MAX_TAFFY_MAGNITUDE` is exactly reached).
///
///    At the time, we concluded from this that "therefore, the current definition branches by sign: if the
///    saturated axis is negative, it's unconditionally ok; if positive, `<= parent.size` is re-inspected
///    as before." There were two reasons — (a) `padding` / `border` / `width` are enforced to be
///    non-negative at parse time, so there is no symmetrical spec-based justification (unlike margin) to
///    justify huge positive `location` as "CSS allows without limit," and (b) positive re-inspection
///    actually has both branches (`<=` is true in `saturated_but_contained_layout_is_not_reset`, and false
///    in the old `saturated_child_outside_parent_resets_subtree_to_zero_layout`), so relaxing it would
///    actually lose existing detection capability.
///
///    **This rationale (a) was later overturned**: margin is symmetric — CSS Box 3 §3.1
///    (<https://www.w3.org/TR/css-box-3/#margin-physical>, "Negative values
///    for margin properties are allowed, but there may be
///    implementation-specific limits") only explicitly allows **negative values**, and does not provide a
///    basis for more strictly constraining positive margins. The grammar of `margin-left`
///    (`<length-percentage> | auto`) makes equally large positive and negative values spec-legal, and
///    reaching an "implementation-specific limit" like [`MAX_TAFFY_MAGNITUDE`] is not evidence of failure
///    in the positive direction for the same reasons it was established in the negative direction. In
///    fact, `margin-left: 1e9%` (within a 100px container) saturates `location.x` in the positive
///    direction, and the old implementation incorrectly reset this. For actual measurements, refer to the
///    regression test via the CSS pipeline, which is the positive counterpart of
///    `saturated_negative_margin_percentage_child_is_not_reset`.
///
///    Rationale (b) ("positive re-inspection currently has detection capability") has **not been
///    disproven** by this change — only (a) was disproven, and the point that (b) "loses detection
///    capability" was correct. That loss was **knowingly accepted** — existing tests only construct
///    TaffyLayout directly and have never demonstrated detection capability via the actual CSS pipeline,
///    while this data loss (complete disappearance of legitimate content) had been proven via actual CSS.
///    **As a result, `axis_ok` was unified to "unconditionally accept if saturated" regardless of sign,
///    and [`child_within_parent_border_box`] always returns `true`** — there is no longer a path to
///    re-inspect containment (see the implementation of `axis_ok` and the doc section "Reason for
///    unconditional acceptance regardless of sign"). Whether "this invariant itself should be maintained"
///    is a separate decision explicitly deferred and unresolved at this point in this document.
///
///    Regression check whose behavior was inverted by this change: The old
///    `saturated_child_outside_parent_resets_subtree_to_zero_layout` was renamed to
///    `saturated_child_outside_parent_is_not_reset` and inverted, and the old
///    `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    is
///    `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
///    renamed/inverted to — see their respective docs for details.
///
///    **Alternative considered and rejected**: The parent saturation gate proposal, "skip re-validation
///    (without looking at the sign) if `parent.size` itself is saturated on the same axis," was rejected
///    because
///    `saturated_negative_margin_percentage_child_is_not_reset`
///    (a single `margin-left: -1e9%` declaration, no nesting, parent not saturated by `width: 100px`)
///    cannot be explained if it is mistakenly left reset. This test is kept separate from the nested chain
///    test above as evidence that "the child's own sign" is the correct discriminant, not "whether the
///    parent is saturated."
///
/// NaN → `0.0` (see `sanitize_finite`'s doc) bypasses invariant 2's gate because it does not match the
/// saturation boundary (`±MAX_TAFFY_MAGNITUDE`), but there is no harm — `location=(0,0)` `size=(0,0)` is
/// always "contained" for any parent of non-negative size, so even if invariant 2 is missed by the check,
/// it would not be detected as a violation in the first place. Cases where padding/border are involved and
/// NaN → 0, causing the content box to become negative, are unconditionally caught by invariant 1 (without
/// a gate).
///
/// # Deterministic fallback: zero out subtree (`zero_layout_subtree`)
///
/// The design decision to adopt "deterministic fallback" instead of "input restriction," "intermediate
/// saturation," or "layout abort" has been finalized. Among the two fallback value proposals considered
/// ("0 size" and "nearest finite parent size"), **0 size** is chosen. [`taffy::Layout::with_order`]
/// (keeping only `order`, all others zero) was selected for two reasons: (a) it trivially and recursively
/// satisfies both invariant 1 (`0 - 0 - 0 = 0 >= 0`) and invariant 2 (`(0,0)` fits into any parent of
/// non-negative size), so filling the entire subtree with this value does not create new invariant
/// violations (whereas "fallback to the nearest finite parent size" would require additional guarantees
/// that the fallback value does not further violate invariant 2, leading to a design with repeated
/// re-checks), and (b) "0 size" was listed first in the original description during consideration.
///
/// # Traversal is not recursive
///
/// The input this function targets (deep percentage nest — see the doc table for [`MAX_TAFFY_MAGNITUDE`])
/// is precisely a **deep DOM tree**. For the same reasons as the deep-nesting countermeasures in
/// [`find_body`] and cascade, writing it with naive recursion would create a new stack overflow path for
/// the same input. Both this function and [`zero_layout_subtree`] are iterative implementations that use
/// `Vec` as an explicit stack.
///
/// # Only look at nodes actually visited by Taffy
///
/// Instead of traversing `document.nodes[idx].children` directly, apply the same `is_in_document()` filter
/// as Taffy's `TraversePartialTree` implementation (`taffy_impl.rs`) to the child traversal. While there
/// is no practical harm in including nodes that Taffy did not lay out at all, such as `<template>`
/// descendants, since `unrounded_layout` remains at its construction-time default
/// (`Layout::with_order(0)`) (meaning old values from other subtrees are not mixed in), aligning with
/// Taffy's traversal contract will be less surprising to the reader.
///
/// # Caller
///
/// Runs exactly once, immediately after Step 5 (`compute_root_layout`) of [`layout_single_page`] and
/// before Step 6 (warning replay) — right after all `unrounded_layout` that taffy actually wrote under the
/// root (`<body>`) are available. Paths that directly call `compute_root_layout` (each unit test of
/// `lib.rs`) do not go through this pass — those tests are for the characterization of
/// `sanitize_taffy_layout` (finiteness only), and semantic invariants are not covered (see
/// `taffy_block_layout_does_not_hang_on_raw_nonfinite_style_geometry`'s doc).
pub(crate) fn enforce_layout_invariants(document: &mut Document, root_idx: usize) {
    let mut content_box_violations = 0usize;
    // `child_within_parent_border_box` now always
    // returns `true` (see its doc), so the `if` below that increments this
    // is unreachable for any input — `containment_violations` can never
    // exceed 0, and the `LayoutWarn::GeometryInvariantViolated { invariant:
    // "child_within_parent_border_box", .. }` warning below can never be
    // emitted. Kept (not deleted) because removing the dead branch is part
    // of the deferred "should this invariant check exist at all" follow-up.
    let mut containment_violations = 0usize;
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        // An element laid out by the inline engine carries the bounding box
        // of its line pieces; a piece that does not end the element leaves
        // out that side's padding and border, so the box can be narrower
        // than the element's paddings without being broken. The flag is set
        // only when the inline engine is switched on.
        if document.nodes[idx]
            .flags
            .contains(NodeFlags::IN_IFC_SUBTREE)
        {
            continue;
        }
        let layout = document.nodes[idx].unrounded_layout;
        if layout.content_box_width() < 0.0 || layout.content_box_height() < 0.0 {
            zero_layout_subtree(document, idx);
            content_box_violations += 1;
            continue;
        }
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if !document.nodes[child_idx].is_in_document() {
                continue;
            }
            let child_layout = document.nodes[child_idx].unrounded_layout;
            if !child_within_parent_border_box(&layout, &child_layout) {
                zero_layout_subtree(document, child_idx);
                containment_violations += 1;
            } else {
                stack.push(child_idx);
            }
        }
    }
    if content_box_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "content_box_non_negative",
                subtree_count: content_box_violations,
            },
        );
    }
    if containment_violations > 0 {
        push_layout_warn(
            &mut document.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "child_within_parent_border_box",
                subtree_count: containment_violations,
            },
        );
    }
}

/// The fallback body of [`enforce_layout_invariants`] — overwrites the `unrounded_layout` of the subtree
/// rooted at `root_idx` (only nodes actually visited by taffy, `is_in_document()` filter) with
/// [`taffy::Layout::with_order`] (which only keeps `order` and sets all others to zero). Iterative (`Vec`
/// stack) — recursion is not used, precisely because the target is a deep DOM (see the "traversal is not
/// recursive" section of [`enforce_layout_invariants`]).
fn zero_layout_subtree(document: &mut Document, root_idx: usize) {
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let order = document.nodes[idx].unrounded_layout.order;
        document.nodes[idx].unrounded_layout = TaffyLayout::with_order(order);
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if document.nodes[child_idx].is_in_document() {
                stack.push(child_idx);
            }
        }
    }
}

/// Whether it is exactly on the symmetric clamp boundary of [`MAX_TAFFY_MAGNITUDE`]. Since
/// `sanitize_finite` (`v.clamp(min, max)`) rounds out-of-range finite values exactly to `min` / `max`,
/// this equality check serves as an accurate proxy for "clamp actually fired on this field" — `NaN` does
/// not appear here because it is a separate path that rounds to `0.0` (see [`enforce_layout_invariants`]'s
/// doc, "NaN → 0.0 ..." section).
fn taffy_magnitude_is_saturated(v: f32) -> bool {
    v == MAX_TAFFY_MAGNITUDE || v == -MAX_TAFFY_MAGNITUDE
}

/// Designed as a predicate to check whether the **origin** of `child`'s border box (`location`,
/// `child.size` is not considered) is inside `parent`'s border box (from parent coordinate system origin
/// `(0,0)` to `parent.size`). **However, this function always returns `true`** — because un-saturated axes
/// were unconditionally "ok" from the start, and saturated axes also became unconditionally "ok"
/// **regardless of sign** (for details, see the "Reason for unconditional acceptance regardless of sign"
/// section below), there is no longer a path that actually re-checks containment. The decision of "whether
/// to maintain this invariant check itself" has been explicitly deferred and is unresolved at the time of
/// this doc (see below).
///
/// # Why `child.size` is not observed
///
/// Initially, containment was checked using `location.x + size.width <= parent.size.width`, which is an
/// **extent** (including the right/bottom edge of the child). However, it was found during implementation
/// that this would falsely detect legitimate CSS like `width: 200%`, where "the child is intentionally
/// larger than the parent." `width: 200%` represents a consistent relationship of "child is twice the
/// parent" at **any depth** (including shallow, unsaturated levels), and this is not a breakdown. Even if
/// individual used values start to be approximated beyond the [`MAX_TAFFY_MAGNITUDE`] band in a deep nest,
/// this relationship itself does not change (the "small parent + large child" checked by
/// `legitimate_negative_margin_overflow_is_not_reset` is also a legitimate overflow of the same class).
/// The design of this function also reflects this distinction, stating only that "the child's **location**
/// is within the parent's border box"—referring to **origin** containment, not extent. This function, as
/// designed, only looks at the origin.
///
/// # Reasons for limiting the gate to axis units and `child.location` itself
///
/// The previous version had a gate that stated: "If any one of `parent.size.width/height`,
/// `child.size.width/height`, or `child.location.x/y` is saturated, then **both** `x`/`y` are checked."
/// This form had two stages of false positives:
///
/// 1. **Per field**: Even if `child.location` itself was not saturated, the check would open just because
///    unrelated `parent.size` (sometimes due to saturation elsewhere in the same subtree) or `child.size`
///    was saturated. This would then incorrectly reset legitimate negative margins (`location.x` being a
///    negative value in the normal range, e.g., `-30`) by rejecting them with `>= 0.0`.
/// 2. **Per axis**: Even if 1 was narrowed down to "`child.location` itself is saturated," if the form
///    remained such that **either one** of `x` or `y` being saturated would check both, then legitimate
///    negative margins on the unsaturated axis could still be falsely detected for the same reason.
///
/// Both versions, when narrowed down to the axis unit, were closed—the gate that distinguished "to be
/// inspected/not to be inspected" at the field unit was limited to **whether the axis's own
/// `child.location` was actually saturated**, so axes with legitimate negative margins (within the normal
/// range, not saturated) were not mistakenly included. The explanation that this was a thorough
/// application of the consistent design principle of this function group (see the "Why
/// `sanitize_taffy_layout` alone is not closed" section in this module's documentation), where only
/// saturated fields are subject to distinction because their "actual values deviate from taffy's raw
/// calculation results," was accurate at this point. **Since this change, saturated axes are also
/// unconditionally accepted, so this gate has converged to the result of "no axis is inspected" rather
/// than "which axes are inspected"**—see the "Reasons for unconditional acceptance regardless of sign"
/// section below.
///
/// `saturated_but_contained_axis_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// originally directly checked the conjunction "inspect only saturated axes, unconditionally OK other
/// axes"—this design was based on the point that the old and new implementations could not be
/// distinguished in fixtures where "the reset can be explained by the `y` axis alone" by making the
/// saturated axis itself actually fit, and giving a legitimate negative margin to the other (unsaturated)
/// axis. After this change, the assert in this test still passes (because the fixture happens to be a
/// "fitting" case), but the content of the assertion has changed to "neither axis is a reason for reset"
/// (see the doc for the same test). The old
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// (now
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`)
/// originally checked the "detection capability of the `y` axis," but with this change, that detection
/// capability itself was lost, so it is no longer reset. The old
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`
/// (now `saturated_child_outside_parent_is_not_reset`) is similar—even if the saturated axis itself
/// (formerly) violated, it is no longer detected after this change.
///
/// # Reason for unconditional accept regardless of sign (changed from negative to positive direction)
///
/// **Negative direction**: Even in the version immediately after narrowing down to the `axis` unit, there
/// was still a false positive specific to the case where the **saturated `axis` itself was negative**. The
/// re-check expression `child.location >= 0.0 && child.location <= parent.size` could not satisfy `>= 0.0`
/// while `child.location` was negative, so it was **identically false** — meaning that for the negative
/// direction, this expression was not "checking for containment" but rather equivalent to "unconditionally
/// resetting if saturated and negative." Under CSS Box 3 §3.1 (see doc for `sanitize_taffy`, negative
/// `margin` values are allowed without limit within "implementation-specific limits"),
/// [`MAX_TAFFY_MAGNITUDE`] is that limit itself, and reaching it is not evidence of a breakdown, but
/// merely that a legitimate negative-direction relationship has begun to be approximated beyond the upper
/// limit of this implementation — it is symmetrical with "unsaturated negative values"
/// (`legitimate_negative_margin_overflow_is_not_reset`'s `-30`) which the same function already
/// unconditionally trusts, and there is no reason to discontinuously reverse the handling only at the
/// moment of crossing the `±MAX_TAFFY_MAGNITUDE` boundary.
///
/// At the time, the conclusion here was "do not loosen the positive direction." There were two reasons:
/// (a) `padding` / `border` / `width` are enforced to be non-negative at parse time, so a large `location`
/// in the positive direction cannot be justified with the same reasoning as margin, "CSS allows it without
/// limit." (b) Re-examination in the positive direction actually has both pass/fail branches
/// (`saturated_but_contained_layout_is_not_reset` passes, old
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout` fails), and loosening it would lose
/// existing detection capability. In the negative direction, there was no path to pass in the first place,
/// so no detection capability would be lost.
///
/// **Forward direction**: The basis (a) is covered—the margin is symmetric. CSS Box 3 §3.1
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>, "Negative values
/// for margin properties are allowed, but there may be
/// implementation-specific limits") explicitly allows **negative values**, but this is not a symmetrical
/// spec-based reason to bind positive margins more strictly. `margin-left: 1e9%` (within the
/// `width: 100px` container) saturates `location.x` in the positive direction, and the old implementation
/// incorrectly reset this — `saturated_positive_margin_percentage_child_is_not_reset` checks this via the
/// actual CSS pipeline (the positive counterpart of
/// `saturated_negative_margin_percentage_child_is_not_reset`). Reason (b) ("loss of detection power") has
/// not been disproven — that loss was knowingly accepted: existing tests
/// (`saturated_but_contained_layout_is_not_reset`, formerly
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout`) only constructed TaffyLayout directly
/// and never demonstrated detection power via the actual CSS pipeline, whereas this data loss (complete
/// disappearance of legitimate content) was already demonstrated via actual CSS.
///
/// **Result**: `axis_ok` is unified to "unconditionally accept if saturated" regardless of sign, and this
/// function always returns `true` — there is no longer a path to re-check containment. The old
/// `saturated_child_outside_parent_resets_subtree_to_zero_layout` was renamed to
/// `saturated_child_outside_parent_is_not_reset`, and the old
/// `saturated_location_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// to
/// `saturated_axis_outside_parent_with_legitimate_negative_margin_on_other_axis_is_not_reset`
/// , respectively, and inverted (see each test's doc for details). Whether "this invariant check itself
/// should be maintained" is a decision explicitly deferred and unresolved at the time of this doc.
///
/// `saturated_negative_margin_percentage_child_is_not_reset` (a single `margin-left: -1e9%` declaration,
/// no nesting) and
/// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset`
/// (a deep nest chain of `width: 200%; margin-left: -100%`, negative counterpart of
/// `nested_percentage_wide_child_chain_is_not_reset`) check this via the actual CSS pipeline. The former
/// is also the minimal fixture that disproves an alternative proposal, which was considered but rejected:
/// "if `parent.size` itself is saturated on the same axis, skip re-validation regardless of sign." This
/// fixture shows that `parent.size.width` is not saturated (it remains `100.0`), so the distinguishing
/// axis must be "the sign of the child itself," not "whether the parent is also saturated" (although this
/// distinguishing axis itself has lost meaning since this change, its value as a fixture and regression
/// check remains unchanged). `saturated_negative_location_is_not_reset` isolates and inspects the same
/// combination in a directly constructed minimal synthetic case (the negative counterpart of the positive
/// case, which pairs with `saturated_but_contained_layout_is_not_reset` — since this change, both are
/// pinned to the same conclusion: "unconditionally accept if saturated").
fn child_within_parent_border_box(parent: &TaffyLayout, child: &TaffyLayout) -> bool {
    /// Judgment for one axis. `location` is the `child.location.{x,y}` of that axis (`parent_size` is no
    /// longer used at all in the main formula — a dead parameter. It is not deleted, and the appearance of the
    /// calling convention "pass the corresponding `parent.size.{width,height}`" is left because whether to
    /// maintain this invariant check, including whether to delete this function/`axis_ok` itself, is a
    /// follow-up that was explicitly deferred separately — anticipating that it may be deleted along with
    /// `axis_ok` in that future follow-up, the decision to preemptively change the signature has not been made
    /// now. For details, see the section "Reason for unconditional acceptance regardless of sign" in the doc
    /// above. For the same reason, the `if !taffy_magnitude_is_saturated(location) { return true; }` branch
    /// immediately below is also effectively a dead branch equivalent to the default statement that always
    /// returns `true` — this too has not been folded into a single `true` until the same deferred follow-up
    /// where it might be deleted along with `axis_ok`).
    fn axis_ok(location: f32, _parent_size: f32) -> bool {
        if !taffy_magnitude_is_saturated(location) {
            return true;
        }
        // If saturated, unconditionally accept regardless of sign. The negative direction was established earlier
        // — this decision extends that precedent symmetrically to the positive direction and removes the old
        // `location <= parent_size` re-check (positive direction only). There is no longer a path to re-check
        // containment.
        true
    }
    axis_ok(child.location.x, parent.size.width) && axis_ok(child.location.y, parent.size.height)
}

#[cfg(test)]
mod tests;
