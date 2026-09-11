//! Per-page immutable snapshot consumed by raikiri downstream consumers
//! (fulgur PDF translator が想定 primary consumer).
//!
//! # Consumer contract
//!
//! [`PageScene`] は raikiri 内部 pipeline (fragmentation + reflow +
//! LayoutBuffer) を通り抜けた後の **1 page 分の immutable snapshot**。
//! Consumer 側は forward iterate + batch operation で消費し、reflow /
//! re-fragmentation の concern は持たない (raikiri 内部完結)。
//!
//! # Node identity
//!
//! [`raikiri_traits::NodeId`] を key として page 内で分布する DOM node を
//! 表現する。raikiri crate の Document (umbrella re-export) と PageScene
//! が **同一 NodeId 空間** を共有するため、
//! consumer は 1 個の `NodeId` を Document 側の property lookup と
//! PageScene 側の drawable / fragment lookup に **conversion なしで**
//! 使い回せる (確立済みの design decision)。
//!
//! # Landing scope: pub type surface
//!
//! Pub type surface のみ landing (`#[non_exhaustive]` で future field 追加を
//! semver-non-breaking に保つ)。内部 pipeline から `PageScene` を実際に
//! populate する path は将来の streaming pagination 対応で fill される。
//!
//! # Landing scope: html_to_png の PageScene 経由 refactor
//!
//! 内部 `html_to_png` pipeline を PageScene 経由に refactor。
//! [`build_page_scene`] で post-layout Document から metadata + fragments を
//! 抽出した [`PageScene`] を organize し、[`PageScene::rasterize`] が
//! `raikiri_paint::paint_single_page` + `anyrender_vello_cpu` + PNG encode の
//! byte-identical triple を verbatim reuse する。
//!
//! この段階では `drawables` field を empty のままにした (glyph run /
//! position 情報を持たない empty entries に populate すると emission order
//! が変わり byte-identical が破れるため、意図的な hard constraint)。
//!
//! # Landing scope: drawables populate (narrowed, items 1-3)
//!
//! [`build_page_scene`] が `drawables` を実際に populate するようになった
//! (post-layout Element node → [`crate::BlockEntry`]、post-layout Text node →
//! [`crate::ParagraphEntry`]、`crate::entries` module doc参照)。ただし
//! **paint pipeline は今回も `PageDrawables` を一切消費しない** —
//! [`PageScene::rasterize`] は引き続き `dom` + `cascade` を thread して
//! `raikiri_paint::paint_single_page` を verbatim call する (下記
//! `rasterize` doc参照)。populate と consume が分離されているため、この
//! 変更は byte-identical VRT に影響しない。`raikiri_paint::paint_single_page`
//! を `PageDrawables` 消費に rework する話 (元 item 4) は crate-topology
//! 判断待ちで将来の作業に分離済み。

use crate::PageDrawables;
use crate::entries::{BlockEntry, ParagraphEntry};
use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use raikiri_dom::Document;
use raikiri_style::CascadeResult;
use raikiri_traits::{NodeId, NodeKind, PageBox};
use std::collections::BTreeMap;

/// Pt (PDF point、1/72 inch) を表す type alias。
///
/// Fulgur units::Pt (下流 consumer の point 型) と同じ underlying を持たせ、
/// consumer が Length / coordinate を conversion なしで扱えるようにする。
/// Newtype ではなく alias とし、arithmetic は Rust の primitive f32 operator
/// を直接使えるようにする (初期の minimal surface 判断、将来
/// unit-safety を強化する場合は newtype 化が別 decision)。
///
/// # Unit contract issue: `Pt` currently carries CSS px, not PDF pt
///
/// `build_page_scene` は `PageMetadata.size` / [`Fragment`] fields
/// (本 `Pt` 型) に **CSS px** 値 (1/96 in、PageBox / taffy `unrounded_layout`
/// 由来) を populate している。本 doc は "PDF point 1/72 in" を promise する
/// ため fulgur consumer が pt として読むと約 33% の geometry drift を起こす
/// (byte-identical raster path は影響なし — paint は Node arena を CSS px で
/// 再 walk する)。unit 変換 or type rename の resolution は今後の作業に
/// 持ち越し (byte-identical maintenance が primary scope だったため defer、
/// 将来 resolve 予定)。
///
/// 後続の `crate::entries` 群 (`BlockEntry` の
/// `layout_size` / `border_widths` 等) も同じ `Pt` alias を再利用し、同じ
/// CSS-px-in-Pt debt を意図的に踏襲する (新たな別種の unit debt を作らない
/// ための選択、`crate::entries` module doc参照)。
pub type Pt = f32;

