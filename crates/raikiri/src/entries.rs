//! [`PageDrawables`](crate::PageDrawables) の per-attribute map に格納される
//! entry 型群 (struct-of-arrays element)。
//!
//! # Landing history
//!
//! 当初は pub type surface のみ landing (全 struct `{}` 空)。その後
//! (narrowed scope — `raikiri_paint::paint_single_page` rework は
//! 別途分離) で fulgur reference shape と照合しつつ minimal field を追加した。
//!
//! 実際に [`build_page_scene`](crate::page_scene::build_page_scene) から
//! instance が construct され `PageDrawables` へ insert されるのは
//! [`BlockEntry`] (post-layout Element node) と [`ParagraphEntry`] (post-
//! layout Text node) の 2 型のみ。他 9 型は fulgur shape に合わせた field は
//! 追加したが、raikiri に対応する pipeline stage (image decode / table
//! layout / list-item marker layout / CSS transform / multicol / PDF
//! bookmark・tag・link-span) がまだ存在しないため常に `Default::default()`
//! のまま — 一度も construct されない。詳細な理由は各 struct doc を参照。
//!
//! # Field type 方針
//!
//! crate-topology 判断 (`PageDrawables`/entries を raikiri-traits へ
//! relocate する案も検討対象) がどの案に転んでもこれまでの作業をやり直さず
//! に済むよう、各 struct の field 型は
//! **raikiri-style / taffy / parley の型を直接参照しない** — `f32` / `bool` /
//! `u8` tuple / `Option<String>` / [`NodeId`] / `Vec<NodeId>` のみを使う。
//! 理由: raikiri-traits は raikiri-style に依存していない (原則5 cleanroom
//! 境界とは別の、純粋な dependency-graph 制約) ため、これらの型を relocate
//! 先の crate が新たに引き込む必要が生じるのを避ける。`ComputedValues` の
//! 値は populate 時に primitive へ変換して埋める。
//!
//! `opacity` (CSS property) は `raikiri_style::ComputedValues` にまだ存在
//! しない (`visibility` / `overflow` も同様)。これらの field は CSS 初期値
//! (`opacity: 1`、`visibility: visible`、descendant-clip 対象 0 件) を
//! hardcode している — これは「未実装だから嘘の値」ではなく「該当
//! property が cascade に存在しない = 常に初期値」を正しく反映した値で
//! ある。cascade がこれらの property を持つようになった時点で、対応する
//! field は hardcode 定数をやめて実 lookup に切り替える。
//!
//! Numeric な長さ系 field は [`crate::page_scene::Pt`] (= `f32` alias) を
//! 再利用する。名前は "Pt" (PDF point) だが、[`crate::page_scene::build_page_scene`] は CSS px
//! 値をそのまま詰める既存の unit debt (`crate::page_scene` module doc 参照)
//! に本 entry 群も従う — 新たに別種の unit debt を作らないための意図的な
//! 選択。
//!
//! `#[non_exhaustive]` を全 struct に付与しているため、future field 追加は
//! **semver-non-breaking** に行える (consumer は literal `BlockEntry { .. }`
//! を書けない → 追加 field で consumer が compile fail しない)。

use crate::page_scene::Pt;
use raikiri_traits::NodeId;

