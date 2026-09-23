//! Cascade orchestration over a parsed [`UncascadedDocument`].
//!
//! Assembles the UA, user, and author stylesheets associated with a parsed
//! document into a [`RuleTree`] and runs the element and `@page` cascades.

use raikiri_style::{
    CascadeResult, ConsumerPropertyRegistration, MediaContext, Origin, PageContextQuery, RuleTree,
    cascade_with_media_context_for_page,
};
use raikiri_traits::StylesheetKind;

use crate::UncascadedDocument;

/// UA + Consumer 提供 stylesheet を Document から取り出し、Origin を割り当てて
/// RuleTree を組み、raikiri-html が parse 時に集約した inline `<style>` element
/// と fetched `<link rel="stylesheet">` source を Author として追加した上で cascade
/// を実行する cascade orchestration entry point。
///
/// 現状 `raikiri_style::cascade` は常に `Ok` を返すため、内部で `expect` する
/// (将来 Result 反映を検討)。
///
/// Consumer は [`crate::parse`] → [`build_cascaded`] の 2 step だけで
/// per-node ComputedValues を得られる。
///
/// # DOM `<style>` の集約 scope
///
/// `UncascadedDocument::stylesheet_sources` を Author として消費する。
/// この Vec は parse 時に [`crate::parse`] 内の `extract_inline_stylesheets`
/// が head の stylesheet-bearing elements を元の順序で集約し、その後に
/// head 外の inline `<style>` elements を document order で追加する。
/// HTML/XHTML と SVG の `<style>` は対象だが、MathML の同名 element は対象外。
/// `<template>` subtree は spec §14.1 の inertness に従って skip 済み。
///
/// # DOM `<style>` (Author) vs `extra_stylesheets` (User)
///
/// `Document.stylesheets()` (parse 時に注入された UA + `extra_stylesheets`) が
/// 先に RuleTree に流し込まれ、次に `stylesheet_sources` (head/body の inline
/// styles と head の fetched links) が Author として追加される。従来は
/// `extra_stylesheets`
/// も `Author` としてタグされており、DOM `<style>` との勝敗は同一 origin 内の
/// source_order tie-break (後から来た方が勝つ) に依存していた。その後
/// `extra_stylesheets` は [`Origin::User`] に retag された
/// ため、両者はもはや同一 origin ではない — 勝敗は origin rank の差で
/// specificity / source_order を問わず決まる。
///
/// **normal 同士なら** [`Origin::Author`] (normal rank 3) > [`Origin::User`]
/// (normal rank 1) なので **DOM `<style>` が `extra_stylesheets` を上書きする**
/// — 旧実装判断が偶然同一 origin tie-break で
/// 実現していたのと同じ勝敗だが、根拠が「同 origin tie-break」から「別
/// origin の rank 差」に変わった。
///
/// **`!important` が絡むとこの勝敗は反転しうる** (CSS Cascading L4 §6.3 の
/// importance による origin 順反転)。`extra_stylesheets` 側が `!important`
/// を持てば ([`Origin::User`] important rank 6) DOM `<style>` 側の
/// importance に関係なく (`Author` は normal rank 3 / important rank 4、
/// いずれも 6 未満) `extra_stylesheets` が勝つ。逆に `extra_stylesheets` 側
/// が normal (rank 1) なら DOM `<style>` は normal/important いずれでも
/// (rank 3 / 4、いずれも 1 より上) 勝つ — 実質、勝敗は `extra_stylesheets`
/// 側の importance だけで決まる。
///
/// # Dep 方向
///
/// `StylesheetKind → Origin` の翻訳は raikiri-style にも raikiri-dom にも置かず、
/// 両者の上位にある本 crate (raikiri-html) 内で明示的に書く。これにより下位
/// crate 間の逆依存を発生させない。
pub fn build_cascaded(doc: &UncascadedDocument) -> CascadeResult {
    build_cascaded_with_media_context(doc, &MediaContext::default())
}

