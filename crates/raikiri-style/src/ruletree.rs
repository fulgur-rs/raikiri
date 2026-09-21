//! Unified rule tree — cascade 側と将来の GCPM 解決側が共有する index。
//! style_rules を populate。@page at-rule も (silently skip せず)
//! [`RuleTree::page_rules`] に格納する (cascade 適用は未実装)。
//! at-rule の generic parse view は、専用の意味論が未実装のもの
//! (@supports / @import 等) を [`RuleTree::opaque_at_rules`] に raw data として
//! 保持する。`@media` もこの raw view に保持しつつ、対応する media type の
//! qualified rule は cascade 用の専用 view に展開する。専用 view を持つ
//! `@counter-style` も raw/source order の確認用にこの viewへ記録し、`@page`
//! は既存の page view に保持する。

use cssparser::{Parser, ParserInput, StyleSheetParser, Token};
use selectors::parser::{ParseRelative, Selector, SelectorList};

use crate::counter_style::{CounterStyleRegistry, parse_counter_style_rules};
use crate::font_face::{FontFaceRegistry, parse_font_face_rules};
use crate::media::{MediaCondition, MediaRule, parse_media_condition};
use crate::page::{
    PageBlockBody, PageRule, PageSelector, parse_page_declaration_block, parse_page_prelude,
};
use crate::property::parse_value;
use crate::rule::{Declaration, StyleRule, parse_declaration_block};
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};
use crate::{PseudoClass, PseudoElem, RaikiriSelectorImpl, RaikiriSelectorParser};

/// Flatten the subset of cascade layers that the rule parser can evaluate.
///
/// The generic rule parser stores unsupported at-rules as opaque records, which
/// would otherwise hide `@page` and ordinary style rules nested in `@layer`.
/// Flattening here keeps the existing parser and rule-tree representation while
/// ordering normal layers from lower to higher precedence.  Unlayered rules are
/// appended last, as required by the normal cascade.  This is intentionally a
/// small source-level pass; nested blocks, strings, and comments are skipped
/// while locating only top-level layer blocks.
/// Remove supported `@supports` blocks before the generic stylesheet parser.
///
/// The rule tree intentionally retains unknown at-rules as opaque records, but a
/// supported condition must expose its qualified rules to the normal cascade.
/// This small source pass handles declaration conditions and `not`/`and`/`or`
/// combinations while preserving unsupported blocks verbatim for inspection.
fn expand_supports(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut plain_start = 0;
    let mut cursor = 0;
    while let Some(start) = find_top_level_at_rule(source, cursor, "supports") {
        let Some((delimiter, delimiter_index)) = find_layer_delimiter(source, start + 9) else {
            break;
        };
        if delimiter != b'{' {
            break;
        }
        let Some(close) = matching_brace(source, delimiter_index) else {
            break;
        };
        output.push_str(&source[plain_start..start]);
        let condition = &source[start + 9..delimiter_index];
        let body = &source[delimiter_index + 1..close];
        if supports_condition(condition) && !body.trim_start().starts_with('@') {
            // Keep the opaque record for inspection, and prepend its qualified
            // rules as ordinary stylesheet input for the cascade.
            output.push_str(&expand_supports(body));
            output.push_str(&source[start..=close]);
        } else {
            output.push_str(&source[start..=close]);
        }
        cursor = close + 1;
        plain_start = cursor;
    }
    output.push_str(&source[plain_start..]);
    output
}

fn supports_condition(raw: &str) -> bool {
    let condition = raw.trim();
    if let Some(rest) = condition.strip_prefix("not ") {
        return !supports_condition(rest);
    }
    if let Some((left, right)) = split_supports_operator(condition, " or ") {
        return supports_condition(left) || supports_condition(right);
    }
    if let Some((left, right)) = split_supports_operator(condition, " and ") {
        return supports_condition(left) && supports_condition(right);
    }
    let condition = condition
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .map(str::trim)
        .unwrap_or(condition);
    let Some((name, value)) = condition.split_once(':') else {
        return false;
    };
    let name = name.trim();
    let value = value.trim();
    if name.is_empty() || value.is_empty() {
        return false;
    }
    let mut input = ParserInput::new(value);
    let mut parser = Parser::new(&mut input);
    parser
        .parse_entirely(|input| {
            parse_value(name, input).ok_or_else(|| input.new_custom_error::<_, ()>(()))
        })
        .is_ok()
}

fn split_supports_operator<'a>(value: &'a str, operator: &str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0_u32;
    let mut index = 0;
    while index + operator.len() <= value.len() {
        match value.as_bytes()[index] {
            b'(' => depth = depth.saturating_add(1),
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        if depth == 0 && value[index..].starts_with(operator) {
            return Some((&value[..index], &value[index + operator.len()..]));
        }
        index += 1;
    }
    None
}

fn find_top_level_at_rule(source: &str, from: usize, name: &str) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        match byte {
            b'{' => depth = depth.saturating_add(1),
            b'}' => depth = depth.saturating_sub(1),
            b'@' if depth == 0
                && bytes[index + 1..].len() >= name.len()
                && bytes[index + 1..]
                    .get(..name.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name.as_bytes()))
                && bytes
                    .get(index + 1 + name.len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'-') =>
            {
                return Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn expand_cascade_layers(source: &str) -> Vec<(u32, String)> {
    let mut cursor = 0;
    let mut plain_start = 0;
    let mut unlayered = String::with_capacity(source.len());
    let mut blocks: Vec<(String, String)> = Vec::new();
    let mut declared_order = Vec::new();
    let mut found_layer = false;

    while let Some(start) = find_top_level_layer(source, cursor) {
        unlayered.push_str(&source[plain_start..start]);
        let prelude_start = start + "@layer".len();
        let Some((delimiter, delimiter_index)) = find_layer_delimiter(source, prelude_start) else {
            unlayered.push_str(&source[start..]);
            break;
        };
        let prelude = source[prelude_start..delimiter_index].trim();
        match delimiter {
            b';' => {
                for name in prelude
                    .split(',')
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                {
                    if !declared_order.iter().any(|existing| existing == name) {
                        declared_order.push(name.to_owned());
                    }
                }
                cursor = delimiter_index + 1;
                plain_start = cursor;
                found_layer = true;
            }
            b'{' => {
                let Some(close) = matching_brace(source, delimiter_index) else {
                    unlayered.push_str(&source[start..]);
                    break;
                };
                if !prelude.is_empty() {
                    blocks.push((
                        prelude.to_owned(),
                        source[delimiter_index + 1..close].to_owned(),
                    ));
                    found_layer = true;
                    cursor = close + 1;
                    plain_start = cursor;
                } else {
                    unlayered.push_str(&source[start..=close]);
                    cursor = close + 1;
                    plain_start = cursor;
                }
            }
            _ => {
                unlayered.push_str(&source[start..]);
                break;
            }
        }
    }

    if !found_layer {
        return vec![(u32::MAX, source.to_owned())];
    }
    unlayered.push_str(&source[plain_start..]);

    let mut order = declared_order;
    for (name, _) in &blocks {
        if !order.iter().any(|existing| existing == name) {
            order.push(name.clone());
        }
    }
    let mut chunks = Vec::with_capacity(order.len() + 1);
    for (layer_order, name) in order.into_iter().enumerate() {
        let mut body = String::new();
        for (block_name, block_body) in &blocks {
            if block_name == &name {
                body.push_str(block_body);
                body.push('\n');
            }
        }
        chunks.push((layer_order as u32, body));
    }
    chunks.push((u32::MAX, unlayered));
    chunks
}

fn find_top_level_layer(source: &str, from: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut depth = 0_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            index += 1;
            continue;
        }
        match byte {
            b'{' => depth = depth.saturating_add(1),
            b'}' => depth = depth.saturating_sub(1),
            b'@' if depth == 0
                && source[index..].len() >= "@layer".len()
                && source[index..index + "@layer".len()].eq_ignore_ascii_case("@layer")
                && source
                    .as_bytes()
                    .get(index + "@layer".len())
                    .is_none_or(|next| !next.is_ascii_alphanumeric() && *next != b'-') =>
            {
                return Some(index);
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn find_layer_delimiter(source: &str, from: usize) -> Option<(u8, usize)> {
    let bytes = source.as_bytes();
    let mut index = from;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'{' || byte == b';' {
            return Some((byte, index));
        }
        index += 1;
    }
    None
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut index = open + 1;
    let mut depth = 1_u32;
    let mut quote = None;
    let mut comment = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if comment {
            if byte == b'*' && bytes.get(index + 1) == Some(&b'/') {
                comment = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            if byte == b'\\' {
                index = index.saturating_add(2);
            } else {
                if byte == delimiter {
                    quote = None;
                }
                index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'*') {
            comment = true;
            index += 2;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
        } else if byte == b'{' {
            depth = depth.saturating_add(1);
        } else if byte == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

/// Cascade origin (CSS Cascading L4 §6.2)。
///
/// [`Origin::AuthorPresentationalHint`] は CSS Cascading L5 §6.5 "Precedence
/// of Non-CSS Presentational Hints"
/// (<https://drafts.csswg.org/css-cascade-5/#preshint>) が定める "author
/// presentational hint origin" (user origin と author origin の間に位置する
/// 独立 origin) に対応する。この origin の rank 上の位置付けは
/// [`crate::cascade::cascade_rank`] の doc 参照。
///
/// [`Origin::User`] は CSS Cascading L4 §6.2
/// <https://www.w3.org/TR/css-cascade-4/#cascading-origins> が定める "user
/// origin" に対応する。raikiri-style 内では variant 自体は完全に機能する
/// ([`crate::cascade::cascade_rank`] の 4-tier ordering に組み込み済み)。
/// consumer 提供の `extra_stylesheets` はこの origin へ route される —
/// raikiri-traits 側の `StylesheetKind::User` variant + raikiri-html 側の
/// retag + umbrella 側の `stylesheet_kind_to_origin` 拡張という 3-crate の
/// integration を経て、`extra_stylesheets` は実際に [`Origin::User`] へ届く。
///
/// `StylesheetKind` (dom-level tag) との対応は raikiri umbrella crate が
/// cascade orchestration の一部として map する。
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    UserAgent,
    /// CSS Cascading L4 §6.2 "user origin"
    /// (<https://www.w3.org/TR/css-cascade-4/#cascading-origins>)。Consumer
    /// が `ParseOptions::extra_stylesheets` 経由で提供する CSS がここに route
    /// される (raikiri-html の `StylesheetKind::User` retag 経由)。
    User,
    /// CSS Cascading L5 §6.5 "author presentational hint origin"
    /// (<https://drafts.csswg.org/css-cascade-5/#preshint>) — HTML
    /// presentational hint (`<img width>`/`<img height>` 等) 用の
    /// user origin と author origin の間の独立 origin。
    AuthorPresentationalHint,
    Author,
}

/// A syntactically valid at-rule body retained for a consumer that does not
/// yet implement the at-rule's semantics.
///
/// The contents exclude the outer braces. `Statement` represents an at-rule
/// terminated by `;` (or by the end of the stylesheet, which CSS syntax also
/// permits). `Block` retains the raw component-value text inside `{ ... }`.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AtRuleBody {
    /// The at-rule had no block body.
    Statement,
    /// The raw contents of the at-rule's curly-bracket block.
    Block(String),
}

impl AtRuleBody {
    /// Return the block contents, or `None` for a statement at-rule.
    pub fn as_block(&self) -> Option<&str> {
        match self {
            Self::Statement => None,
            Self::Block(body) => Some(body),
        }
    }
}

/// A raw qualified rule found inside an opaque at-rule block.
///
/// The prelude and body are intentionally not interpreted. A later semantic
/// pass can parse them according to that at-rule's grammar, while an
/// inspector can still walk the nested rule list without reparsing bytes.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualifiedRuleRecord {
    /// Raw component-value text before the nested rule's `{`.
    pub prelude: String,
    /// Raw component-value text inside the nested rule's braces.
    pub body: String,
    /// Zero-based order among sibling nested rules.
    pub source_order: u32,
    /// The stylesheet origin inherited from the containing at-rule.
    pub origin: Origin,
}

/// A rule node retained inside an opaque at-rule block.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleNode {
    /// A qualified rule retained without selector/property interpretation.
    Qualified(QualifiedRuleRecord),
    /// A nested at-rule retained recursively.
    AtRule(Box<AtRuleRecord>),
}

/// A valid at-rule that is retained as opaque data.
///
/// This is deliberately an owned representation. The parser input belongs to
/// the caller of [`RuleTree::add_stylesheet`], so retaining `&str` slices here
/// would make the rule tree borrow the stylesheet source. `prelude` includes
/// the source text between the at-rule name and its terminator; comments and
/// whitespace are retained in that text. For a block at-rule, `children` is a
/// best-effort recursive view of nested rules; `body` remains authoritative
/// for declaration lists and arbitrary component-value content.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtRuleRecord {
    /// The at-rule name without the leading `@`.
    pub name: String,
    /// The raw prelude, including source whitespace/comments after `name`.
    pub prelude: String,
    /// The statement or raw block body.
    pub body: AtRuleBody,
    /// Nested rule nodes in source order, if this is a block at-rule.
    pub children: Vec<RuleNode>,
    /// Zero-based cross-kind source order for this top-level at-rule.
    /// Nested records use sibling-local order instead.
    pub source_order: u32,
    /// The stylesheet origin supplied to [`RuleTree::add_stylesheet`].
    pub origin: Origin,
}

/// The kind of a retained top-level rule in [`RuleTree::rules`].
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CssRuleKind {
    /// Index into [`RuleTree::style_rules`].
    Style { index: usize },
    /// Index into [`RuleTree::page_rules`].
    Page { index: usize },
    /// Index into [`RuleTree::opaque_at_rules`].
    AtRule { index: usize },
}

/// A source-order entry for a retained top-level stylesheet rule.
///
/// The compatibility views keep their historical independent source-order
/// counters. This ordered view supplies the cross-kind order needed when a
/// later semantic pass expands an at-rule into ordinary style rules.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CssRule {
    /// Zero-based order across all retained top-level style, page, and opaque
    /// at-rules in the tree.
    pub source_order: u32,
    /// Cascade origin for this rule.
    pub origin: Origin,
    /// Which compatibility view contains the rule's parsed payload.
    pub kind: CssRuleKind,
}

impl AtRuleRecord {
    /// Return a best-effort CSS serialization of this retained at-rule.
    ///
    /// The name and component-value text are retained, while the outer syntax
    /// is reconstructed. This is intended for inspection and forwarding, not
    /// as a byte-for-byte source-map representation.
    pub fn to_css(&self) -> String {
        match &self.body {
            AtRuleBody::Statement => format!("@{}{};", self.name, self.prelude),
            AtRuleBody::Block(body) => format!("@{}{}{{{body}}}", self.name, self.prelude),
        }
    }

    /// Return nested rules in source order.
    pub fn children(&self) -> &[RuleNode] {
        &self.children
    }
}

/// Unified rule tree。cascade + 将来の GCPM 解決側が消費する index。
///
/// `style_rules` を populate。`page_rules` は @page at-rule を追加 (parse
/// のみ、cascade 適用は未実装)。`counter_styles`
/// ([`CounterStyleRegistry`]) は `@counter-style` at-rule の registry 化を
/// 担い、cascade が下流の marker / generated-content paint 用に snapshot
/// する。`opaque_at_rules` は generic parse
/// view として at-rule の raw data を保持する。`@media` の `all` / `print` /
/// `screen` 条件に対応する qualified rules は、互換用 `style_rules` とは
/// 別の内部 view にも展開され、cascade 前に評価される。将来 field
/// (font_face_rules / supports_rules / import_rules) は今後追加予定 —
/// `#[non_exhaustive]` の恩恵で非破壊的に拡張可能。
#[non_exhaustive]
pub struct RuleTree {
    /// Qualified style rules (`selectors { declarations }`)、source order 保持。
    pub(crate) style_rules: Vec<StyleRule>,
    /// `@page` at-rules。source_order は `style_rules` とは独立の 0-index。
    /// cascade は [`crate::page::cascade_page`] が適用する; per-page `PageBox`
    /// derivation と margin-box slot layout は未実装。
    ///
    /// `style_rules` と違い `pub` のまま — 意図的な選択。canonical な記述
    /// (docs.rs から到達可能) は [`crate::page::PageRule::declarations`] の
    /// doc の「Why this field ... is still `pub`」節にある。
    /// 旧 pointer 先だった [`crate::rule::expand_shorthand_into`] は
    /// `pub(crate)` で docs.rs に出ないため dead end だった。
    pub page_rules: Vec<PageRule>,
    /// `@counter-style` at-rule の name → rule registry。
    ///
    /// [`RuleTree::add_stylesheet`] が呼ばれるたび (origin を問わず)、同じ
    /// 文字列に対して [`crate::counter_style::parse_counter_style_rules`] を
    /// 独立にもう一度走らせ、得られた各 [`crate::counter_style::CounterStyleRule`]
    /// を呼び出し時の `origin` と一緒に
    /// [`CounterStyleRegistry::insert_with_origin`] へ渡す。同名 rule 間の
    /// 勝敗は `CounterStyleRegistry` 自体が origin ごとに追跡して解決する
    /// ([`CounterStyleRegistry`] 型 doc の解決表参照) — CSS Counter Styles L3
    /// §3 の "standard cascade rules" (origin が第一基準、同一 origin 内は
    /// source order) を [`crate::cascade::cascade_rank`] ベースの rank
    /// 比較でそのまま実装しており ([`Origin`] の variant 数に依存しない —
    /// `add_stylesheet` に実際に渡る origin は現状 `UserAgent`/`Author` の
    /// 2 つだけだが、それは呼び出し側の実態であって本 field の実装が
    /// 2-origin 前提にハードコードされているわけではない、詳細は
    /// [`CounterStyleRegistry::insert_with_origin`] doc 参照)、呼び出し側
    /// (`add_stylesheet`) は origin でフィルタする必要がない。詳細は
    /// [`RuleTree::add_stylesheet`] doc 参照。
    ///
    /// `style_rules` 用の parser とは意図的に別 pass ([`crate::counter_style`] module doc
    /// の "What's implemented" 節が元々の設計意図として明記) — このフィールドは
    /// 「同じ source 文字列を追加でもう一度 `counter_style` 側の entry point に
    /// 渡す」配線のみを担い、2 つの parser を 1 pass に融合する話ではない。
    pub(crate) counter_styles: CounterStyleRegistry,
    /// `@font-face` at-rule の family-name → rule registry。
    ///
    /// [`RuleTree::add_stylesheet`] が呼ばれるたび (origin を問わず)、同じ
    /// 文字列に対して [`crate::font_face::parse_font_face_rules`] を
    /// 独立にもう一度走らせ、得られた各 [`crate::font_face::FontFaceRule`]
    /// を呼び出し時の `origin` と一緒に
    /// [`FontFaceRegistry::insert_with_origin`] へ渡す。同名 rule 間の
    /// 勝敗は `FontFaceRegistry` 自体が origin ごとに追跡して解決する
    /// ([`FontFaceRegistry`] 型 doc の解決表参照) — standard cascade
    /// (origin が第一基準、同一 origin 内は source order) を
    /// [`crate::cascade::cascade_rank`] ベースの rank
    /// 比較でそのまま実装しており、呼び出し側
    /// (`add_stylesheet`) は origin でフィルタする必要がない。
    ///
    /// `style_rules` 用の parser とは意図的に別 pass ([`crate::font_face`] module doc
    /// の "RuleTree integration" 節参照 — [`crate::counter_style`] と同じ設計)。
    pub(crate) font_faces: FontFaceRegistry,
    /// Generic records for at-rules that do not use the `@page` compatibility
    /// view.
    ///
    /// This includes valid statement and block at-rules such as `@media`,
    /// `@supports`, `@import`, and `@counter-style`. The records are
    /// intentionally separate from `style_rules` and `page_rules`: retaining
    /// an at-rule must not make its declarations execute accidentally.
    pub(crate) opaque_at_rules: Vec<AtRuleRecord>,
    /// Executable qualified rules parsed from `@media`, kept apart from the
    /// historical direct-rule compatibility view.
    pub(crate) media_rules: Vec<MediaRule>,
    /// Next source order shared by direct and media-qualified style rules.
    next_style_order: u32,
    /// Cross-kind source-order index. The individual compatibility views keep
    /// their historical counters; this vector records their retained order.
    pub(crate) rules: Vec<CssRule>,
    /// Remaining budget for overlapping bodies retained by nested opaque rules.
    opaque_body_budget: usize,
}

