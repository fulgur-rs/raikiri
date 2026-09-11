//! Border paint helpers — `BorderColor` used-value resolution.
//!
//! CSS Backgrounds 3 §5.3 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
//! defines `border-*-color`'s initial value as `currentcolor`. In raikiri's
//! cascade (raikiri-spike-0vv.17) that keyword is preserved as
//! [`BorderColor::CurrentColor`] — a specified-value sentinel — rather than
//! baked to a concrete color at cascade time. The true used value is
//! `currentcolor` → "the value of the `color` property" (CSS Color 3 §4.4
//! <https://www.w3.org/TR/css-color-3/#currentColor-def>), resolved here at
//! paint time by looking up the same node's computed `color`.
//!
//! This module is intentionally tiny: one `match` plus docs. Border *drawing*
//! itself is still deferred (see [`crate`] docs `Non-goals`), so this helper
//! has no call site yet — it exists so the hazard in `raikiri-spike-q7qf`
//! can be closed with a pinned, tested bridge between `BorderColor` and
//! `CascadeResult.computed[node].color`.
//!
//! Reference pattern: `crate::text::draw_text_node` reads
//! `cascade.computed[node_id].color` for the text brush in the same shape
//! (`CascadeResult.computed[node].color`). A future border paint site will
//! call [`resolve_border_color`] with that same `color` value.

use raikiri_style::property::{BorderColor, CssColor};

/// Resolve a single [`BorderColor`] to a concrete [`CssColor`] using the
/// owning node's computed `color` property.
///
/// - [`BorderColor::CurrentColor`] → `current_color` (CSS Color 3 §4.4:
///   "`currentcolor` is the value of the `color` property").
/// - [`BorderColor::Resolved`] → the contained color unchanged.
///
/// `current_color` is expected to be `cascade.computed[node_id].color` — the
/// same lookup `crate::text::draw_text_node` uses for the text brush
/// (`cv.color`). No other node, no inheritance walk, no extra indirection.
///
/// # Examples
///
/// ```ignore
/// // future border paint site (not yet wired):
/// let cv = &cascade.computed[node_id];
/// let top = resolve_border_color(cv.border.top.color, cv.color);
/// ```
#[inline]
pub fn resolve_border_color(border_color: BorderColor, current_color: CssColor) -> CssColor {
    match border_color {
        BorderColor::CurrentColor => current_color,
        BorderColor::Resolved(c) => c,
        // `#[non_exhaustive]` — future variants (e.g. system colors) must
        // not silently fall through; `todo!()` makes the gap loud.
        _ => todo!("unhandled BorderColor variant — update resolve_border_color"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raikiri_style::property::{BorderColor, CssColor};

    const RED: CssColor = CssColor {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    const BLACK: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    const BLUE: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 255,
        a: 255,
    };
    const TRANSPARENT: CssColor = CssColor {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    #[test]
    fn current_color_resolves_to_computed_color() {
        // Hazard 1: <div style="color: red; border-color: currentColor">
        assert_eq!(resolve_border_color(BorderColor::CurrentColor, RED), RED);
    }

    #[test]
    fn initial_current_color_resolves_to_computed_color() {
        // Hazard 2: <div style="color: red"> (border-color omitted → initial
        // is CurrentColor per CSS Backgrounds 3 §3.1). Same helper path;
        // cascade-side pin is in raikiri-style's initial_values_match_spec,
        // here we pin the paint-side resolution.
        assert_eq!(resolve_border_color(BorderColor::CurrentColor, RED), RED);
    }

    #[test]
    fn resolved_black_is_independent_of_current_color() {
        // Hazard 3: <div style="border-color: black"> with a different
        // currentColor — Resolved must ignore current_color.
        assert_eq!(
            resolve_border_color(BorderColor::Resolved(BLACK), RED),
            BLACK
        );
        assert_eq!(
            resolve_border_color(BorderColor::Resolved(BLACK), BLUE),
            BLACK
        );
    }

    #[test]
    fn resolved_transparent_preserved() {
        // `transparent` is a valid <color> (CssColor::TRANSPARENT) that must
        // pass through unchanged even when currentColor is opaque red.
        assert_eq!(
            resolve_border_color(BorderColor::Resolved(TRANSPARENT), RED),
            TRANSPARENT
        );
    }

    #[test]
    fn current_color_with_transparent_current() {
        // CurrentColor where the computed color itself is transparent — still
        // just returns current_color verbatim (no special-casing).
        assert_eq!(
            resolve_border_color(BorderColor::CurrentColor, TRANSPARENT),
            TRANSPARENT
        );
    }

    #[test]
    fn resolved_red_ignores_transparent_current() {
        assert_eq!(
            resolve_border_color(BorderColor::Resolved(RED), TRANSPARENT),
            RED
        );
    }
}
