//! Page-related neutral model types.
//!
//! ここに集めた型は §5 (Pipeline), §7 (GCPM), §9 (PageBox), §11 (Paint) が
//! authoritative なので、M1.1 では opaque placeholder として置き、後続の
//! task が field / method を段階的に populate する。
//!
//! 全 struct は `#[non_exhaustive]` + `impl Default` + `pub fn new()` を持ち、
//! external consumer crate から `X::default()` で construct 可能。

/// PageFragment — 1 ページの painted output (glyph run / decoration / target slot 含む)。
/// M1.7 paint-basic + M2 pagestream で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageFragment {
    // M1.7 / M2 で populate:
    //   pub page_index: u32,
    //   pub page_box: PageBox,
    //   pub items: Vec<PaintedBoxItem>,
    //   pub target_slots: Vec<TargetSlot>,
    //   ...
}

impl PageFragment {
    /// Construct an empty PageFragment. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// PageBox — @page rule 解決結果 (size, margins, margin box slots)。
/// M1.6 layout-single-page で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageBox {
    // M1.6 で populate:
    //   pub width: f32,
    //   pub height: f32,
    //   pub margins: Margins,
    //   pub margin_boxes: [Option<MarginBox>; 16],
    //   ...
}

impl PageBox {
    /// Construct an empty PageBox. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// PageContext — GCPM runtime state (counter tree, named string 4-snapshot,
/// running bindings)。M4 GCPM で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct PageContext {
    // M4 で populate。
}

impl PageContext {
    /// Construct an empty PageContext. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// LayoutBuffer — widow / orphan / break-inside / container probe lookahead
/// buffer の中立モデル。実装は raikiri-dom 側 (§5 参照)。
/// M2 layoutbuffer-skeleton で populate。
#[allow(missing_docs)]
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LayoutBuffer {
    // M2 で populate。
}

impl LayoutBuffer {
    /// Construct an empty LayoutBuffer. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// TargetRegistry — target-* placeholder emit + resolve の runtime registry。
/// M4 target-* で populate (§7.2 参照)。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct TargetRegistry {
    // M4 で populate。
}

impl TargetRegistry {
    /// Construct an empty TargetRegistry. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// RunningTemplate — `position: running(name)` の template 登録。
/// M4 で populate。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct RunningTemplate {
    // M4 で populate。
}

impl RunningTemplate {
    /// Construct an empty RunningTemplate. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// FormData — application/x-www-form-urlencoded body の中立モデル。
/// Consumer 側 network 実装で参照 (Body::Form(FormData))。
#[allow(missing_docs)]
#[derive(Debug, Default, Clone)]
#[non_exhaustive]
pub struct FormData {
    // 将来 populate:
    //   pub pairs: Vec<(String, String)>,
}

impl FormData {
    /// Construct an empty FormData. M1.1 placeholder.
    pub fn new() -> Self {
        Self::default()
    }
}

/// GCPM directive emitted by raikiri-style cascade (`counter-increment` /
/// `string-set` / `position: running(name)` 等)。
///
/// M4 で variant を populate (§7.1 参照)。M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum GcpmDirective {
    // M4 で populate:
    //   CounterIncrement { name: Symbol, delta: i32 },
    //   CounterReset { name: Symbol, value: i32 },
    //   StringSet { name: Symbol, value: Vec<ContentValueItem> },
    //   RunningRegister { name: Symbol },
}

/// resolved `content` property の item (`content: string(...)`, `counter(...)`,
/// `target-counter(...)`, `element(...)` 等の資産)。
///
/// M4 で variant を populate (§7 参照)。M1.1 では uninhabited。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ContentValueItem {
    // M4 で populate:
    //   Literal(String),
    //   Counter { name: Symbol, style: CounterStyle },
    //   String { name: Symbol, snapshot: NamedStringSnapshot },
    //   TargetCounter { url: Url, name: Symbol, style: CounterStyle },
    //   ...
}
