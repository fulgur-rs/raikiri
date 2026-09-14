//! Arena node type for raikiri-dom's Document.
//!
//! Node は Element / Text / Document (root) の 3 種を union で表現する
//! flat struct。fields は crate-private (raikiri-dom 内部のみ mutate)、
//! 外部 Consumer は `raikiri_traits::Dom / Node / Element` trait 経由で
//! 参照する。

use smol_str::SmolStr;
use taffy::{Cache, Layout, Style};

use raikiri_style::property::{BorderCollapseValue, DisplayValue, TableLayoutValue};
use raikiri_traits::NodeKind;

bitflags::bitflags! {
    /// Node に付随する per-node boolean 属性。blitz `NodeFlags` と **raw bit
    /// 値まで完全一致**。
    ///
    /// blitz reference (blitz-dom/src/node/node.rs:50-58):
    /// ```text
    /// const IS_INLINE_ROOT = 0b00000001;   // = 1 << 0
    /// const IS_TABLE_ROOT  = 0b00000010;   // = 1 << 1
    /// const IS_IN_DOCUMENT = 0b00000100;   // = 1 << 2
    /// ```
    ///
    /// `IS_IN_DOCUMENT` と `IS_INLINE_ROOT` は使用中。`IS_TABLE_ROOT` (将来の
    /// table formatting root 用) は blitz と同 bit 位置で予約定義するのみ
    /// (今は誰も set/clear しないが、bit 位置を確保することで raw-bit 変換
    /// `NodeFlags::from_bits(blitz_flags.bits())` が将来の blitz-compat 変換で
    /// 正しく動く)。
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct NodeFlags: u32 {
        /// Inline formatting context root。[`mod@crate::layout`] の
        /// `establish_minimal_line_boxes` が、その node が同関数の
        /// minimal-line-box 条件を満たす block container かどうかで
        /// set/clear する (`layout_single_page` 呼び出し毎に再計算、set
        /// のままにも clear のままにもなる — write-only ではない)。bit
        /// 位置は blitz と同じ。
        const IS_INLINE_ROOT = 1 << 0;
        /// Table formatting context root。将来使用予定 (blitz と同 bit 位置)。
        const IS_TABLE_ROOT = 1 << 1;
        /// この Node が flat tree に含まれるか。`<template>` element の子孫は
        /// clear、Document root から flat-tree-parent 経由で到達可能な node は
        /// set。将来 shadow DOM / slot の "shadow-including tree" 意味論を
        /// 追加する場合、slot 割当てられない host 直下や shadow root 外の
        /// light-DOM 子孫も同 bit で表現する予定。
        ///
        /// 維持タイミング:
        /// - parse: `raikiri-html::sink::finish` の `mark_in_document_flags`
        ///   phase で single-pass DFS が set/clear
        /// - mutation runtime (将来): mutator の `process_added_subtree` /
        ///   `process_removed_subtree` 相当が set/unset
        const IS_IN_DOCUMENT = 1 << 2;
    }
}

/// Element attribute (null namespace only, for now)。
///
/// namespaced attribute (`xlink:href` on SVG 等) は将来に defer。html5ever の
/// `Attribute.name.ns` が null namespace (`ns!("")`) の attr のみここに格納する。
/// `raikiri-html::sink::finish` が side-table から wire する。
#[derive(Debug, Clone)]
pub(crate) struct Attr {
    pub(crate) local: SmolStr,
    pub(crate) value: SmolStr,
}