impl RuleTree {
    /// Qualified style rules への read-only accessor (source order 順)。
    ///
    /// **qualified style rules に関しては**書き込み経路が
    /// [`RuleTree::add_stylesheet`] のみになった。`page_rules` 側は引き続き
    /// `pub` のまま — 同 field の doc 参照。
    ///
    /// # `style_rules` field 自体への到達不能性
    ///
    /// `style_rules` field は `pub(crate)` — external crate から届くのは
    /// この accessor だけである。`RuleTree` は `#[non_exhaustive]` かつ
    /// `Clone` を derive していないので、struct literal / functional-update
    /// による構築も、所有値としての複製も external crate からはできない。
    /// 以下は field 名そのものが private であることの compile-fail check —
    /// `pub` に戻れば compile が通るようになる:
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.style_rules;
    /// ```
    pub fn style_rules(&self) -> &[StyleRule] {
        &self.style_rules
    }

    /// `@counter-style` registry への read-only accessor。
    ///
    /// [`RuleTree::add_stylesheet`] が呼ばれるたびに (origin を問わず) populate
    /// される — 同名 rule 間の origin 優先順位の解決は `CounterStyleRegistry`
    /// 自体が担う (field doc 参照)。空の `RuleTree` ([`RuleTree::empty`]) では
    /// [`CounterStyleRegistry::is_empty`] が `true`。`generate a counter` の実行
    /// (`counter()`/`counters()` の値 resolve) は cascade result 経由で
    /// `raikiri-paint` が担う — [`crate::counter_style::resolve_custom_counter`]
    /// にこの registry を渡す marker / generated-content 配線がその consumer
    /// 境界にある。
    ///
    /// # `counter_styles` field 自体への到達不能性
    ///
    /// `style_rules`/[`RuleTree::style_rules`] と同じ
    /// check — `counter_styles` field は `pub(crate)` で、external crate から
    /// 届くのはこの accessor だけである。以下は field 名そのものが private で
    /// あることの compile-fail check — `pub` に戻れば compile が通るようになる:
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.counter_styles;
    /// ```
    pub fn counter_styles(&self) -> &CounterStyleRegistry {
        &self.counter_styles
    }

    /// `@font-face` registry への read-only accessor。
    ///
    /// [`RuleTree::add_stylesheet`] が呼ばれるたびに (origin を問わず) populate
    /// される — 同名 rule 間の origin 優先順位の解決は `FontFaceRegistry`
    /// 自体が担う (field doc 参照)。空の `RuleTree` ([`RuleTree::empty`]) では
    /// [`FontFaceRegistry::is_empty`] が `true`。`src: url(...)` の fetch と
    /// face 選択 (matching) はこの registry を読む consumer 側の責務。
    ///
    /// # `font_faces` field 自体への到達不能性
    ///
    /// `style_rules`/[`RuleTree::style_rules`] と同じ
    /// check — `font_faces` field は `pub(crate)` で、external crate から
    /// 届くのはこの accessor だけである。以下は field 名そのものが private で
    /// あることの compile-fail check — `pub` に戻れば compile が通るようになる:
    ///
    /// ```compile_fail
    /// use raikiri_style::RuleTree;
    ///
    /// let tree = RuleTree::empty();
    /// let _ = &tree.font_faces;
    /// ```
    pub fn font_faces(&self) -> &FontFaceRegistry {
        &self.font_faces
    }

    /// Generic retained records for at-rules outside the `@page`
    /// compatibility view.
    ///
    /// The returned records retain their cross-kind source order in the
    /// corresponding [`CssRule`] entries. The raw records are observational;
    /// supported `@media` descendants are separately evaluated by the cascade
    /// and are not inserted into this compatibility view.
    pub fn opaque_at_rules(&self) -> &[AtRuleRecord] {
        &self.opaque_at_rules
    }

    /// Alias for [`RuleTree::opaque_at_rules`] for callers that use the CSSOM
    /// term "at-rules" for the retained opaque records.
    pub fn at_rules(&self) -> &[AtRuleRecord] {
        self.opaque_at_rules()
    }

    /// Retained top-level rules in one cross-kind source-order sequence.
    ///
    /// Use the `index` in [`CssRuleKind`] to access the parsed payload through
    /// [`RuleTree::style_rules`], [`RuleTree::page_rules`], or
    /// [`RuleTree::opaque_at_rules`]. Unsupported selectors are absent from
    /// this sequence because they do not produce a retained rule.
    pub fn rules(&self) -> &[CssRule] {
        &self.rules
    }

    /// 空の RuleTree (0 rule)。
    pub fn empty() -> Self {
        Self {
            style_rules: Vec::new(),
            page_rules: Vec::new(),
            counter_styles: CounterStyleRegistry::new(),
            font_faces: FontFaceRegistry::new(),
            opaque_at_rules: Vec::new(),
            media_rules: Vec::new(),
            next_style_order: 0,
            rules: Vec::new(),
            opaque_body_budget: MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES,
        }
    }

    /// Stylesheet 文字列を parse して rule を append する。
    ///
    /// - `source_order` は既存 rule 数を起点に呼び出し順で自動採番
    ///   (`style_rules` / `page_rules` は別カウンタ — [`PageRule::source_order`]
    ///   の doc 参照)
    /// - `origin` は style rule と `@page` rule の両方に伝播する。cascade
    ///   rank 化 (`!important` 反転扱い) は未実装 — `PageRule` に origin を
    ///   保持しているため、それが wire されるときに cascade 側で re-index
    ///   する必要はない。CSS Cascading L4 §"cascade-origin"
    ///   (<https://www.w3.org/TR/css-cascade-4/#cascade-origin>)
    /// - Invalid selector / 未サポート property は既存の silent-drop 挙動を
    ///   継承する
    /// - Syntactically valid at-rules other than `@page` are retained in
    ///   [`RuleTree::opaque_at_rules`]. Declarations in unsupported at-rules
    ///   are not applied by the cascade. Supported `@media` descendants are
    ///   additionally parsed into the cascade's media-qualified view.
    /// - `@counter-style` at-rule は `origin` を問わず、
    ///   [`crate::counter_style::parse_counter_style_rules`] が同じ `source` に対して
    ///   独立にもう一度 parse し、得られた各 rule を呼び出し時の `origin` と共に
    ///   [`CounterStyleRegistry::insert_with_origin`] へ渡す。
    ///
    ///   CSS Counter Styles L3 §3 は同名 `@counter-style` の勝者決定を "according
    ///   to standard cascade rules" (origin が第一基準、UA は常に他 origin に負ける)
    ///   と規定する。この解決自体は [`CounterStyleRegistry`] が origin ごとに
    ///   entry を追跡して実装している (型 doc の解決表参照) — 呼び出し側の
    ///   `add_stylesheet` は origin でフィルタする必要がなく、単に origin を
    ///   そのまま伝播するだけでよい。結果として、他 origin との同名衝突がない
    ///   単独の `Origin::UserAgent` `@counter-style` も (かつては Author-only
    ///   gate により無条件 drop されていたが) `counter_styles` に反映される
    ///   ようになった。
    pub fn add_stylesheet(&mut self, source: &str, origin: Origin) {
        let source = expand_supports(source);
        for (layer_order, chunk) in expand_cascade_layers(&source) {
            self.add_stylesheet_chunk(&chunk, origin, layer_order);
        }
    }

    fn add_stylesheet_chunk(&mut self, source: &str, origin: Origin, layer_order: u32) {
        let mut input = ParserInput::new(source);
        let mut parser = Parser::new(&mut input);
        let mut rule_parser = StyleRuleParser {
            source,
            opaque_body_budget: &mut self.opaque_body_budget,
        };
        let mut style_order = self.next_style_order;
        let mut page_order = self.page_rules.len() as u32;
        let mut rule_order = self.rules.len() as u32;
        if let Some(prelude) = leading_charset_prelude(source) {
            let index = self.opaque_at_rules.len();
            self.opaque_at_rules.push(AtRuleRecord {
                name: "charset".to_owned(),
                prelude,
                body: AtRuleBody::Statement,
                children: Vec::new(),
                source_order: rule_order,
                origin,
            });
            self.rules.push(CssRule {
                source_order: rule_order,
                origin,
                kind: CssRuleKind::AtRule { index },
            });
            rule_order = rule_order.wrapping_add(1);
        }
        for rule in StyleSheetParser::new(&mut parser, &mut rule_parser).flatten() {
            match rule {
                ParsedRule::Style(selectors, declarations) => {
                    // 未サポート component (pseudo-class 等、
                    // `is_supported_selector_list` doc 参照) を含む selector は
                    // drop — descendant/child combinator と
                    // next-sibling/general-sibling combinator は受理対象に
                    // 含まれる (詳細は `is_supported_selector_list` doc)。
                    if !is_supported_selector_list(&selectors) {
                        continue;
                    }
                    let index = self.style_rules.len();
                    self.style_rules.push(StyleRule {
                        selectors,
                        declarations,
                        source_order: style_order,
                        origin,
                    });
                    self.rules.push(CssRule {
                        source_order: rule_order,
                        origin,
                        kind: CssRuleKind::Style { index },
                    });
                    style_order = style_order.wrapping_add(1);
                    rule_order = rule_order.wrapping_add(1);
                }
                ParsedRule::Page(selector, body) => {
                    let PageBlockBody {
                        declarations,
                        size_declarations,
                        marks_declarations,
                        bleed_declarations,
                        margin_box_rules,
                    } = body;
                    let index = self.page_rules.len();
                    self.page_rules.push(PageRule {
                        selector,
                        declarations,
                        size_declarations,
                        marks_declarations,
                        bleed_declarations,
                        margin_box_rules,
                        source_order: page_order,
                        layer_order,
                        origin,
                    });
                    self.rules.push(CssRule {
                        source_order: rule_order,
                        origin,
                        kind: CssRuleKind::Page { index },
                    });
                    page_order = page_order.wrapping_add(1);
                    rule_order = rule_order.wrapping_add(1);
                }
                ParsedRule::OpaqueAtRule(mut record) => {
                    record.source_order = rule_order;
                    set_at_rule_origin(&mut record, origin);
                    if record.name.eq_ignore_ascii_case("media") {
                        collect_media_style_rules(
                            &record,
                            None,
                            &mut self.media_rules,
                            &mut style_order,
                        );
                    }
                    let index = self.opaque_at_rules.len();
                    self.opaque_at_rules.push(record);
                    self.rules.push(CssRule {
                        source_order: rule_order,
                        origin,
                        kind: CssRuleKind::AtRule { index },
                    });
                    rule_order = rule_order.wrapping_add(1);
                }
            }
        }
        for rule in parse_counter_style_rules(source) {
            self.counter_styles.insert_with_origin(rule, origin);
        }
        for rule in parse_font_face_rules(source) {
            self.font_faces.insert_with_origin(rule, origin);
        }
        self.next_style_order = style_order;
    }
}

/// DOM を DFS walk して全 `<style>` element の text を Author stylesheet として
/// 集約する convenience。UA CSS は含めない (`raikiri-html::parse` が
/// Document.add_stylesheet 経由で inject 済み、`Document.stylesheets()` を
/// raikiri umbrella が RuleTree に流し込む責務)。
pub fn build_rule_tree<D: StyleDom>(dom: &D) -> RuleTree {
    let mut tree = RuleTree::empty();
    walk_and_collect(dom, dom.root_id(), &mut |source| {
        tree.add_stylesheet(source, Origin::Author);
    });
    tree
}

/// DOM を root から DFS walk して全 `<style>` element の text を callback に渡す。
///
/// UA CSS は含まれない — `raikiri-html::parse` が `Document::add_stylesheet` 経由で
/// UA を注入しており、`Document::stylesheets()` 経路で raikiri umbrella が別途消費
/// する契約。本 walker は DOM `<style>` element の text 収集のみを担当する。
///
/// 呼び出し順は `walk_and_collect` の iterative DFS に従い document order。
/// stack overflow 保護は `walk_and_collect` と共有する。
pub fn walk_style_elements<D: StyleDom, F: FnMut(&str)>(dom: &D, mut on_style_text: F) {
    walk_and_collect(dom, dom.root_id(), &mut on_style_text);
}

/// DOM walk 本体。深いネストで stack overflow しないよう explicit `Vec` stack
/// で iterative DFS。children を reverse push してから
/// LIFO で pop するため (下記 stack push 箇所参照)、sibling 間の訪問順は素朴な
/// recursion 版と一致する — document order の保持は単なる互換目的ではなく、
/// [`RuleTree::add_stylesheet`] が呼び出し順で `source_order` を単調採番する
/// (`build_rule_tree` がこの walk の callback から呼ぶ) ため正しさ上の要請。
fn walk_and_collect<D: StyleDom, F: FnMut(&str)>(dom: &D, id: StyleNodeId, on_style_text: &mut F) {
    let mut stack: Vec<StyleNodeId> = vec![id];
    while let Some(id) = stack.pop() {
        if let Some(node) = dom.node(id) {
            // <template> 子孫 + 将来の inert subtree を統一 skip。
            // 実 Document (sink 経由 populate 済) では is_in_document() bit が
            // primary skip 経路。
            if !node.is_in_document() {
                continue;
            }
            if node.kind() == StyleNodeKind::Element
                && let Some(elem) = node.as_element()
            {
                let tag = elem.tag_name();
                // NOTE: 通常経路 (sink 経由 populate
                // 済 Document) では上の is_in_document() gate で subsumed。本 arm
                // は TestDoc 等の default true な Node trait 実装からの呼び出しで
                // template 内 <style> が cascade に流れ込むのを防ぐ safety net。
                // 実本番経路の "1 か所集約" contract は sink 側の判定を primary
                // とし、この safety net は 2nd-line defense として明示的に維持する。
                //
                // namespace check を追加し HTML
                // `<template>` のみを対象とする (SVG element `<template>` は spec
                // 定義が無いが raw parser で local="template" になり得る)。sink 側
                // の判定 (`namespace.is_none()`) と一貫。
                if tag.eq_ignore_ascii_case("template") && elem.namespace_uri().is_none() {
                    continue;
                }
                if tag.eq_ignore_ascii_case("style") {
                    // 子 Text node を concat
                    let mut concat = String::new();
                    for child_id in dom.child_ids(id) {
                        if let Some(child) = dom.node(child_id)
                            && let Some(t) = child.text_content()
                        {
                            concat.push_str(t);
                        }
                    }
                    if !concat.is_empty() {
                        on_style_text(&concat);
                    }
                }
            }
            // 全 kind で children を stack に push。stack は LIFO なので document
            // order を保つため reverse push。`child_ids` イテレータを直接
            // `stack` へ `extend` し、今回追加した末尾スライスだけを in-place
            // `reverse()` する — 都度捨てる中間 `Vec` を経由しない (cascade.rs
            // 側の同型の技法)。`stack` 自体の capacity growth は元の
            // `for .. { stack.push(..) }` と同じ amortized pattern のままで、
            // ここで削れるのは「今回だけの捨て Vec」1 本分のみ。
            //
            // なぜここでは document order 保持が正しさ上の要請か: 本関数冒頭
            // のコメントの通り `RuleTree::add_stylesheet` は呼び出し順で
            // source_order を単調採番する。`build_rule_tree` はこの walk の
            // callback から `<style>` element 訪問順にそれを呼ぶため、訪問順
            // がそのまま source_order — ひいては cascade tie-break — に反映
            // される。cascade.rs の同型 2 箇所 (collect_cascaded /
            // resolve_inheritance) は訪問順に依存しない挙動保持のみが目的
            // だったのと対照的。
            // `walk_style_elements_pub_visits_all_style_texts_in_document_order`
            // test が兄弟 `<style>` 2 個 (同一 depth) の text を visit 順で固定
            // している。ただし同一 depth の兄弟だけでは DFS/BFS を判別できない
            // (同深度なら両戦略の visit 順が一致してしまう) — mixed depth の
            // regression net は
            // `walk_and_collect_preserves_document_order_across_mixed_sibling_descendant_depths`
            // test が別途固定している。
            let start = stack.len();
            stack.extend(dom.child_ids(id));
            stack[start..].reverse();
        }
    }
}

fn parse_css_ident(bytes: &[u8], source: &str, position: usize) -> Option<(usize, String)> {
    let mut i = position;
    let mut name = String::new();
    while i < bytes.len() {
        if is_css_ident_byte(bytes[i]) {
            name.push(bytes[i] as char);
            i += 1;
            continue;
        }
        if bytes[i] != b'\\' {
            break;
        }
        i += 1;
        let first = *bytes.get(i)?;
        if first.is_ascii_hexdigit() {
            let mut value = 0u32;
            let mut digits = 0;
            while digits < 6 {
                let Some(byte) = bytes.get(i) else { break };
                let Some(digit) = (*byte as char).to_digit(16) else {
                    break;
                };
                value = value * 16 + digit;
                digits += 1;
                i += 1;
            }
            if bytes.get(i).is_some_and(|byte| byte.is_ascii_whitespace()) {
                if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if let Some(ch) = char::from_u32(value) {
                name.push(ch);
            }
        } else if let Some(ch) = source[i..].chars().next() {
            name.push(ch);
            i += ch.len_utf8();
        } else {
            return None;
        }
    }
    (!name.is_empty()).then_some((i, name))
}

fn is_css_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
}

