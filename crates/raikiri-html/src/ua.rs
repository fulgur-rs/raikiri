//! Bundled UA stylesheet for HTML documents.
//!
//! `MINIMAL_UA_CSS` supplies the minimal UA CSS derived from HTML LS §14 "Rendering"
//! through an independent implementation. `raikiri-html::parse` automatically injects
//! this CSS into the Document when parsing completes
//! (as `StylesheetKind::UserAgent`).
//!
//! Allowed specification sources (independent implementation; do not import another UA CSS):
//!
//! - Allowed: CSS 2.1 App.D, HTML Living Standard §14, CSS module sample
//!   style sheet
//! - Disallowed: Chromium `html.css`, Firefox `layout/style/res/html.css`,
//!   WebKit UA CSS, and UA CSS bundled with blitz.

/// Bundled minimal UA CSS for HTML documents.
///
/// The source is embedded via `include_str!` from `crates/raikiri-html/src/ua/minimal.css`.
/// See the comments at the top of that file for details.
pub const MINIMAL_UA_CSS: &str = include_str!("ua/minimal.css");

/// An independent tree seeded by the fixed UA, or normal parsing when the
/// stylesheet grows beyond the prepared representation's supported state.
pub(crate) fn prepared_rule_tree() -> Option<raikiri_style::RuleTree> {
    static PREPARED: std::sync::OnceLock<Option<raikiri_style::PreparedStylesheet>> =
        std::sync::OnceLock::new();
    PREPARED
        .get_or_init(|| {
            raikiri_style::PreparedStylesheet::new(MINIMAL_UA_CSS, raikiri_style::Origin::UserAgent)
        })
        .as_ref()
        .map(raikiri_style::PreparedStylesheet::to_rule_tree)
}
