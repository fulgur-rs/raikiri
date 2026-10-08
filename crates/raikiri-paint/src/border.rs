//! Border paint helpers — `BorderColor` used-value resolution.
//!
//! CSS Backgrounds 3 §5.3 <https://www.w3.org/TR/css-backgrounds-3/#border-color>
//! defines `border-*-color`'s initial value as `currentcolor`. In raikiri's
//! cascade that keyword is preserved as
//! [`BorderColor::CurrentColor`] — a specified-value sentinel — rather than
//! baked to a concrete color at cascade time. The true used value is
//! `currentcolor` → "the value of the `color` property" (CSS Color 3 §4.4
//! <https://www.w3.org/TR/css-color-3/#currentColor-def>), resolved here at
//! paint time by looking up the same node's computed `color`.
//!
//! This module is intentionally tiny: one `match` plus docs. Border *drawing*
//! itself is still deferred (see [`crate`] docs `Non-goals`), so this helper
//! has no call site yet — it exists so the hazard in this implementation
//! can be closed with a pinned, tested bridge between `BorderColor` and
//! `CascadeResult.computed[node].color`.
//!
//! Reference pattern: [`crate::ifc_text::draw_ifc_lines_with_resources`] reads
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
/// same lookup [`crate::ifc_text::draw_ifc_lines_with_resources`] uses for the text brush
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
mod tests;