/// Block box (background / border / opacity / anchor id 等) の per-node
/// paint state。
///
/// Fulgur drawables.rs:142-177 の `BlockEntry` shape (`style` / `opacity` /
/// `visible` / `id` / `layout_size` / `clip_descendants` /
/// `opacity_descendants`) を reference に、raikiri で実際に取得できる
/// minimal field を選定。
///
/// `build_page_scene` (crate::page_scene) が post-layout
/// Element node ごとに construct し `PageDrawables::block_styles` へ insert
/// する — この段階で populate される 2 型のうちの 1 つ (もう 1 つは
/// [`ParagraphEntry`])。
///
/// # `style` を bundle 型にしなかった理由
///
/// Fulgur は `style: BlockStyle` (background / border / box-shadow をまとめた
/// 専用 paint-primitive 型) を持つが、raikiri には対応する bundle 型が無い。
/// 新設すると umbrella pub surface (新規 `pub use`) が増え、item 4 の
/// topology 判断が確定するまで型を固めたくない (module doc の field type
/// 方針参照)。代わりに background-color / border-width をこの struct へ
/// 直接 flatten した。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct BlockEntry {
    /// `background-color` の (r, g, b, a)。`raikiri_style::CssColor` を
    /// primitive tuple に変換したもの (module doc の field type 方針、
    /// `CssColor` 型は直接持たない)。
    pub background_color: (u8, u8, u8, u8),
    /// `border-{top,right,bottom,left}-width` の computed px 値
    /// (CSS shorthand と同じ top/right/bottom/left 順)。CSS の computed 層は
    /// `border-style: none` の場合に width を 0 へ強制する (CSS Backgrounds 3
    /// §3.3) ため、`0.0` は「border が実質無い」ことも正しく表現する。
    /// border-style (`solid` 等) と border-color は primitive へ安全に
    /// flatten する型が無いため未収録 (gap、paint が実際に消費する段階で
    /// 追加を検討)。
    pub border_widths: (Pt, Pt, Pt, Pt),
    /// `opacity`。property が cascade に無いため常に CSS 初期値 `1.0`
    /// (module doc参照)。
    pub opacity: f32,
    /// `visibility`。同上の理由で常に `true` (= visible)。
    pub visible: bool,
    /// `id` attribute ([`raikiri_traits::Element::id`] 経由)。空文字列は
    /// `None` (trait 既定 contract に一致)。
    pub id: Option<String>,
    /// Taffy が計算した border-box size (module doc の unit 注記どおり
    /// CSS px)。post-layout Element には常に存在するため実質常に `Some` —
    /// fulgur の "fallback to fragment" semantics 用の `Option` shape のみ
    /// 形式的に踏襲する。
    pub layout_size: Option<(Pt, Pt)>,
    /// Overflow-clip scope 内の descendant [`NodeId`]。`overflow` property が
    /// cascade に無いため clip する要素は存在せず、常に空 (「未実装だから
    /// 空」ではなく「clip という概念が cascade に存在しないので対象 0 件」
    /// の正しい反映)。
    pub clip_descendants: Vec<NodeId>,
    /// Opacity-group scope 内の descendant [`NodeId`]。同上の理由で常に空。
    pub opacity_descendants: Vec<NodeId>,
}

impl Default for BlockEntry {
    fn default() -> Self {
        // `#[derive(Default)]` だと `opacity: 0.0` (= 完全透明) /
        // `visible: false` になり、CSS 初期値 (opacity:1, visible) と逆の
        // 意味になってしまうため手書き (advisor 指摘の "silently dead-wrong
        // data" 回避)。
        Self {
            background_color: (0, 0, 0, 0),
            border_widths: (0.0, 0.0, 0.0, 0.0),
            opacity: 1.0,
            visible: true,
            id: None,
            layout_size: None,
            clip_descendants: Vec::new(),
            opacity_descendants: Vec::new(),
        }
    }
}

/// Paragraph (shaped inline text lines) の per-node paint state。
///
/// Fulgur drawables.rs:183-191 の `ParagraphEntry` shape (`lines` /
/// `opacity` / `visible` / `id`) を reference に、raikiri の現状の text model
/// (inline formatting context 未実装、[`raikiri_dom::Node::text_layout`] が
/// Text node 自身に付く — `crates/raikiri-paint/src/walk.rs` module doc
/// 参照) に合わせて minimal field を選定。
///
/// `build_page_scene` (crate::page_scene) が post-layout
/// Text node ごとに construct し `PageDrawables::paragraphs` へ insert する。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ParagraphEntry {
    /// Shaped line 数 (`parley::Layout::lines().count()`)。実 glyph run /
    /// position データ (fulgur の `Vec<ShapedLine>`) は `parley::Layout<()>`
    /// 型そのものであり、raikiri-style/parley に依存しない field 型方針
    /// (module doc参照) の対象外のため保持しない — paint は引き続き
    /// [`raikiri_dom::Node::text_layout`] から直接読む。この field は数だけの
    /// lightweight summary。
    pub line_count: usize,
    /// `opacity`。[`BlockEntry::opacity`] と同じ理由で常に CSS 初期値
    /// `1.0`。
    pub opacity: f32,
    /// `visibility`。[`BlockEntry::visible`] と同じ理由で常に `true`。
    pub visible: bool,
    /// Anchor id (`id="..."` on the inline root)。現状の per-node 粒度は
    /// Text node 自身であり、inline root (親 Element、例: `<p id="...">`) の
    /// id を引くには `build_page_scene` の DFS stack に parent element id を
    /// thread する変更が要る。現時点ではその変更を持ち込まず常に `None`
    /// (documented gap、id-anchored hyperlink 解決は将来の領域)。
    pub id: Option<String>,
}

impl Default for ParagraphEntry {
    fn default() -> Self {
        // BlockEntry と同じ理由 (opacity/visible の CSS 初期値) で手書き。
        Self {
            line_count: 0,
            opacity: 1.0,
            visible: true,
            id: None,
        }
    }
}