fn leading_charset_prelude(source: &str) -> Option<String> {
    let bytes = source.as_bytes();
    let mut start = 0;
    loop {
        while start < bytes.len() && bytes[start].is_ascii_whitespace() {
            start += 1;
        }
        if source.get(start..)?.starts_with("/*") {
            let end = source.get(start + 2..)?.find("*/")?;
            start += end + 4;
            continue;
        }
        if source.get(start..)?.starts_with("<!--") {
            start += 4;
            continue;
        }
        if source.get(start..)?.starts_with("-->") {
            start += 3;
            continue;
        }
        break;
    }

    let name_start = start.checked_add(1)?;
    if bytes.get(start) != Some(&b'@') {
        return None;
    }
    let (after_name, name) = parse_css_ident(bytes, source, name_start)?;
    if !name.eq_ignore_ascii_case("charset") {
        return None;
    }
    let next = source.get(after_name..)?.chars().next();
    if next.is_some_and(|ch| {
        !ch.is_ascii() || ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '\\')
    }) {
        return None;
    }

    let mut i = after_name;
    let mut stack = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = source.get(i + 2..)?.find("*/")?;
                i += end + 4;
            }
            b'\\' => {
                i += 1;
                i += source.get(i..)?.chars().next()?.len_utf8();
            }
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'\\' => {
                            i += 1;
                            i += source.get(i..)?.chars().next()?.len_utf8();
                        }
                        byte if byte == quote => {
                            i += 1;
                            break;
                        }
                        _ => i += 1,
                    }
                }
            }
            b'(' | b'[' => {
                stack.push(bytes[i]);
                i += 1;
            }
            b')' | b']' => {
                let expected = if bytes[i] == b')' { b'(' } else { b'[' };
                if stack.pop() != Some(expected) {
                    return None;
                }
                i += 1;
            }
            b'{' | b'}' if stack.is_empty() => return None,
            b'{' | b'}' => i += 1,
            b';' if stack.is_empty() => {
                let prelude = source.get(after_name..i)?.to_owned();
                if css_component_values_are_balanced(&prelude) {
                    return Some(prelude);
                }
                return None;
            }
            _ => i += 1,
        }
    }
    None
}

fn consume_raw_component_values<'i, 't>(
    input: &mut Parser<'i, 't>,
) -> Result<String, cssparser::ParseError<'i, ()>> {
    let start = input.position();
    loop {
        let token_is_error = match input.next_including_whitespace_and_comments() {
            Ok(token) => token.is_parse_error(),
            Err(_) => break,
        };
        if token_is_error {
            return Err(input.new_custom_error(()));
        }
    }
    let raw = input.slice(start..input.position()).to_owned();
    if css_component_values_are_balanced(&raw) {
        Ok(raw)
    } else {
        Err(input.new_custom_error(()))
    }
}

/// Validate one component-value list with cssparser's own tokenization.
///
/// The recursive walk is deliberately bounded. Once the bound is reached,
/// cssparser still consumes the remaining nested block iteratively, while the
/// caller only relies on this pass to keep malformed input out of the
/// executable views. The raw data itself remains available for inspection.
fn css_component_values_are_balanced(source: &str) -> bool {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    parse_component_values(&mut parser, source, None, 0)
}

fn parse_component_values<'i, 't>(
    parser: &mut Parser<'i, 't>,
    source: &str,
    expected_close: Option<u8>,
    depth: usize,
) -> bool {
    loop {
        let token = match parser.next_including_whitespace_and_comments() {
            Ok(token) => token,
            Err(_) => {
                return expected_close.is_none()
                    || source
                        .get(parser.position().byte_index()..)
                        .and_then(|suffix| suffix.as_bytes().first())
                        == expected_close.as_ref();
            }
        };

        let closing_delimiter = match token {
            Token::Function(_) | Token::ParenthesisBlock => Some(b')'),
            Token::SquareBracketBlock => Some(b']'),
            Token::CurlyBracketBlock => Some(b'}'),
            _ => None,
        };
        if token.is_parse_error() {
            return false;
        }
        let Some(closing_delimiter) = closing_delimiter else {
            continue;
        };

        if depth >= MAX_OPAQUE_RULE_NESTING_DEPTH {
            let mut ended_at_delimiter = false;
            let result =
                parser.parse_nested_block(|nested| -> Result<(), cssparser::ParseError<'i, ()>> {
                    loop {
                        match nested.next_including_whitespace_and_comments() {
                            Ok(token) => {
                                if token.is_parse_error() {
                                    return Err(nested.new_custom_error(()));
                                }
                            }
                            Err(_) => {
                                ended_at_delimiter = source
                                    .get(nested.position().byte_index()..)
                                    .and_then(|suffix| suffix.as_bytes().first())
                                    == Some(&closing_delimiter);
                                return Ok(());
                            }
                        }
                    }
                });
            if result.is_err() || !ended_at_delimiter {
                return false;
            }
            continue;
        }

        let result =
            parser.parse_nested_block(|nested| -> Result<(), cssparser::ParseError<'i, ()>> {
                if parse_component_values(nested, source, Some(closing_delimiter), depth + 1) {
                    Ok(())
                } else {
                    Err(nested.new_custom_error(()))
                }
            });
        if result.is_err() {
            return false;
        }
    }
}

/// A raw rule list used to expose nested structure without assigning semantics
/// to an unknown at-rule body.
enum NestedParsedRule {
    AtRule(AtRuleRecord),
    Qualified(QualifiedRuleRecord),
}

const MAX_OPAQUE_RULE_NESTING_DEPTH: usize = 128;
// Descendant bodies overlap their ancestors, so cap their aggregate owned copies.
const MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES: usize = 8 * 1024 * 1024;

fn nested_block_has_closing_brace(source: &str, input: &Parser<'_, '_>) -> bool {
    let position = input.position().byte_index();
    let Some(mut suffix) = source.get(position..) else {
        return false;
    };
    loop {
        suffix = suffix.trim_start_matches(|ch: char| ch.is_ascii_whitespace());
        if let Some(rest) = suffix.strip_prefix("/*") {
            let Some(end) = rest.find("*/") else {
                return false;
            };
            suffix = &rest[end + 2..];
        } else {
            return suffix.starts_with('}');
        }
    }
}

struct RawRuleParser<'s, 'b> {
    depth: usize,
    source: &'s str,
    remaining_body_bytes: &'b mut usize,
}

impl<'i, 's, 'b> cssparser::AtRuleParser<'i> for RawRuleParser<'s, 'b> {
    type Prelude = (String, String);
    type AtRule = NestedParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        Ok((name.to_string(), consume_raw_component_values(input)?))
    }

    fn rule_without_block(
        &mut self,
        (name, prelude): Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        Ok(NestedParsedRule::AtRule(AtRuleRecord {
            name,
            prelude,
            body: AtRuleBody::Statement,
            children: Vec::new(),
            source_order: 0,
            origin: Origin::Author,
        }))
    }

    fn parse_block<'t>(
        &mut self,
        (name, prelude): Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, Self::Error>> {
        let body = consume_raw_component_values(input)?;
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        if body.len() > *self.remaining_body_bytes {
            return Err(input.new_custom_error(()));
        }
        *self.remaining_body_bytes -= body.len();
        let children = if self.depth < MAX_OPAQUE_RULE_NESTING_DEPTH {
            parse_nested_rule_nodes_at_depth(&body, self.depth + 1, self.remaining_body_bytes)
        } else {
            Vec::new()
        };
        Ok(NestedParsedRule::AtRule(AtRuleRecord {
            name,
            prelude,
            body: AtRuleBody::Block(body),
            children,
            source_order: 0,
            origin: Origin::Author,
        }))
    }
}

impl<'i, 's, 'b> cssparser::QualifiedRuleParser<'i> for RawRuleParser<'s, 'b> {
    type Prelude = String;
    type QualifiedRule = NestedParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        consume_raw_component_values(input)
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let body = consume_raw_component_values(input)?;
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        if body.len() > *self.remaining_body_bytes {
            return Err(input.new_custom_error(()));
        }
        *self.remaining_body_bytes -= body.len();
        Ok(NestedParsedRule::Qualified(QualifiedRuleRecord {
            prelude,
            body,
            source_order: 0,
            origin: Origin::Author,
        }))
    }
}

fn parse_nested_rule_nodes(source: &str, remaining_body_bytes: &mut usize) -> Vec<RuleNode> {
    parse_nested_rule_nodes_at_depth(source, 0, remaining_body_bytes)
}

fn parse_nested_rule_nodes_at_depth(
    source: &str,
    depth: usize,
    remaining_body_bytes: &mut usize,
) -> Vec<RuleNode> {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut rule_parser = RawRuleParser {
        depth,
        source,
        remaining_body_bytes,
    };
    let mut nodes = Vec::new();
    for (source_order, rule) in StyleSheetParser::new(&mut parser, &mut rule_parser)
        .flatten()
        .enumerate()
    {
        let node = match rule {
            NestedParsedRule::AtRule(mut record) => {
                record.source_order = source_order as u32;
                RuleNode::AtRule(Box::new(record))
            }
            NestedParsedRule::Qualified(mut record) => {
                record.source_order = source_order as u32;
                RuleNode::Qualified(record)
            }
        };
        nodes.push(node);
    }
    nodes
}

fn set_at_rule_origin(record: &mut AtRuleRecord, origin: Origin) {
    record.origin = origin;
    for node in &mut record.children {
        match node {
            RuleNode::Qualified(qualified) => qualified.origin = origin,
            RuleNode::AtRule(nested) => set_at_rule_origin(nested, origin),
        }
    }
}

fn parse_media_style_rule(record: &QualifiedRuleRecord) -> Option<StyleRule> {
    let mut input = ParserInput::new(&record.prelude);
    let mut parser = Parser::new(&mut input);
    let selectors = parser
        .parse_entirely(|input| {
            SelectorList::parse(&RaikiriSelectorParser, input, ParseRelative::No)
        })
        .ok()?;
    if !is_supported_selector_list(&selectors) {
        return None;
    }

    let mut input = ParserInput::new(&record.body);
    let mut parser = Parser::new(&mut input);
    Some(StyleRule {
        selectors,
        declarations: parse_declaration_block(&mut parser),
        source_order: 0,
        origin: record.origin,
    })
}

fn collect_media_style_rules(
    record: &AtRuleRecord,
    parent_condition: Option<MediaCondition>,
    out: &mut Vec<MediaRule>,
    style_order: &mut u32,
) {
    let Some(local_condition) = parse_media_condition(&record.prelude) else {
        return;
    };
    let condition =
        parent_condition.map_or(local_condition, |parent| parent.intersect(local_condition));
    if condition.is_empty() {
        return;
    }

    for child in &record.children {
        match child {
            RuleNode::Qualified(qualified) => {
                let Some(mut rule) = parse_media_style_rule(qualified) else {
                    continue;
                };
                rule.source_order = *style_order;
                *style_order = style_order.wrapping_add(1);
                out.push(MediaRule { rule, condition });
            }
            RuleNode::AtRule(nested) if nested.name.eq_ignore_ascii_case("media") => {
                collect_media_style_rules(nested, Some(condition), out, style_order);
            }
            // An unknown wrapper may have a completely different grammar. Do
            // not accidentally execute its descendants as ordinary CSS rules.
            RuleNode::AtRule(_) => {}
        }
    }
}

/// Intermediate at-rule prelude emitted by [`StyleRuleParser`].
///
/// `@page` keeps its structured selector for the existing page compatibility
/// view. Every other syntactically valid at-rule is retained with its raw
/// component-value prelude so a later semantic pass can reinterpret it.
enum ParsedAtRulePrelude {
    Page(PageSelector),
    Opaque { name: String, prelude: String },
}

/// Top-level parsed rule shape emitted by [`StyleRuleParser`].
///
/// cssparser requires `AtRuleParser::AtRule` と `QualifiedRuleParser::QualifiedRule`
/// を同一型にする必要があるため、両方をこの enum に流し込む
/// (`StyleSheetParser::next` の `Item = R` 制約)。
enum ParsedRule {
    Style(SelectorList<RaikiriSelectorImpl>, Vec<Declaration>),
    Page(PageSelector, PageBlockBody),
    OpaqueAtRule(AtRuleRecord),
}

/// StyleSheetParser 実装。qualified rule + `@page` を受理し、その他の at-rule
/// は意味論を実行せず opaque record として保持する。
struct StyleRuleParser<'s, 'b> {
    source: &'s str,
    opaque_body_budget: &'b mut usize,
}

impl<'i, 's, 'b> cssparser::AtRuleParser<'i> for StyleRuleParser<'s, 'b> {
    type Prelude = ParsedAtRulePrelude;
    type AtRule = ParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        name: cssparser::CowRcStr<'i>,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        if name.eq_ignore_ascii_case("page") {
            return parse_page_prelude(input).map(ParsedAtRulePrelude::Page);
        }

        // cssparser gives this closure a parser delimited at `;`, `{`, or the
        // end of the current rule list. Consume component values rather than
        // rejecting the at-rule, retaining comments and whitespace from the
        // original source through `slice`.
        Ok(ParsedAtRulePrelude::Opaque {
            name: name.to_string(),
            prelude: consume_raw_component_values(input)?,
        })
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
    ) -> Result<Self::AtRule, ()> {
        match prelude {
            ParsedAtRulePrelude::Page(_) => Err(()),
            ParsedAtRulePrelude::Opaque { name, prelude } => {
                Ok(ParsedRule::OpaqueAtRule(AtRuleRecord {
                    name,
                    prelude,
                    body: AtRuleBody::Statement,
                    children: Vec::new(),
                    source_order: 0,
                    origin: Origin::Author,
                }))
            }
        }
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::AtRule, cssparser::ParseError<'i, Self::Error>> {
        match prelude {
            ParsedAtRulePrelude::Page(selector) => {
                // `@page` body = declaration list + `size` / `marks` / `bleed`
                // descriptors + nested margin-box at-rules — parsed by a
                // dedicated `crate::page::parse_page_declaration_block` (not
                // the generic `parse_declaration_block` qualified rules use).
                // Unsupported properties/descriptors retain the existing
                // silent-drop behavior.
                let body = parse_page_declaration_block(input);
                if !nested_block_has_closing_brace(self.source, input) {
                    return Err(input.new_custom_error(()));
                }
                Ok(ParsedRule::Page(selector, body))
            }
            ParsedAtRulePrelude::Opaque { name, prelude } => {
                let body = consume_raw_component_values(input)?;
                if !nested_block_has_closing_brace(self.source, input) {
                    return Err(input.new_custom_error(()));
                }
                let children = parse_nested_rule_nodes(&body, self.opaque_body_budget);
                Ok(ParsedRule::OpaqueAtRule(AtRuleRecord {
                    name,
                    prelude,
                    body: AtRuleBody::Block(body),
                    children,
                    source_order: 0,
                    origin: Origin::Author,
                }))
            }
        }
    }
}

impl<'i, 's, 'b> cssparser::QualifiedRuleParser<'i> for StyleRuleParser<'s, 'b> {
    type Prelude = SelectorList<RaikiriSelectorImpl>;
    type QualifiedRule = ParsedRule;
    type Error = ();

    fn parse_prelude<'t>(
        &mut self,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::Prelude, cssparser::ParseError<'i, Self::Error>> {
        SelectorList::parse(&RaikiriSelectorParser, input, ParseRelative::No)
            .map_err(|_| input.new_custom_error(()))
    }

    fn parse_block<'t>(
        &mut self,
        prelude: Self::Prelude,
        _start: &cssparser::ParserState,
        input: &mut Parser<'i, 't>,
    ) -> Result<Self::QualifiedRule, cssparser::ParseError<'i, Self::Error>> {
        let declarations = parse_declaration_block(input);
        if !nested_block_has_closing_brace(self.source, input) {
            return Err(input.new_custom_error(()));
        }
        Ok(ParsedRule::Style(prelude, declarations))
    }
}