/// Page 向きを示す enum。
///
/// CSS Paged Media の `size: portrait | landscape` を反映する consumer-facing
/// property。将来 `size: <named-size>` (A4 / Letter etc.) を
/// [`PageMetadata::page_name`] と組み合わせて解釈する時の primary axis。
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    /// 縦長 (width < height)。CSS の default。
    #[default]
    Portrait,
    /// 横長 (width > height)。
    Landscape,
}

/// Page 全体の metadata (size / 名前 / 向き)。
///
/// `size` は Pt 単位 `(width, height)`。CSS `@page` rule の `size` descriptor と
/// consumer 側の canvas size を紐付ける consumer contract。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageMetadata {
    /// Page の (幅, 高さ) を Pt で保持。
    pub size: (Pt, Pt),
    /// CSS `@page :first { size: A4 landscape; }` 等で名前付き page を
    /// 選択する場合の page name。無名 page (default) の場合は `None`。
    pub page_name: Option<String>,
    /// [`Orientation::Portrait`] / [`Orientation::Landscape`]。
    pub orientation: Orientation,
}

/// 1 個の [`NodeId`] に対応する 1 fragment の座標 (body-content-area-relative Pt、
/// fulgur drawables.rs:430-438 の per-fragment coordinate semantic 準拠)。
///
/// Multi-page block (`<div>` が page break を跨いで 2 fragment に分割) を
/// PageScene 側では `fragments: BTreeMap<NodeId, Vec<Fragment>>` の
/// entry 複数として表現する。`page_index` は fragment がどの page に属するか
/// を identify するための cross-page 参照 (単一 PageScene 内では固定値)。
///
/// # Note on `PageFragment`
///
/// raikiri umbrella には別途 [`raikiri::PageFragment`](crate::PageFragment)
/// (raikiri-traits 由来) が re-export 済で shape が異なる。
/// `Fragment` (本 struct、per-node per-fragment 座標) と `PageFragment`
/// (page 全体を sink に渡す container 型) は role が異なるが、名前の類似は
/// consumer confusion risk。将来 rename する余地あり。
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Fragment {
    /// この fragment を含む page の 0-based index。
    pub page_index: u32,
    /// Fragment 左上 x (border-box、body-content-area origin 起点、Pt)。
    pub x: Pt,
    /// Fragment 左上 y (border-box、body-content-area origin 起点、Pt)。
    pub y: Pt,
    /// Fragment 幅 (border-box、Pt)。
    pub width: Pt,
    /// Fragment 高さ (border-box、Pt)。
    pub height: Pt,
}

