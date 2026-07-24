//! Per-page immutable snapshot consumed by raikiri downstream consumers
//! (fulgur PDF translator が想定 primary consumer).
//!
//! # Consumer contract (raikiri-spike-0icy Axis 1)
//!
//! [`PageScene`] は raikiri 内部 pipeline (fragmentation + reflow +
//! LayoutBuffer) を通り抜けた後の **1 page 分の immutable snapshot**。
//! Consumer 側は forward iterate + batch operation で消費し、reflow /
//! re-fragmentation の concern は持たない (raikiri 内部完結)。
//!
//! # Node identity
//!
//! [`raikiri_traits::NodeId`] を key として page 内で分布する DOM node を
//! 表現する。raikiri crate の Document (raikiri-spike-m1.23 で umbrella
//! re-export) と PageScene が **同一 NodeId 空間** を共有するため、
//! consumer は 1 個の `NodeId` を Document 側の property lookup と
//! PageScene 側の drawable / fragment lookup に **conversion なしで**
//! 使い回せる (coord 2026-07-24 decision on raikiri-spike-os52、bd
//! comment 参照)。
//!
//! # Sprint 22 (raikiri-spike-os52) landing scope
//!
//! Pub type surface のみ landing (`#[non_exhaustive]` で future field 追加を
//! semver-non-breaking に保つ)。内部 pipeline から `PageScene` を実際に
//! populate する path は future sprint (M2 streaming pagination) で fill。
//!
//! # Sprint 23 (raikiri-spike-bkkm) landing scope
//!
//! 内部 `html_to_png` pipeline を PageScene 経由に refactor。
//! [`build_page_scene`] で post-layout Document から metadata + fragments を
//! 抽出した [`PageScene`] を organize し、[`PageScene::rasterize`] が
//! `raikiri_paint::paint_single_page` + `anyrender_vello_cpu` + PNG encode の
//! byte-identical triple を verbatim reuse する。
//!
//! `drawables` field は Sprint 23 では empty のまま (glyph run / position 情報を
//! 持たない Sprint 22 landed empty entries に populate すると emission order
//! が変わり byte-identical が破れるため、advisor consult で確定した hard
//! constraint)。M4+ で BlockEntry / ParagraphEntry に paint 情報が landing
//! した時点で真の PageDrawables 経由 paint に切り替える。

use crate::PageDrawables;
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
/// を直接使えるようにする (Sprint 22 の minimal surface 判断、future で
/// unit-safety を強化する場合は newtype 化が別 decision)。
///
/// # Sprint 23 (bkkm) unit contract issue — bd raikiri-spike-v0zm
///
/// `build_page_scene` は Sprint 23 で `PageMetadata.size` / [`Fragment`] fields
/// (本 `Pt` 型) に **CSS px** 値 (1/96 in、PageBox / taffy `unrounded_layout`
/// 由来) を populate している。本 doc は "PDF point 1/72 in" を promise する
/// ため fulgur consumer が pt として読むと約 33% の geometry drift を起こす
/// (byte-identical raster path は影響なし — paint は Node arena を CSS px で
/// 再 walk する)。unit 変換 or type rename の resolution は bd raikiri-spike-v0zm
/// (wall/umbrella)。Sprint 23 は byte-identical maintenance が primary scope の
/// ため coordinator 判断で defer、Sprint 24+ で resolve 予定。
pub type Pt = f32;

