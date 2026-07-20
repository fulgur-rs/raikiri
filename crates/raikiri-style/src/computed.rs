//! Per-node computed CSS values (M1.4 scope: 4 inherited properties)。
//!
//! Cascade + inheritance walk が populate。M1.6 で ComputedValues → taffy::Style
//! + paint 用色情報の抽出 layer が入る予定。

use std::sync::Arc;

use smol_str::SmolStr;

use crate::Atom;
use crate::property::{
    ContentComponent, CssColor, DisplayValue, Length, LengthOrAuto, Sides, empty_content_list,
    empty_counter_entries, empty_string_set_entries,
};

/// `position: running(<custom-ident>)` により登録された template の cascade-time seed。
///
/// CSS GCPM 3 §1.2.1 "The running() value"
/// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>: `position: running(name)`
/// された element は body flow から除去、`@page` margin box の
/// `content: element(name)` から参照される。
///
/// 本 struct は design doc §7.3 の **2-tier キャッシュ** の static side seed —
/// cascade で per-node に `name` を捕捉し、下流 (raikiri-dom) 側が subtree_root /
/// pre-cascaded style / dynamic flags を association する
/// (`ParsedRunningTemplate` — 本 crate は leaf、DOM node identity を持たない)。
///
/// `#[non_exhaustive]` により future field (e.g. `alternative_hint` 等の per-name
/// override) を non-breaking で追加可能。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningTemplate {
    /// `running(<name>)` の name (case-preserved smol str)。
    pub name: SmolStr,
}

