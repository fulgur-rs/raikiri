//! Per-node computed CSS values (M1.4 scope: 4 inherited properties)。
//!
//! Cascade + inheritance walk が populate。M1.6 で ComputedValues → taffy::Style
//! + paint 用色情報の抽出 layer が入る予定。

use crate::property::{CssColor, Length};
use crate::Atom;

/// Per-node computed style。M1.4 では 4 property のみ (全て inherited)。
///
/// `#[non_exhaustive]` により future property (background-color / display /
/// margin / padding / width / height 等) の追加が non-breaking。
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq)]
pub struct ComputedValues {
    /// `color`。initial: opaque black。
    pub color: CssColor,
    /// `font-family` — 優先順位順。initial: `[Atom::from("serif")]`。
    pub font_family: Vec<Atom>,
    /// `font-size`。initial: `Length::Px(16.0)` (browser default medium)。
    pub font_size: Length,
    /// `font-weight`。initial: 400 (normal)。
    pub font_weight: u16,
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
    }

    #[test]
    fn computed_values_is_send_and_clone() {
        fn assert_send<T: Send>() {}
        fn assert_clone<T: Clone>() {}
        assert_send::<ComputedValues>();
        assert_clone::<ComputedValues>();
    }
}