/// 1 page 分の immutable snapshot。Consumer が page 単位で forward iterate
/// して消費する raikiri の primary consumer-facing pub 型。
///
/// # Field 意味
///
/// - [`page_metadata`](PageScene::page_metadata): page の size / 名前 / 向き
/// - [`node_ids`](PageScene::node_ids): この page に含まれる全 DOM node の
///   [`NodeId`]、raikiri 内部 fragmentation pass が確定した順序で並ぶ
/// - [`fragments`](PageScene::fragments): NodeId ごとの per-fragment 座標。
///   単一 node が page 内で 1 fragment に収まる場合は `Vec` 長 1
/// - [`drawables`](PageScene::drawables): per-attribute node map ([`PageDrawables`]
///   参照)、ECS 風 shape
/// - [`root_id`](PageScene::root_id) / [`body_id`](PageScene::body_id):
///   `<html>` / `<body>` node の NodeId、consumer が root-level styling
///   (background / opacity) を lookup する時の entry point
/// - [`body_offset_pt`](PageScene::body_offset_pt): html → body の margin
///   collapse を折り込んだ page-absolute offset (fulgur drawables.rs:430-438
///   の `body_offset_pt` semantic 準拠)。consumer が [`fragments`](PageScene::fragments)
///   の per-fragment (x, y) (body-content-area-relative) に加算することで
///   page-absolute 座標を得る
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct PageScene {
    /// Page の size / 名前 / 向き。
    pub page_metadata: PageMetadata,
    /// この page に含まれる全 NodeId (raikiri 内部 pass 確定順)。
    pub node_ids: Vec<NodeId>,
    /// NodeId ごとの per-fragment 座標。
    pub fragments: BTreeMap<NodeId, Vec<Fragment>>,
    /// Per-attribute node map ([`PageDrawables`] 参照)。
    pub drawables: PageDrawables,
    /// `<html>` root NodeId。存在しない document は `None`。
    pub root_id: Option<NodeId>,
    /// `<body>` NodeId。存在しない document は `None`。
    pub body_id: Option<NodeId>,
    /// html → body margin collapse を折り込んだ page-absolute offset (Pt, Pt) —
    /// Fragment 座標 (body-content-area-relative) に加算して page-absolute 座標を得る。
    pub body_offset_pt: (Pt, Pt),
}

impl PageScene {
    /// 1 page 分の snapshot を A4 (or 指定 `PageBox`) canvas に raster して PNG bytes を返す。
    ///
    /// # なぜ snapshot が `dom` + `cascade` を param に取るのか
    ///
    /// [`PageDrawables`] entries (BlockEntry / ParagraphEntry) は
    /// minimal field を populate されているが、paint 消費
    /// 側はまだ切り替わっていない — glyph run 自体 (実 shape / position) は
    /// 依然 `parley::Layout` 型そのものであり `crate::entries` の field type
    /// 方針 (raikiri-style/parley 型を持たない) の対象外なので、真の paint
    /// truth は post-layout `Node.text_layout` (DOM arena) に住んだまま
    /// [`raikiri_paint::paint_single_page`] が DFS で消費する。従って
    /// byte-identical output を維持するため rasterize は `dom` + `cascade` を
    /// thread して既存 paint pipeline を verbatim reuse する。`paint_single_page`
    /// を `PageDrawables` 消費に rework する話 (真の snapshot semantics へ
    /// 到達し `dom` / `cascade` param を drop する) は crate-topology 判断待ちで
    /// 将来の作業に分離済み。
    ///
    /// # 内部
    /// 1. `page_box.{width,height}.ceil() as u32` で pixel buffer 寸法を得る (html_to_png と同一)
    /// 2. `anyrender::render_to_buffer::<VelloCpuImageRenderer, _>` で `PaintScene` を build
    /// 3. `raikiri_paint::paint_single_page(scene, dom, cascade, page_box)` を verbatim call
    /// 4. `encode_png` (`tiny_skia::Pixmap::encode_png`) で RGBA8 → PNG serialize
    ///
    /// # Panics
    /// - `page_box.width.ceil()` / `page_box.height.ceil()` を u32 に cast した結果が 0
    ///   (invalid `tiny_skia::IntSize`)
    /// - `anyrender_vello_cpu` 出力 buffer 長 != `width * height * 4` (invariant violation)
    /// - `tiny_skia::Pixmap::encode_png` 失敗 (well-formed pixmap では実際には起きない)
    /// - (debug build のみ) `cascade` と `dom` が同じ `cascade()` 呼び出しに由来しない
    ///   (`cascade.computed.len() != dom.node_count()`) —
    ///   `raikiri_paint` の module doc `## Contract` の caller-responsibility 契約違反。
    ///   [`raikiri_paint::paint_single_page`] 冒頭の `debug_assert!` が検査する。
    ///   release build ではこの assert 自体が消える。その場合の挙動は違反の
    ///   方向で異なる: `cascade.computed.len() < dom.node_count()` なら walk
    ///   中の raw index site (`cascade.computed[node_id]`) が in-bounds を
    ///   超えて "index out of bounds" で panic するが、逆方向
    ///   (`cascade.computed.len() > dom.node_count()`) は同じ index が常に
    ///   in-bounds のまま残るため panic せず、別 document の computed values
    ///   を silent に誤用したまま raster が完了する。
    ///
    /// pre-layout Document を渡すと `Node.text_layout` が空で glyph 抜けの PNG が出る
    /// (undefined、caller は `layout_single_page` 完了後に呼ぶ責任)。
    #[must_use]
    pub fn rasterize(&self, dom: &Document, cascade: &CascadeResult, page_box: PageBox) -> Vec<u8> {
        // PageBox = 793.7008 × 1122.5197 CSS px → 794 × 1123 u32 buffer (html_to_png と同一 rounding)
        let width = page_box.width.ceil() as u32;
        let height = page_box.height.ceil() as u32;

        let rgba = render_to_buffer::<VelloCpuImageRenderer, _>(
            |scene| raikiri_paint::paint_single_page(scene, dom, cascade, page_box),
            width,
            height,
        );

        encode_png(&rgba, width, height)
    }
}

