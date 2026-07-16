//! raikiri — umbrella crate: primary consumer API and re-exports.
//!
//! M1 の umbrella-facade slice。Consumer が単一 `raikiri` crate だけを dep に
//! 追加すれば HTML parse → cascade された ComputedValues まで得られるように
//! sub-crate から必要な type / trait / function を re-export し、cascade
//! orchestration entry point `build_cascaded` を提供する
//! (spec §M1、raikiri-spike-m1.23)。
//!
//! # Example
//!
//! ```
//! use raikiri::{build_cascaded, parse, ParseOptions};
//!
//! let opts = ParseOptions { extra_stylesheets: &[], network: None, base_url: None };
//! let doc = parse(&b"<p>Hi</p>"[..], &opts).expect("parse");
//! let result = build_cascaded(&doc);
//! // result.computed に per-node ComputedValues が populate される。
//! # let _ = result;
//! ```

use raikiri_style::{Origin, RuleTree, cascade, walk_style_elements};

// ── raikiri-traits: shared vocabulary + DOM traits + error taxonomy ────
pub use raikiri_traits::{
    CascadeError, Dom, Element, Node, NodeId, NodeKind, ParseError, QuirksMode, RenderError,
    RenderWarning, StylesheetKind,
};

// ── raikiri-html: parse pipeline entry ─────────────────────────────────
pub use raikiri_html::{MINIMAL_UA_CSS, ParseOptions, UncascadedDocument, parse};

// ── raikiri-dom: Document (owns DOM + stylesheets state) ───────────────
pub use raikiri_dom::Document as DomDocument;

// ── raikiri-style: cascade pipeline output types ───────────────────────
pub use raikiri_style::{
    CascadeResult, ComputedValues, DisplayValue, Origin as CascadeOrigin, PropertyValue,
    RuleTree as StyleRuleTree,
};

/// UA + Consumer 提供 stylesheet を Document から取り出し、Origin を割り当てて
/// RuleTree を組み、DOM 内 `<style>` element の text を Author として追加した
/// 上で cascade を実行する umbrella orchestration entry point
/// (spec §M1.4a、raikiri-spike-m1.23)。
///
/// M1 では `raikiri_style::cascade` は常に `Ok` を返すため、内部で `expect` する
/// (M2+ で Result 反映を検討)。
///
/// Consumer は `raikiri_html::parse` → `raikiri::build_cascaded` の 2 step だけで
/// per-node ComputedValues を得られる。
///
/// # Dep 方向
///
/// `StylesheetKind → Origin` の翻訳は raikiri-html にも raikiri-style にも置かず、
/// umbrella (本 crate) 内で明示的に書く。これにより下位 crate 間の逆依存を発生
/// させない (raikiri-spike-m1.23 Acceptance #2)。
pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult {
    let mut tree = RuleTree::empty();

    // Document に associate されている全 stylesheet を kind に応じて Origin
    // に map。呼び出し順 (=注入順) が cascade の source_order を決める。
    for (source, kind) in doc.dom.stylesheets() {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // DOM 内 `<style>` element の text を Author として追加。UA / extra_stylesheets
    // は上のループで既に取り込まれているため、ここでは重複しない。
    walk_style_elements(&doc.dom, |css| {
        tree.add_stylesheet(css, Origin::Author);
    });

    cascade(&doc.dom, &tree).expect("m1 では cascade は常に Ok")
}

/// dom-level の [`StylesheetKind`] (raikiri-traits) を cascade-level の
/// [`Origin`] (raikiri-style) に翻訳。dep 方向を保つため umbrella 内で保持。
///
/// `#[non_exhaustive]` により将来 variant が追加された場合の compile-time 網羅
/// 保証を維持する。
fn stylesheet_kind_to_origin(kind: StylesheetKind) -> Origin {
    match kind {
        StylesheetKind::UserAgent => Origin::UserAgent,
        StylesheetKind::Author => Origin::Author,
        // `StylesheetKind` は `#[non_exhaustive]`。将来 User 等が追加された時点で
        // Origin 側も対応が必要になるため、ここは compile-time 網羅性を保持する。
        _ => Origin::Author,
    }
}

/// Cascade orchestration が Document 内 `<style>` を Author 経路で使うこと、および
/// Document.stylesheets 経由の UA CSS がここに二重計上されないことを再確認する
/// smoke-test は tests/build_cascaded.rs 側で担当する。
#[cfg(test)]
mod smoke_tests {
    use super::*;

    #[test]
    fn stylesheet_kind_to_origin_matches_spec() {
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::UserAgent),
            Origin::UserAgent,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::Author),
            Origin::Author,
        );
    }
}