/// Per-node computed style。M1.4 では 4 property のみ (全て inherited)。
///
/// `#[non_exhaustive]` により future property (background-color / display /
/// margin / padding / width / height 等) の追加が non-breaking。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedValues {
    /// `color`。inherited、initial: opaque black。
    pub color: CssColor,
    /// `font-family` — 優先順位順。inherited、initial: `[Atom::from("serif")]`。
    pub font_family: Vec<Atom>,
    /// `font-size`。inherited、initial: `Length::Px(16.0)` (browser default medium)。
    pub font_size: Length,
    /// `font-weight`。inherited、initial: 400 (normal)。
    pub font_weight: u16,
    /// `display`。**non-inherited**、initial: `DisplayValue::Inline` (CSS §9.2.4)。
    /// (spec §M1.4a、raikiri-spike-m1.22)
    pub display: DisplayValue,
    /// `counter-reset`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + initial value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`pick_winners` value.clone、
    /// `apply_value` move) と inheritance walk clone (`resolve_inheritance` の
    /// stack push + `out[idx] = computed.clone()`) が **shallow (Arc bump)** に
    /// なる。`* { counter-reset: c0 c1 ... cN }` × M element の O(N × M) memory
    /// blow-up を単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.2 SEC HIGH、d9y.1
    /// Content/StringSet pattern の踏襲)。`Arc<Vec<T>>: Deref<Target = Vec<T>>`
    /// により downstream の `.iter()` / `.len()` / `.is_empty()` は既存 pattern
    /// そのままで通る (dom/paint consumer 波及 0)。
    pub counter_reset: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-increment`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + increment pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    pub counter_increment: Arc<Vec<(SmolStr, i32)>>,
    /// `counter-set`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::counter_reset`] と同 rationale
    /// (raikiri-spike-d9y.2)。
    pub counter_set: Arc<Vec<(SmolStr, i32)>>,
    /// `content` の resolved 中間表現。**non-inherited**、initial: empty list
    /// (spec §2.1 "content" property の `normal` / `none` を空 list として扱う
    /// — 本 crate は cascade static side、pseudo-element 生成判断は下流 layer)。
    /// M5 gcpm-directive-emit (raikiri-spike-m5.1)。
    /// 下流 (raikiri-dom) が `raikiri_traits::ContentValueItem` に mapping する
    /// (raikiri-style は raikiri-traits に依存しない leaf crate = 3ps/94e Phase B、
    /// counter-* wire-through pattern を踏襲、raikiri-spike-s85)。
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    ///
    /// [`Arc<Vec<..>>`] wrap: cascade winner clone (`pick_winners` value.clone、
    /// `apply_value` move) と inheritance walk clone (`resolve_inheritance` の
    /// stack push + `out[idx] = computed.clone()`) が **shallow (Arc bump)** に
    /// なる。`* { content: "<large>" }` × N element の O(N × M) memory blow-up
    /// を単一 heap slot 共有で塞ぐ (raikiri-spike-d9y.1 SEC HIGH)。
    /// `Arc<Vec<T>>: Deref<Target = Vec<T>>` により downstream の `.iter()` /
    /// `.len()` / `.is_empty()` は既存 pattern そのままで通る (dom/paint
    /// consumer 波及 0)。
    pub content: Arc<Vec<ContentComponent>>,
    /// `string-set` の parse 結果 — `(name, content-list)` entry の列。
    /// **non-inherited**、initial: empty list (CSS GCPM 3 §3.1)。
    /// 名前解決と runtime `string()` 参照は下流 (raikiri-dom) 責務。
    /// See <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>.
    ///
    /// [`Arc<Vec<..>>`] wrap は [`Self::content`] と同 rationale
    /// (raikiri-spike-d9y.1、`* { string-set: name "<large>" }` × N element の
    /// 同種 DoS 経路を塞ぐ)。
    pub string_set: Arc<Vec<(SmolStr, Vec<ContentComponent>)>>,
    /// `position: running(<custom-ident>)` の seed。**non-inherited**、initial:
    /// empty list。CSS GCPM 3 §1.2.1
    /// <https://www.w3.org/TR/css-gcpm-3/#running-syntax>。
    ///
    /// 本 field は **per-node で常に 0 または 1 要素** (`position` は spec 上
    /// 単一値の property、element は最大 1 つの `running(name)` しか持たない):
    /// - `position: static` / 他 keyword / rule 無し → empty
    /// - `position: running(name)` → `[RunningTemplate { name }]`
    ///
    /// `Vec` shape を採るのは m5.1 `content` / m5.3 `string_set` と同じ
    /// SmolStr wire-through pattern の踏襲 (原則 1 前例主義)。下流 (raikiri-dom)
    /// が per-document `Vec<RunningTemplate>` を組み立てる際に per-node seed を
    /// concatenate する。design doc §7.3 の 2-tier キャッシュ static side に相当。
    /// (raikiri-spike-m5.4)
    pub running_templates: Vec<RunningTemplate>,
    /// `margin` 4-side quad (top / right / bottom / left)。**non-inherited**、
    /// initial: `0` on each side (`Sides::all(LengthOrAuto::Length(Length::Px(0.0)))`).
    ///
    /// Author CSS の box model 中核 property。raikiri-style は cascade static side
    /// に留まり、`LengthOrAuto::Length(Em/Rem/Percent/Pt)` の resolve は下流
    /// (raikiri-dom `apply_computed_to_style` bridge、future task) 責務 —
    /// font-size context / containing block % / DPI 変換で `taffy::LengthPercentageAuto`
    /// 相当に翻訳される。
    ///
    /// # Primary sources (§ title + anchor)
    ///
    /// - CSS Box 3 §3.1 "Page-relative (Physical) Margin Properties":
    ///   [`margin-top` / `margin-right` / `margin-bottom` / `margin-left`](https://www.w3.org/TR/css-box-3/#margin-physical)
    ///   — "Value: `<length-percentage> | auto`", "Initial: 0", "Inherited: no",
    ///   "Applies to: all elements except internal table elements".
    /// - CSS Box 3 §3.2 "Margin Shorthand: the margin property":
    ///   [`margin`](https://www.w3.org/TR/css-box-3/#margin-shorthand) —
    ///   "Value: `<'margin-top'>{1,4}`", 1/2/3/4 value expansion rules.
    ///
    /// raikiri-spike-0vv.5。
    pub margin: Sides<LengthOrAuto>,
}

impl ComputedValues {
    /// CSS spec に沿った initial value。cascade で何も matching しなかった root
    /// node と、inheritance chain の terminate に使う。
    pub fn initial() -> Self {
        Self {
            color: CssColor::BLACK,
            font_family: vec![Atom::from("serif")],
            font_size: Length::Px(16.0),
            font_weight: 400,
            display: DisplayValue::Inline,
            // CSS Lists 3 §3: counter-* initial is empty list (raikiri-spike-s85)
            // d9y.2: shared empty Arc slot — per-node allocation 回避
            // (advisor calibration、property.rs `empty_counter_entries` doc 参照)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // CSS Content 3 §2.1: content initial (normal) は下流にとって「no
            // generated content」= empty list として扱う (raikiri-spike-m5.1)。
            // d9y.1: shared empty Arc slot — per-node allocation 回避
            // (advisor calibration、property.rs `empty_content_list` doc 参照)。
            content: empty_content_list(),
            // CSS GCPM 3 §3.1: string-set initial は empty list (raikiri-spike-m5.3)。
            // d9y.1: same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // CSS GCPM 3 §1.2.1: position: running() seed initial は empty
            // (position の initial は `static`、running(name) 無し)。
            running_templates: Vec::new(),
            // CSS Box 3 §3.1: margin-* physical の initial は `0` (`Sides::all(0)`
            // で全 4 side に spread)。raikiri-spike-0vv.5。
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
        }
    }

