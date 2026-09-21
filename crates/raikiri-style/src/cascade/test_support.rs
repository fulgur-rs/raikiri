//! `#[cfg(test)]`-only helpers shared by 3 or more of `cascade`'s submodule
//! test modules. A helper used by only one submodule's tests lives directly
//! in that submodule's own `mod tests` instead — this module exists purely
//! to avoid duplicating a helper body across multiple files.

use crate::computed::ComputedValues;
use crate::property::CssColor;
use crate::ruletree::{Origin, build_rule_tree};
use crate::test_dom::TestDoc;

use super::cascade;

pub(crate) fn cascade_doc(css: &str, tag: &str, inline: Option<&str>) -> ComputedValues {
    let mut doc = TestDoc::new();
    if !css.is_empty() {
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, css);
    }
    let e = doc.push_element(0, tag, inline);
    let tree = build_rule_tree(&doc);
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[e].clone()
}

pub(crate) const RED: CssColor = CssColor {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};

pub(crate) const BLUE: CssColor = CssColor {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};

pub(crate) fn cascade_with_ua(
    ua_css: &str,
    author_css: &str,
    target_tag: &str,
    inline: Option<&str>,
) -> ComputedValues {
    // UA rule + Author rule + inline を一気に組み立てて cascade 実行
    let mut doc = TestDoc::new();
    // Author の <style> は DOM 側から build_rule_tree に読ませる
    if !author_css.is_empty() {
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, author_css);
    }
    let e = doc.push_element(0, target_tag, inline);

    // build_rule_tree (Author 集約) + UA add_stylesheet
    let mut tree = build_rule_tree(&doc);
    // UA CSS を先頭に inject するのではなく、既存の Author rule の後ろに
    // add してから rank 化で origin 順序を担保する (source_order より rank
    // が優位)
    // ただし現状 add_stylesheet の呼び出し順で source_order が振られ
    // Author が先 (source_order 小)、UA が後 (source_order 大) となる。
    // rank 化により Origin::UserAgent の Normal は Origin::Author の
    // Normal より常に低い rank になる (`cascade_rank` doc に正確な値
    // あり) ので UA rule が Author を上書きすることはない (source_order
    // に関わらず rank が優先)。
    if !ua_css.is_empty() {
        tree.add_stylesheet(ua_css, Origin::UserAgent);
    }
    let result = cascade(&doc, &tree).expect("cascade Ok");
    result.computed[e].clone()
}
