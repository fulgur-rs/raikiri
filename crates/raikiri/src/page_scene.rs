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

use crate::PageDrawables;
use raikiri_traits::NodeId;
use std::collections::BTreeMap;

/// Pt (PDF point、1/72 inch) を表す type alias。
///
/// Fulgur units::Pt (下流 consumer の point 型) と同じ underlying を持たせ、
/// consumer が Length / coordinate を conversion なしで扱えるようにする。
/// Newtype ではなく alias とし、arithmetic は Rust の primitive f32 operator
/// を直接使えるようにする (Sprint 22 の minimal surface 判断、future で
/// unit-safety を強化する場合は newtype 化が別 decision)。
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