/// Page 向きを示す enum。
///
/// CSS Paged Media の `size: portrait | landscape` を反映する consumer-facing
/// property。future sprint で `size: <named-size>` (A4 / Letter etc.) を
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
/// (raikiri-traits 由来、m1.23 landed) が re-export 済で shape が異なる。
/// `Fragment` (本 struct、per-node per-fragment 座標) と `PageFragment`
/// (page 全体を sink に渡す container 型) は role が異なるが、名前の類似は
/// consumer confusion risk。reviewer:spec の判断で future sprint に rename
/// する余地あり (raikiri-spike-os52 escalation comment §non-blocking flag)。
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
///   参照)、raikiri-spike-0icy Axis 2 の ECS 風 shape
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
    /// Sprint 23 landed の [`PageDrawables`] entries (BlockEntry / ParagraphEntry
    /// etc.) は Sprint 22 の empty struct のままで、glyph run / position 情報を
    /// carry しない。paint truth (shaped text lines / decoration) は post-layout
    /// `Node.text_layout` (DOM arena) に住み、[`raikiri_paint::paint_single_page`]
    /// が DFS で消費する。従って byte-identical output を維持するため rasterize は
    /// `dom` + `cascade` を thread して既存 paint pipeline を verbatim reuse する。
    /// M4+ で `PageDrawables` に paint 情報が完全 populate された時点で `dom` /
    /// `cascade` param は drop 予定 (真の snapshot semantics に到達)。populate 追跡:
    /// bd raikiri-spike-4hp1、param drop: bd raikiri-spike-dmoo (4hp1 に blocked-on)。
    ///
    /// # 内部
    /// 1. `page_box.{width,height}.ceil() as u32` で pixel buffer 寸法を得る (M1.14 と同一)
    /// 2. `anyrender::render_to_buffer::<VelloCpuImageRenderer, _>` で `PaintScene` を build
    /// 3. `raikiri_paint::paint_single_page(scene, dom, cascade, page_box)` を verbatim call
    /// 4. `encode_png` (`tiny_skia::Pixmap::encode_png`) で RGBA8 → PNG serialize
    ///
    /// # Panics
    /// - `page_box.width.ceil()` / `page_box.height.ceil()` を u32 に cast した結果が 0
    ///   (invalid `tiny_skia::IntSize`)
    /// - `anyrender_vello_cpu` 出力 buffer 長 != `width * height * 4` (invariant violation)
    /// - `tiny_skia::Pixmap::encode_png` 失敗 (well-formed pixmap では実際には起きない)
    ///
    /// pre-layout Document を渡すと `Node.text_layout` が空で glyph 抜けの PNG が出る
    /// (undefined、caller は `layout_single_page` 完了後に呼ぶ責任)。
    #[must_use]
    pub fn rasterize(&self, dom: &Document, cascade: &CascadeResult, page_box: PageBox) -> Vec<u8> {
        // PageBox = 793.7008 × 1122.5197 CSS px → 794 × 1123 u32 buffer (M1.14 と同一 rounding)
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

/// Post-layout Document から metadata + fragments を抽出し [`PageScene`] を construct する。
///
/// Sprint 23 (raikiri-spike-bkkm) の internal wiring — `html_to_png_impl` から
/// `layout_single_page` 完了後に呼ばれる。`drawables` は Sprint 22 landed の
/// empty entries のままにする (§module doc の Sprint 23 landing scope 参照)。
/// `_cascade` param は entries populate 時 (bd raikiri-spike-4hp1) に BlockEntry /
/// ParagraphEntry / etc. の cascaded property lookup で live 化する forward-reservation。
///
/// # NodeId identity mapping
///
/// raikiri-dom arena index (`usize`) を `raikiri_traits::NodeId(u64)` に
/// `NodeId::new(idx as u64)` で 1:1 射影する (raikiri-spike-0icy / shc6 で
/// promise した "同一 NodeId 空間")。M4+ で真の PageDrawables 経由 paint に
/// 切り替わる時に本 mapping の correctness が effective になる、現状は
/// paint に影響しない cosmetic 属性。
///
/// # Fragment coordinate semantics
///
/// [`Fragment`] は body-content-area-relative Pt。body 自身は
/// `(x, y) = (0, 0)`、size は page dimensions。以降の descendant は
/// [`raikiri_paint::walk::paint_document`] と同じ DFS stack 順で
/// `parent_abs + node.unrounded_layout.location` を積算した body-relative 座標。
///
/// `body_offset_pt` は page-absolute origin における body 位置 — M1 は @page
/// margin なしで body_id が taffy root として (0, 0) から compute されるため
/// 常に `(0.0, 0.0)`。M4+ で @page margin が導入された時点で cascade から
/// 引き出す予定 (bd raikiri-spike-pie2、raikiri-spike-m4 discovered-from)。
pub(crate) fn build_page_scene(
    dom: &Document,
    _cascade: &CascadeResult,
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

    // body が無い document (fragment parse) は node_ids / fragments 空で return。
    // layout_single_page が Err を先に返すため実質 unreachable、defense-in-depth。
    let mut node_ids: Vec<NodeId> = Vec::new();
    let mut fragments: BTreeMap<NodeId, Vec<Fragment>> = BTreeMap::new();

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

            if node.kind() == NodeKind::Element {
                // reverse push で pop 時に document order — paint_document 準拠。
                for &child in node.children.iter().rev() {
                    stack.push((child, abs_x, abs_y));
                }
            }
        }
    }

    // M1: @page margin なし、body は page origin (0, 0) 起点。
    // M4+ で cascade @page margin から引き出す: bd raikiri-spike-pie2。
    let body_offset_pt: (Pt, Pt) = (0.0, 0.0);

    PageScene {
        page_metadata,
        node_ids,
        fragments,
        drawables: PageDrawables::default(),
        root_id,
        body_id,
        body_offset_pt,
    }
}

/// DFS から最初の `<tag>` element を返す (in-document のみ)。root_id / body_id
/// 抽出用の internal helper。`raikiri_paint::walk::find_body` と同じ contract
/// (inert subtree skip、tag 一致で確定) を tag 汎化した shape。
/// 本 helper は find_body-shaped pattern の 3rd copy — consolidation 判断は
/// bd raikiri-spike-94wp (wall-respecting duplication として retro-accept 済)。
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
    /// populate する — advisor consult で recommended integration assertion
    /// (node_ids non-empty + body_id/root_id populated)。reviewer:debt に対する
    /// dead-untested 防止 pin。
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
        // M1: @page margin なし、body_offset_pt = (0, 0)
        assert_eq!(scene.body_offset_pt, (0.0, 0.0));
        // Page metadata reflects A4
        assert_eq!(
            scene.page_metadata.size,
            (PageBox::A4.width, PageBox::A4.height)
        );
        assert_eq!(scene.page_metadata.orientation, Orientation::Portrait);
        // Drawables stays empty (Sprint 23 Finding 1 constraint)
        assert!(scene.drawables.block_styles.is_empty());
    }

    /// PageScene::rasterize が html_to_png と同じ PNG bytes を返す
    /// (byte-identical triple の verbatim reuse pin — Sprint 23 raikiri-spike-bkkm
    /// primary regression signal を module scope でも local に固定する)。
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