/// NodeData: kind 固有 field を集約した tagged union。
///
/// blitz `blitz-dom::node::node::NodeData` に対応する shape。将来の
/// blitz-compat で nominal 変換 (`match data { NodeData::Element(e) => BlitzElement { ... }, ... }`)
/// できるように field 名を揃える。`Element` variant のみ `Box` で indirection
/// を挟むのは blitz と同じ選択 (Element の field 数が多く、Text / Document 側の
/// サイズに Element を引きずられさせないため)。
///
/// 注意: この Box は `size_of::<NodeData>()` を小さく抑えるものではない —
/// `TextData` が `parley::Layout<()>` を直接持つため `Element` variant
/// (Box 経由でポインタ幅) より大きく、結局 enum 全体は `TextData` のサイズで
/// 決まる (`clippy::large_enum_variant` が発火するのはこのため)。それでも
/// `Text` を Box しないのは意図した trade-off: `text_layout()` は paint hot
/// path から呼ばれるため、追加の indirection を持ち込みたくない。
#[derive(Debug)]
#[non_exhaustive]
#[allow(
    clippy::large_enum_variant,
    reason = "Element only is boxed by design (blitz-compat shape, see doc comment); \
              Text carries parley::Layout<()> inline to avoid extra indirection on \
              the paint hot path"
)]
pub enum NodeData {
    /// HTML / XML element (tag_name + attributes + namespace + inline_style +
    /// template_contents slot を持つ)。
    Element(Box<ElementData>),
    /// Character data node。
    Text(TextData),
    /// Document root (arena index 0 の virtual node)。
    Document,
    /// HTML / XML comment node (`<!-- ... -->`)。以前は `"#comment"` tag な
    /// Element として保持後 sink.finish() で strip していたが、恒久 variant
    /// 化した。character data を保持するが Element ではない
    /// (`kind() == NodeKind::Comment`、`as_element() == None`)。
    /// `mark_in_document_flags` が明示的に `IS_IN_DOCUMENT` bit を clear するため、
    /// cascade / paint / layout / stylesheet extraction の全 traversal は
    /// `is_in_document()` gate で自動的に skip する (defense-in-depth: 追加の
    /// `matches!(kind, Comment)` gate を traversal 側に散らさない)。
    Comment(SmolStr),
    /// Processing instruction node (`<?target data?>`)。target + data の
    /// pair を保持。同上、`IS_IN_DOCUMENT` bit を clear
    /// することで traversal から自然に消える。
    ProcessingInstruction {
        /// PI target (`<?xml-stylesheet ...?>` の `xml-stylesheet` 部分)。
        target: SmolStr,
        /// PI data (`<?xml-stylesheet href="..."?>` の `href="..."` 部分)。
        data: SmolStr,
    },
    /// Document fragment root (`<template>` contents 等の detached subtree の
    /// 仮想 root)。以前 `"#document-fragment"` pseudo-tag な Element として
    /// 実装していた shape を恒久 variant 化したもの。
    /// `kind() == NodeKind::DocumentFragment`、
    /// `as_element() == None`。Document root からは reachable でないため
    /// `mark_in_document_flags` は自然に `IS_IN_DOCUMENT` bit を clear する。
    DocumentFragment,
}

impl NodeData {
    /// Element variant を crate-private に mut borrow (Document setter 用)。
    #[inline]
    pub(crate) fn as_element_mut(&mut self) -> Option<&mut ElementData> {
        match self {
            NodeData::Element(e) => Some(e.as_mut()),
            _ => None,
        }
    }

    /// Text variant を crate-private に mut borrow (layout::preshape_text 用)。
    #[inline]
    pub(crate) fn as_text_mut(&mut self) -> Option<&mut TextData> {
        match self {
            NodeData::Text(t) => Some(t),
            _ => None,
        }
    }
}

