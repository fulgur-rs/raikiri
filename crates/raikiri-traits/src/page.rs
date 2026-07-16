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
/// M1.6 layout-single-page で width / height + `A4` / `US_LETTER` const を populate。
/// margins / margin_boxes は M4 で populate。
///
/// **単位 = CSS px** (1 CSS px = 1/96 in in print context per CSS Values L4 §5.2)。
/// pt / mm / in への換算は Consumer 責務。
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct PageBox {
    /// Page 幅 (CSS px)。
    pub width: f32,
    /// Page 高 (CSS px)。
    pub height: f32,
    // M4 で populate:
    //   pub margins: Margins,
    //   pub margin_boxes: [Option<MarginBox>; 16],
}

impl PageBox {
    /// A4 portrait: 210×297 mm = **793.70 × 1122.52 px** (@ 96 DPI anchor)。
    /// CSS Paged Media Level 3 §7 default size。
    pub const A4: PageBox = PageBox {
        width: 793.7008,   // 210mm × 96/25.4
        height: 1122.5197, // 297mm × 96/25.4
    };

    /// US Letter portrait: 8.5×11 in = **816 × 1056 px** ちょうど。
    pub const US_LETTER: PageBox = PageBox {
        width: 816.0,
        height: 1056.0,
    };

    /// Construct a `PageBox` = `A4`。`#[non_exhaustive]` の下でも安定した
    /// zero-arg constructor を残すため保持。
    pub fn new() -> Self {
        Self::default()
    }
}

impl Default for PageBox {
    fn default() -> Self {
        Self::A4
    }
}

/// Consumer が render 開始時に渡す page-level default 値。M1 は paper size
/// のみを持つ最小 shape。M2+ で margin / orientation / named pages 等を追加予定。
///
/// 全 field は CSS px 単位 (`PageBox` 参照)。pt/mm/in 換算は Consumer 責務。
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PageDefaults {
    /// Default paper サイズ (`@page size` で override しない場合の initial value)。
    /// 既定 = A4。
    pub page_box: PageBox,
}

impl PageDefaults {
    /// Default 相当の shortcut。
    pub fn new() -> Self {
        Self::default()
    }

    /// Fluent builder を返す。
    pub fn builder() -> PageDefaultsBuilder {
        PageDefaultsBuilder::default()
    }
}

impl Default for PageDefaults {
    fn default() -> Self {
        Self {
            page_box: PageBox::default(),
        }
    }
}

/// `PageDefaults` の fluent builder。
#[derive(Debug, Default, Clone)]
pub struct PageDefaultsBuilder {
    page_box: Option<PageBox>,
}

impl PageDefaultsBuilder {
    /// `page_box` を設定。
    pub fn page_box(mut self, v: PageBox) -> Self {
        self.page_box = Some(v);
        self
    }

    /// Build。未設定 field は Default 値。
    pub fn build(self) -> PageDefaults {
        PageDefaults {
            page_box: self.page_box.unwrap_or_default(),
        }
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

#[cfg(test)]
mod pagebox_px_baseline_tests {
    use super::*;

    #[test]
    fn a4_dimensions_match_css_px_conversion() {
        // 210mm × 297mm を CSS px (1/96 in) 換算:
        //   width  = 210mm × 96/25.4 ≈ 793.7008
        //   height = 297mm × 96/25.4 ≈ 1122.5197
        assert!(
            (PageBox::A4.width - 793.7008).abs() < 0.001,
            "A4.width should be ~793.7008 px, got {}",
            PageBox::A4.width
        );
        assert!(
            (PageBox::A4.height - 1122.5197).abs() < 0.001,
            "A4.height should be ~1122.5197 px, got {}",
            PageBox::A4.height
        );
    }

    #[test]
    fn us_letter_dimensions_match_exact_integers() {
        // 8.5in × 11in @ 96 DPI = 816 × 1056 px exactly
        assert_eq!(PageBox::US_LETTER.width, 816.0);
        assert_eq!(PageBox::US_LETTER.height, 1056.0);
    }

    #[test]
    fn pagebox_default_is_a4() {
        assert_eq!(PageBox::default(), PageBox::A4);
    }
}

#[cfg(test)]
mod pagedefaults_tests {
    use super::*;

    #[test]
    fn pagedefaults_default_uses_a4() {
        let d = PageDefaults::default();
        assert_eq!(d.page_box, PageBox::A4);
    }

    #[test]
    fn pagedefaults_new_is_default() {
        assert_eq!(
            PageDefaults::new().page_box,
            PageDefaults::default().page_box
        );
    }

    #[test]
    fn pagedefaults_builder_sets_page_box() {
        let d = PageDefaults::builder().page_box(PageBox::US_LETTER).build();
        assert_eq!(d.page_box, PageBox::US_LETTER);
    }

    #[test]
    fn pagedefaults_builder_default_matches_pagedefaults_default() {
        let via_builder = PageDefaults::builder().build();
        let via_default = PageDefaults::default();
        assert_eq!(via_builder.page_box, via_default.page_box);
    }
}