    /// 親 node の computed values から child node の 「inheritance walk 開始値」
    /// を生成する。
    ///
    /// - **inherited** property (color / font-family / font-size / font-weight)
    ///   は親からコピー
    /// - **non-inherited** property (display) は `initial()` と同じ値を保持
    ///
    /// 新 property を追加する際は分類に応じてこの struct 直下の該当行を追加する
    /// (inherited なら parent からのコピー、non-inherited なら初期値を直接指定)。
    /// initial 値との drift を避けるため、対応する `initial()` の値も同時に更新
    /// すること。
    /// (spec §M1.4a、raikiri-spike-m1.22)
    pub fn inherit_from(parent: &Self) -> Self {
        // 直接 struct literal で初期化する — Self::initial() 経由だと
        // font_family の Vec を 1 度 allocate → drop してから parent から
        // clone し直すことになり無駄 (roborev job 217 medium 対応)。
        Self {
            // inherited (親からコピー)
            color: parent.color,
            font_family: parent.font_family.clone(),
            font_size: parent.font_size,
            font_weight: parent.font_weight,
            // non-inherited (initial 値、CSS §9.2.4 initial value of display)
            display: DisplayValue::Inline,
            // non-inherited (CSS Lists 3 §3、raikiri-spike-s85)。
            // d9y.2: shared empty Arc slot (`empty_counter_entries`)、per-node
            // allocation 回避。inherit_from は child stack entry のたびに走る
            // ため、`Vec::new()` を直に書くと 1-doc あたり 3 × N 個の Vec
            // struct が生まれる (advisor calibration、d9y.1 content/string_set
            // pattern と同 rationale)。
            counter_reset: empty_counter_entries(),
            counter_increment: empty_counter_entries(),
            counter_set: empty_counter_entries(),
            // non-inherited (CSS Content 3 §2.1、raikiri-spike-m5.1)。
            // d9y.1: shared empty Arc slot (`empty_content_list`)、per-node
            // allocation 回避。inherit_from は child stack entry のたびに走る
            // ため、Arc::new(Vec::new()) を直に書くと 1-doc あたり O(N) 個の
            // small heap alloc regression になる (advisor calibration)。
            content: empty_content_list(),
            // non-inherited (CSS GCPM 3 §3.1、raikiri-spike-m5.3)。
            // d9y.1: same shared-empty-Arc pattern。
            string_set: empty_string_set_entries(),
            // non-inherited (CSS GCPM 3 §1.2.1、raikiri-spike-m5.4)
            running_templates: Vec::new(),
            // non-inherited (CSS Box 3 §3.1 "Inherited: no")。initial 値と drift
            // しないよう `Self::initial()` と同 shape で 0 spread。raikiri-spike-0vv.5。
            margin: Sides::all(LengthOrAuto::Length(Length::Px(0.0))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_values_match_spec() {
        let cv = ComputedValues::initial();
        assert_eq!(cv.color, CssColor::BLACK);
        assert_eq!(cv.font_family, vec![Atom::from("serif")]);
        assert_eq!(cv.font_size, Length::Px(16.0));
        assert_eq!(cv.font_weight, 400);
        assert_eq!(cv.display, DisplayValue::Inline);
        // CSS Lists 3 §3: counter-* initial は empty list (raikiri-spike-s85)
        assert!(cv.counter_reset.is_empty());
        assert!(cv.counter_increment.is_empty());
        assert!(cv.counter_set.is_empty());
        // CSS Content 3 §2.1 + CSS GCPM 3 §3.1 (raikiri-spike-m5.1 / m5.3)
        assert!(cv.content.is_empty());
        assert!(cv.string_set.is_empty());
        // CSS GCPM 3 §1.2.1 (raikiri-spike-m5.4): position initial は `static` →
        // running() seed 無し。
        assert!(cv.running_templates.is_empty());
        // CSS Box 3 §3.1 (raikiri-spike-0vv.5): margin initial は 0 on each side。
        assert_eq!(cv.margin, Sides::all(LengthOrAuto::Length(Length::Px(0.0))));
    }

    #[test]
    fn computed_values_is_send_and_clone() {
        fn assert_send<T: Send>() {}
        fn assert_clone<T: Clone>() {}
        assert_send::<ComputedValues>();
        assert_clone::<ComputedValues>();
    }

    // ── display + inherit_from (M1.4a、raikiri-spike-m1.22) ─────

    #[test]
    fn initial_display_is_inline() {
        // CSS §9.2.4: initial value of display is inline
        assert_eq!(ComputedValues::initial().display, DisplayValue::Inline);
    }

    #[test]
    fn inherit_from_copies_inherited_fields() {
        let parent = ComputedValues {
            color: CssColor {
                r: 200,
                g: 100,
                b: 50,
                a: 255,
            },
            font_family: vec![Atom::from("sans-serif")],
            font_size: Length::Px(24.0),
            font_weight: 700,
            display: DisplayValue::Block,
            // d9y.2: counter-* は Arc<Vec<..>>、fixture literal は Arc::new(vec![..]) で包む。
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            content: empty_content_list(),
            string_set: empty_string_set_entries(),
            running_templates: Vec::new(),
            // 0vv.5: parent に explicit margin を持たせ、child が initial に落ちる
            // ことを他 non-inherited fixture (下の inherit_from_leaves_* 系) で pin。
            margin: Sides::all(LengthOrAuto::Length(Length::Px(12.0))),
        };
        let child = ComputedValues::inherit_from(&parent);
        // inherited: 親からコピー
        assert_eq!(child.color, parent.color);
        assert_eq!(child.font_family, parent.font_family);
        assert_eq!(child.font_size, parent.font_size);
        assert_eq!(child.font_weight, parent.font_weight);
    }

    #[test]
    fn inherit_from_leaves_counter_properties_at_initial() {
        // CSS Lists 3 §3: counter-reset / counter-increment / counter-set は
        // non-inherited → 親が値を持っていても child は empty (initial) となる
        // (raikiri-spike-s85)
        // d9y.2: 親 fixture の counter-* は Arc<Vec<..>> になったため Arc::new でラップ。
        let parent = ComputedValues {
            counter_reset: Arc::new(vec![(SmolStr::new("chapter"), 3)]),
            counter_increment: Arc::new(vec![(SmolStr::new("section"), 2)]),
            counter_set: Arc::new(vec![(SmolStr::new("page"), 5)]),
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert!(child.counter_reset.is_empty());
        assert!(child.counter_increment.is_empty());
        assert!(child.counter_set.is_empty());
    }

    #[test]
    fn inherit_from_leaves_display_at_initial() {
        // display は non-inherited → 親が Block でも child は Inline (initial)
        let parent = ComputedValues {
            display: DisplayValue::Block,
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(child.display, DisplayValue::Inline);
    }

    #[test]
    fn inherit_from_leaves_margin_at_initial() {
        // CSS Box 3 §3.1 "Inherited: no" — 親が margin を持っていても child は
        // initial (0 on each side) に戻る (raikiri-spike-0vv.5)。37n sibling:
        // display / counter-* / content / string_set / running_templates と同 shape。
        let parent = ComputedValues {
            margin: Sides {
                top: LengthOrAuto::Length(Length::Px(10.0)),
                right: LengthOrAuto::Auto,
                bottom: LengthOrAuto::Length(Length::Percent(50.0)),
                left: LengthOrAuto::Length(Length::Em(2.0)),
            },
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert_eq!(
            child.margin,
            Sides::all(LengthOrAuto::Length(Length::Px(0.0)))
        );
    }

    #[test]
    fn inherit_from_leaves_running_templates_at_initial() {
        // CSS GCPM 3 §1.2.1: position property は non-inherited (CSS Positioned
        // Layout 由来)。親が running(hdr) を持っていても child は initial (empty)。
        // 37n sibling pattern (string_set / content / counter-* non-inheritance
        // test を踏襲、raikiri-spike-m5.4)。
        let parent = ComputedValues {
            running_templates: vec![RunningTemplate {
                name: SmolStr::new("hdr"),
            }],
            ..ComputedValues::initial()
        };
        let child = ComputedValues::inherit_from(&parent);
        assert!(child.running_templates.is_empty());
    }
}