/// Element-only data。blitz `ElementData` に対応。
///
/// `template_contents` は `<template>` element の contents fragment root への
/// arena index を保持する slot。live 化され、
/// raikiri-html sink が `create_element` の `ElementFlags::template=true` を
/// 観測した時 [`crate::Document::allocate_template_fragment_root`] 経由で
/// populate する。詳細は field 側の doc comment を参照。
#[derive(Debug)]
pub struct ElementData {
    /// HTML / XML tag name (例: `"p"`, `"div"`)。html5ever の QualName.local から
    /// SmolStr に写し取る。
    pub(crate) tag_name: SmolStr,
    /// HTML `style="..."` attribute の生 string (kind == Element 時のみ populate、
    /// 空文字列 `style=""` は Element trait contract 上 `None` として view 化
    /// されるが、storage はここでは正規化せず raw 値を持つ)。
    pub(crate) inline_style: Option<SmolStr>,
    /// Element namespace URI (non-HTML の場合のみ `Some`、HTML default は
    /// `None` を fast path とする)。例: `Some("http://www.w3.org/2000/svg")`。
    pub(crate) namespace: Option<SmolStr>,
    /// null-namespace attribute list (順序保持、cascade tie-breaking で使う想定)。
    /// `style` attribute は [`ElementData::inline_style`] に分離済のためここには
    /// 含めない。
    pub(crate) attributes: Vec<Attr>,
    /// `<template>` element の contents fragment root への arena index
    /// (fragment root shape を `NodeData::DocumentFragment` variant として
    /// 実装)。
    ///
    /// raikiri-html sink が `create_element` で html5ever の
    /// `ElementFlags::template = true` を観測した時、[`crate::Document::allocate_template_fragment_root`]
    /// で detached な [`NodeData::DocumentFragment`] node を allocate し、その arena
    /// index をここに格納する。`TreeSink::get_template_contents` はこの slot
    /// を返し、以降 html5ever は template contents を fragment root の子として
    /// append する (template element 自身の children は空のまま)。
    ///
    /// blitz `blitz-dom::node::element::ElementData::template_contents` と
    /// 同名・同 shape。sink が populate しなかった (template 判定を経ずに
    /// 直接組み立てる test / 将来の manual construction) 場合は `None` のまま。
    pub(crate) template_contents: Option<usize>,
}

/// Text-only data。blitz `TextNodeData` (nominally) に対応。
#[derive(Debug)]
pub struct TextData {
    /// Character data。
    pub(crate) text_content: SmolStr,
    /// Text node の pre-shaped parley Layout。
    ///
    /// - Populated by [`crate::layout::preshape_text`]
    /// - Consumed by taffy leaf measure closure (intrinsic size) と paint
    ///   (glyph 位置)
    /// - Brush type `()` は意図的な choice: color / decoration は持たせない
    /// - Invalidation: `layout_single_page` 呼び出し毎に全 None にクリア + 再走
    pub text_layout: Option<parley::Layout<()>>,
}