/// Build a cascade that retains the supplied consumer-owned properties.
pub fn build_cascaded_with_consumer_properties(
    doc: &UncascadedDocument,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> CascadeResult {
    build_cascaded_with_media_context_for_page_and_consumer_properties(
        doc,
        &MediaContext::default(),
        &PageContextQuery::default(),
        consumer_properties,
    )
}

/// Build the cascade for one page-context query using the default media context.
pub fn build_cascaded_for_page(
    doc: &UncascadedDocument,
    page_query: &PageContextQuery,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page(doc, &MediaContext::default(), page_query)
}

/// Build the rule tree and run the cascade for an explicit media context.
///
/// [`build_cascaded`] remains the compatibility entry point and uses the
/// default paged (`print`) context.
pub fn build_cascaded_with_media_context(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page(doc, media_context, &PageContextQuery::default())
}

/// Build the element and `@page` cascades for one page-context query.
///
/// The first-page render path uses this entry point with `is_first` and
/// `is_right` set. A future page-stream driver can call it once per page with
/// the page name and pseudo-page state selected by its break algorithm.
pub fn build_cascaded_with_media_context_for_page(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
) -> CascadeResult {
    build_cascaded_with_media_context_for_page_and_consumer_properties(
        doc,
        media_context,
        page_query,
        &[],
    )
}

/// Build a page-aware cascade while retaining registered consumer properties.
///
/// Registration is optional and has no effect on the compatibility cascade.
/// Registered properties are parsed into the existing inherited custom-property
/// environment, then exposed through the neutral observer at render time.
pub fn build_cascaded_with_media_context_for_page_and_consumer_properties(
    doc: &UncascadedDocument,
    media_context: &MediaContext,
    page_query: &PageContextQuery,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> CascadeResult {
    let tree = build_rule_tree_with_consumer_properties(doc, consumer_properties);
    cascade_with_media_context_for_page(&doc.dom, &tree, media_context, page_query)
        .expect("cascade は常に Ok のはず")
}

/// Build the stylesheet rule tree used by the document cascade.
///
/// Keeping this operation separate lets a paged renderer retain the parsed
/// `@page` rules while it performs a per-page cascade in a later page loop.
pub fn build_rule_tree(doc: &UncascadedDocument) -> RuleTree {
    build_rule_tree_with_consumer_properties(doc, &[])
}

/// Build a rule tree configured for the supplied consumer-owned properties.
pub fn build_rule_tree_with_consumer_properties(
    doc: &UncascadedDocument,
    consumer_properties: &[ConsumerPropertyRegistration],
) -> RuleTree {
    let mut tree = RuleTree::empty_with_consumer_properties(consumer_properties);

    // Document に associate されている全 stylesheet を kind に応じて Origin
    // に map。呼び出し順 (=注入順) が cascade の source_order を決める。
    for (source, kind) in doc.dom.stylesheets() {
        let origin = stylesheet_kind_to_origin(kind);
        tree.add_stylesheet(source, origin);
    }

    // raikiri-html が parse 時に template-inert filter 越しに集約した
    // head/body inline style と fetched head links を Author として追加。
    for source in &doc.stylesheet_sources {
        tree.add_stylesheet(source, Origin::Author);
    }

    tree
}

/// dom-level の [`StylesheetKind`] (raikiri-traits) を cascade-level の
/// [`Origin`] (raikiri-style) に翻訳。dep 方向を保つため本 crate 内で保持。
///
/// `StylesheetKind` は他 crate の `#[non_exhaustive]` enum のため exhaustive match
/// はできないが、将来 variant が追加された場合の silent misroute を防ぐため
/// `_` arm は `unreachable!` で loud fail させる (現時点で
/// UserAgent / User / Author の 3 variant で網羅済み)。
fn stylesheet_kind_to_origin(kind: StylesheetKind) -> Origin {
    match kind {
        StylesheetKind::UserAgent => Origin::UserAgent,
        StylesheetKind::User => Origin::User,
        StylesheetKind::Author => Origin::Author,
        // `StylesheetKind` は `#[non_exhaustive]`。現時点で
        // UserAgent / User / Author の 3 variant を上で網羅済み。将来別の variant が
        // 追加された時点で対応が漏れるとここに到達し、silent misroute を防ぐため
        // panic で loud fail する (dev が cascade origin map の更新に気付ける)。
        // cov:ignore: defensive `_` arm for a cross-crate `#[non_exhaustive]` enum —
        // unreachable by construction while all 3 current variants are matched above;
        // only becomes reachable if a future variant is added upstream without a
        // corresponding arm here (the panic message tells the dev to add one).
        _ => unreachable!(
            "StylesheetKind variant not yet mapped to Origin — update stylesheet_kind_to_origin in raikiri-html crate"
        ),
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
            stylesheet_kind_to_origin(StylesheetKind::User),
            Origin::User,
        );
        assert_eq!(
            stylesheet_kind_to_origin(StylesheetKind::Author),
            Origin::Author,
        );
    }
}