/// SelectorList 内全 selector が現在サポート済みの component のみで構成されて
/// いるか判定。type / universal / class / id / null-namespace 属性 selector
/// (存在チェック `[foo]` と値付き `[foo=bar]` 系一式) に加え、descendant
/// (space) / child (`>`) combinator、next-sibling (`+`) / general-sibling
/// (`~`) combinator、
/// `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
/// (`:lang()` / `:dir()`) も受理する — ただし同じ `Component`
/// variant を持つ `PseudoClass::Hover` / `PseudoClass::Active` (`:hover` /
/// `:active`) は引き続き対象外。
/// `Component::Negation` (`:not()`)、`Component::Is` (`:is()`)、
/// `Component::Where` (`:where()`)、`Component::Has` (`:has()`) は、それぞれの
/// selector-list / relative-selector-list を同じ gate で再帰的に検査する。
/// `:is()`/`:where()` の forgiving な無効 branch は
/// `Component::Invalid` として受理するが、matcher がその branch を無視する。
/// それ以外の pseudo-class / 属性 selector 形態を 1 つでも含めば false →
/// rule ごと drop。
/// 「それ以外の属性 selector 形態」= `Component::AttributeOther` に束ねられる
/// 2 パターン、ただし両者は対称ではない (selectors crate v0.39.0
/// `parser.rs` の実 parse 分岐で確認): namespace 付き (`[ns|foo]`) は存在
/// チェック/値付き両形態とも常に `AttributeOther`。非 ASCII-lowercase local
/// name (`[Data-Foo]` 等、
/// namespace 無指定) は**値付き形態のみ** `AttributeOther` に回る — 存在
/// チェック形態は namespace 無指定である限り
/// `Component::AttributeInNoNamespaceExists` のまま受理される。両形態の
/// 非対称の理由: 値付き形態の `local_name` は selector 自身の parse 時点で
/// 既に ASCII-lowercase であることが保証される (そうでなければ
/// `AttributeOther` に回るため — この保証だけで足り、element 側の
/// namespace は attribute *name* の lookup key 選択には影響しない。値
/// **文字列**自体の case-sensitivity 解決は別の話で、match 時に
/// `cascade.rs::resolve_case_sensitivity` が行う)。一方、存在チェック
/// 形態には比較すべき値がなく、local name の大文字小文字はそのまま
/// selector 内に保持される — そのため element 側の namespace に応じて
/// `local_name`/`local_name_lower` のどちらを attribute name の lookup key
/// にすべきかが変わる (`cascade.rs::compound_matches` の該当 arm 参照)。
///
/// 元は type/universal selector のみを受理する関数だった (旧名
/// `is_type_or_universal_only`) — class/id/attribute selector、
/// `Component::Combinator(Combinator::Descendant | Combinator::Child)`、
/// `Component::Combinator(Combinator::NextSibling | Combinator::LaterSibling)`、
/// `Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_))`
/// (`:lang()`/`:dir()`)、`Component::Root` / `Component::Empty` /
/// `Component::Nth(_)` (`:root` / `:empty` /
/// `:first-child`/`:last-child`/`:only-child`/`:nth-child()`/`:nth-last-child()`
/// / `:first-of-type`/`:last-of-type`/`:only-of-type`/`:nth-of-type()`/
/// `:nth-last-of-type()`)、`Component::PseudoElement(PseudoElem::Before |
/// PseudoElem::After)` + その bridge である
/// `Component::Combinator(Combinator::PseudoElement)` (`::before`/`::after`)
/// を段階的に追加受理するよう拡張され、type/universal only という名前が
/// 実態と合わなくなったため rename した。他 combinator
/// (`Combinator::SlotAssignment` / `Combinator::Part`) と `:hover`/`:active`
/// pseudo-class は引き続き scope 外 (`cascade.rs::match_combinator_chain` の
/// doc 参照 — この 2 つは対応する pseudo 構文 (`::slotted()`/`::part()`) 自体を
/// `RaikiriSelectorParser` が `parse_slotted`/`parse_part` を override せず
/// default `false` のままにしているため、本 crate の
/// `parse_selector_list` がそもそも parse error にする、つまり到達不能)。
///
/// `::before`/`::after` は `Component::PseudoElement` 単体として cascade 側の
/// `compound_matches`/`match_combinator_chain` には一切渡らない (次節
/// "Invariant" のただし書き参照) — real element 自身への直接 match は
/// `cascade.rs::compound_matches` の `_ => false` safety net が
/// `Component::PseudoElement` 単体の compound を安全に reject し続けるので、
/// `.foo::before { .. }` が real `.foo` element に誤って直接適用されることは
/// 無い。real element ではなく `::before`/`::after` 自体への適用は
/// `cascade.rs::selector_matches_pseudo_element` という独立した matcher が
/// 別途担当する。
///
/// **4 combinator 間の混在に制限は無い**: 同じ
/// complex selector の中で祖先系 (`>`/space) と兄弟系 (`+`/`~`)
/// を任意の順序・任意回数組み合わせてよい — 例えば `.x > .y ~ .z` も
/// `.x ~ .y > .z` も両方受理される。根拠は `cascade.rs`
/// `match_combinator_chain`/`match_from_element` の相互再帰にある: 兄弟
/// ジャンプは `ancestors` を不変のまま引き継ぐ (兄弟は親を共有するため) の
/// で祖先系 combinator へそのまま繋げられ、祖先ジャンプは遷移先の「自分
/// 自身の祖先チェーン」を正しく truncate 済みで引き継ぐため、その chain の
/// `.last()` が遷移先自身の親を指し、兄弟系 combinator へもそのまま
/// 繋げられる — どちらの合成方向にも追加の状態は要らない
/// (`match_combinator_chain` doc の "親の解決" note 参照)。
///
/// `:root`/`:empty`/`:first-child` 等はいずれも
/// `selectors` crate 自身の `parse_simple_pseudo_class`/
/// `parse_functional_pseudo_class` (selectors 0.39.0 `parser.rs`、依存
/// crate の公開 parse 分岐を直接読んで確認 — Stylo 実装は参照していない)
/// がこれら専用の `Component` variant へ直接 parse する —
/// `RaikiriSelectorParser::parse_non_ts_pseudo_class`/
/// `parse_non_ts_functional_pseudo_class` 経由の `Component::NonTSPseudoClass`
/// には一切ならない (`:hover`/`:active`/`:lang()`/`:dir()` のような
/// non-tree-structural pseudo-class だけがそちら経由)。`:nth-child(An+B of
/// S)` / `:nth-last-child(An+B of S)` (Selectors Level 4 §13.3.1/§13.3.2)
/// は `RaikiriSelectorParser::parse_nth_child_of()` が有効化しているため、
/// `selectors` crate により `Component::NthOf` として parse される。
/// `Component::NthOf` に保持された selector-list は `is_supported_selector` が
/// 再帰的に検査し、通常の selector と同じ対応済み component だけなら rule
/// tree に格納する。cascade 側ではその list に一致する兄弟だけを 1-based
/// の `An+B` 対象列に含め、`:nth-last-child()` では同じ filtered list を末尾
/// から数える。従って `of S` は省略時の全要素兄弟列へ暗黙に広げられず、
/// 未対応 component を含む selector は従来どおり rule ごと drop される。
/// `S` 内の nested `Component::Nth` / `Component::NthOf` は意図的に未対応
/// として扱う。Selectors L4 の文法上は `S` に complex-real-selector-list
/// を許している (Selectors L4 §13.3.1
/// <https://www.w3.org/TR/selectors-4/#the-nth-child-pseudo> /
/// §13.3.2 <https://www.w3.org/TR/selectors-4/#the-nth-last-child-pseudo>)
/// が、これは本実装が意図的に狭める bounded-support boundary
/// 既知の制約である。cascade matcher は `S` を各兄弟
/// について評価するため、nested nth component を許可すると sibling scan
/// が再帰的に増幅する。`is_supported_selector` は外側 selector の nth
/// component だけを許可し、`S` を検査するときは nested nth component を
/// scan 前に reject する。一方、通常の type/class/id/attribute selector と
/// 対応済み combinator を含む flat `S` は引き続き受理する。
///
/// spec: CSS Selectors Level 4 — class selector
/// <https://www.w3.org/TR/selectors-4/#class-html>、ID selector
/// <https://www.w3.org/TR/selectors-4/#id-selectors>、attribute selector
/// <https://www.w3.org/TR/selectors-4/#attribute-selectors>、descendant
/// combinator <https://www.w3.org/TR/selectors-4/#descendant-combinators>、
/// child combinator <https://www.w3.org/TR/selectors-4/#child-combinators>、
/// next-sibling combinator
/// <https://www.w3.org/TR/selectors-4/#adjacent-sibling-combinators>、
/// general-sibling combinator
/// <https://www.w3.org/TR/selectors-4/#general-sibling-combinators>、
/// `:lang()` <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>、`:dir()`
/// <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>。
///
/// # Invariant with `cascade.rs::compound_matches` / `match_combinator_chain`
///
/// この関数が受理する `Component` variant は、`cascade.rs` 側
/// (simple selector component は `compound_matches`、combinator は
/// `match_combinator_chain`) に対応する match arm が**必ず**存在しなければ
/// ならない — なければ、rule tree には乗るが cascade では絶対に match しない
/// (safety net の `_ => false` に落ちる) rule を静かに作ってしまう。逆方向の
/// 対応関係 (`compound_matches` の doc から本関数への pointer) は
/// `cascade.rs` 側に既にある。両者は独立した enumerate で、shared helper 化は
/// されていない (premature abstraction として見送り —
/// doc pointer で invariant を明示するに留める)。この関数と `cascade.rs`
/// 側の対応 match arm は combinator/pseudo-class の追加のたびに複数回
/// 同時編集することになるため、変更のたびにこの対応関係を保つこと。
///
/// **例外 (`Component::PseudoElement` / `Component::Combinator(Combinator::
/// PseudoElement)`)**: この 2 つだけはこの invariant の対象外 —
/// `compound_matches`/`match_combinator_chain` に対応 arm を追加する代わりに、
/// `cascade.rs::selector_matches_pseudo_element` という独立した matcher が
/// 丸ごと引き受けている (real element 自身への直接 match は
/// `compound_matches`'s `_ => false` safety net が `Component::PseudoElement`
/// を安全に reject し続ける — 詳細はこの関数の doc の `::before`/`::after`
/// 節、および `selector_matches_pseudo_element` 自身の doc)。
fn is_supported_selector_list(list: &SelectorList<RaikiriSelectorImpl>) -> bool {
    list.slice()
        .iter()
        .all(|selector| is_supported_selector(selector, true))
}

/// `allow_nth` は名前の通り `Component::Nth`/`NthOf` の可否を切り替えるが、
/// 同時に「この呼び出しは selector list の**外側** (top-level candidate
/// selector) か、それとも `:nth-child(An+B of S)` の `S` の**内側**か」も
/// 表す — `Component::PseudoElement`/`Combinator::PseudoElement` の受理も
/// この同じフラグで gate する (専用の第 2 引数を増やさず再利用): CSS
/// Selectors Level 4 は `:nth-child(of S)` の `S` にも pseudo-element を
/// 禁じており (`selectors` crate 自身がこの禁止を **grammar 側で** 強制済み
/// — 経験的に確認: `p:nth-child(2 of .x::before)` は `selectors` v0.39.0
/// 自体が `InvalidState` で parse error にする、`S` として
/// `Component::PseudoElement` を含む `SelectorList` はそもそも構築され得ない
/// — `lib.rs`'s
/// `parse_pseudo_element_rejected_inside_nth_child_of_selector_list` test
/// 参照)、`allow_nth == false` の再帰呼び出しでは受理しないことで
/// defense-in-depth を維持する (他の受理 component と同じ「upstream が防いで
/// いるはずでも safety net を重ねる」姿勢)。
fn is_supported_selector(selector: &Selector<RaikiriSelectorImpl>, allow_nth: bool) -> bool {
    is_supported_selector_with_relative_anchor(selector, allow_nth, false, false)
}