/// Arena node (NodeData tagged union として実装)。
///
/// paint / cascade / layout に必要な kind 非依存の field (children /
/// unrounded_layout) は Node に残し、kind 固有 field は [`NodeData`] variant
/// に集約する。かつて pub 化していた 5 field のうち `kind` /
/// `tag_name` / `text_layout` は accessor method 経由に移行 (`node.kind()` /
/// `node.tag_name()` / `node.text_layout()`)、`children` / `unrounded_layout`
/// は pub field 継続。external contract は Node/Element field access 0 件
/// なので無影響、raikiri-dom 内部 pub_surface pin のみ accessor 経由に再 pin。
#[derive(Debug)]
pub struct Node {
    /// Taffy layout style。
    pub(crate) style: Style,
    /// Computed `display` for table dispatch. Bridged to `style.display` for Block/Flex/Grid/None
    /// but retains the full `DisplayValue` (including `Table` family) so table engine routing
    /// does not need post-bridge `style.display` which collapses table variants to `Block`.
    pub(crate) display: DisplayValue,
    /// Computed `table-layout` for the table engine. No `taffy::Style`
    /// counterpart exists (taffy 0.12 has no table layout), so the keyword
    /// is carried here alongside [`Self::display`] and read by
    /// [`crate::layout::table`] (`Node::display` bridge precedent).
    pub(crate) table_layout: TableLayoutValue,
    /// Computed `border-collapse` for the table engine. Same `Node`-side
    /// carrying shape as [`Self::table_layout`] above (`border-collapse`
    /// is inherited — the value here is already the post-inheritance
    /// computed value, seed handling lives in raikiri-style).
    pub(crate) border_collapse: BorderCollapseValue,
    /// Child arena indices (`Document::nodes` の usize)。
    pub children: Vec<usize>,
    /// Taffy layout cache (per-node)。
    pub(crate) cache: Cache,
    /// Taffy layout 結果 (compute_root_layout が populate)。
    ///
    /// # 値域契約: 全 `f32` field は有限・`[-1e7, 1e7]` に飽和
    ///
    /// `location.{x,y}` / `size.{width,height}` / `content_size.{width,height}`
    /// / `scrollbar_size.{width,height}` / `border.{left,right,top,bottom}` /
    /// `padding.{left,right,top,bottom}` / `margin.{left,right,top,bottom}` は
    /// 常に **有限**であり、`[-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE]` (=
    /// `[-1e7, 1e7]`、`raikiri_dom::layout` module-private 定数
    /// `MAX_TAFFY_MAGNITUDE`) に対称飽和している。生成過程で `NaN` になった
    /// 値は `0.0` に fallback 済み。`±Inf` を
    /// 含む taffy の生出力がこの field に書き込まれることはない。`order`
    /// (`u32`) はこの契約の対象外 (非有限になり得ない型)。
    ///
    /// 保証するのは finiteness と magnitude 上限のみで、box model の包含関係
    /// (CSS Box 3 の content ⊆ padding ⊆ border) は保存しない — field ごとに
    /// 独立に clamp するため、`size.width` と `padding.{left,right}` が
    /// 同時に飽和すると `size.width - padding.left - padding.right` が負に
    /// なりうる。
    ///
    /// この契約は [`crate::layout::sanitize_taffy_layout`] が
    /// `<Document as taffy::LayoutPartialTree>::set_unrounded_layout`
    /// (`taffy_impl.rs`) という taffy → arena 書き込みの唯一の choke point
    /// で強制する構造的 invariant であり、呼び忘れで破れることがない
    /// (詳細・証拠は [`crate::layout::sanitize_taffy_layout`] の doc)。
    /// この field 自体は `pub` だが、`Node` を外部から `&mut` で得る公開
    /// API は存在しない (`Document::nodes` は `pub(crate)`、[`Node::new_document`]
    /// 等の constructor もすべて `pub(crate)`、[`crate::Document::get_node`]
    /// は `&Node` のみ返す) ため、crate 外からこの choke point を経由せず
    /// 直接書き込む経路は無い。
    ///
    /// この field は `pub` であり [`crate::Document::get_node`] 経由で crate
    /// 外からも直接観測できる。raikiri-paint の walker
    /// (`crates/raikiri-paint/src/walk.rs`、`location.x`/`y` を直接積算) と
    /// raikiri crate の page-scene 抽出 (`crates/raikiri/src/page_scene.rs`、
    /// `location.{x,y}` / `size.{width,height}` を直接 `Fragment` へ写す) の
    /// 両方が、この field を再検証せず直接読む — 本契約が dom → paint 境界で
    /// 非有限幾何から consumer を守る唯一の防壁である。type / field shape /
    /// access pattern はこの契約と無関係で不変 — 本 doc は dom → paint
    /// 境界の **観測可能な値の集合**を定める behavioral contract を明文化
    /// するのみ。
    pub unrounded_layout: Layout,
    /// Per-node metadata bits (IS_IN_DOCUMENT etc.)。crate-private mutation。
    pub(crate) flags: NodeFlags,
    /// Node kind + kind 固有 field (tagged union)。
    pub(crate) data: NodeData,
}