/// Post-layout Document から metadata + fragments + drawables を抽出し
/// [`PageScene`] を construct する。
///
/// Internal wiring — `html_to_png_impl` から
/// `layout_single_page` 完了後に呼ばれる。`cascade` param が live 化し
/// (旧 `_cascade`)、DFS walk 中に Element node を
/// [`BlockEntry`]、Text node を [`ParagraphEntry`] として `drawables` へ
/// insert する (§module doc の landing scope 参照)。他 9 Entry 型は
/// 対応する pipeline stage が無いため insert されない (`crate::entries`
/// module doc参照)。
///
/// # NodeId identity mapping
///
/// raikiri-dom arena index (`usize`) を `raikiri_traits::NodeId(u64)` に
/// `NodeId::new(idx as u64)` で 1:1 射影する ("同一 NodeId 空間" を promise)。
/// 将来 真の PageDrawables 経由 paint に切り替わる時に本 mapping の
/// correctness が effective になる、現状は paint に影響しない cosmetic 属性。
///
/// # Fragment coordinate semantics
///
/// [`Fragment`] は body-content-area-relative Pt。body 自身は
/// `(x, y) = (0, 0)`、size は page dimensions。以降の descendant は
/// `raikiri_paint::walk::paint_document` と同じ DFS stack 順で
/// `parent_abs + node.unrounded_layout.location` を積算した body-relative 座標。
///
/// `body_offset_pt` は page-absolute origin における body 位置 — 現状 @page
/// margin なしで body_id が taffy root として (0, 0) から compute されるため
/// 常に `(0.0, 0.0)`。将来 @page margin が導入された時点で cascade から
/// 引き出す予定。
pub fn build_page_scene(
    dom: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> PageScene {
    let page_metadata = PageMetadata {
        size: (page_box.width, page_box.height),
        page_name: None,
        orientation: if page_box.width > page_box.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        },
    };

    let root_id = find_first_element_by_tag(dom, "html").map(|idx| NodeId::new(idx as u64));
    let body_arena_idx = find_first_element_by_tag(dom, "body");
    let body_id = body_arena_idx.map(|idx| NodeId::new(idx as u64));

    // body が無い document (fragment parse) は node_ids / fragments / drawables
    // 空で return。layout_single_page が Err を先に返すため実質 unreachable、
    // defense-in-depth。
    let mut node_ids: Vec<NodeId> = Vec::new();
    let mut fragments: BTreeMap<NodeId, Vec<Fragment>> = BTreeMap::new();
    let mut drawables = PageDrawables::default();

    if let Some(body_idx) = body_arena_idx {
        // DFS from body、paint_document と同じ traversal 順で collect。
        // Stack frame = (arena_idx, parent_abs_x, parent_abs_y)。
        // 各 node の absolute pos = parent_abs + node.unrounded_layout.location。
        let mut stack: Vec<(usize, Pt, Pt)> = vec![(body_idx, 0.0, 0.0)];
        while let Some((idx, parent_abs_x, parent_abs_y)) = stack.pop() {
            let Some(node) = dom.get_node(idx) else {
                continue;
            };
            if !node.is_in_document() {
                continue;
            }
            if node.is_non_rendered_html_element() {
                continue;
            }
            // display:none は paint_document で subtree skip されるため node_ids からも除外
            // (fragment 集合 vs 実 paint 集合を drift させない)。
            if node.kind() == NodeKind::Element && node.is_display_none() {
                continue;
            }
            let layout = node.unrounded_layout;
            let abs_x = parent_abs_x + layout.location.x;
            let abs_y = parent_abs_y + layout.location.y;

            let node_id = NodeId::new(idx as u64);
            node_ids.push(node_id);
            fragments.entry(node_id).or_default().push(Fragment {
                page_index: 0,
                x: abs_x,
                y: abs_y,
                width: layout.size.width,
                height: layout.size.height,
            });

            // `crate::entries` module doc の field type 方針 (raikiri-style 型を
            // 直接持たない) に従い、cascade の値は primitive へ変換して詰める。
            // Element → BlockEntry / Text → ParagraphEntry の 2 型のみ populate
            // (他 9 型は対応する pipeline stage が無い、同 module doc参照)。
            match node.kind() {
                NodeKind::Element => {
                    let cv = &cascade.computed[idx];
                    let entry = BlockEntry {
                        background_color: (
                            cv.background_color.r,
                            cv.background_color.g,
                            cv.background_color.b,
                            cv.background_color.a,
                        ),
                        border_widths: (
                            cv.border.top.width().0,
                            cv.border.right.width().0,
                            cv.border.bottom.width().0,
                            cv.border.left.width().0,
                        ),
                        id: element_id(dom, node_id),
                        layout_size: Some((layout.size.width, layout.size.height)),
                        ..BlockEntry::default()
                    };
                    drawables.block_styles.insert(node_id, entry);
                }
                NodeKind::Text => {
                    let line_count = node.text_layout().map_or(0, |l| l.lines().count());
                    let entry = ParagraphEntry {
                        line_count,
                        ..ParagraphEntry::default()
                    };
                    drawables.paragraphs.insert(node_id, entry);
                }
                _ => {}
            }

            if node.kind() == NodeKind::Element {
                // reverse push で pop 時に document order — paint_document 準拠。
                for &child in node.children.iter().rev() {
                    stack.push((child, abs_x, abs_y));
                }
            }
        }
    }

    // 現状: @page margin なし、body は page origin (0, 0) 起点。
    // 将来 cascade @page margin から引き出す予定。
    let body_offset_pt: (Pt, Pt) = (0.0, 0.0);

    PageScene {
        page_metadata,
        node_ids,
        fragments,
        drawables,
        root_id,
        body_id,
        body_offset_pt,
    }
}

