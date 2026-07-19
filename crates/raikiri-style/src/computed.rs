//! Per-node computed CSS values (M1.4 scope: 4 inherited properties)。
//!
//! Cascade + inheritance walk が populate。M1.6 で ComputedValues → taffy::Style
//! + paint 用色情報の抽出 layer が入る予定。

use smol_str::SmolStr;

use crate::Atom;
use crate::property::{ContentComponent, CssColor, DisplayValue, Length};

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
    pub counter_reset: Vec<(SmolStr, i32)>,
    /// `counter-increment`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + increment pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    pub counter_increment: Vec<(SmolStr, i32)>,
    /// `counter-set`。**non-inherited**、initial: empty list (CSS Lists 3 §3)。
    /// counter-name + value pairs。M5 pre-work (raikiri-spike-s85)、
    /// counter tree resolve は M5 本編。
    pub counter_set: Vec<(SmolStr, i32)>,
    /// `content` の resolved 中間表現。**non-inherited**、initial: empty list
    /// (spec §2.1 "content" property の `normal` / `none` を空 list として扱う
    /// — 本 crate は cascade static side、pseudo-element 生成判断は下流 layer)。
    /// M5 gcpm-directive-emit (raikiri-spike-m5.1)。
    /// 下流 (raikiri-dom) が `raikiri_traits::ContentValueItem` に mapping する
    /// (raikiri-style は raikiri-traits に依存しない leaf crate = 3ps/94e Phase B、
    /// counter-* wire-through pattern を踏襲、raikiri-spike-s85)。
    /// See <https://www.w3.org/TR/css-content-3/#content-property>.
    pub content: Vec<ContentComponent>,
    /// `string-set` の parse 結果 — `(name, content-list)` entry の列。
    /// **non-inherited**、initial: empty list (CSS GCPM 3 §3.1)。
    /// 名前解決と runtime `string()` 参照は下流 (raikiri-dom) 責務。
    /// See <https://www.w3.org/TR/css-gcpm-3/#propdef-string-set>.
    pub string_set: Vec<(SmolStr, Vec<ContentComponent>)>,
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
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            counter_set: Vec::new(),
            // CSS Content 3 §2.1: content initial (normal) は下流にとって「no
            // generated content」= empty list として扱う (raikiri-spike-m5.1)。
            content: Vec::new(),
            // CSS GCPM 3 §3.1: string-set initial は empty list (raikiri-spike-m5.3)。
            string_set: Vec::new(),
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
            // non-inherited (CSS Lists 3 §3、raikiri-spike-s85)
            counter_reset: Vec::new(),
            counter_increment: Vec::new(),
            counter_set: Vec::new(),
            // non-inherited (CSS Content 3 §2.1、raikiri-spike-m5.1)
            content: Vec::new(),
            // non-inherited (CSS GCPM 3 §3.1、raikiri-spike-m5.3)
            string_set: Vec::new(),
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
            counter_reset: vec![(SmolStr::new("chapter"), 3)],
            counter_increment: vec![(SmolStr::new("section"), 2)],
            counter_set: vec![(SmolStr::new("page"), 5)],
            content: Vec::new(),
            string_set: Vec::new(),
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
        let parent = ComputedValues {
            counter_reset: vec![(SmolStr::new("chapter"), 3)],
            counter_increment: vec![(SmolStr::new("section"), 2)],
            counter_set: vec![(SmolStr::new("page"), 5)],
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
}
