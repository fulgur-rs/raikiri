//! `@page` at-rule shape — parser scaffolding for CSS Paged Media Level 3.
//!
//! # Status
//!
//! The rule tree stores parsed `@page` rules and [`cascade_page`] resolves the
//! winning declarations for a given page context (origin / specificity /
//! source-order cascade, then phase 2 = resolution against the page context's
//! inheritance parent, then phase 3 = absolutization against the page context's
//! own font-size + `border-*-width` style gating — see that function's doc).
//! Nested margin-box at-rules (`@top-left { … }` etc.) are recognized and
//! their declaration bodies stored per [`PageRule::margin_box_rules`] (see
//! [`PageMarginBoxRule`]) — but, like the rest of this module, only up to the
//! parse boundary. Per-page `PageBox` derivation and margin-box slot
//! *layout* (the sixteen boxes' geometry, and cascade resolution across
//! multiple rules naming the same slot) remain deferred future work that
//! lives downstream; this module produces the declaration bags they
//! consume.
//!
//! # Primary source
//!
//! Seven anchors are cited, one per production the parser consumes:
//!
//! - `<page-selector-list>` / `<page-selector>` grammar and the "No whitespace
//!   is allowed between the productions" compound rule — CSS Paged Media
//!   Module Level 3, §4.3 "@page rule grammar":
//!   <https://www.w3.org/TR/css-page-3/#syntax-page-selector>
//! - `<pseudo-page>` grammar production (the four `:first` / `:left` /
//!   `:right` / `:blank` idents) — CSS Paged Media Module Level 3 §4.3, dfn
//!   anchor for the production:
//!   <https://www.w3.org/TR/css-page-3/#typedef-pseudo-page>
//! - `<ident-token>` = named-page ident. This ident originates from the
//!   `page` property (§8.1 "Using named pages: page"); its value is a
//!   `<custom-ident>` and therefore case-sensitive even in ASCII (see
//!   [`PageSelectorEntry::ident`] for the case-sensitivity citation chain):
//!   <https://www.w3.org/TR/css-page-3/#using-named-pages>
//! - `size` descriptor grammar (`<length>{1,2} | auto | [ <page-size> || [
//!   portrait | landscape ] ]`) — CSS Paged Media Module Level 3, §7.1
//!   "Page size: the size property":
//!   <https://www.w3.org/TR/css-page-3/#page-size-prop>
//! - `marks` descriptor grammar (`none | [ crop || cross ]`) — CSS Paged
//!   Media Module Level 3, §7.2 "Crop and Registration Marks: the marks
//!   property":
//!   <https://www.w3.org/TR/css-page-3/#marks>
//! - `bleed` descriptor grammar (`auto | <length>`) — CSS Paged Media Module
//!   Level 3, §7.3 "Bleed Area: the bleed property":
//!   <https://www.w3.org/TR/css-page-3/#bleed>
//! - Margin-box at-rule grammar (the sixteen `@top-left` etc. idents, each
//!   introducing a plain declaration list) — CSS Paged Media Module Level 3,
//!   §5.1 "At-rules for page-margin boxes":
//!   <https://www.w3.org/TR/css-page-3/#margin-at-rules>
//!
//! # Grammar coverage
//!
//! The spec grammar is:
//!
//! ```text
//! <page-selector-list> = <page-selector>#
//! <page-selector>      = [ <ident-token>? <pseudo-page>* ]!
//! <pseudo-page>        = ':' [ left | right | first | blank ]
//! ```
//!
//! The `<page-selector>` and `<pseudo-page>` productions are *compound*: the
//! spec explicitly states "No whitespace is allowed between the productions
//! in `<page-selector>` or `<pseudo-page>` (similar to the rule for
//! `<compound-selector>`)". Whitespace around the `,` separator of
//! `<page-selector-list>` is permitted (it is not a compound at that level).
//!
//! This parser accepts the full L3 shape:
//!
//! - Empty prelude (`@page { … }`) — matches every page.
//! - Named-page ident alone (`@page named { … }`).
//! - One-or-more `<pseudo-page>` alone (`@page :first { … }`,
//!   `@page :first:left { … }`).
//! - Ident followed by one-or-more `<pseudo-page>` (`@page named:first { … }`).
//! - Comma-separated list of the above (`@page :first, :left { … }`).
//!
//! Whitespace *within* a compound (e.g. `@page : left`, `@page named :first`,
//! `@page :first :left`) is rejected per the compound rule — the whole
//! `@page` rule is dropped.
//!
//! # Margin-box at-rules
//!
//! An `@page` block body may nest one of the sixteen margin-box at-rules
//! (`@top-left { … }`, `@bottom-center { … }`, etc. — see [`PageMarginBoxSlot`]
//! for the full list) per CSS Paged Media Level 3 §5.1. Each is recognized
//! by ident, takes no prelude, and its body is parsed as an ordinary
//! declaration list — `content:` (§5.2 "Populating page-margin boxes")
//! generates the box's actual content; spec §5.1 also states "The margin
//! at-rules can only contain page-margin properties", a restriction this
//! parser does not enforce (no applicable-property filtering happens for
//! the `@page` block body either — see [`PageMarginBoxRule`]'s doc). An
//! unrecognized nested at-rule name is dropped (the whole nested block is
//! skipped, declarations before and after it in the `@page` block still
//! parse) — see [`PageMarginBoxRule`] for the stored shape.
//!
mod types;
pub use types::*;

mod parse;
pub(crate) use parse::*;

mod cascade;
pub use cascade::*;