/// Element node の `id` attribute を [`raikiri_traits::Dom`] trait 経由で取得
/// する。
///
/// `Document::get_node(usize) -> &raikiri_dom::Node` (本 module の DFS が使う
/// raw arena accessor) には attribute lookup が無い
/// (`crates/raikiri-dom/src/node.rs` 参照)。`NodeId` を経由した trait-based
/// lookup (`Dom::node` → `Node::as_element` → `Element::id`) を別途呼ぶ
/// (`crates/raikiri-dom/src/dom_impl.rs` 参照 — raw arena accessor と
/// trait-based element accessor が element-attribute access で unify されて
/// いないための second lookup path)。空文字列 `id=""` は trait 既定 contract
/// どおり `None`。
fn element_id(dom: &Document, node_id: NodeId) -> Option<String> {
    use raikiri_traits::{Dom as _, Element as _, Node as _};
    dom.node(node_id)?.as_element()?.id().map(str::to_string)
}

/// DFS から最初の `<tag>` element を返す (in-document のみ)。root_id / body_id
/// 抽出用の internal helper。`raikiri_paint::walk::find_body` と同じ contract
/// (inert subtree skip、tag 一致で確定) を tag 汎化した shape。
/// 本 helper は find_body-shaped pattern の 3rd copy — consolidation 判断は
/// 意図的に見送り済み (duplication として容認)。
fn find_first_element_by_tag(dom: &Document, tag: &str) -> Option<usize> {
    let mut stack: Vec<usize> = vec![dom.root_index()];
    while let Some(idx) = stack.pop() {
        let node = dom.get_node(idx)?;
        if !node.is_in_document() {
            continue;
        }
        if node.kind() == NodeKind::Element && node.tag_name() == Some(tag) {
            return Some(idx);
        }
        for &c in node.children.iter().rev() {
            stack.push(c);
        }
    }
    None
}