/// Raster image (jpg / png / gif etc.) の per-node paint state。
///
/// Fulgur drawables.rs:205-213 の `ImageEntry` shape (`image_data` /
/// `format` / `width` / `height` / `opacity` / `visible`) が reference。
/// raikiri には image decode pipeline が存在しない (`<img>` の byte 取得・
/// format 判定・raster decode のいずれも未実装) ため `image_data` /
/// `format` は保持しない — 型を用意しても常に空/ダミーの construct しか
/// できず「populate 詐称」になるため (task の field-type conservatism 指示)。
/// `width` / `height` / `opacity` / `visible` のみ shape を保持する。
///
/// **未 populate**: `build_page_scene` (crate::page_scene)
/// はこの型の instance を一度も construct しない (現状
/// [`BlockEntry`] / [`ParagraphEntry`] の 2 型のみ populate、他 9 型は
/// 「fulgur shape 由来の field 追加」のみがこれまでの scope)。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ImageEntry {
    /// Display width (module doc の unit 注記どおり CSS px)。
    pub width: Pt,
    /// Display height (CSS px)。
    pub height: Pt,
    /// `opacity`。常に CSS 初期値 `1.0` ([`BlockEntry::opacity`] 参照)。
    pub opacity: f32,
    /// `visibility`。常に `true` ([`BlockEntry::visible`] 参照)。
    pub visible: bool,
}

impl Default for ImageEntry {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

/// SVG image の per-node paint state。
///
/// Fulgur drawables.rs:225-232 の `SvgEntry` shape (`tree` / `width` /
/// `height` / `opacity` / `visible`) が reference。raikiri は SVG parse /
/// render pipeline (`usvg` 相当) を持たないため `tree` は保持しない (理由は
/// [`ImageEntry`] doc参照)。
///
/// **未 populate**: [`ImageEntry`] と同じ理由で一度も construct されない。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct SvgEntry {
    /// Display width (CSS px)。
    pub width: Pt,
    /// Display height (CSS px)。
    pub height: Pt,
    /// `opacity`。常に CSS 初期値 `1.0`。
    pub opacity: f32,
    /// `visibility`。常に `true`。
    pub visible: bool,
}

impl Default for SvgEntry {
    fn default() -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

/// Table (outer frame の background / border) の per-node paint state。
///
/// Fulgur drawables.rs:243-257 の `TableEntry` shape (`style` / `opacity` /
/// `visible` / `id` / `layout_size` / `width` / `cached_height` /
/// `clip_descendants`) が reference。field 型の選定は [`BlockEntry`] と同じ
/// 方針 (`style` を flatten、opacity/visible は CSS 初期値、
/// clip_descendants は overflow property 不在のため常に空)。
///
/// **未 populate**: raikiri-dom に table-specific layout algorithm が無く
/// (`<table>` は他 Element と同じ generic block layout を通る)、
/// `build_page_scene` は `<table>` tag を特別扱いしていない — 常に
/// [`BlockEntry`] として `block_styles` に入る。本 struct は fulgur shape に
/// 合わせた field を用意するのみで、実 table layout が着地するまで一度も
/// construct されない。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TableEntry {
    /// `background-color` (r, g, b, a)。[`BlockEntry::background_color`]
    /// 参照。
    pub background_color: (u8, u8, u8, u8),
    /// Border width 4 side。[`BlockEntry::border_widths`] 参照。
    pub border_widths: (Pt, Pt, Pt, Pt),
    /// `opacity`。常に CSS 初期値 `1.0`。
    pub opacity: f32,
    /// `visibility`。常に `true`。
    pub visible: bool,
    /// `id` attribute。
    pub id: Option<String>,
    /// Taffy border-box size (CSS px)。[`BlockEntry::layout_size`] 参照。
    pub layout_size: Option<(Pt, Pt)>,
    /// Table 全体幅。fulgur は multi-page header 継続幅算出に使うが raikiri
    /// は multi-page table 未対応 (single page 前提)。
    pub width: Pt,
    /// Table 全体高さ (fulgur "cached" — raikiri では毎回 layout から取れる
    /// ため cache という意味は無いが shape を揃えるため同名を保持)。
    pub cached_height: Pt,
    /// Overflow-clip descendant。[`BlockEntry::clip_descendants`] と同じ
    /// 理由で常に空。
    pub clip_descendants: Vec<NodeId>,
}

impl Default for TableEntry {
    fn default() -> Self {
        Self {
            background_color: (0, 0, 0, 0),
            border_widths: (0.0, 0.0, 0.0, 0.0),
            opacity: 1.0,
            visible: true,
            id: None,
            layout_size: None,
            width: 0.0,
            cached_height: 0.0,
            clip_descendants: Vec::new(),
        }
    }
}