impl Node {
    /// Document root node (arena index 0 用) を構築する。
    pub(crate) fn new_document() -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            border_collapse: BorderCollapseValue::Separate,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Document,
        }
    }

    /// Element node を tag name / style / inline_style と共に構築する。
    /// `namespace` / `attributes` / `template_contents` は初期空/None で、raikiri-html
    /// sink が finish 時に [`crate::Document::set_element_namespace`] /
    /// [`crate::Document::set_element_attributes`] で populate する。
    /// `template_contents` は `<template>` element のみ、sink の `create_element`
    /// が [`crate::Document::allocate_template_fragment_root`] 経由で eager
    /// populate する。
    pub(crate) fn new_element(tag: SmolStr, style: Style, inline_style: Option<SmolStr>) -> Self {
        Self {
            style,
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            border_collapse: BorderCollapseValue::Separate,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Element(Box::new(ElementData {
                tag_name: tag,
                inline_style,
                namespace: None,
                attributes: Vec::new(),
                template_contents: None,
            })),
        }
    }

    /// Text node を character data と共に構築する。
    pub(crate) fn new_text(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            border_collapse: BorderCollapseValue::Separate,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Text(TextData {
                text_content: text,
                text_layout: None,
            }),
        }
    }

    /// Comment node を character data と共に構築する。
    /// `IS_IN_DOCUMENT` bit は default true で作られるが、
    /// [`crate::Document::mark_in_document_flags`] が step 2 の DFS で必ず
    /// clear する契約 (blitz-compat: comment は flat-tree 上不可視、layout /
    /// paint / cascade は `is_in_document()` gate で自動 skip)。
    pub(crate) fn new_comment(text: SmolStr) -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            border_collapse: BorderCollapseValue::Separate,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::Comment(text),
        }
    }

    /// Processing instruction node を target + data と共に構築する。
    /// Comment と同じく `IS_IN_DOCUMENT` bit は
    /// [`crate::Document::mark_in_document_flags`] で clear される。
    pub(crate) fn new_processing_instruction(target: SmolStr, data: SmolStr) -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            border_collapse: BorderCollapseValue::Separate,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::ProcessingInstruction { target, data },
        }
    }

    /// Document fragment root を構築する。detached 状態で
    /// arena に置くのが典型 (parent なし)、`<template>` contents の virtual
    /// root として使用する。`IS_IN_DOCUMENT` bit は default true で作られるが、
    /// Document root から reachable でないため `mark_in_document_flags` で
    /// clear される。
    pub(crate) fn new_document_fragment() -> Self {
        Self {
            style: Style::default(),
            display: DisplayValue::Inline,
            table_layout: TableLayoutValue::Auto,
            border_collapse: BorderCollapseValue::Separate,
            children: Vec::new(),
            cache: Cache::new(),
            unrounded_layout: Layout::with_order(0),
            flags: NodeFlags::IS_IN_DOCUMENT,
            data: NodeData::DocumentFragment,
        }
    }

    // ─── inherent accessor methods ─────────────────

    /// この Node の [`NodeKind`] を返す。
    ///
    /// 旧 `pub kind: NodeKind` field の accessor 版。
    /// 呼び出し側は `node.kind` → `node.kind()` の syntax 変更のみ。
    #[inline]
    pub fn kind(&self) -> NodeKind {
        match &self.data {
            NodeData::Element(_) => NodeKind::Element,
            NodeData::Text(_) => NodeKind::Text,
            NodeData::Document => NodeKind::Document,
            NodeData::Comment(_) => NodeKind::Comment,
            NodeData::ProcessingInstruction { .. } => NodeKind::ProcessingInstruction,
            NodeData::DocumentFragment => NodeKind::DocumentFragment,
        }
    }

    /// Element の場合 tag_name を、それ以外は `None` を返す。
    ///
    /// 旧 `pub tag_name: Option<SmolStr>` field の accessor 版。
    /// `Option<&str>` に射影する
    /// (SmolStr の内部 view で Copy 相当のコスト)。
    #[inline]
    pub fn tag_name(&self) -> Option<&str> {
        match &self.data {
            NodeData::Element(e) => Some(e.tag_name.as_str()),
            _ => None,
        }
    }

    /// Text の場合 text_layout を、それ以外は `None` を返す。
    ///
    /// 旧 `pub text_layout: Option<parley::Layout<()>>` field の accessor 版。
    /// paint hot path から呼ばれるため `#[inline]`。
    #[inline]
    pub fn text_layout(&self) -> Option<&parley::Layout<()>> {
        match &self.data {
            NodeData::Text(t) => t.text_layout.as_ref(),
            _ => None,
        }
    }

    /// このノードの `taffy::Style.display == Display::None` を返す。
    ///
    /// paint 段で display:none subtree を skip する目的の predicate。size 0
    /// による代理判定は overflow: visible の legitimate な zero-size 要素を
    /// silent drop するため誤り。style field
    /// は crate-private のまま維持し、paint に必要な最小の boolean 述語のみ
    /// pub で公開する (gradual exposure)。
    #[inline]
    pub fn is_display_none(&self) -> bool {
        self.style.display == taffy::Display::None
    }

    /// この Node が flat tree の一員かを返す。
    #[inline]
    pub fn is_in_document(&self) -> bool {
        self.flags.contains(NodeFlags::IS_IN_DOCUMENT)
    }

    /// `<template>` element の contents fragment root への arena index。
    /// Element 以外 / fragment root 未 wire の場合は `None`。
    ///
    /// html5ever `TreeSink::get_template_contents` 実装が sink 経由で消費する。
    /// blitz `blitz-dom::node::element::ElementData::template_contents` field
    /// と同等の read-side accessor。
    #[inline]
    pub fn template_contents(&self) -> Option<usize> {
        match &self.data {
            NodeData::Element(e) => e.template_contents,
            _ => None,
        }
    }

    /// HTML namespace の "non-rendered" element (metadata content / raw text
    /// container / ruby parenthesis fallback) を判定する。paint 段で subtree
    /// ごと skip する gate 用。
    ///
    /// 対象 (HTML LS §15.3.1 "Hidden elements"、
    /// <https://html.spec.whatwg.org/multipage/rendering.html#hidden-elements>):
    /// `<head>`, `<title>`, `<meta>`, `<link>`, `<base>`, `<noscript>`,
    /// `<script>`, `<style>`, `<template>`、
    /// `<datalist>` (§4.10.8、
    /// <https://html.spec.whatwg.org/multipage/form-elements.html#the-datalist-element>)、
    /// `<noembed>` / `<noframes>` (§13.2 RAWTEXT parsing)、`<rp>` (§4.5.12
    /// ruby parenthesis fallback、§15.3.1 hidden-elements rule 直下で
    /// `display: none` — §15.3.4 "Phrasing content" の ruby CSS も
    /// `ruby { display: ruby }` / `rt { display: ruby-text }` のみで rp を
    /// 可視化しないため、ruby-supporting UA 上でも rp は hidden のまま)。
    /// namespace が
    /// HTML default (`Node.namespace == None`) or 明示 xhtml
    /// (`"http://www.w3.org/1999/xhtml"`) の場合のみ true、SVG / MathML
    /// namespace の同名要素は false (SVG `<style>` / `<script>` は SVG 側
    /// rendering 責務、HTML paint filter の対象外)。
    ///
    /// §15.3.1 hidden-elements rule には `<area>` / `<basefont>` / `<param>`
    /// も列挙されているが、これらは通常 child content を持たない (`<area>` は
    /// void、`<basefont>` は obsolete-void、`<param>` は object 内の attribute
    /// 相当) ため content leak 経路が存在せず本 predicate では扱わない。
    ///
    /// # 動機
    ///
    /// UA CSS (`style { display: none }` etc、HTML LS §15.3.1) による hide は
    /// author / user CSS で override 可能なため、attacker-controlled HTML +
    /// override CSS で `<style>` `<script>` 内 text が rendered artifact に
    /// 混入する security surface が残る。paint 側で cascade-independent に
    /// gate することで defense-in-depth 保証する (datalist / noembed /
    /// noframes / rp は §15.3.1 完全化のため後から追加)。
    ///
    /// # Non-goals
    ///
    /// - `[hidden]` attribute / `inert` attribute の filter は本 predicate
    ///   scope 外 (将来 `is_display_none()` 側の cascade 経路で扱う予定)
    /// - `<template>` は既に `is_in_document() == false` の gate で
    ///   redundant に skip されるが、defense-in-depth で本 predicate にも
    ///   含める (両 gate 独立に fail-close する)
    #[inline]
    pub fn is_non_rendered_html_element(&self) -> bool {
        let NodeData::Element(e) = &self.data else {
            return false;
        };
        // namespace check: HTML default (None) or explicit xhtml のみ対象。
        // SVG / MathML の同名 element は SVG rendering 側で処理する。
        match e.namespace.as_deref() {
            None => {}
            Some("http://www.w3.org/1999/xhtml") => {}
            _ => return false,
        }
        // HTML tag name は html5ever が lowercase 化済 (QualName.local)。
        // Ordering: 元からあった arms を先頭、§15.3.1 完全化のために追加した
        // 4 arms を末尾にグループ化 (既存 style を preserve しつつ差分の由来を
        // 明示するため)。
        matches!(
            e.tag_name.as_str(),
            "head"
                | "title"
                | "meta"
                | "link"
                | "base"
                | "noscript"
                | "script"
                | "style"
                | "template"
                // §15.3.1 完全化のため追加:
                | "datalist"
                | "noembed"
                | "noframes"
                | "rp"
        )
    }

    /// [`NodeFlags::IS_IN_DOCUMENT`] bit を明示的に上書きする (crate-private)。
    ///
    /// `Document::mark_in_document_flags` (sink.finish() から
    /// 呼ばれる single-pass DFS) が消費する。
    #[inline]
    pub(crate) fn set_in_document(&mut self, v: bool) {
        self.flags.set(NodeFlags::IS_IN_DOCUMENT, v);
    }
}

