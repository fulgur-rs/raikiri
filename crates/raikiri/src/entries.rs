//! [`PageDrawables`](crate::PageDrawables) の per-attribute map に格納される
//! entry 型群 (raikiri-spike-0icy Axis 2 の struct-of-arrays element)。
//!
//! # Sprint 22 (raikiri-spike-os52) landing scope
//!
//! Pub type surface のみ landing、**field は空** (`#[derive(Default)]` で
//! consumer が `Default::default()` から construct 可能)。
//! 各 entry の実際の field は future sprint で fulgur reference shape と
//! 照合しつつ、raikiri 内部 pipeline (fragmentation → drawable extraction)
//! が populate する時点で minimal fields から拡張する。
//!
//! `#[non_exhaustive]` を全 struct に付与しているため、future field 追加は
//! **semver-non-breaking** に行える (consumer は literal `BlockEntry { .. }`
//! を書けない → 追加 field で consumer が compile fail しない)。

/// Block box (background / border / shadow / opacity / anchor id 等) の
/// per-node paint state。
///
/// Fulgur drawables.rs:142-177 の `BlockEntry` shape
/// (`style` / `opacity` / `visible` / `id` / `layout_size` / `clip_descendants`
/// / `opacity_descendants`) が future sprint で minimal fields を選定する時の
/// reference material。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct BlockEntry {}

/// Paragraph (shaped inline text lines) の per-node paint state。
///
/// Fulgur drawables.rs:183-191 の `ParagraphEntry` shape
/// (`lines` / `opacity` / `visible` / `id`) が future populate 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct ParagraphEntry {}

/// Raster image (jpg / png / gif etc.) の per-node paint state。
///
/// Fulgur drawables.rs:205-213 の `ImageEntry` shape
/// (`image_data` / `format` / `width` / `height` / `opacity` / `visible`) が
/// future populate 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct ImageEntry {}

/// SVG image の per-node paint state。
///
/// Fulgur drawables.rs:225-232 の `SvgEntry` shape
/// (`tree` / `width` / `height` / `opacity` / `visible`) が future populate
/// 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct SvgEntry {}

/// Table (outer frame の background / border / shadow) の per-node paint state。
///
/// Fulgur drawables.rs:243-257 の `TableEntry` shape
/// (`style` / `opacity` / `visible` / `id` / `layout_size` / `width` /
/// `cached_height` / `clip_descendants`) が future populate 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct TableEntry {}

/// List item marker (text / image / none) の per-node paint state。
///
/// Fulgur drawables.rs:271-299 の `ListItemEntry` と `ListItemMarker` shape
/// (`marker` / `marker_line_height` / `opacity` / `visible`) が future populate
/// 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct ListItemEntry {}

/// CSS transform (matrix + origin + descendants scope) の per-node state。
///
/// Fulgur drawables.rs:392-405 の `TransformEntry` shape
/// (`matrix` / `origin` / `descendants`) が future populate 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct TransformEntry {}

/// Multicol の column-rule 描画 spec と per-column-group geometry。
///
/// Fulgur drawables.rs:316-340 の `MulticolRuleEntry` と `ColumnRuleGeometry`
/// shape が future populate 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct MulticolRuleEntry {}

/// Bookmark anchor (PDF bookmark tree の source) の per-node state。
///
/// Fulgur drawables.rs:409-413 の `BookmarkAnchorEntry` shape
/// (`level` / `label`) が future populate 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct BookmarkAnchorEntry {}

/// Paragraph 内 glyph run 上に張る hyperlink target span。
///
/// Fulgur drawables.rs:418-419 の `LinkSpanEntry` shape が future populate
/// 時の reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct LinkSpanEntry {}

/// Tagged-PDF semantics (tag / parent / alt_text 等) の per-node state。
///
/// Fulgur tagging.rs:57 の `SemanticEntry` shape が future populate 時の
/// reference。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct SemanticEntry {}