/// List item marker (text / image / none) の per-node paint state。
///
/// Fulgur drawables.rs:271-299 の `ListItemEntry` / `ListItemMarker` shape
/// (`marker` / `marker_line_height` / `opacity` / `visible`) が reference。
/// `marker` (Text/Image/None の 3-variant enum、shaped line や image data を
/// 抱える) は raikiri に list-item marker rendering が無いため保持しない —
/// 新 enum を興すと item 4 の topology 判断前に型を固定してしまう (module
/// doc の field type 方針参照)。
///
/// **未 populate**: raikiri-dom は `<li>` の marker box を生成しない
/// (generic block layout のみ)。`build_page_scene` は `<li>` を特別扱いせず
/// 常に [`BlockEntry`] として扱う。marker layout が着地するまで一度も
/// construct されない。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ListItemEntry {
    /// Marker の line-height (image marker の垂直中央揃えに使う想定)。
    pub marker_line_height: Pt,
    /// `opacity`。常に CSS 初期値 `1.0`。
    pub opacity: f32,
    /// `visibility`。常に `true`。
    pub visible: bool,
}

impl Default for ListItemEntry {
    fn default() -> Self {
        Self {
            marker_line_height: 0.0,
            opacity: 1.0,
            visible: true,
        }
    }
}

/// CSS transform (matrix + origin + descendants scope) の per-node state。
///
/// Fulgur drawables.rs:392-405 の `TransformEntry` shape (`matrix` /
/// `origin` / `descendants`) が reference。`raikiri_style::ComputedValues`
/// に `transform` property が無いため常に identity/空で、一度も construct
/// されない (transform CSS support 未着手)。
///
/// `matrix` は 2D affine を `[a, b, c, d, e, f]` (row-major、fulgur の
/// `Affine2D` 相当) の生 array で保持する — 新型を興さず module doc の
/// field type 方針を守る。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct TransformEntry {
    /// 2D affine matrix `[a, b, c, d, e, f]`。
    pub matrix: [f32; 6],
    /// Transform origin。
    pub origin: (Pt, Pt),
    /// Transform scope 内の descendant [`NodeId`]。`transform` property が
    /// cascade に無いため常に空。
    pub descendants: Vec<NodeId>,
}

impl Default for TransformEntry {
    fn default() -> Self {
        // `#[derive(Default)]` の全 0 matrix は非可逆な退化 affine になる
        // ため、identity (`[1,0,0,1,0,0]`) を明示する手書き Default。
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            origin: (0.0, 0.0),
            descendants: Vec::new(),
        }
    }
}

/// Multicol の column-rule 描画 spec と per-column-group geometry。
///
/// Fulgur drawables.rs:316-340 の `MulticolRuleEntry` / `ColumnRuleGeometry`
/// shape が reference。`raikiri_style::ComputedValues` に `column-count` /
/// `column-rule` 等 multicol property が一切無いため、fulgur の
/// `ColumnRuleSpec` / `Vec<ColumnRuleGeometry>` に相当する型は興さず
/// (module doc の field type 方針)、shape を示す最小 field のみ置く。常に
/// 空で construct されない。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct MulticolRuleEntry {
    /// Balance 対象の column 数。multicol property が cascade に無いため
    /// 常に `0`。
    pub column_count: u32,
}

/// Bookmark anchor (PDF bookmark tree の source) の per-node state。
///
/// Fulgur drawables.rs:409-413 の `BookmarkAnchorEntry` shape (`level` /
/// `label`) が reference。raikiri は PDF bookmark tree 生成の author 指定
/// 手段 (どの element を bookmark にするかを決める CSS/API) を持たないため
/// 常に construct されない — field 自体は primitive (`u8` / `String`) なので
/// そのまま採用する。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct BookmarkAnchorEntry {
    /// Bookmark tree 上の深さ (0-based)。
    pub level: u8,
    /// Bookmark label 文字列。
    pub label: String,
}

/// Paragraph 内 glyph run 上に張る hyperlink target span。
///
/// Fulgur drawables.rs:418-419 の `LinkSpanEntry` shape が reference —
/// fulgur 自体もこの entry を "PR 3 target" (未実装、unit struct のまま) と
/// しており、raikiri 側で先行して field を追加する reference 元が無い。
/// raikiri でも同様に空のまま保つ (fulgur の実装が先行した時点で shape を
/// 再確認する)。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct LinkSpanEntry {}

/// Tagged-PDF semantics (tag / parent / alt_text 等) の per-node state。
///
/// Fulgur tagging.rs:57 の `SemanticEntry` shape が reference。raikiri は
/// tagged-PDF 生成 (構造 tag 分類・tag tree 組み立て) を一切実装していない
/// ため常に construct されない。Field は primitive (`Option<String>` /
/// `Option<NodeId>`) のみで shape を示す。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct SemanticEntry {
    /// PDF structure tag 名 (例: `"P"`, `"H1"`)。
    pub tag: Option<String>,
    /// Tag tree 上の親 [`NodeId`]。
    pub parent: Option<NodeId>,
    /// Alt text (image 等の代替テキスト)。
    pub alt_text: Option<String>,
}