#[cfg(test)]
mod flags_tests {
    use super::*;

    #[test]
    fn node_flags_default_is_empty() {
        let f = NodeFlags::default();
        assert!(!f.contains(NodeFlags::IS_IN_DOCUMENT));
    }

    #[test]
    fn node_new_document_has_is_in_document_set_by_default() {
        // Node::new_document() は Document root 用、常に flat tree の一員。
        let n = Node::new_document();
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_element_has_is_in_document_set_by_default() {
        let n = Node::new_element(SmolStr::new("p"), taffy::Style::default(), None);
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_text_has_is_in_document_set_by_default() {
        let n = Node::new_text(SmolStr::new("hi"));
        assert!(n.is_in_document());
    }

    #[test]
    fn set_in_document_toggles_bit() {
        let mut n = Node::new_document();
        n.set_in_document(false);
        assert!(!n.is_in_document());
        n.set_in_document(true);
        assert!(n.is_in_document());
    }

    #[test]
    fn node_new_comment_kind_and_default_flag_state() {
        // Comment constructor は kind = NodeKind::Comment、
        // IS_IN_DOCUMENT は default true (mark_in_document_flags で後段 clear
        // される optimistic 初期値、Element / Text と同じ posture)。
        let n = Node::new_comment(SmolStr::new("hello"));
        assert_eq!(n.kind(), NodeKind::Comment);
        assert!(
            n.is_in_document(),
            "constructor default follows Element/Text pattern"
        );
        // tag_name accessor は Comment に対して None を返す (Element でない)。
        assert_eq!(n.tag_name(), None);
        // text_layout accessor は Comment に対して None を返す (Text でない)。
        assert!(n.text_layout().is_none());
    }

    #[test]
    fn node_new_processing_instruction_kind_and_default_flag_state() {
        let n = Node::new_processing_instruction(
            SmolStr::new("xml-stylesheet"),
            SmolStr::new("href='x.css'"),
        );
        assert_eq!(n.kind(), NodeKind::ProcessingInstruction);
        assert!(n.is_in_document());
        assert_eq!(n.tag_name(), None);
        assert!(n.text_layout().is_none());
    }

    #[test]
    fn node_new_document_fragment_kind_and_default_flag_state() {
        let n = Node::new_document_fragment();
        assert_eq!(n.kind(), NodeKind::DocumentFragment);
        // Fragment root は使用時 detached 状態で作られるため、mark 後に
        // false に落ちる。constructor 単体では default true。
        assert!(n.is_in_document());
        assert_eq!(n.tag_name(), None);
        assert!(n.text_layout().is_none());
    }

    #[test]
    fn node_flags_bit_values_match_blitz_raw() {
        // Regression pin。blitz `NodeFlags`
        // (blitz-dom/src/node/node.rs:50-58) と raw bit 値まで一致:
        //   IS_INLINE_ROOT = 0b001, IS_TABLE_ROOT = 0b010, IS_IN_DOCUMENT = 0b100
        //
        // これにより将来の blitz-compat の変換が `NodeFlags::from_bits(x)` の
        // trivial cast で成立する。将来 bit を追加する際は blitz と同 bit
        // 位置に揃えること。
        assert_eq!(NodeFlags::IS_INLINE_ROOT.bits(), 0b001);
        assert_eq!(NodeFlags::IS_TABLE_ROOT.bits(), 0b010);
        assert_eq!(NodeFlags::IS_IN_DOCUMENT.bits(), 0b100);
    }
}

#[cfg(test)]
mod is_non_rendered_html_element_tests {
    //! `<template>` 経路 test は
    //! `is_in_document()` が先に発火するため、predicate 自体の direct
    //! coverage が薄い。DOM predicate を builder + namespace mutation で
    //! namespace 分岐まで含めて直接 pin する。

    use super::*;

    fn html_element(tag: &str) -> Node {
        Node::new_element(SmolStr::new(tag), taffy::Style::default(), None)
    }

    fn set_ns(n: &mut Node, ns: &str) {
        if let NodeData::Element(e) = &mut n.data {
            e.namespace = Some(SmolStr::new(ns));
        }
    }

    /// d9y.5 の 9 element + s8w で追加した §15.3.1 完全化 4 element
    /// (datalist / noembed / noframes / rp)。tag 列挙は
    /// `Node::is_non_rendered_html_element` の match arms と 1:1 対応。
    const SKIP_SET_TAGS: &[&str] = &[
        // d9y.5 original:
        "head", "title", "meta", "link", "base", "noscript", "script", "style", "template",
        // s8w additions (§15.3.1 完全化):
        "datalist", "noembed", "noframes", "rp",
    ];

    #[test]
    fn predicate_true_for_html_default_namespace_skip_set() {
        for tag in SKIP_SET_TAGS {
            let n = html_element(tag);
            assert!(
                n.is_non_rendered_html_element(),
                "{tag} in HTML default namespace (None) must be non-rendered"
            );
        }
    }

    #[test]
    fn predicate_true_for_explicit_xhtml_namespace_skip_set() {
        for tag in SKIP_SET_TAGS {
            let mut n = html_element(tag);
            set_ns(&mut n, "http://www.w3.org/1999/xhtml");
            assert!(
                n.is_non_rendered_html_element(),
                "{tag} with explicit xhtml namespace must be non-rendered"
            );
        }
    }

    #[test]
    fn predicate_false_for_svg_namespace_same_named_elements() {
        // SVG <title>, <style>, <script> は rendered / effective in SVG context。
        // predicate は HTML namespace のみ filter するのが契約 (paint 側は
        // SVG rendering を将来別 pipeline で扱う)。
        for tag in ["title", "style", "script"] {
            let mut n = html_element(tag);
            set_ns(&mut n, "http://www.w3.org/2000/svg");
            assert!(
                !n.is_non_rendered_html_element(),
                "SVG {tag} must NOT be filtered — SVG rendering owns these"
            );
        }
    }

    #[test]
    fn predicate_false_for_mathml_namespace_same_named_elements() {
        for tag in ["style", "script"] {
            let mut n = html_element(tag);
            set_ns(&mut n, "http://www.w3.org/1998/Math/MathML");
            assert!(
                !n.is_non_rendered_html_element(),
                "MathML {tag} must NOT be filtered"
            );
        }
    }

    #[test]
    fn predicate_false_for_normal_html_elements() {
        for tag in [
            "p", "div", "span", "h1", "a", "body", "html", "img", "table",
        ] {
            let n = html_element(tag);
            assert!(
                !n.is_non_rendered_html_element(),
                "{tag} is rendered content — predicate must return false"
            );
        }
    }

    #[test]
    fn predicate_false_for_non_element_nodes() {
        // Text / Document node は Element でないので false。
        let text = Node::new_text(SmolStr::new("hi"));
        assert!(!text.is_non_rendered_html_element());
        let doc = Node::new_document();
        assert!(!doc.is_non_rendered_html_element());
    }
}
