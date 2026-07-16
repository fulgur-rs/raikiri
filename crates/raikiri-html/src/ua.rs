//! Bundled UA stylesheet for HTML documents (spec §M1.4a、raikiri-spike-m1.22)。
//!
//! `MINIMAL_UA_CSS` は raikiri-html crate が HTML LS §14 "Rendering" 由来
//! (cleanroom) の必要最小 UA CSS を提供する。`raikiri-html::parse` は parse
//! 完了時に自動的にこの CSS を Document に inject する
//! (`StylesheetKind::UserAgent` として)。
//!
//! spec 参照ソース制約 (memory `raikiri-implementation-independence`
//! §"UA CSS の扱い" 準拠):
//!
//! - 参照 OK: CSS 2.1 App.D、HTML Living Standard §14、CSS module Sample
//!   style sheet
//! - 参照 NG: Chromium `html.css`、Firefox `layout/style/res/html.css`、
//!   WebKit UA CSS、blitz が bundle する UA CSS

/// Bundled minimal UA CSS for HTML documents。
///
/// 実体は `crates/raikiri-html/src/ua/minimal.css` に `include_str!` で
/// 埋め込まれた文字列。詳細は同ファイル頭部コメント参照。
pub const MINIMAL_UA_CSS: &str = include_str!("ua/minimal.css");