/// Premultiplied RGBA8 buffer を PNG bytes に serialize する (`tiny_skia::Pixmap` 経由)。
///
/// `rgba` は `width * height * 4` bytes 厳密要求。`anyrender_vello_cpu` の出力は
/// premultiplied RGBA8 で `tiny_skia` も同 format のため wrap → serialize。
///
/// # Panics
/// - `rgba.len() != width * height * 4`
/// - `width == 0 || height == 0` (invalid `tiny_skia::IntSize`)
/// - PNG serialization 失敗 (well-formed pixmap では実際には起きない、invariant violation 扱い)
fn encode_png(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let expected = (width as usize) * (height as usize) * 4;
    assert_eq!(
        rgba.len(),
        expected,
        "encode_png: expected {expected} bytes for {width}x{height}, got {}",
        rgba.len(),
    );
    let size =
        tiny_skia::IntSize::from_wh(width, height).expect("encode_png: width/height must be > 0");
    let pixmap = tiny_skia::Pixmap::from_vec(rgba.to_vec(), size)
        .expect("encode_png: Pixmap::from_vec rejected pre-validated buffer (tiny-skia invariant violation)");
    pixmap
        .encode_png()
        .expect("encode_png: tiny_skia::Pixmap::encode_png should not fail for a valid pixmap")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_cascaded;
    use parley::FontContext;
    use raikiri_html::{ParseOptions, parse};

    /// hello-world 相当 HTML を parse → cascade → layout し、post-layout Document
    /// + CascadeResult を返す。build_page_scene の smoke test 共通 setup。
    fn hello_world_post_layout() -> (Document, CascadeResult) {
        let opts = ParseOptions {
            extra_stylesheets: &[],
            network: None,
            base_url: None,
        };
        let uncascaded = parse(&b"<p>Hi</p>"[..], &opts).expect("parse Ok");
        let cascade = build_cascaded(&uncascaded);
        let mut dom = uncascaded.dom;
        raikiri_dom::layout_single_page(&mut dom, &cascade, PageBox::A4, FontContext::new())
            .expect("layout Ok");
        (dom, cascade)
    }

    /// build_page_scene が hello-world post-layout Document から metadata + fragments を
    /// populate する — recommended integration assertion
    /// (node_ids non-empty + body_id/root_id populated)。dead-untested
    /// 防止 pin。
    #[test]
    fn build_page_scene_populates_metadata_from_hello_world() {
        let (dom, cascade) = hello_world_post_layout();
        let scene = build_page_scene(&dom, &cascade, PageBox::A4);

        assert!(
            scene.root_id.is_some(),
            "root_id must resolve to <html> for well-formed document"
        );
        assert!(
            scene.body_id.is_some(),
            "body_id must resolve to <body> for well-formed document"
        );
        assert!(
            !scene.node_ids.is_empty(),
            "node_ids must include at least body + descendants"
        );
        // DFS starts at body, so body_id must be the first entry in node_ids
        assert_eq!(
            scene.node_ids.first().copied(),
            scene.body_id,
            "node_ids[0] must be body_id (DFS starts at body, parity with paint_document)"
        );
        // Fragments coverage: every id in node_ids has at least one fragment
        for id in &scene.node_ids {
            assert!(
                scene.fragments.get(id).is_some_and(|f| !f.is_empty()),
                "fragments must have at least one entry for each id in node_ids ({id:?})",
            );
        }
        // 現状: @page margin なし、body_offset_pt = (0, 0)
        assert_eq!(scene.body_offset_pt, (0.0, 0.0));
        // Page metadata reflects A4
        assert_eq!(
            scene.page_metadata.size,
            (PageBox::A4.width, PageBox::A4.height)
        );
        assert_eq!(scene.page_metadata.orientation, Orientation::Portrait);
    }

    /// build_page_scene が Element node → `BlockEntry` / Text node →
    /// `ParagraphEntry` を `drawables` へ populate する (regression pin —
    /// `TrackedMap::insert` の非-test call site がこの production path
    /// 経由で exercise されることも同時に確認する)。
    #[test]
    fn build_page_scene_populates_block_and_paragraph_entries_from_hello_world() {
        let (dom, cascade) = hello_world_post_layout();
        let scene = build_page_scene(&dom, &cascade, PageBox::A4);

        // Every Element NodeId in node_ids has a block_styles entry, every
        // Text NodeId has a paragraphs entry — coverage must exactly match
        // node_ids (fragments と同じ集合、drift させない)。
        for id in &scene.node_ids {
            let node = dom
                .get_node(id.0 as usize)
                .expect("node_ids entries resolve");
            match node.kind() {
                NodeKind::Element => {
                    assert!(
                        scene.drawables.block_styles.contains_key(id),
                        "Element {id:?} must have a block_styles entry"
                    );
                }
                NodeKind::Text => {
                    assert!(
                        scene.drawables.paragraphs.contains_key(id),
                        "Text {id:?} must have a paragraphs entry"
                    );
                }
                _ => {}
            }
        }

        // body_id resolves to an Element and must carry a BlockEntry with a
        // real (non-placeholder) layout_size — proves the cascade/layout
        // param is actually threaded, not just structurally accepted.
        let body_id = scene.body_id.expect("hello-world has <body>");
        let body_entry = scene
            .drawables
            .block_styles
            .get(&body_id)
            .expect("body must have a BlockEntry");
        assert!(
            body_entry.layout_size.is_some(),
            "BlockEntry.layout_size must be populated from post-layout Node.unrounded_layout"
        );
        // Gap fields hold the documented CSS-initial-value placeholders
        // (entries.rs module doc: opacity/visibility properties don't exist
        // in ComputedValues yet).
        assert_eq!(body_entry.opacity, 1.0);
        assert!(body_entry.visible);

        // At least one paragraph entry must have shaped lines (the "Hi" text
        // node) — proves text_layout() is actually read, not defaulted.
        assert!(
            scene
                .drawables
                .paragraphs
                .values()
                .any(|p| p.line_count > 0),
            "at least one ParagraphEntry must have line_count > 0 for shaped \"Hi\" text"
        );
    }

    /// PageScene::rasterize が html_to_png と同じ PNG bytes を返す
    /// (byte-identical triple の verbatim reuse pin — primary regression
    /// signal を module scope でも local に固定する)。
    #[test]
    fn rasterize_matches_html_to_png_bytes() {
        let (dom, cascade) = hello_world_post_layout();
        let scene = build_page_scene(&dom, &cascade, PageBox::A4);
        let via_scene = scene.rasterize(&dom, &cascade, PageBox::A4);
        let via_umbrella = crate::html_to_png(&b"<p>Hi</p>"[..]).expect("html_to_png Ok");
        assert_eq!(
            via_scene, via_umbrella,
            "PageScene::rasterize must produce byte-identical output to html_to_png"
        );
    }
}
