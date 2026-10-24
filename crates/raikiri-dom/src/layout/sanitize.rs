use super::*;
use crate::layout::context::LayoutSession;

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
/// [`BoxLayout`] **after resolution** with the same `[-MAX, MAX]`. This is depth-independent.
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
/// `<LayoutSession as taffy::LayoutPartialTree>::set_unrounded_layout` — this is a fixed trait method signature
/// on the `taffy` crate side, so **an observation argument cannot be added**.
///
/// I introduced a common [`crate::diag::emit_warn_via`] mechanism and solved these two points as follows:
/// - The `bridge_*` → `apply_computed_to_style` chain consists only of private functions within the crate,
///   so passing `diag: &mut Vec<LayoutWarn>` is completed with a crate-internal signature change (the pub
///   signature remains intact).
/// - `set_unrounded_layout` can accept `self` (`&mut LayoutSession`), so the observer can be an **owned
///   buffer** in the session instead of a closure. The Taffy callback pushes into it without changing the
///   trait signature. `layout_single_page` drains this buffer at the end of the path and sends each event
///   to the observer-or-eprintln path via `fonts.rs`, which is the same as `emit_warn_via`.
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
    /// sink boundaries: `taffy::Style` or the `Node.layout` arena
    /// field (sites 1-5, `sanitize_finite` / `sanitize_taffy` /
    /// `sanitize_taffy_layout`). Only
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
    /// One or more subtrees had a broken **box model invariant** and were
    /// reset to a deterministic zero [`BoxLayout`] by
    /// [`enforce_layout_invariants`]. This is a different failure class than
    /// [`LayoutWarn::NonFiniteClamped`]: that variant fires when a single
    /// `f32` field was out of range, this one fires when every individual
    /// field of the (already per-field-clamped) `Layout` was in range, but
    /// the *relationship* between its fields was not (a node's content box
    /// went negative once its size or paddings had their actual values
    /// approximated by [`sanitize_taffy_layout`]). See
    /// [`enforce_layout_invariants`]'s doc for which invariant is checked
    /// and why checking cross-node containment would be spec-incorrect.
    ///
    /// Aggregated per invariant (at most one event per invariant kind per
    /// `layout_single_page` pass, each counting every subtree it reset)
    /// rather than one event per reset subtree, so a pathological input
    /// that trips the same invariant on many nodes cannot reintroduce the
    /// per-node spam [`LAYOUT_WARN_CAP`] exists to bound.
    GeometryInvariantViolated {
        /// Which invariant was violated — currently always
        /// `"content_box_non_negative"` (see [`enforce_layout_invariants`]'s
        /// doc). Like `NonFiniteClamped`'s `site`, a human-readable category,
        /// not a stable machine-parseable identifier.
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
/// entry point: `layout_single_page` is consumed by `raikiri-paint` and the
/// `raikiri` crate, so exposing an observer requires a public API addition.
/// This scaffolding lets a future observer API pass one new parameter through
/// the existing warning path.
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

/// An **output-side** guard that passes all f32 fields of [`BoxLayout`] resolved by taffy to
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
/// `<LayoutSession as taffy::LayoutPartialTree>::set_unrounded_layout`
/// (`layout/taffy_adapter/tree.rs`) — the **only** path through which Taffy writes layout back to the arena (all block /
/// flexbox / grid / leaf algorithms in `taffy-0.12.1` pass through this single method). Therefore,
/// "`Node.layout` never contains non-finite values" is a structural invariant and cannot be
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
/// `"layout.border"`, `"layout.padding"`, `"layout.margin"`). Its only
/// caller is [`LayoutSession::store_layout`], which passes the
/// operation's warning buffer because Taffy's `set_unrounded_layout`
/// signature cannot receive a diagnostic observer.
pub(crate) fn sanitize_taffy_layout(layout: &BoxLayout, diag: &mut Vec<LayoutWarn>) -> BoxLayout {
    fn size(s: Size<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Size<f32> {
        Size {
            width: sanitize_taffy(s.width, site, diag),
            height: sanitize_taffy(s.height, site, diag),
        }
    }
    fn rect(r: Edges<f32>, site: &'static str, diag: &mut Vec<LayoutWarn>) -> Edges<f32> {
        Edges {
            left: sanitize_taffy(r.left, site, diag),
            right: sanitize_taffy(r.right, site, diag),
            top: sanitize_taffy(r.top, site, diag),
            bottom: sanitize_taffy(r.bottom, site, diag),
        }
    }
    BoxLayout {
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

impl LayoutSession<'_> {
    /// Write `layout` to the node's `layout` after clamping every
    /// field to a finite value with [`sanitize_taffy_layout`].
    ///
    /// Taffy's `set_unrounded_layout` and the table algorithm's own
    /// placements store whole layouts through this one path. Later passes
    /// that shift or scale an already stored layout in place (page breaking,
    /// out-of-flow placement, table cell offsets) write the field directly.
    pub(crate) fn store_layout(&mut self, idx: usize, layout: &BoxLayout) {
        let sanitized = sanitize_taffy_layout(layout, &mut self.context.layout_warnings);
        self.nodes[idx].layout = sanitized;
        // The rows an anonymous table wraps are DOM children of its parent,
        // so they move with the table.
        self.offset_anonymous_table_parts(idx);
    }

    /// Store a table's grid rectangle, clamped to finite values like
    /// [`Self::store_layout`].
    pub(crate) fn store_table_grid_box(
        &mut self,
        table_idx: usize,
        location: Point<f32>,
        size: Size<f32>,
    ) {
        let diag = &mut self.context.layout_warnings;
        let grid_box = raikiri_traits::PaintRect::new(
            sanitize_taffy(location.x, "table_grid_box.location", diag),
            sanitize_taffy(location.y, "table_grid_box.location", diag),
            sanitize_taffy(size.width, "table_grid_box.size", diag),
            sanitize_taffy(size.height, "table_grid_box.size", diag),
        );
        self.nodes[table_idx].table_grid_box = Some(grid_box);
    }
}

/// A layer above the **finiteness** guaranteed by [`sanitize_taffy_layout`] — it inspects the **semantic**
/// box model invariant of each laid-out node and replaces broken subtrees with deterministic default geometry
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
/// fills the gap in box model containment within a single node.
///
/// # Invariant checked
///
/// **Content box non-negative**: both `Layout::content_box_width()` and `content_box_height()` are
/// `>= 0.0`. This applies **unconditionally to all nodes**. A probe with
/// `<div style="width: 10px; padding: 50px; box-sizing: border-box;">` through the normal layout path
/// (`layout_single_page`) showed that Taffy stretches the border box until
/// `size.width == padding_left + padding_right` (= `100.0`) and floors the content box to exactly `0.0`
/// (internal logic similar to `maybe_max(padding_border_size)` in `taffy-0.12.1/src/compute/mod.rs`).
/// So **in normal CSS not involving non-finite clamps, the content box is never negative**, and checking
/// this invariant unconditionally does not falsely detect legitimate layouts. Cases where padding or
/// border saturate, or where `NaN` rounds to `0.0` and makes the content box negative, are caught here.
///
/// # Parent-child containment is not checked
///
/// A child's border box is not required to sit inside its parent's. CSS lets children overflow their
/// parent's border box (`overflow: visible` is the initial value, negative margins, `width: 200%`), and a
/// probe of a `50px` parent with a `200px` child at `margin-left: -30px` gives the child
/// `location=(-30,0)`, which is correct layout. Reaching [`MAX_TAFFY_MAGNITUDE`] on a child's `location`
/// is not evidence of failure either: CSS Box 3 §3.1
/// (<https://www.w3.org/TR/css-box-3/#margin-physical>) allows margins of either sign up to
/// "implementation-specific limits", and the clamp is exactly that limit. Resetting such subtrees made
/// legitimate content disappear (`margin-left: ±1e9%`, deep `width: 200%; margin-left: -100%` chains), so
/// saturated and unsaturated locations alike are accepted. The tests
/// `saturated_child_outside_parent_is_not_reset`,
/// `saturated_negative_margin_percentage_child_is_not_reset`,
/// `deep_nested_negative_percentage_margin_saturating_location_is_not_reset` and
/// `nested_percentage_wide_child_chain_is_not_reset` pin this.
///
/// # Deterministic fallback: zero out subtree (`zero_layout_subtree`)
///
/// The design decision to adopt "deterministic fallback" instead of "input restriction," "intermediate
/// saturation," or "layout abort" has been finalized. Among the two fallback value proposals considered
/// ("0 size" and "nearest finite parent size"), **0 size** is chosen. [`BoxLayout::with_order`]
/// (keeping only `order`, all others zero) was selected for two reasons: (a) it trivially and recursively
/// satisfies the content box invariant (`0 - 0 - 0 = 0 >= 0`) and places the subtree at its parent's
/// origin, so filling the entire subtree with this value does not create new violations (whereas
/// "fallback to the nearest finite parent size" would need extra guarantees about the fallback value,
/// leading to a design with repeated re-checks), and (b) "0 size" was listed first in the original description during consideration.
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
/// descendants, since `layout` remains at its construction-time default
/// (`Layout::with_order(0)`) (meaning old values from other subtrees are not mixed in), aligning with
/// Taffy's traversal contract will be less surprising to the reader.
///
/// # Caller
///
/// Runs exactly once, immediately after Step 5 (`compute_root_layout`) of [`layout_single_page`] and
/// before Step 6 (warning replay) — right after all `layout` that taffy actually wrote under the
/// root (`<body>`) are available. Paths that directly call `compute_root_layout` (each unit test of
/// `lib.rs`) do not go through this pass — those tests are for the characterization of
/// `sanitize_taffy_layout` (finiteness only), and semantic invariants are not covered (see
/// `taffy_block_layout_does_not_hang_on_raw_nonfinite_style_geometry`'s doc).
pub(crate) fn enforce_layout_invariants(document: &mut LayoutSession<'_>, root_idx: usize) {
    let mut content_box_violations = 0usize;
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
        let layout = document.nodes[idx].layout;
        if layout.content_box_width() < 0.0 || layout.content_box_height() < 0.0 {
            zero_layout_subtree(document, idx);
            content_box_violations += 1;
            continue;
        }
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if document.nodes[child_idx].is_in_document() {
                stack.push(child_idx);
            }
        }
    }
    if content_box_violations > 0 {
        push_layout_warn(
            &mut document.context.layout_warnings,
            LayoutWarn::GeometryInvariantViolated {
                invariant: "content_box_non_negative",
                subtree_count: content_box_violations,
            },
        );
    }
}

/// The fallback body of [`enforce_layout_invariants`] — overwrites the `layout` of the subtree
/// rooted at `root_idx` (only nodes actually visited by taffy, `is_in_document()` filter) with
/// [`BoxLayout::with_order`] (which only keeps `order` and sets all others to zero). Iterative (`Vec`
/// stack) — recursion is not used, precisely because the target is a deep DOM (see the "traversal is not
/// recursive" section of [`enforce_layout_invariants`]).
fn zero_layout_subtree(document: &mut LayoutSession<'_>, root_idx: usize) {
    let mut stack = vec![root_idx];
    while let Some(idx) = stack.pop() {
        let order = document.nodes[idx].layout.order;
        document.nodes[idx].layout = BoxLayout::with_order(order);
        let child_count = document.nodes[idx].children.len();
        for i in 0..child_count {
            let child_idx = document.nodes[idx].children[i];
            if document.nodes[child_idx].is_in_document() {
                stack.push(child_idx);
            }
        }
    }
}

#[cfg(test)]
mod tests;