/// Same support check as [`is_supported_selector`], with an explicit allowance
/// for the anchor component that `selectors` inserts into a `:has()` relative
/// selector. The anchor is not a selector that can appear in ordinary CSS
/// input; it is an internal marker and must stay rejected outside that one
/// parser-produced context. `allow_invalid` is restricted to the selector
/// arguments of forgiving `:is()`/`:where()` lists, where `selectors` stores a
/// syntactically invalid branch as `Component::Invalid` for the matcher to
/// ignore.
fn is_supported_selector_with_relative_anchor(
    selector: &Selector<RaikiriSelectorImpl>,
    allow_nth: bool,
    allow_relative_anchor: bool,
    allow_invalid: bool,
) -> bool {
    use selectors::parser::{Combinator, Component};

    selector
        .iter_raw_match_order()
        .all(|component| match component {
            Component::LocalName(_)
            | Component::ExplicitUniversalType
            | Component::ExplicitAnyNamespace
            | Component::ExplicitNoNamespace
            | Component::DefaultNamespace(_)
            | Component::ID(_)
            | Component::Class(_)
            | Component::AttributeInNoNamespaceExists { .. }
            | Component::AttributeInNoNamespace { .. }
            | Component::Combinator(
                Combinator::Descendant
                | Combinator::Child
                | Combinator::NextSibling
                | Combinator::LaterSibling,
            )
            | Component::Root
            | Component::Empty => true,
            Component::Negation(selectors) => selectors.slice().iter().all(|selector| {
                // `:not()` is non-forgiving. A nested `:is()`/`:where()` still
                // applies its own forgiving handling when this recursion sees
                // that component.
                is_supported_selector_with_relative_anchor(selector, allow_nth, false, false)
            }),
            Component::Is(selectors) | Component::Where(selectors) => {
                selectors.slice().iter().all(|selector| {
                    // A nested logical selector matches an ordinary element;
                    // only the outer relative selector owns the `:has()`
                    // anchor marker. Invalid branches are legal here and are
                    // ignored by `selector_slice_matches_with_anchor`.
                    is_supported_selector_with_relative_anchor(selector, allow_nth, false, true)
                })
            }
            // cov:ignore: nested :has() is rejected by `selectors`; this is a future-parser safety net
            Component::Has(_) if allow_relative_anchor => false,
            Component::Has(relative_selectors) => relative_selectors.iter().all(|relative| {
                is_supported_selector_with_relative_anchor(
                    &relative.selector,
                    allow_nth,
                    true,
                    false,
                )
            }),
            Component::Nth(_) => allow_nth,
            Component::NthOf(data) => {
                allow_nth
                    && data.selectors().iter().all(|selector| {
                        is_supported_selector_with_relative_anchor(selector, false, false, false)
                    })
            }
            Component::NonTSPseudoClass(PseudoClass::Lang(_) | PseudoClass::Dir(_)) => true,
            Component::Combinator(Combinator::PseudoElement) => allow_nth,
            Component::PseudoElement(
                PseudoElem::Before | PseudoElem::After | PseudoElem::Marker,
            ) => allow_nth,
            Component::RelativeSelectorAnchor => allow_relative_anchor,
            Component::Invalid(_) => allow_invalid,
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dom::TestDoc;

    fn dom_with_style(css: &str) -> TestDoc {
        let mut doc = TestDoc::new();
        let style = doc.push_element(0, "style", None);
        doc.push_text(style, css);
        doc
    }

    #[test]
    fn empty_dom_returns_empty_ruletree() {
        let doc = TestDoc::new();
        assert!(build_rule_tree(&doc).style_rules.is_empty());
    }

    #[test]
    fn dom_without_style_returns_empty_ruletree() {
        let mut doc = TestDoc::new();
        doc.push_element(0, "p", None);
        assert!(build_rule_tree(&doc).style_rules.is_empty());
    }

    #[test]
    fn single_style_type_selector_captured() {
        let doc = dom_with_style("p { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[0].declarations.len(), 1);
    }

    #[test]
    fn class_selector_is_captured() {
        // class selector はもう drop されない — 両方残る。
        let doc = dom_with_style(".foo { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn id_selector_is_captured() {
        let doc = dom_with_style("#header { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn attribute_exists_selector_is_captured() {
        let doc = dom_with_style("[data-foo] { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn attribute_exists_selector_with_mixed_case_local_name_is_captured() {
        // `[Data-Foo]` (no value, no namespace) — unlike the value-bearing
        // form (`non_lowercase_attribute_name_with_value_selector_still_dropped`
        // below), the selectors crate parser routes this to
        // `Component::AttributeInNoNamespaceExists` regardless of the local
        // name's case (only `namespace.is_some()` sends the exists-only form
        // to `AttributeOther` — see `is_supported_selector_list`'s doc for
        // the parser trace). Must therefore be captured, not dropped.
        let doc = dom_with_style("[Data-Foo] { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn attribute_value_selector_is_captured() {
        let doc = dom_with_style("[data-foo=\"bar\"] { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn descendant_combinator_selector_is_captured() {
        // descendant combinator (`div p`) は もう
        // drop されない — both rules kept (was
        // `combinator_selector_still_dropped` when combinators
        // were entirely out of scope and this asserted `len() == 1`).
        let doc = dom_with_style("div p { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn child_combinator_selector_is_captured() {
        // child combinator acceptance: `ol > li` must be captured.
        let doc = dom_with_style("ol > li { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn structural_pseudo_class_selectors_are_captured() {
        // structural pseudo-class acceptance: `:root`/`:empty`/
        // `:first-child`/`:nth-child()`/`-of-type` counterparts must no
        // longer be dropped by `is_supported_selector_list` — pairs with
        // `pseudo_class_selector_still_dropped` (which pins that
        // `:hover`/`:active`, true `NonTSPseudoClass` components, remain
        // dropped; these are architecturally different `Component`
        // variants the `selectors` crate parses directly, see
        // `is_supported_selector_list`'s doc).
        let doc = dom_with_style(
            ":root { color: red } \
             p:empty { color: red } \
             li:first-child { color: red } \
             li:last-child { color: red } \
             li:only-child { color: red } \
             li:nth-child(2n+1) { color: red } \
             li:nth-last-child(1) { color: red } \
             h2:first-of-type { color: red } \
             h2:last-of-type { color: red } \
             h2:only-of-type { color: red } \
             h2:nth-of-type(2) { color: red } \
             h2:nth-last-of-type(1) { color: red }",
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(
            tree.style_rules.len(),
            12,
            "all 12 structural pseudo-class rules must be kept"
        );
    }

    #[test]
    fn logical_and_relational_pseudo_class_selectors_are_captured() {
        let doc = dom_with_style(
            "div:is(.featured, .selected) { color: red } \
             div:where(.featured, .selected) { color: blue } \
             div:has(> .featured) { color: green }",
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 3);
    }

    #[test]
    fn unsupported_pseudo_class_inside_logical_selector_is_dropped() {
        // `:hover` is syntactically parseable but not a supported matching
        // state. The support gate must not let `:is()`/`:has()` turn its
        // fail-closed matcher result into a false positive.
        let doc = dom_with_style(
            "div:is(:hover, .featured) { color: red } \
             div:has(:hover) { color: blue } \
             div { color: green }",
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    #[test]
    fn non_forgiving_has_rejects_invalid_and_nested_branches() {
        // Unlike :is()/:where(), :has() uses a non-forgiving relative
        // selector list. A malformed list or a directly nested :has() drops
        // the complete style rule.
        let doc = dom_with_style(
            "div:has(.featured, 123) { color: red } \
             div:has(.featured:has(.nested)) { color: blue } \
             div { color: green }",
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    #[test]
    fn forgiving_logical_selector_keeps_valid_branches() {
        // `selectors` represents a syntactically invalid branch in a
        // forgiving `:is()`/`:where()` list as `Component::Invalid`. That
        // branch must not make the whole stylesheet rule disappear; the
        // cascade matcher will ignore it and still try the valid branch.
        let doc = dom_with_style(
            "div:is(.featured, :unknown-pseudo) { color: red } \
             div:where(.selected, 123) { color: blue }",
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
    }

    #[test]
    fn nth_child_of_extended_syntax_selector_is_captured() {
        let doc = dom_with_style(
            "p:nth-child(2n+1 of .foo) { color: red } \
             p:nth-last-child(1 of [data-kind=selected]) { color: blue }",
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn nested_nth_child_of_selector_is_dropped() {
        let doc = dom_with_style("li:nth-child(2 of li:nth-child(2 of .featured)) { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 0);
    }

    #[test]
    fn chained_combinator_selector_is_captured() {
        // CSS Selectors L4 child-combinators
        // (<https://www.w3.org/TR/selectors-4/#child-combinators>) example
        // selector `div ol>li p`, verbatim from the spec — mixes descendant
        // and child combinators in one complex selector. Must be captured
        // whole (not partially, `is_supported_selector_list` walks every
        // component in the selector regardless of which combinator
        // separates it from its neighbours).
        let doc = dom_with_style("div ol>li p { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn sibling_combinator_selector_is_captured() {
        // next-sibling (`+`) / subsequent-sibling
        // (`~`) combinators are no longer dropped — both rules kept (was
        // `sibling_combinator_selector_still_dropped`, asserting
        // `len() == 1` / only the second `p` rule surviving).
        let doc = dom_with_style("p + p { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);

        let doc = dom_with_style("p ~ p { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn mixed_ancestor_and_sibling_combinator_selector_is_captured() {
        // mixing ancestor-chain (`>`/space) and
        // sibling-chain (`+`/`~`) combinators within one complex selector is
        // fully supported in both compositional orders (see
        // `is_supported_selector_list`'s "4 combinator 間の混在" doc note for
        // why no extra tracking state is needed either way) — must be
        // captured whole, not dropped.
        let doc = dom_with_style(".x > .y ~ .z { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);

        let doc = dom_with_style(".x ~ .y > .z { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn pseudo_class_selector_still_dropped() {
        // `:hover` (NonTSPseudoClass) は scope 外 —
        // 引き続き drop (safety net regression)。
        let doc = dom_with_style("p:hover { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    #[test]
    fn lang_and_dir_pseudo_class_selectors_are_captured() {
        // acceptance counterpart to
        // `descendant_combinator_selector_is_captured` /
        // `child_combinator_selector_is_captured` above — `:lang()`/`:dir()`
        // are the first `Component::NonTSPseudoClass` variants admitted by
        // `is_supported_selector_list` (siblings `:hover`/`:active` remain
        // dropped, pinned by `pseudo_class_selector_still_dropped` above).
        let doc = dom_with_style(":lang(ja) { color: red } :dir(ltr) { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn non_lowercase_attribute_name_with_value_selector_still_dropped() {
        // `[Data-Foo="bar"]` — local name が ASCII-lowercase でない**値付き**
        // 属性 selector は selectors crate 側の parse で
        // `Component::AttributeOther` (`is_supported_selector_list` 未対応の
        // 形態) になる。これは値付き形態限定の gate — 値なしの存在チェック
        // 形態 (`[Data-Foo]`) は非小文字でも namespace 無指定なら
        // `AttributeInNoNamespaceExists` のまま受理される
        // (`attribute_exists_selector_with_mixed_case_local_name_is_captured`
        // 参照)。値付き形態のみ引き続き drop (safety net regression)。
        let doc = dom_with_style("[Data-Foo=\"bar\"] { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    #[test]
    fn universal_selector_captured() {
        let doc = dom_with_style("* { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn multiple_style_elements_source_order() {
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(s1, "p { color: red }");
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(s2, "div { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn invalid_rule_silently_dropped() {
        // @nope; is retained as opaque data; malformed selector syntax is
        // still dropped from the compatibility style view.
        let doc = dom_with_style("@nope; p { color: red } ;;garbage;;");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn important_flag_captured() {
        let doc = dom_with_style("p { color: red !important }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert!(tree.style_rules[0].declarations[0].important);
    }

    /// `walk_and_collect` was recursive DFS —
    /// a deeply nested DOM (e.g. approaching `max_dom_nodes = 1M`) could
    /// stack-overflow the process. 5000-level linear chain with `<style>`
    /// at the deepest level (forcing the walk all the way down before
    /// finding rule text) must complete without overflow and still find
    /// the rule.
    #[test]
    fn deep_nesting_5000_build_rule_tree_no_overflow() {
        let mut doc = TestDoc::new();
        let mut parent = 0usize;
        for _ in 0..5000 {
            parent = doc.push_element(parent, "div", None);
        }
        let style = doc.push_element(parent, "style", None);
        doc.push_text(style, "div { color: red }");

        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].declarations.len(), 1);
    }

    // ── Origin + add_stylesheet ──

    #[test]
    fn origin_is_copy_eq() {
        fn assert_copy<T: Copy + PartialEq + Eq>() {}
        assert_copy::<Origin>();
        assert_ne!(Origin::UserAgent, Origin::Author);
        // 3rd variant (CSS Cascading L5 §6.5 "author
        // presentational hint origin") is pairwise distinct from both.
        assert_ne!(Origin::UserAgent, Origin::AuthorPresentationalHint);
        assert_ne!(Origin::AuthorPresentationalHint, Origin::Author);
    }

    #[test]
    fn add_stylesheet_ua_and_author_populate_rule_tree() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p { color: red }", Origin::UserAgent);
        tree.add_stylesheet("p { color: blue }", Origin::Author);

        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].origin, Origin::UserAgent);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].origin, Origin::Author);
        assert_eq!(tree.style_rules[1].source_order, 1);
    }

    #[test]
    fn add_stylesheet_source_order_monotonic_across_calls() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p { color: red }", Origin::UserAgent);
        tree.add_stylesheet("div { color: green }", Origin::UserAgent);
        tree.add_stylesheet("span { color: blue }", Origin::Author);
        let orders: Vec<u32> = tree.style_rules.iter().map(|r| r.source_order).collect();
        assert_eq!(orders, vec![0, 1, 2]);
    }

    #[test]
    fn add_stylesheet_dropped_selectors_do_not_consume_source_order() {
        // `div:hover` (pseudo-class, `NonTSPseudoClass`) is still unsupported
        // — dropped, `p` survives (was `div + p`: the
        // next-sibling combinator it used is now accepted, so this fixture
        // moved to a selector that remains genuinely unsupported — regression
        // intent unchanged: a dropped rule must not consume the
        // `source_order` counter).
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("div:hover { color: red } p { color: blue }", Origin::Author);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    #[test]
    fn build_rule_tree_produces_author_origin_for_dom_style_elements() {
        let doc = dom_with_style("p { color: red }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].origin, Origin::Author);
    }

    #[test]
    fn walk_style_elements_pub_visits_all_style_texts_in_document_order() {
        // 兄弟の <style> 2 個 → 呼び出し順で collected される。
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(s1, "p { color: red }");
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(s2, "div { color: blue }");

        let mut collected: Vec<String> = Vec::new();
        super::walk_style_elements(&doc, |css| collected.push(css.to_string()));

        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], "p { color: red }");
        assert_eq!(collected[1], "div { color: blue }");
    }

    /// regression net for document-order preservation
    /// across MIXED sibling/descendant depths — the existing sibling-only
    /// test above (2 flat `<style>` at the same depth) cannot distinguish a
    /// depth-first walk from a naive breadth-first one, because same-depth
    /// visit order happens to coincide for both strategies. This test uses
    /// a shape where the doc-order-earlier `<style>` sits *deeper* than the
    /// doc-order-later one:
    ///
    /// ```text
    /// root
    /// ├── section          (depth 1)
    /// │     └── mid        (depth 2)
    /// │           └── style A   (depth 3, text "p { color: red }")
    /// └── aside             (depth 1, later sibling of `section`)
    ///       └── style B    (depth 2, text "div { background-color: blue }")
    /// ```
    ///
    /// (A and B intentionally use different properties — `color` vs.
    /// `background-color` — so the `build_rule_tree` half below can check
    /// *which* rule landed at which `source_order`, not just that 2 rules
    /// exist.)
    ///
    /// Correct document order is A then B (pre-order: all of `section`'s
    /// subtree, including the depth-3 A, precedes `aside`'s subtree)
    /// regardless of A being deeper than B. A level-order (BFS) walk would
    /// instead visit the shallower B (depth 2) before the deeper A
    /// (depth 3), flipping the order — this is exactly the class of bug
    /// `walk_and_collect`'s doc comment warns is a correctness requirement,
    /// not just a behavior-compat nicety, because `source_order` feeds the
    /// cascade order-of-appearance tie-break (CSS Cascading L4
    /// <https://www.w3.org/TR/css-cascade-4/#cascade-sort>).
    ///
    /// Empirically confirmed: this test is
    /// the only one of the 57 tests in this module that goes red when
    /// `walk_and_collect` is mutated from `Vec`/LIFO-pop DFS to
    /// `VecDeque`/`pop_front` BFS — the pre-existing
    /// `walk_style_elements_pub_visits_all_style_texts_in_document_order`
    /// test (2 flat siblings, same depth) stays green under that mutation
    /// because same-depth visit order happens to coincide for DFS and BFS.
    #[test]
    fn walk_and_collect_preserves_document_order_across_mixed_sibling_descendant_depths() {
        use crate::property::PropertyValue;

        let mut doc = TestDoc::new();
        let section = doc.push_element(0, "section", None);
        let mid = doc.push_element(section, "mid", None);
        let style_a = doc.push_element(mid, "style", None);
        // A uses `color` — distinguishable from B's `background-color` below
        // so the build_rule_tree assertions can check *which* rule landed at
        // which source_order, not just that 2 rules exist.
        doc.push_text(style_a, "p { color: red }");

        let aside = doc.push_element(0, "aside", None); // later sibling of `section`
        let style_b = doc.push_element(aside, "style", None);
        doc.push_text(style_b, "div { background-color: blue }");

        // Entry point 1: raw text collection order via `walk_style_elements`.
        let mut collected: Vec<String> = Vec::new();
        super::walk_style_elements(&doc, |css| collected.push(css.to_string()));
        assert_eq!(collected.len(), 2);
        assert_eq!(collected[0], "p { color: red }");
        assert_eq!(collected[1], "div { background-color: blue }");

        // Entry point 2: `source_order` assigned via `build_rule_tree`, which
        // is the value that actually feeds the cascade tie-break — check it
        // too so a regression here is caught even if a future change routes
        // rule extraction through `build_rule_tree` without going through
        // the raw-text collection path in the same way. Distinguish A vs B
        // by declaration kind (Color vs BackgroundColor) rather than just
        // counting, so a swap is actually detected.
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
        assert!(matches!(
            tree.style_rules[0].declarations()[0].value(),
            PropertyValue::Color(_)
        ));
        assert!(matches!(
            tree.style_rules[1].declarations()[0].value(),
            PropertyValue::BackgroundColor(_)
        ));
    }

    #[test]
    fn style_inside_template_is_skipped_per_html_spec_inertness() {
        // <template> は spec 上 inert (HTML spec)。内部の <style> は cascade に流れない。
        // raikiri-html/src/sink.rs:315-318 の invariant と consistent。
        let mut doc = TestDoc::new();
        let template = doc.push_element(0, "template", None);
        let style_in_template = doc.push_element(template, "style", None);
        doc.push_text(style_in_template, "p { color: red }");

        // <template> 外の <style> は拾う必要がある (baseline)。
        let style_outer = doc.push_element(0, "style", None);
        doc.push_text(style_outer, "div { color: blue }");

        let tree = build_rule_tree(&doc);
        // <style> outer の 1 rule のみ (div{...})、template 内は skip。
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 0);
    }

    // ── @page at-rule scaffolding ──
    //
    // Spec: CSS Paged Media Level 3, §4.3 "@page rule grammar"
    // <https://www.w3.org/TR/css-page-3/#syntax-page-selector>
    //
    // Test で使う body は property.rs でサポート済み (color / font-*) を
    // 選ぶ — `crate::page::parse_page_declaration_block` は `size` / `marks` /
    // `bleed` descriptor を専用 grammar で受理するが (下の page_size_* /
    // page_marks_* / page_bleed_* test 群参照。ただしこれらは各 descriptor
    // 固有の grammar level の正しさ — 受理される値と拒否される値の境界 —
    // を検証するテストであり、次の一般的な drop mechanism 自体の証拠では
    // ない)、それ以外の未サポート property は `parse_value` が `None` を
    // 返し、その declaration ごと silent drop される (下の
    // page_body_unknown_property_is_dropped_declaration_survives がその
    // regression guard)。`PageDeclParser::parse_value` のこの分岐は
    // qualified rule 側の `crate::rule::DeclParser` と同型 (`None` を
    // `.ok_or_else` で `Err` に変換し、cssparser の error recovery が
    // その 1 declaration だけを skip する) だが、コード上は別コピーであり
    // qualified-rule 側の analogue
    // (`crate::rule::tests::drops_invalid_property_and_value` /
    // `crate::property::tests::unknown_property_returns_none`) はこの
    // `@page` 側の分岐までは検証しない。
    //
    // margin-box at-rule (`@top-left { … }` 等、L3 §5.1) は別の経路 —
    // そもそも declaration ではないため `parse_value` に届かず、
    // `PageDeclParser`'s `AtRuleParser::parse_prelude` が ident を
    // sixteen-slot 表と照合する。一致すれば本文を通常 declaration list
    // として parse し `PageRule::margin_box_rules` へ格納 (下の
    // "margin-box at-rules" test group 参照)。一致しない nested at-rule 名は
    // 引き続き `parse_prelude` の `Err` 経由で cssparser の error recovery が
    // block ごと skip する (下の
    // page_unknown_nested_at_rule_body_is_skipped_declaration_survives が
    // その regression guard)。
    //
    // NB: `margin` は author scope の supported property になった
    // (`parse_page_declaration_block` 出口で 4 longhand に展開) — margin-box
    // at-rule の本文でも同じ展開を通る (`parse_declaration_block` を再利用
    // するため)。margin-box **slot の幾何学的配置** (L3 §5 の sixteen slot
    // layout) は依然未実装のまま — 本 group が検証するのは構文解析と
    // declaration 保持のみ。

    use crate::page::{
        PageBleed, PageMarginBoxSlot, PageMarks, PageOrientation, PagePseudo, PageSelector,
        PageSelectorEntry, PageSize, PageSizeKeyword,
    };
    use crate::{Atom, Length, PageRule};

    /// Test helper — build a `PageSelector` with a single compound entry
    /// containing exactly the given ident and pseudo-page list. Reduces the
    /// verbosity of `PageSelector { entries: vec![PageSelectorEntry { ident,
    /// pseudos, .. }] }` at every assertion site.
    fn ps_single(ident: Option<Atom>, pseudos: Vec<PagePseudo>) -> PageSelector {
        PageSelector {
            entries: vec![PageSelectorEntry {
                ident,
                pseudos,
                ..PageSelectorEntry::default()
            }],
        }
    }

    fn page_rules(source: &str) -> Vec<PageRule> {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(source, Origin::Author);
        tree.page_rules
    }

    #[test]
    fn page_default_selector_no_prelude() {
        // `@page { color: red }` → empty prelude represented as one default
        // entry (PageSelector is a Vec<Entry> shape, uniform for future
        // cascade iteration). declarations は 1 個、origin は
        // `page_rules(...)` helper が Author hardcode。
        let rules = page_rules("@page { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![]));
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].source_order, 0);
        assert_eq!(rules[0].origin, Origin::Author);
    }

    #[test]
    fn page_pseudo_first() {
        let rules = page_rules("@page :first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
    }

    #[test]
    fn page_pseudo_left_right_blank() {
        let rules = page_rules(
            "@page :left { color: red } \
             @page :right { color: red } \
             @page :blank { color: red }",
        );
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::Left]));
        assert_eq!(rules[1].selector, ps_single(None, vec![PagePseudo::Right]));
        assert_eq!(rules[2].selector, ps_single(None, vec![PagePseudo::Blank]));
        // page_order は @page 独立の counter。
        assert_eq!(rules[0].source_order, 0);
        assert_eq!(rules[1].source_order, 1);
        assert_eq!(rules[2].source_order, 2);
    }

    #[test]
    fn page_named_selector() {
        let rules = page_rules("@page my-cover { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(Some(Atom::from("my-cover")), vec![])
        );
    }

    #[test]
    fn page_multi_pseudo_is_accepted() {
        // L3 `<page-selector>` = `[ <ident-token>?
        // <pseudo-page>* ]!` permits any number of adjacent pseudo-pages.
        // Compound rule ("No whitespace is allowed between the productions
        // in `<page-selector>` or `<pseudo-page>`") — the input must be
        // written without whitespace between the two pseudos, hence
        // `:first:left`, not `:first :left`. The spaced form is pinned by
        // `page_multi_pseudo_with_whitespace_between_is_dropped` below.
        let rules = page_rules("@page :first:left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(None, vec![PagePseudo::First, PagePseudo::Left])
        );
    }

    #[test]
    fn page_functional_pseudo_is_dropped() {
        // Spec-outside functional pseudo (e.g. `:nth-page(...)`) は CSS Paged
        // Media L3 (anchor `#syntax-page-selector`) も L4 Editor's Draft も
        // 定義していないため、raikiri-style としては未知 pseudo として rule ごと
        // drop する。もし raikiri-local な拡張として実装する日が来れば、そのときは
        // 明示的に variant を追加し (現在のこの guard test を反転) スコープを
        // 人間 ledger で決める。
        let rules = page_rules("@page :nth-page(2n+1) { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_ident_plus_pseudo_is_accepted() {
        // L3 `<page-selector>` allows an ident followed
        // by pseudo-pages (`named:first`). No whitespace between them per
        // the compound rule — the spaced form (`named :first`) is dropped
        // by `page_named_with_whitespace_before_pseudo_is_dropped` below.
        let rules = page_rules("@page named:first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(Some(Atom::from("named")), vec![PagePseudo::First])
        );
    }

    #[test]
    fn page_selector_list_with_comma_is_accepted() {
        // L3 `<page-selector-list>` = `<page-selector>#`
        // — a comma-separated list of compound page-selectors. Whitespace
        // around the `,` is spec-permitted (the list is not itself a
        // compound). Expected: one rule with two entries.
        let rules = page_rules("@page :first, :left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            PageSelector {
                entries: vec![
                    PageSelectorEntry {
                        ident: None,
                        pseudos: vec![PagePseudo::First],
                        ..PageSelectorEntry::default()
                    },
                    PageSelectorEntry {
                        ident: None,
                        pseudos: vec![PagePseudo::Left],
                        ..PageSelectorEntry::default()
                    },
                ],
            }
        );
    }

    #[test]
    fn page_unknown_pseudo_is_dropped() {
        // `:cover` は the L3 grammar (spec anchor `#syntax-page-selector`) に存在しないため drop。
        let rules = page_rules("@page :cover { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_pseudo_is_case_insensitive() {
        // CSS keyword は ASCII case-insensitive (`match_ignore_ascii_case!` 経由)。
        let rules = page_rules("@page :FIRST { color: red } @page :Left { color: red }");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
        assert_eq!(rules[1].selector, ps_single(None, vec![PagePseudo::Left]));
    }

    // ── Compound whitespace tightening ──
    //
    // CSS Paged Media L3 (anchor `#syntax-page-selector`) states: "No
    // whitespace is allowed between the productions in `<page-selector>` or
    // `<pseudo-page>` (similar to the rule for `<compound-selector>`)".
    // Whitespace *around* the `,` separator of `<page-selector-list>` is
    // allowed (the list is not a compound); that variant is exercised by
    // `page_comma_list_with_whitespace_around_comma_is_accepted` below.

    #[test]
    fn page_whitespace_between_colon_and_pseudo_is_dropped() {
        // `@page : left` — whitespace between `:` and pseudo ident violates
        // the `<pseudo-page>` compound rule. Whole rule dropped.
        let rules = page_rules("@page : left { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_multi_pseudo_with_whitespace_between_is_dropped() {
        // `@page :first :left` — whitespace between two pseudo-pages of the
        // same compound violates the `<page-selector>` compound rule.
        let rules = page_rules("@page :first :left { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_named_with_whitespace_before_pseudo_is_dropped() {
        // `@page my-cover :first` — whitespace between ident and pseudo of
        // the same compound violates the `<page-selector>` compound rule.
        let rules = page_rules("@page my-cover :first { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_comma_list_with_whitespace_around_comma_is_accepted() {
        // Whitespace around the `,` of `<page-selector-list>` is spec-
        // permitted (the list is not a compound). Both `,` and ` , ` and
        // `, ` MUST all yield the same two-entry result.
        for src in [
            "@page :first, :left { color: red }",
            "@page :first , :left { color: red }",
            "@page :first ,:left { color: red }",
        ] {
            let rules = page_rules(src);
            assert_eq!(rules.len(), 1, "source: {src:?}");
            assert_eq!(rules[0].selector.entries.len(), 2, "source: {src:?}");
        }
    }

    #[test]
    fn page_ident_with_multi_pseudo_is_accepted() {
        // Ident + multiple pseudo-pages, all adjacent — the fullest shape
        // the L3 compound grammar permits.
        let rules = page_rules("@page named:first:left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(
                Some(Atom::from("named")),
                vec![PagePseudo::First, PagePseudo::Left]
            )
        );
    }

    #[test]
    fn page_margin_box_at_rule_is_parsed_declaration_survives_alongside_it() {
        // A margin-box at-rule (`@top-left { … }`, CSS Paged Media Level 3
        // §5.1) is now recognized and its body stored on
        // `PageRule::margin_box_rules`, separate from the surrounding
        // `@page` block's own `declarations`. This pins both halves at
        // once: the ordinary `color: red` declaration lands in
        // `declarations` unaffected, and the nested `@top-left` block lands
        // in `margin_box_rules` with its own `content:` declaration parsed.
        use crate::property::PropertyValue;

        let rules = page_rules("@page :first { @top-left { content: 'x' } color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].margin_box_rules.len(), 1);
        assert_eq!(
            rules[0].margin_box_rules[0].slot,
            PageMarginBoxSlot::TopLeft
        );
        assert_eq!(rules[0].margin_box_rules[0].declarations.len(), 1);
        assert!(matches!(
            rules[0].margin_box_rules[0].declarations[0].value(),
            PropertyValue::Content(_)
        ));
    }

    // ── margin-box at-rules ──
    //
    // Spec: CSS Paged Media Level 3, §5.1 "At-rules for page-margin boxes"
    // <https://www.w3.org/TR/css-page-3/#margin-at-rules>. Sixteen at-rules,
    // each naming one page-margin box, no prelude, body = ordinary
    // declaration list (§5.2 "Populating page-margin boxes" calls out
    // `content:` specifically; §5.1 additionally restricts the body to
    // "page-margin properties" — a restriction this parser does not
    // enforce, see `PageMarginBoxRule`'s doc "Not filtered against the
    // applicable-property list" section).

    #[test]
    fn page_margin_box_all_sixteen_slots_recognized() {
        // Every one of the sixteen margin-box idents (CSS Paged Media Level
        // 3 §5.1) round-trips to its matching `PageMarginBoxSlot` variant.
        let cases: &[(&str, PageMarginBoxSlot)] = &[
            ("top-left-corner", PageMarginBoxSlot::TopLeftCorner),
            ("top-left", PageMarginBoxSlot::TopLeft),
            ("top-center", PageMarginBoxSlot::TopCenter),
            ("top-right", PageMarginBoxSlot::TopRight),
            ("top-right-corner", PageMarginBoxSlot::TopRightCorner),
            ("right-top", PageMarginBoxSlot::RightTop),
            ("right-middle", PageMarginBoxSlot::RightMiddle),
            ("right-bottom", PageMarginBoxSlot::RightBottom),
            ("bottom-right-corner", PageMarginBoxSlot::BottomRightCorner),
            ("bottom-right", PageMarginBoxSlot::BottomRight),
            ("bottom-center", PageMarginBoxSlot::BottomCenter),
            ("bottom-left", PageMarginBoxSlot::BottomLeft),
            ("bottom-left-corner", PageMarginBoxSlot::BottomLeftCorner),
            ("left-bottom", PageMarginBoxSlot::LeftBottom),
            ("left-middle", PageMarginBoxSlot::LeftMiddle),
            ("left-top", PageMarginBoxSlot::LeftTop),
        ];
        for (ident, expected_slot) in cases {
            let source = format!("@page {{ @{ident} {{ content: 'x' }} }}");
            let rules = page_rules(&source);
            assert_eq!(rules.len(), 1, "source: {source}");
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                rules[0].margin_box_rules.len(),
                1,
                "ident {ident} was not recognized as a margin-box at-rule"
            );
            assert_eq!(rules[0].margin_box_rules[0].slot, *expected_slot);
        }
    }

    #[test]
    fn page_margin_box_ident_is_case_insensitive() {
        // Matches every other at-rule/keyword production in this module —
        // `match_ignore_ascii_case!` in `PageDeclParser`'s `AtRuleParser`
        // impl.
        let rules = page_rules("@page { @TOP-LEFT { content: 'x' } }");
        assert_eq!(rules[0].margin_box_rules.len(), 1);
        assert_eq!(
            rules[0].margin_box_rules[0].slot,
            PageMarginBoxSlot::TopLeft
        );

        let rules = page_rules("@page { @Bottom-Right-Corner { content: 'x' } }");
        assert_eq!(rules[0].margin_box_rules.len(), 1);
        assert_eq!(
            rules[0].margin_box_rules[0].slot,
            PageMarginBoxSlot::BottomRightCorner
        );
    }

    #[test]
    fn page_unknown_nested_at_rule_body_is_skipped_declaration_survives() {
        // regression guard: a nested at-rule name that is *not* one of the
        // sixteen margin-box idents (here `@foo`, standing in for e.g. a
        // stray `@media`) still falls through `PageDeclParser`'s
        // `AtRuleParser::parse_prelude` to `Err`, and cssparser's
        // error-recovery skips just that nested block. The sibling
        // `color: blue` declaration survives, and no `PageMarginBoxRule` is
        // produced.
        let rules = page_rules("@page { @foo { color: red } color: blue }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 1);
        assert!(rules[0].margin_box_rules.is_empty());
    }

    #[test]
    fn page_margin_box_non_slot_hyphenated_ident_is_dropped() {
        // `@top-middle` is not one of the sixteen spec idents (the top edge
        // has left/center/right, not "middle" — that name is reserved for
        // the *right*/*left* edges' vertical slots). Confirms the ident
        // match is exact, not a prefix/substring match.
        let rules = page_rules("@page { @top-middle { content: 'x' } color: red }");
        assert_eq!(rules[0].declarations.len(), 1);
        assert!(rules[0].margin_box_rules.is_empty());
    }

    #[test]
    fn page_margin_box_non_empty_prelude_is_rejected() {
        // Margin-box at-rules take no prelude (`@top-left { <declaration-list> }`,
        // nothing between the ident and `{`). A stray token there —
        // `@top-left foo { … }` — is spec-invalid and the whole nested
        // block is dropped, same as an unrecognized ident. The sibling
        // declaration survives.
        let rules = page_rules("@page { @top-left foo { content: 'x' } color: red }");
        assert_eq!(rules[0].declarations.len(), 1);
        assert!(rules[0].margin_box_rules.is_empty());
    }

    #[test]
    fn page_margin_box_statement_form_without_block_is_rejected() {
        // `@top-left;` (no `{ … }` block, terminated by `;` instead) is
        // spec-invalid — the margin-box grammar is `@top-left { <declaration-list> }`,
        // always with a block. `PageDeclParser`'s `AtRuleParser` impl does
        // not override `rule_without_block`, so cssparser's default (`Err`)
        // applies and the whole statement is dropped, same as any other
        // malformed nested at-rule. The sibling declaration survives.
        let rules = page_rules("@page { @top-left; color: red }");
        assert_eq!(rules[0].declarations.len(), 1);
        assert!(rules[0].margin_box_rules.is_empty());
    }

    #[test]
    fn page_margin_box_content_counter_page_and_pages() {
        // `counter(page)` / `counter(pages)` — the automatic `page` counter
        // CSS Paged Media Level 3 §6.1 "Page-based counters"
        // (<https://www.w3.org/TR/css-page-3/#page-based-counters>) defines
        // ("A counter named page is automatically created and incremented
        // by 1 on every page of the document"); `pages` is its
        // document-total counterpart, defined further in the same section —
        // canonical use case a margin-box `content:` declaration exists for
        // (page-number headers/footers) — parses inside a margin-box body
        // via the same `ContentComponent` grammar
        // `crate::property::parse_content` already implements for ordinary
        // style rules.
        use crate::property::{ContentComponent, CounterStyle, PropertyValue};
        use smol_str::SmolStr;

        let rules = page_rules(
            "@page { @bottom-center { content: counter(page) \" / \" counter(pages) } }",
        );
        assert_eq!(rules[0].margin_box_rules.len(), 1);
        let decl = &rules[0].margin_box_rules[0].declarations[0];
        let items = match decl.value() {
            PropertyValue::Content(items) => items,
            // cov:ignore: defensive-only arm — `content: counter(page) " / "
            // counter(pages)` always parses to `PropertyValue::Content`, so
            // this panic is unreachable while the test passes.
            other => panic!("expected PropertyValue::Content, got {other:?}"),
        };
        assert_eq!(
            (**items).clone(),
            vec![
                ContentComponent::Counter {
                    name: SmolStr::new("page"),
                    style: CounterStyle::Decimal,
                },
                ContentComponent::Literal(SmolStr::new(" / ")),
                ContentComponent::Counter {
                    name: SmolStr::new("pages"),
                    style: CounterStyle::Decimal,
                },
            ]
        );
    }

    #[test]
    fn page_margin_box_body_is_not_filtered_at_parse_time() {
        // CSS Paged Media Level 3 §5.1 states "The margin at-rules can only
        // contain page-margin properties" — but this parser does not
        // enforce that restriction (see `PageMarginBoxRule`'s doc "Not filtered
        // against the applicable-property list" section), so a property
        // with no obvious margin-box meaning (`color` here) still parses
        // and is kept, same as `content:`. This also confirms the body
        // reuses `crate::rule::parse_declaration_block`'s shorthand
        // expansion (`margin:` here expands to 4 longhands, matching
        // `page_declaration_block_never_emits_shorthand_keys`'s guarantee
        // for the outer `@page` body), and that none of it leaks into the
        // surrounding `@page` block's own `declarations`.
        use crate::property::PropertyValue;

        let rules =
            page_rules("@page { @top-left { content: 'x'; color: red; margin: 1px 2px 3px 4px } }");
        assert_eq!(rules[0].margin_box_rules.len(), 1);
        assert!(rules[0].declarations.is_empty());
        let decls = &rules[0].margin_box_rules[0].declarations;
        assert_eq!(decls.len(), 6, "content + color + 4 margin longhands");
        for decl in decls {
            assert!(!matches!(decl.value(), PropertyValue::Margin(_)));
        }
    }

    #[test]
    fn page_margin_box_duplicate_slot_kept_as_separate_entries() {
        // Two `@top-left` blocks in one `@page` rule are both kept, in
        // source order — cascade winner selection across duplicate slots is
        // future scope (`PageMarginBoxRule`'s doc, "Scope" section), mirroring
        // `PageRule::size_declarations`'s own "kept in source order"
        // treatment of duplicate `size:` declarations.
        let rules = page_rules("@page { @top-left { content: 'a' } @top-left { content: 'b' } }");
        assert_eq!(rules[0].margin_box_rules.len(), 2);
        assert_eq!(
            rules[0].margin_box_rules[0].slot,
            PageMarginBoxSlot::TopLeft
        );
        assert_eq!(
            rules[0].margin_box_rules[1].slot,
            PageMarginBoxSlot::TopLeft
        );
    }

    #[test]
    fn page_margin_box_rules_preserve_source_order_interleaved_with_declarations() {
        // Margin-box at-rules interleaved with ordinary declarations and
        // `size`/`marks`/`bleed` descriptors all land in their own field,
        // each in its own source order — none of the four lists disturbs
        // another's ordering or count.
        let rules = page_rules(
            "@page { \
                @top-left { content: counter(page) } \
                color: red; \
                size: A4; \
                @bottom-right { content: 'end' } \
                marks: crop; \
             }",
        );
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 1);
        assert_eq!(rules[0].size_declarations.len(), 1);
        assert_eq!(rules[0].marks_declarations.len(), 1);
        assert_eq!(rules[0].margin_box_rules.len(), 2);
        assert_eq!(
            rules[0].margin_box_rules[0].slot,
            PageMarginBoxSlot::TopLeft
        );
        assert_eq!(
            rules[0].margin_box_rules[1].slot,
            PageMarginBoxSlot::BottomRight
        );
    }

    #[test]
    fn page_source_order_independent_from_style_rules() {
        // page_rules の source_order は style_rules と独立の counter。
        // 全 rule が同一 add_stylesheet call の origin (Author) を継承する
        // ことも同時に check する (`PageRule.origin`、cascade 適用の pre-work)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "p { color: red } \
             @page :first { color: red } \
             div { color: blue } \
             @page :left { color: red }",
            Origin::Author,
        );
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.style_rules[0].source_order, 0);
        assert_eq!(tree.style_rules[1].source_order, 1);
        assert_eq!(tree.page_rules.len(), 2);
        assert_eq!(tree.page_rules[0].source_order, 0);
        assert_eq!(tree.page_rules[0].origin, Origin::Author);
        assert_eq!(tree.page_rules[1].source_order, 1);
        assert_eq!(tree.page_rules[1].origin, Origin::Author);
        assert_eq!(tree.rules().len(), 4);
        assert_eq!(tree.rules()[0].source_order, 0);
        assert_eq!(tree.rules()[0].kind, CssRuleKind::Style { index: 0 });
        assert_eq!(tree.rules()[1].source_order, 1);
        assert_eq!(tree.rules()[1].kind, CssRuleKind::Page { index: 0 });
        assert_eq!(tree.rules()[2].source_order, 2);
        assert_eq!(tree.rules()[2].kind, CssRuleKind::Style { index: 1 });
        assert_eq!(tree.rules()[3].source_order, 3);
        assert_eq!(tree.rules()[3].kind, CssRuleKind::Page { index: 1 });
    }

    #[test]
    fn cross_kind_rule_order_preserves_stylesheet_order() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "p { color: red } @media print { p { color: blue } } \
             @page :first { color: green } div { color: black }",
            Origin::Author,
        );
        assert_eq!(
            tree.rules()
                .iter()
                .map(|rule| rule.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                CssRuleKind::Style { index: 0 },
                CssRuleKind::AtRule { index: 0 },
                CssRuleKind::Page { index: 0 },
                CssRuleKind::Style { index: 1 },
            ]
        );
        assert_eq!(tree.opaque_at_rules()[0].source_order, 1);
    }

    #[test]
    fn cross_kind_rule_order_continues_across_stylesheets() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p { color: red } @future { x: y }", Origin::Author);
        tree.add_stylesheet(
            "@page :first { color: blue } div { color: green }",
            Origin::User,
        );

        assert_eq!(
            tree.rules()
                .iter()
                .map(|rule| rule.source_order)
                .collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
        assert_eq!(tree.rules()[2].origin, Origin::User);
        assert_eq!(tree.opaque_at_rules()[0].source_order, 1);
    }

    #[test]
    fn page_source_order_monotonic_across_add_stylesheet_calls() {
        // 複数 add_stylesheet 呼び出し間で page_order は継続する。
        // 各 rule の origin は当該 add_stylesheet call の引数に一致することを
        // check する (`PageRule.origin` は per-call の origin を保持し、cascade
        // 側で re-index せず per-origin cascade を組めるようにするための
        // pre-work — CSS Cascading L4
        // <https://www.w3.org/TR/css-cascade-4/#cascade-origin>)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page :first { color: red }", Origin::UserAgent);
        tree.add_stylesheet("@page :left { color: blue }", Origin::Author);
        assert_eq!(tree.page_rules.len(), 2);
        assert_eq!(tree.page_rules[0].source_order, 0);
        assert_eq!(tree.page_rules[0].origin, Origin::UserAgent);
        assert_eq!(tree.page_rules[1].source_order, 1);
        assert_eq!(tree.page_rules[1].origin, Origin::Author);
    }

    #[test]
    fn page_body_size_marks_bleed_all_parsed_together() {
        // `size` / `marks` / `bleed` each have a dedicated grammar
        // (`crate::page::parse_page_size_value` /
        // `parse_page_marks_value` / `parse_page_bleed_value`) — this test
        // pins that all three parse independently and land in their own
        // `*_declarations` field when they appear together in one `@page`
        // block (the `page_size_*` / `page_marks_*` / `page_bleed_*` test
        // groups elsewhere in this module each exercise only one descriptor
        // in isolation).
        //
        // The fixture also throws in a margin-box at-rule (`@top-left { … }`,
        // CSS Paged Media Level 3 §5.1) to confirm it lands in its own
        // `margin_box_rules` field alongside the three descriptors, rather
        // than in `declarations` or disturbing their counts — the
        // "margin-box at-rules" test group below covers that field's
        // grammar/shape in depth; this is the "all four coexist in one
        // block" cross-check.
        let rules =
            page_rules("@page { size: A4; marks: crop; bleed: 6pt; @top-left { content: 'x' } }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![]));
        assert!(rules[0].declarations.is_empty());
        assert_eq!(rules[0].margin_box_rules.len(), 1);
        assert_eq!(
            rules[0].margin_box_rules[0].slot,
            PageMarginBoxSlot::TopLeft
        );
        assert_eq!(rules[0].size_declarations.len(), 1);
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Named {
                keyword: Some(PageSizeKeyword::A4),
                orientation: None,
            }
        );
        assert!(!rules[0].size_declarations[0].important);
        assert_eq!(rules[0].marks_declarations.len(), 1);
        assert_eq!(
            rules[0].marks_declarations[0].value,
            PageMarks::Marks {
                crop: true,
                cross: false,
            }
        );
        assert!(!rules[0].marks_declarations[0].important);
        assert_eq!(rules[0].bleed_declarations.len(), 1);
        assert_eq!(
            rules[0].bleed_declarations[0].value,
            PageBleed::Length(Length::Pt(6.0))
        );
        assert!(!rules[0].bleed_declarations[0].important);
    }

    #[test]
    fn page_declaration_block_never_emits_shorthand_keys() {
        // Call site 4 of `expand_shorthand_into` (see that function's doc in
        // `crate::rule`) — `parse_page_declaration_block` must expand
        // `margin`/`padding`/`border`/`outline` shorthand the same way the
        // qualified-rule parse exit does, so a consumer reading
        // `PageRule::declarations` directly never observes a raw shorthand
        // `PropertyValue` straight out of parsing (before `cascade_page`'s
        // own defense-in-depth re-expansion even runs).
        use crate::property::PropertyValue;

        let rules = page_rules("@page { margin: 1cm 2cm 3cm 4cm; outline: auto 2px red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 7);
        for decl in &rules[0].declarations {
            assert!(!matches!(
                decl.value(),
                PropertyValue::Margin(_) | PropertyValue::Outline(_)
            ));
        }
    }

    // ── `size` descriptor grammar ──
    //
    // Spec: CSS Paged Media Level 3, §7.1 "Page size: the size property"
    // <https://www.w3.org/TR/css-page-3/#page-size-prop>. Grammar:
    // `<length>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]`.

    #[test]
    fn page_size_auto() {
        let rules = page_rules("@page { size: auto }");
        assert_eq!(rules[0].size_declarations.len(), 1);
        assert_eq!(rules[0].size_declarations[0].value, PageSize::Auto);
    }

    #[test]
    fn page_size_two_lengths() {
        let rules = page_rules("@page { size: 210mm 297mm }");
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Lengths {
                width: Length::Mm(210.0),
                height: Length::Mm(297.0),
            }
        );
    }

    #[test]
    fn page_size_single_length_sets_both_width_and_height() {
        // "If only one length value is specified, it sets both the width
        // and height of the page box (i.e., the box is a square)."
        let rules = page_rules("@page { size: 10em }");
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Lengths {
                width: Length::Em(10.0),
                height: Length::Em(10.0),
            }
        );
    }

    #[test]
    fn page_size_unitless_zero() {
        // CSS Values 3 §5 unitless-zero clause — reaches `size` the same
        // way it reaches every other `<length>` consumer in this crate.
        let rules = page_rules("@page { size: 0 }");
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Lengths {
                width: Length::Px(0.0),
                height: Length::Px(0.0)
            }
        );
    }

    #[test]
    fn page_size_percentage_rejected() {
        // Grammar alternative 1 is `<length>`, not `<length-percentage>` —
        // `%` is outside the `size` descriptor grammar entirely.
        let rules = page_rules("@page { size: 50% }");
        assert!(rules[0].size_declarations.is_empty());
    }

    #[test]
    fn page_size_negative_length_rejected() {
        // "Negative lengths are illegal."
        let rules = page_rules("@page { size: -10px }");
        assert!(rules[0].size_declarations.is_empty());
    }

    #[test]
    fn page_size_three_lengths_rejected() {
        // Grammar caps at `{1,2}` — a third length is trailing garbage that
        // must drop the whole declaration, not silently truncate to the
        // first two.
        let rules = page_rules("@page { size: 10px 20px 30px }");
        assert!(rules[0].size_declarations.is_empty());
    }

    #[test]
    fn page_size_second_length_negative_drops_whole_declaration() {
        // Non-obvious path: the second length fails
        // `parse_non_negative_length` and `try_parse` rewinds, so
        // `parse_page_size_value` falls through to the "only one length was
        // authored" arm (`Lengths { width: 10px, height: 10px }`) — but the
        // leftover `-5px` tokens are still unconsumed, so
        // `PageDeclParser::parse_value`'s `expect_exhausted` call must
        // reject the whole declaration rather than silently accepting that
        // truncated-to-one-length reading.
        let rules = page_rules("@page { size: 10px -5px }");
        assert!(rules[0].size_declarations.is_empty());
    }

    #[test]
    fn page_size_named_keyword_alone() {
        let rules = page_rules("@page { size: A4 }");
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Named {
                keyword: Some(PageSizeKeyword::A4),
                orientation: None,
            }
        );
    }

    #[test]
    fn page_size_orientation_alone() {
        // `||` combinator: the `<page-size>` half is optional as long as
        // `portrait | landscape` is present.
        let rules = page_rules("@page { size: landscape }");
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Named {
                keyword: None,
                orientation: Some(PageOrientation::Landscape),
            }
        );
    }

    #[test]
    fn page_size_keyword_and_orientation_either_order() {
        // `||` combinator: both sub-components may appear, in either source
        // order (spec examples this with `A4 landscape`).
        let forward = page_rules("@page { size: A4 landscape }");
        let backward = page_rules("@page { size: landscape A4 }");
        let expected = PageSize::Named {
            keyword: Some(PageSizeKeyword::A4),
            orientation: Some(PageOrientation::Landscape),
        };
        assert_eq!(forward[0].size_declarations[0].value, expected);
        assert_eq!(backward[0].size_declarations[0].value, expected);
    }

    #[test]
    fn page_size_duplicate_keyword_rejected() {
        // Each of `<page-size>` / `portrait|landscape` may appear at most
        // once under `||` — a second page-size keyword is not a second
        // valid alternative, it's trailing garbage.
        let rules = page_rules("@page { size: A4 A3 }");
        assert!(rules[0].size_declarations.is_empty());
    }

    #[test]
    fn page_size_duplicate_orientation_rejected() {
        let rules = page_rules("@page { size: A4 landscape portrait }");
        assert!(rules[0].size_declarations.is_empty());
    }

    #[test]
    fn page_size_important() {
        let rules = page_rules("@page { size: A4 !important }");
        assert_eq!(rules[0].size_declarations.len(), 1);
        assert!(rules[0].size_declarations[0].important);
    }

    // Runs the `value()` accessor body: doctests aren't covered by this
    // repo's coverage toolchain, so the compile-fail check on
    // `PageSizeDeclaration`'s struct doc doesn't exercise it. Every other
    // test above reaches into the same-crate `pub(crate)` field directly,
    // which never calls the accessor at all. Asserted against a concrete
    // expected value (not `decl.value`) so this can't degrade into a
    // tautological "the accessor returns the field" check.
    #[test]
    fn page_size_declaration_value_accessor_matches_the_field() {
        let rules = page_rules("@page { size: A4 landscape }");
        let decl = rules[0].size_declarations[0];
        assert_eq!(
            decl.value(),
            PageSize::Named {
                keyword: Some(PageSizeKeyword::A4),
                orientation: Some(PageOrientation::Landscape),
            }
        );
    }

    #[test]
    fn page_size_is_case_insensitive() {
        // CSS keyword は ASCII case-insensitive (`page_pseudo_is_case_insensitive`
        // と同じ規約) — `auto` は `expect_ident_matching` (内部で
        // `eq_ignore_ascii_case`), `<page-size>` / orientation keyword は
        // `match_ignore_ascii_case!` 経由で、どちらも大文字小文字を区別しない。
        let auto = page_rules("@page { size: AUTO }");
        assert_eq!(auto[0].size_declarations[0].value, PageSize::Auto);

        let named = page_rules("@page { size: A4 LANDSCAPE }");
        assert_eq!(
            named[0].size_declarations[0].value,
            PageSize::Named {
                keyword: Some(PageSizeKeyword::A4),
                orientation: Some(PageOrientation::Landscape),
            }
        );

        // Spec writes this one as `JIS-B5` (mixed case) — lowercase must
        // still match.
        let jis = page_rules("@page { size: jis-b5 }");
        assert_eq!(
            jis[0].size_declarations[0].value,
            PageSize::Named {
                keyword: Some(PageSizeKeyword::JisB5),
                orientation: None,
            }
        );
    }

    #[test]
    fn page_size_all_10_keyword_variants_accepted() {
        // All 10 `<page-size>` alternatives smoke-tested individually (arm
        // deletion regression detector) — same pattern as
        // `border_style_all_10_variants_accepted` in property.rs.
        fn named(source: &str, keyword: PageSizeKeyword) {
            let rules = page_rules(&format!("@page {{ size: {source} }}"));
            // cov:ignore: panic-message literal only executed on assertion
            // failure, which doesn't happen while this test passes.
            assert_eq!(
                rules[0].size_declarations[0].value,
                PageSize::Named {
                    keyword: Some(keyword),
                    orientation: None,
                },
                "size: {source}"
            );
        }
        named("A5", PageSizeKeyword::A5);
        named("A4", PageSizeKeyword::A4);
        named("A3", PageSizeKeyword::A3);
        named("B5", PageSizeKeyword::B5);
        named("B4", PageSizeKeyword::B4);
        named("JIS-B5", PageSizeKeyword::JisB5);
        named("JIS-B4", PageSizeKeyword::JisB4);
        named("letter", PageSizeKeyword::Letter);
        named("legal", PageSizeKeyword::Legal);
        named("ledger", PageSizeKeyword::Ledger);
    }

    #[test]
    fn page_size_both_orientation_variants_accepted() {
        let portrait = page_rules("@page { size: portrait }");
        assert_eq!(
            portrait[0].size_declarations[0].value,
            PageSize::Named {
                keyword: None,
                orientation: Some(PageOrientation::Portrait),
            }
        );
        let landscape = page_rules("@page { size: landscape }");
        assert_eq!(
            landscape[0].size_declarations[0].value,
            PageSize::Named {
                keyword: None,
                orientation: Some(PageOrientation::Landscape),
            }
        );
    }

    #[test]
    fn page_body_unknown_property_is_dropped_declaration_survives() {
        // regression guard: `PageDeclParser::parse_value`'s ordinary-property
        // arm dispatches to `crate::property::parse_value` and turns a
        // `None` return into an `Err` (`.ok_or_else`) for that one
        // declaration; cssparser's error recovery then skips just that
        // declaration, leaving the ones before and after it alone. This
        // pins the actually-unknown-property-name case specifically —
        // trailing garbage *after* a known property's value is a different
        // failure inside the same arm (`expect_exhausted` rejecting a
        // successful `parse_value` result), covered separately by
        // `page_body_ordinary_property_trailing_garbage_drops_declaration`
        // below.
        //
        // `cursor` is used as the unsupported-property canary, matching the
        // choice already made by `crate::rule::tests::drops_invalid_property_and_value`
        // and `crate::property::tests::unknown_property_returns_none` —
        // relocate to a different still-unimplemented property name if
        // `cursor` gains support.
        let rules = page_rules("@page { cursor: pointer; color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].declarations.len(), 1);
    }

    #[test]
    fn page_body_ordinary_property_trailing_garbage_drops_declaration() {
        // `PageDeclParser::parse_value`'s ordinary-property arm (the
        // `crate::property::parse_value` dispatch, distinct from the `size`
        // arm) has its own `expect_exhausted` guard — this exercises *that*
        // copy specifically (the `size` arm's twin is already covered by
        // `page_size_three_lengths_rejected` and friends): `color: red` on
        // its own parses cleanly, but trailing garbage after the value must
        // still drop the whole declaration, exactly like the qualified-rule
        // path's `crate::rule::DeclParser` does.
        let rules = page_rules("@page { color: red garbage }");
        assert_eq!(rules.len(), 1);
        assert!(rules[0].declarations.is_empty());
    }

    #[test]
    fn page_size_duplicate_declarations_kept_in_source_order() {
        // `PageRule::size_declarations` mirrors `PageRule::declarations` —
        // every declaration is kept in source order rather than reduced to
        // a single winner at parse time (see that field's doc for why).
        let rules = page_rules("@page { size: A4 !important; size: A5 }");
        assert_eq!(rules[0].size_declarations.len(), 2);
        assert_eq!(
            rules[0].size_declarations[0].value,
            PageSize::Named {
                keyword: Some(PageSizeKeyword::A4),
                orientation: None,
            }
        );
        assert!(rules[0].size_declarations[0].important);
        assert_eq!(
            rules[0].size_declarations[1].value,
            PageSize::Named {
                keyword: Some(PageSizeKeyword::A5),
                orientation: None,
            }
        );
        assert!(!rules[0].size_declarations[1].important);
    }

    // ── `marks` descriptor grammar ──
    //
    // Spec: CSS Paged Media Level 3, §7.2 "Crop and Registration Marks: the
    // marks property" <https://www.w3.org/TR/css-page-3/#marks>. Grammar:
    // `none | [ crop || cross ]`.

    #[test]
    fn page_marks_none() {
        let rules = page_rules("@page { marks: none }");
        assert_eq!(rules[0].marks_declarations.len(), 1);
        assert_eq!(rules[0].marks_declarations[0].value, PageMarks::None);
    }

    #[test]
    fn page_marks_crop_alone() {
        // `||` combinator: `cross` is optional as long as `crop` is present.
        let rules = page_rules("@page { marks: crop }");
        assert_eq!(
            rules[0].marks_declarations[0].value,
            PageMarks::Marks {
                crop: true,
                cross: false,
            }
        );
    }

    #[test]
    fn page_marks_cross_alone() {
        // `||` combinator: `crop` is optional as long as `cross` is present.
        let rules = page_rules("@page { marks: cross }");
        assert_eq!(
            rules[0].marks_declarations[0].value,
            PageMarks::Marks {
                crop: false,
                cross: true,
            }
        );
    }

    #[test]
    fn page_marks_crop_and_cross_either_order() {
        // `||` combinator: both sub-components may appear, in either source
        // order.
        let forward = page_rules("@page { marks: crop cross }");
        let backward = page_rules("@page { marks: cross crop }");
        let expected = PageMarks::Marks {
            crop: true,
            cross: true,
        };
        assert_eq!(forward[0].marks_declarations[0].value, expected);
        assert_eq!(backward[0].marks_declarations[0].value, expected);
    }

    #[test]
    fn page_marks_duplicate_crop_rejected() {
        // Each of `crop` / `cross` may appear at most once under `||` — a
        // second `crop` is trailing garbage, not a second valid alternative.
        let rules = page_rules("@page { marks: crop crop }");
        assert!(rules[0].marks_declarations.is_empty());
    }

    #[test]
    fn page_marks_duplicate_cross_rejected() {
        // Mirrors `page_marks_duplicate_crop_rejected` for the other `||`
        // alternative — `parse_page_marks_value`'s `!cross` guard must reject
        // a second `cross` the same way its `!crop` guard rejects a second
        // `crop`.
        let rules = page_rules("@page { marks: cross cross }");
        assert!(rules[0].marks_declarations.is_empty());
    }

    #[test]
    fn page_marks_unknown_keyword_rejected() {
        let rules = page_rules("@page { marks: foo }");
        assert!(rules[0].marks_declarations.is_empty());
    }

    #[test]
    fn page_marks_none_with_trailing_garbage_rejected() {
        // `none` is a distinct top-level alternative, not combinable with
        // `crop`/`cross` — trailing garbage after it drops the whole
        // declaration (`expect_exhausted` in `PageDeclParser::parse_value`).
        let rules = page_rules("@page { marks: none crop }");
        assert!(rules[0].marks_declarations.is_empty());
    }

    #[test]
    fn page_marks_important() {
        let rules = page_rules("@page { marks: crop !important }");
        assert_eq!(rules[0].marks_declarations.len(), 1);
        assert!(rules[0].marks_declarations[0].important);
    }

    #[test]
    fn page_marks_is_case_insensitive() {
        let none = page_rules("@page { marks: NONE }");
        assert_eq!(none[0].marks_declarations[0].value, PageMarks::None);

        let mixed = page_rules("@page { marks: Crop Cross }");
        assert_eq!(
            mixed[0].marks_declarations[0].value,
            PageMarks::Marks {
                crop: true,
                cross: true,
            }
        );
    }

    // Runs the `value()` accessor body — see
    // `page_size_declaration_value_accessor_matches_the_field`'s doc for why
    // this needs its own dedicated test (direct field access elsewhere never
    // calls the accessor).
    #[test]
    fn page_marks_declaration_value_accessor_matches_the_field() {
        let rules = page_rules("@page { marks: crop cross }");
        let decl = rules[0].marks_declarations[0];
        assert_eq!(
            decl.value(),
            PageMarks::Marks {
                crop: true,
                cross: true,
            }
        );
    }

    // ── `bleed` descriptor grammar ──
    //
    // Spec: CSS Paged Media Level 3, §7.3 "Bleed Area: the bleed property"
    // <https://www.w3.org/TR/css-page-3/#bleed>. Grammar: `auto | <length>`.

    #[test]
    fn page_bleed_auto() {
        let rules = page_rules("@page { bleed: auto }");
        assert_eq!(rules[0].bleed_declarations.len(), 1);
        assert_eq!(rules[0].bleed_declarations[0].value, PageBleed::Auto);
    }

    #[test]
    fn page_bleed_length() {
        let rules = page_rules("@page { bleed: 6pt }");
        assert_eq!(
            rules[0].bleed_declarations[0].value,
            PageBleed::Length(Length::Pt(6.0))
        );
    }

    #[test]
    fn page_bleed_negative_length_accepted() {
        // Unlike `size`'s `<length>` alternative ("Negative lengths are
        // illegal"), `bleed`'s explicitly permits negative values: "Values
        // may be negative, but there may be implementation-specific
        // limits."
        let rules = page_rules("@page { bleed: -6pt }");
        assert_eq!(
            rules[0].bleed_declarations[0].value,
            PageBleed::Length(Length::Pt(-6.0))
        );
    }

    #[test]
    fn page_bleed_percentage_rejected() {
        // Grammar is `<length>`, not `<length-percentage>` — `%` is outside
        // the `bleed` descriptor grammar entirely.
        let rules = page_rules("@page { bleed: 50% }");
        assert!(rules[0].bleed_declarations.is_empty());
    }

    #[test]
    fn page_bleed_unknown_keyword_rejected() {
        let rules = page_rules("@page { bleed: foo }");
        assert!(rules[0].bleed_declarations.is_empty());
    }

    #[test]
    fn page_bleed_trailing_garbage_drops_whole_declaration() {
        // Unlike `page_bleed_unknown_keyword_rejected` (nothing in the
        // grammar matches at all), this exercises the `bleed` arm's own
        // `expect_exhausted` guard in `PageDeclParser::parse_value`: `6pt`
        // parses cleanly as a valid `<length>`, but leftover tokens after it
        // must still drop the whole declaration rather than silently
        // truncating to the successfully-parsed prefix.
        let rules = page_rules("@page { bleed: 6pt garbage }");
        assert!(rules[0].bleed_declarations.is_empty());
    }

    #[test]
    fn page_bleed_important() {
        let rules = page_rules("@page { bleed: 6pt !important }");
        assert_eq!(rules[0].bleed_declarations.len(), 1);
        assert!(rules[0].bleed_declarations[0].important);
    }

    #[test]
    fn page_bleed_is_case_insensitive() {
        let rules = page_rules("@page { bleed: AUTO }");
        assert_eq!(rules[0].bleed_declarations[0].value, PageBleed::Auto);
    }

    // Runs the `value()` accessor body — see
    // `page_size_declaration_value_accessor_matches_the_field`'s doc for why
    // this needs its own dedicated test.
    #[test]
    fn page_bleed_declaration_value_accessor_matches_the_field() {
        let rules = page_rules("@page { bleed: 6pt }");
        let decl = rules[0].bleed_declarations[0];
        assert_eq!(decl.value(), PageBleed::Length(Length::Pt(6.0)));
    }

    #[test]
    fn leading_charset_is_retained_despite_cssparser_special_case() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            " /* comment */ @ChArSeT \"utf-8\"; p { color: red }",
            Origin::Author,
        );

        assert_eq!(tree.opaque_at_rules().len(), 1);
        assert_eq!(tree.opaque_at_rules()[0].name, "charset");
        assert_eq!(tree.opaque_at_rules()[0].prelude, " \"utf-8\"");
        assert_eq!(tree.rules()[0].kind, CssRuleKind::AtRule { index: 0 });
        assert_eq!(tree.rules()[1].kind, CssRuleKind::Style { index: 0 });
    }

    #[test]
    fn escaped_leading_charset_name_is_retained() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(r#"@ch\61 rset "utf-8"; p { color: red }"#, Origin::Author);

        assert_eq!(tree.opaque_at_rules().len(), 1);
        assert_eq!(tree.opaque_at_rules()[0].name, "charset");
        assert_eq!(tree.rules()[1].kind, CssRuleKind::Style { index: 0 });
    }

    #[test]
    fn valid_at_rules_are_retained_and_media_rules_are_indexed() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@media print { p { color: red } @nested feature; } \
             @supports (display: block) { p { color: green } } \
             p { color: blue }",
            Origin::Author,
        );

        assert_eq!(tree.page_rules.len(), 0);
        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.media_rules.len(), 1);
        assert_eq!(tree.media_rules[0].rule.source_order, 0);
        assert_eq!(tree.style_rules[0].source_order, 1);
        assert_eq!(tree.style_rules[1].source_order, 2);
        assert_eq!(tree.opaque_at_rules().len(), 2);
        assert_eq!(tree.opaque_at_rules()[0].name, "media");
        assert_eq!(tree.opaque_at_rules()[1].name, "supports");
        assert_eq!(tree.opaque_at_rules()[0].source_order, 0);
        assert_eq!(tree.opaque_at_rules()[1].source_order, 2);

        let media = &tree.opaque_at_rules()[0];
        assert_eq!(
            media.body.as_block(),
            Some(" p { color: red } @nested feature; ")
        );
        assert_eq!(media.children().len(), 2);
        assert!(matches!(media.children()[0], RuleNode::Qualified(_)));
        let RuleNode::AtRule(nested) = &media.children()[1] else {
            panic!("nested at-rule was not retained");
        };
        assert_eq!(nested.name, "nested");
        assert_eq!(nested.body, AtRuleBody::Statement);
        assert_eq!(nested.source_order, 1);
    }

    #[test]
    fn supports_exposes_supported_qualified_rules_but_keeps_opaque_record() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@supports (display: block) { p { color: red } } \
             @supports (display: definitely-unsupported) { p { color: blue } } \
             @supports not (display: definitely-unsupported) { p { color: green } }",
            Origin::Author,
        );

        assert_eq!(tree.style_rules.len(), 2);
        assert_eq!(tree.opaque_at_rules().len(), 3);
        assert_eq!(tree.opaque_at_rules()[0].name, "supports");
        assert_eq!(tree.opaque_at_rules()[1].name, "supports");
        assert_eq!(tree.opaque_at_rules()[2].name, "supports");
    }

    #[test]
    fn media_style_order_continues_across_stylesheets() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@media print { p { color: red } }", Origin::Author);
        tree.add_stylesheet("p { color: blue }", Origin::Author);

        assert_eq!(tree.media_rules.len(), 1);
        assert_eq!(tree.media_rules[0].rule.source_order, 0);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.style_rules[0].source_order, 1);
    }

    #[test]
    fn opaque_at_rule_preserves_statement_prelude_and_source_text() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@future /* keep */ feature;", Origin::User);
        let record = &tree.opaque_at_rules()[0];
        assert_eq!(record.name, "future");
        assert_eq!(record.prelude, " /* keep */ feature");
        assert_eq!(record.body, AtRuleBody::Statement);
        assert_eq!(record.origin, Origin::User);
        assert_eq!(record.to_css(), "@future /* keep */ feature;");
    }

    #[test]
    fn malformed_opaque_at_rule_is_not_retained() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@future { x: url(\"bad\n) } p { color: blue }",
            Origin::Author,
        );
        assert!(tree.opaque_at_rules().is_empty());
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn unterminated_rule_blocks_are_not_retained() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@future { p { color: red }", Origin::Author);
        assert!(tree.opaque_at_rules().is_empty());
        assert!(tree.rules().is_empty());

        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@page { color: red", Origin::Author);
        assert!(tree.page_rules.is_empty());
        assert!(tree.rules().is_empty());

        let mut tree = RuleTree::empty();
        tree.add_stylesheet("p { color: red", Origin::Author);
        assert!(tree.style_rules.is_empty());
        assert!(tree.rules().is_empty());
    }

    #[test]
    fn opaque_at_rule_accepts_braces_in_unquoted_url() {
        for prelude in ["url(foo{bar)", r"u\72l(foo{bar)"] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet(
                &format!("@future {prelude}; p {{ color: blue }}"),
                Origin::Author,
            );
            assert_eq!(tree.opaque_at_rules().len(), 1, "prelude: {prelude:?}");
            assert_eq!(tree.style_rules.len(), 1);
        }
    }

    #[test]
    fn url_like_text_in_identifiers_does_not_hide_delimiters() {
        for prelude in [
            "@url(foo{bar)",
            "#url(foo{bar)",
            "éurl(foo{bar)",
            r"\.\url(foo{bar)",
            r"\2e \url(foo{bar)",
        ] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet(
                &format!("@future {prelude}; p {{ color: blue }}"),
                Origin::Author,
            );
            assert!(
                tree.opaque_at_rules().is_empty(),
                "malformed delimiter sequence was retained for prelude {prelude:?}"
            );
        }
    }

    #[test]
    fn tokenizer_valid_eof_strings_and_comments_are_retained() {
        for source in [
            "@future \"unterminated",
            "@future /* unterminated",
            "@future url(foo\\",
        ] {
            let mut tree = RuleTree::empty();
            tree.add_stylesheet(source, Origin::Author);
            assert_eq!(tree.opaque_at_rules().len(), 1, "source: {source:?}");
        }
    }

    #[test]
    fn nested_opaque_at_rule_bodies_respect_cumulative_byte_budget() {
        let mut css = "payload".repeat(32);
        for _ in 0..16 {
            css = format!("@future {{{css}}}");
        }

        let budget = css.len() * 2;
        let mut remaining = budget;
        let nodes = parse_nested_rule_nodes(&css, &mut remaining);

        assert!(!nodes.is_empty());
        assert!(remaining < budget);
        fn retained_body_bytes(nodes: &[RuleNode]) -> usize {
            nodes
                .iter()
                .map(|node| match node {
                    RuleNode::AtRule(record) => {
                        record.body.as_block().map_or(0, str::len)
                            + retained_body_bytes(record.children())
                    }
                    RuleNode::Qualified(record) => record.body.len(),
                })
                .sum()
        }
        assert_eq!(retained_body_bytes(&nodes), budget - remaining);

        let mut depth = 0;
        let mut node = nodes.first();
        while let Some(RuleNode::AtRule(record)) = node {
            depth += 1;
            node = record.children().first();
        }
        assert!(depth < 16, "the byte budget must truncate the owned chain");
    }

    #[test]
    fn rule_tree_caps_nested_opaque_body_retention() {
        let mut css = "x".repeat(512 * 1024);
        for _ in 0..20 {
            css = format!("@future {{{css}}}");
        }

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::Author);
        let record = &tree.opaque_at_rules()[0];

        fn retained_nested_body_bytes(nodes: &[RuleNode]) -> usize {
            nodes
                .iter()
                .map(|node| match node {
                    RuleNode::AtRule(record) => {
                        record.body.as_block().map_or(0, str::len)
                            + retained_nested_body_bytes(record.children())
                    }
                    RuleNode::Qualified(record) => record.body.len(),
                })
                .sum()
        }

        let retained = retained_nested_body_bytes(record.children());
        assert!(retained > 0);
        assert!(retained <= MAX_CUMULATIVE_NESTED_OPAQUE_BODY_BYTES);
        assert_eq!(record.children().len(), 1);
    }

    #[test]
    fn deeply_nested_opaque_at_rules_are_bounded_but_retained() {
        let mut css = String::new();
        for _ in 0..256 {
            css.push_str("@future {");
        }
        css.push_str("p { color: red }");
        for _ in 0..256 {
            css.push('}');
        }

        let mut tree = RuleTree::empty();
        tree.add_stylesheet(&css, Origin::Author);
        let record = &tree.opaque_at_rules()[0];
        assert_eq!(record.name, "future");
        assert!(record.body.as_block().is_some());

        let mut depth = 1;
        let mut node = record;
        while let Some(RuleNode::AtRule(nested)) = node.children().first() {
            depth += 1;
            node = nested;
        }
        // The top-level record is followed by raw-parser levels 0 through
        // `MAX_OPAQUE_RULE_NESTING_DEPTH`, so the retained chain has two
        // records beyond the configured recursion count.
        assert_eq!(depth, MAX_OPAQUE_RULE_NESTING_DEPTH + 2);
    }

    #[test]
    fn valid_unknown_top_level_margin_box_at_rule_is_retained() {
        // A top-level margin-box at-rule has no semantics in this parser, but
        // it is still valid component-value syntax and must remain inspectable.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            "@top-left { content: 'x' } p { color: red }",
            Origin::Author,
        );
        assert_eq!(tree.page_rules.len(), 0);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.opaque_at_rules().len(), 1);
        assert_eq!(tree.opaque_at_rules()[0].name, "top-left");
    }

    // ── Comment / whitespace transparency within compound ──
    //
    // CSS Syntax L3 §4.3.2 Consume comments specifies that a /*…*/
    // sequence is consumed and the algorithm returns nothing — no token
    // is emitted into the token stream (see
    // <https://www.w3.org/TR/css-syntax-3/#consume-comment>). Within a
    // `<page-selector>` compound, therefore, a comment between two
    // components MUST behave as if absent —
    // `:first/*x*/:left` == `:first:left`. Whitespace, by contrast, IS
    // emitted as a <whitespace-token> and still breaks the compound per
    // L3 §4.3.

    #[test]
    fn page_comment_between_pseudos_is_transparent() {
        // `@page :first/*sep*/:left` — comment only, no whitespace within
        // compound. Per CSS Syntax L3 tokenization, comments vanish, so this
        // is equivalent to `@page :first:left` → 1 rule with two pseudos.
        let rules = page_rules("@page :first/*sep*/:left { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(
            rules[0].selector,
            ps_single(None, vec![PagePseudo::First, PagePseudo::Left])
        );
    }

    #[test]
    fn page_comment_between_colon_and_pseudo_is_transparent() {
        // `@page :/*x*/first` — comment between `:` and pseudo ident.
        // Comment vanishes at tokenization, so this equals `@page :first` →
        // 1 rule. Contrast with `@page : first` (whitespace-separated),
        // which is dropped by `page_whitespace_between_colon_and_pseudo_is_dropped`.
        let rules = page_rules("@page :/*x*/first { color: red }");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].selector, ps_single(None, vec![PagePseudo::First]));
    }

    #[test]
    fn page_whitespace_plus_comment_between_pseudos_is_dropped() {
        // `@page :first /*sep*/:left` — comment is transparent, but the
        // leading whitespace is a real compound-boundary token per L3 §4.3.
        // Whole rule dropped (matches `page_multi_pseudo_with_whitespace_between_is_dropped`).
        let rules = page_rules("@page :first /*sep*/:left { color: red }");
        assert!(rules.is_empty());
    }

    // ── <custom-ident> case-sensitivity for named-page ident ──
    //
    // The named-page ident in a `<page-selector>` derives from the `page`
    // property (CSS Paged Media L3 §8.1 `#using-named-pages`), which types
    // its value as `<custom-ident>`. Per CSS Values L4 §4.2
    // `#custom-idents`: "Such identifiers are fully case-sensitive
    // (meaning they're compared using the 'identical to' operation), even
    // in the ASCII range (e.g. example and EXAMPLE are two different,
    // unrelated user-defined identifiers)." So `Cover` and `cover` MUST be
    // preserved verbatim and treated as distinct named-pages.

    #[test]
    fn page_named_ident_is_case_sensitive() {
        let rules = page_rules("@page Cover { color: red } @page cover { color: blue }");
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].selector,
            ps_single(Some(Atom::from("Cover")), vec![])
        );
        assert_eq!(
            rules[1].selector,
            ps_single(Some(Atom::from("cover")), vec![])
        );
    }

    // ── <page-selector># list-boundary invariants ──
    //
    // `<page-selector-list> = <page-selector>#` per CSS Paged Media L3 §4.3
    // (anchor `#syntax-page-selector`). The `#` multiplier is "one or more,
    // comma-separated" per CSS Values L4 `#component-multipliers`, so each
    // list entry must be a *non-empty* `<page-selector>`. Trailing,
    // leading, and empty-middle commas violate this and drop the whole
    // `@page` rule. These 3 tests close 3 of 6 malformed-prelude cases;
    // the remaining 3 (trailing colon on named page, adjacent idents, etc.)
    // are covered by the tests below.

    #[test]
    fn page_trailing_comma_is_dropped() {
        let rules = page_rules("@page :first, { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_leading_comma_is_dropped() {
        let rules = page_rules("@page ,:first { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_empty_middle_entry_is_dropped() {
        let rules = page_rules("@page :first, , :left { color: red }");
        assert!(rules.is_empty());
    }

    // ── Malformed prelude — trailing/isolated colon and adjacent idents ──
    //
    // Companion to the list-boundary banner above, which pinned the 3
    // list-boundary cases (trailing / leading / empty-middle commas) of
    // `<page-selector>#`; the tests below check the 3 compound-internal cases
    // against the CSS Paged Media L3 §4.3 (anchor `#syntax-page-selector`)
    // compound grammar `<page-selector> = [ <ident-token>? <pseudo-page>* ]!`
    // with `<pseudo-page> = ':' [ left | right | first | blank ]`. Together
    // the two banners close a 6-case malformed-prelude set:
    //   - `named:`      — trailing colon, missing required left|right|first|blank keyword
    //   - `:`           — bare colon, same
    //   - `named other` — two adjacent idents, compound allows only one
    // Each case drops the whole `@page` rule (declarations not captured).

    #[test]
    fn page_named_trailing_colon_is_dropped() {
        // `@page named:` — named-page ident に `:` が付いて次に来るべき
        // `<pseudo-page>` の keyword (left/right/first/blank) が来ずに block
        // が始まる shape。compound grammar `[ <ident-token>? <pseudo-page>* ]!`
        // の `<pseudo-page> = ':' [ left | right | first | blank ]` が
        // required keyword を欠くため drop。
        let rules = page_rules("@page named: { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_bare_colon_is_dropped() {
        // `@page :` — colon 単独。`<pseudo-page> = ':' [ left | right | first |
        // blank ]` が required keyword を欠くため drop。
        let rules = page_rules("@page : { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_two_adjacent_idents_are_dropped() {
        // `@page named other` — compound 内で `<ident-token>` は先頭 1 個のみ。
        // 2 個目の ident は compound 継続でも次 entry (comma がない) でもない
        // ため、`parse_comma_separated` の entry-parses-entirely 検査が leftover
        // を見て Err → whole rule drop。
        let rules = page_rules("@page named other { color: red }");
        assert!(rules.is_empty());
    }

    #[test]
    fn page_rules_captured_via_build_rule_tree_from_dom() {
        // build_rule_tree (DOM 経由) でも page_rules が populate される。
        let doc = dom_with_style("@page :first { color: red } p { color: blue }");
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.page_rules.len(), 1);
        assert_eq!(
            tree.page_rules[0].selector,
            ps_single(None, vec![PagePseudo::First])
        );
        assert_eq!(tree.style_rules.len(), 1);
    }

    // ── counter_styles integration (origin-aware) ──

    #[test]
    fn empty_rule_tree_has_empty_counter_styles() {
        assert!(RuleTree::empty().counter_styles().is_empty());
    }

    #[test]
    fn add_stylesheet_populates_counter_styles() {
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 1);
    }

    #[test]
    fn add_stylesheet_invalid_counter_style_rule_is_dropped() {
        // `system: cyclic` に symbols 0 個 — is_valid() が false になり drop
        // される (counter_style.rs の同型 test と同じ shape)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet("@counter-style foo { system: cyclic; }", Origin::Author);
        assert!(tree.counter_styles().is_empty());
    }

    #[test]
    fn add_stylesheet_counter_style_alongside_style_and_page_rules() {
        // 1 回の add_stylesheet 呼び出しで 3 種の rule が同じ source から
        // それぞれ populate されることを確認 — 独立 2nd pass であって
        // style_rules/page_rules の収集を妨げない。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"
            @counter-style thumbs { system: cyclic; symbols: "*"; }
            @page :first { color: red }
            p { color: blue }
            "#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        assert_eq!(tree.page_rules.len(), 1);
        assert_eq!(tree.style_rules.len(), 1);
        assert_eq!(tree.opaque_at_rules().len(), 1);
        assert_eq!(tree.opaque_at_rules()[0].name, "counter-style");
        assert_eq!(
            tree.rules()
                .iter()
                .map(|rule| rule.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                CssRuleKind::AtRule { index: 0 },
                CssRuleKind::Page { index: 0 },
                CssRuleKind::Style { index: 0 },
            ]
        );
    }

    #[test]
    fn counter_styles_same_name_later_author_call_replaces_earlier_entirely() {
        // CounterStyleRegistry 型 doc が明記する「同名は atomically 後勝ち」を、
        // 複数 add_stylesheet(Origin::Author) 呼び出しをまたいで確認する
        // (page_source_order_monotonic_across_add_stylesheet_calls の
        // counter-style 版)。両方 Author origin — 同一 origin 内での source
        // order tie-break が CSS Counter Styles L3 §3 の "standard cascade
        // rules" と一致する場合。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::Author,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_useragent_origin_alone_populates_counter_styles() {
        // CSS Counter Styles L3 §3: defining an @counter-style makes it
        // available unconditionally — the "only one wins, according to
        // standard cascade rules" sentence only applies when there IS a
        // same-name conflict. A standalone Origin::UserAgent rule with no
        // competing Origin::Author rule has no conflict, so it must be
        // available. An Author-only gate in add_stylesheet would drop this
        // unconditionally regardless of conflict — this test pins that it
        // doesn't (previously named *_does_not_populate_counter_styles and
        // asserted the opposite). style_rules 側が origin を問わず populate
        // される ことは既存の add_stylesheet_ua_and_author_populate_rule_tree
        // が別途 check 済み。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 1);
    }

    #[test]
    fn add_stylesheet_useragent_after_author_does_not_override_counter_styles() {
        // CSS Counter Styles L3 §3: "only one wins, according to standard
        // cascade rules" — origin が第一基準で UA は常に Author に負ける。
        // Author を先に定義し、同名 @counter-style を UA 側で後から
        // add_stylesheet しても、Author の定義が生き残ることを確認する。
        // この保証は add_stylesheet 側の Origin::Author ゲート (UA を無条件
        // drop) ではなく、CounterStyleRegistry::insert_with_origin が同名
        // entry の origin を個別に追跡して行う origin-precedence 解決 (型
        // doc の解決表) が担う — 「flat call-order last-wins だと UA が後から
        // Author を上書きし得る」spec 違反の regression check は変わらず有効。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::Author,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::UserAgent,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        // UA 側の 2-symbol 定義ではなく、Author 側の 1-symbol 定義のまま。
        assert_eq!(rule.symbols.len(), 1);
    }

    #[test]
    fn add_stylesheet_author_after_useragent_overrides_counter_styles() {
        // 上のテストの call-order を反転させた版 — UA を先に定義し、同名
        // @counter-style を Author 側で後から add_stylesheet する。origin
        // 優先順位 (Author > UserAgent) は call order 非依存であるべきなので、
        // こちらも Author が勝つ (CSS Counter Styles L3 §3, "standard cascade
        // rules" は origin が第一基準)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        // Author 側の 2-symbol 定義が勝つ。
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_author_after_user_overrides_counter_styles() {
        // add_stylesheet_author_after_useragent_overrides_counter_styles の
        // Origin::User 版。consumer 提供の `extra_stylesheets` が実際に
        // Origin::User へ route されるため (raikiri-html の retag + umbrella
        // の stylesheet_kind_to_origin 拡張)、この pair (User → Author call
        // order) は production からも到達しうる genuine な組み合わせである
        // — User を先に定義し、同名 @counter-style を Author 側で後から
        // add_stylesheet する。origin 優先順位 (Author normal rank 3 > User
        // normal rank 1、cascade::cascade_rank) は call order 非依存で
        // あるべきなので、こちらも Author が勝つ (CSS Counter Styles L3 §3,
        // "standard cascade rules" は origin が第一基準)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::User,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        // Author 側の 2-symbol 定義が勝つ。
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_useragent_same_name_later_call_replaces_earlier() {
        // counter_styles_same_name_later_author_call_replaces_earlier_entirely
        // の Origin::UserAgent 版 — 同一 origin (UserAgent) 内での
        // source-order tie-break (「後勝ち」) が Author 側と対称に効くことを
        // 確認する (CounterStyleRegistry::insert_with_origin の型 doc 解決表)。
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        tree.add_stylesheet(
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::UserAgent,
        );
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 2);
    }

    #[test]
    fn add_stylesheet_useragent_and_author_different_names_both_populate_counter_styles() {
        // The headline spec claim this whole set of tests is about: CSS
        // Counter Styles L3 §3 makes defining an @counter-style
        // available unconditionally — availability, not just same-name
        // conflict resolution. Every other origin-mixing test above reuses
        // the same rule name ("thumbs") specifically to exercise conflict
        // resolution; this one pins the non-conflicting case those can't:
        // two differently-named rules from different origins must both
        // survive together in the registry.
        let mut tree = RuleTree::empty();
        tree.add_stylesheet(
            r#"@counter-style ua-thumbs { system: cyclic; symbols: "*"; }"#,
            Origin::UserAgent,
        );
        tree.add_stylesheet(
            r#"@counter-style author-thumbs { system: cyclic; symbols: "+" "-"; }"#,
            Origin::Author,
        );
        assert_eq!(tree.counter_styles().len(), 2);
        assert!(tree.counter_styles().get("ua-thumbs").is_some());
        assert!(tree.counter_styles().get("author-thumbs").is_some());
    }

    #[test]
    fn build_rule_tree_populates_counter_styles_from_dom_style_element() {
        let doc = dom_with_style(
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; } p { color: red }"#,
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.counter_styles().len(), 1);
        assert!(tree.counter_styles().get("thumbs").is_some());
        // 同 source の style rule も引き続き populate される (2nd pass が既存
        // walk を妨げないことの確認)。
        assert_eq!(tree.style_rules.len(), 1);
    }

    #[test]
    fn build_rule_tree_counter_styles_across_multiple_style_elements_last_wins() {
        // 兄弟 <style> 2 個、同名 @counter-style — walk_and_collect の
        // document order 呼び出しに従い、後の <style> の定義が勝つ。
        let mut doc = TestDoc::new();
        let s1 = doc.push_element(0, "style", None);
        doc.push_text(
            s1,
            r#"@counter-style thumbs { system: cyclic; symbols: "*"; }"#,
        );
        let s2 = doc.push_element(0, "style", None);
        doc.push_text(
            s2,
            r#"@counter-style thumbs { system: cyclic; symbols: "+" "-"; }"#,
        );
        let tree = build_rule_tree(&doc);
        assert_eq!(tree.counter_styles().len(), 1);
        let rule = tree
            .counter_styles()
            .get("thumbs")
            .expect("thumbs registered");
        assert_eq!(rule.symbols.len(), 2);
    }
}
