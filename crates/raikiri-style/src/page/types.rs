use crate::Atom;
use crate::property::Length;
use crate::rule::Declaration;
use crate::ruletree::Origin;

// Cross-references to `parse_page_size_value`, `PageDeclParser`, and
// `cascade_page` in the doc comments below resolve to their new homes in
// sibling modules via this glob (they are all `pub`/`pub(crate)` in
// `page::parse` / `page::cascade`, re-exported at `page::*`).
#[allow(unused_imports)]
use super::*;

/// Parsed `@page` selector list — a comma-separated list of compound
/// `<page-selector>` productions from CSS Paged Media L3 §4.3 (anchor
/// [`#syntax-page-selector`](https://www.w3.org/TR/css-page-3/#syntax-page-selector)).
///
/// An empty prelude (`@page { … }`) is represented as a single empty
/// [`PageSelectorEntry`] so the cascade code can uniformly iterate `entries`
/// without a special "default" enum arm.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageSelector {
    /// The list of compound page-selectors (comma-separated in source
    /// order). Always non-empty for a successfully-parsed `@page` rule.
    pub entries: Vec<PageSelectorEntry>,
}

/// A single compound `<page-selector>` = `[ <ident-token>? <pseudo-page>* ]!`.
///
/// The `!` marker in the L3 grammar means at least one component (ident or
/// pseudo) must be present, EXCEPT for the special case where the entire
/// `@page` prelude is empty — that maps to `PageSelector { entries: vec![
/// PageSelectorEntry::default() ] }`. Anything appearing between compound
/// components (whitespace, extra tokens) is a compound-rule violation and
/// causes the whole `@page` rule to be dropped.
///
/// See [`PageSelector`] for the surrounding list shape.
#[non_exhaustive]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageSelectorEntry {
    /// Optional named-page ident, e.g. `named` in `@page named { … }`.
    ///
    /// The ident originates from the `page` property (CSS Paged Media L3
    /// §8.1, anchor
    /// [`#using-named-pages`](https://www.w3.org/TR/css-page-3/#using-named-pages))
    /// and is typed as `<custom-ident>`; per CSS Values L4 §4.2
    /// [`#custom-idents`](https://www.w3.org/TR/css-values-4/#custom-idents)
    /// `<custom-ident>` is "fully case-sensitive … even in the ASCII range".
    /// The stored [`Atom`] therefore preserves the source casing verbatim,
    /// and rules like `@page Cover { … }` vs `@page cover { … }` are two
    /// distinct named-pages (pinned by
    /// `page_named_ident_is_case_sensitive`).
    pub ident: Option<Atom>,
    /// Zero-or-more pseudo-pages, e.g. `[First, Left]` for `@page :first:left`.
    pub pseudos: Vec<PagePseudo>,
}

/// The four `<pseudo-page>` idents defined in CSS Paged Media L3 §4.3
/// (production dfn anchor
/// [`#typedef-pseudo-page`](https://www.w3.org/TR/css-page-3/#typedef-pseudo-page)).
///
/// Case is not preserved — parsing is ASCII case-insensitive per CSS Syntax
/// keyword rules (`match_ignore_ascii_case!`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PagePseudo {
    /// `:first` — the first page of the document.
    First,
    /// `:left` — left (verso) pages.
    Left,
    /// `:right` — right (recto) pages.
    Right,
    /// `:blank` — blank pages inserted by forced page breaks.
    Blank,
}

/// One of the ten named paper sizes CSS Paged Media Level 3 §7.1 "Page size:
/// the size property" enumerates as the `<page-size>` production
/// (<https://www.w3.org/TR/css-page-3/#page-size-prop>):
/// `A5 | A4 | A3 | B5 | B4 | JIS-B5 | JIS-B4 | letter | legal | ledger`.
///
/// This only stores *which* keyword was authored. The spec gives concrete
/// dimensions in prose next to the grammar (e.g. "A4: 210 mm wide and 297 mm
/// high") rather than in the grammar itself, so resolving a keyword to a
/// physical width/height is a used-value computation, not a parse-time one —
/// out of scope here (see [`PageRule::size_declarations`]).
///
/// ASCII case-insensitive on parse, matching every other keyword production
/// in this module (`match_ignore_ascii_case!`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageSizeKeyword {
    /// `A5` — "148mm wide and 210 mm high".
    A5,
    /// `A4` — "210 mm wide and 297 mm high".
    A4,
    /// `A3` — "297mm wide and 420mm high".
    A3,
    /// `B5` — "176mm wide by 250mm high".
    B5,
    /// `B4` — "250mm wide by 353mm high".
    B4,
    /// `JIS-B5` — "182mm wide by 257mm high".
    JisB5,
    /// `JIS-B4` — "257mm wide by 364mm high".
    JisB4,
    /// `letter` — "8.5 inches wide and 11 inches high".
    Letter,
    /// `legal` — "8.5 inches wide by 14 inches high".
    Legal,
    /// `ledger` — "11 inches wide by 17 inches high".
    Ledger,
}

/// The `portrait | landscape` orientation keyword half of the `size`
/// descriptor's third grammar alternative (CSS Paged Media Level 3 §7.1
/// "Page size: the size property",
/// <https://www.w3.org/TR/css-page-3/#page-size-prop>).
///
/// ASCII case-insensitive on parse (`match_ignore_ascii_case!`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageOrientation {
    /// `portrait` — height ≥ width.
    Portrait,
    /// `landscape` — width ≥ height.
    Landscape,
}

/// Parsed value of the `size` descriptor (CSS Paged Media Level 3 §7.1 "Page
/// size: the size property", <https://www.w3.org/TR/css-page-3/#page-size-prop>).
///
/// Grammar (spec verbatim): `<length>{1,2} | auto | [ <page-size> || [
/// portrait | landscape ] ]`. Each of the three top-level alternatives is a
/// variant below; [`PageSize::Named`] additionally covers the third
/// alternative's `||` combinator (at least one of the two sub-components
/// present, either source order — see that variant's doc for the invariant
/// this implies).
///
/// Resolving this into an actual page-box width/height (absolutizing
/// `<length>`s, translating a [`PageSizeKeyword`] to millimetres, combining
/// with orientation, applying the UA default when [`PageSize::Auto`]) is
/// layout/used-value work that consumes this type; it is not performed here
/// — see [`PageRule::size_declarations`] for the parse/cascade scope
/// boundary this type sits on.
///
/// # Example
///
/// ```
/// use raikiri_style::{Origin, PageOrientation, PageSize, PageSizeKeyword, RuleTree};
///
/// let mut tree = RuleTree::empty();
/// tree.add_stylesheet("@page { size: A4 landscape }", Origin::Author);
///
/// let decl = &tree.page_rules[0].size_declarations[0];
/// assert_eq!(
///     decl.value(),
///     PageSize::Named {
///         keyword: Some(PageSizeKeyword::A4),
///         orientation: Some(PageOrientation::Landscape),
///     }
/// );
/// assert!(!decl.important);
/// ```
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageSize {
    /// `auto` — the descriptor's initial value (`Initial: auto` in the
    /// descriptor definition table). Resolving it to a concrete page-box
    /// size is UA-defined used-value behavior, not specified by this
    /// parser.
    Auto,
    /// `<length>{1,2}` — one or two `<length>` values (no `<percentage>`;
    /// the grammar's first alternative is `<length>`, not
    /// `<length-percentage>`). Spec: "If only one length value is
    /// specified, it sets both the width and height of the page box (i.e.,
    /// the box is a square). If two length values are specified, the first
    /// establishes the page box width, and the second the page box
    /// height." — a lone authored length is therefore canonicalized here
    /// into equal `width`/`height`, so a consumer never has to special-case
    /// the 1-vs-2-length authored forms.
    Lengths {
        /// Page box width (authored first, or the sole authored length).
        width: Length,
        /// Page box height (authored second, or the sole authored length —
        /// equal to `width` in that case).
        height: Length,
    },
    /// The grammar's third alternative, `[ <page-size> || [ portrait |
    /// landscape ] ]` — a [`PageSizeKeyword`] and/or a [`PageOrientation`],
    /// at least one present (the `||` combinator), in either source order
    /// (spec examples this alternative with `A4 landscape`; `landscape A4`
    /// and either component alone are equally spec-legal under `||`).
    ///
    /// # Invariant: never both `None`
    ///
    /// [`parse_page_size_value`] never constructs this variant with both
    /// fields `None` — that state is not reachable through this module's
    /// parser (an empty match of neither sub-component is a parse error,
    /// not a value). `#[non_exhaustive]` does not, and cannot, enforce this
    /// invariant against a hand-built value from outside the crate — a
    /// consumer building `PageSize::Named { keyword: None, orientation:
    /// None }` directly is a caller bug, not a state this crate's parser
    /// ever emits. That stays true regardless of `#[non_exhaustive]`: the
    /// attribute is on the enum, not this variant, and `Named`'s two
    /// fields are both `pub` — enum-level `#[non_exhaustive]` blocks
    /// exhaustive external matching, not external struct-literal
    /// construction of an already-public variant.
    ///
    /// [`PageSizeDeclaration::value`]'s doc narrows one specific
    /// reachability path for this state: a hand-built
    /// `PageSize::Named { keyword: None, orientation: None }` can no
    /// longer be attached to a `PageSizeDeclaration`, so it can no longer
    /// reach [`PageRule::size_declarations`] through this crate's public
    /// surface — even though the bare `PageSize` value above remains
    /// constructible on its own.
    Named {
        /// The `<page-size>` keyword, if authored.
        keyword: Option<PageSizeKeyword>,
        /// The `portrait | landscape` keyword, if authored.
        orientation: Option<PageOrientation>,
    },
}

/// One `size:` declaration from an `@page` block, in source order.
///
/// Same shape as [`Declaration`] (value + `!important`) but for the `size`
/// descriptor's own value type — `size` is an `@page` descriptor (CSS Paged
/// Media Level 3 §7.1), not a [`PropertyValue`] variant, so it cannot reuse
/// [`Declaration`] itself (which is defined over [`PropertyValue`]).
///
/// # Why a `Vec`, not a single resolved value
///
/// [`PageRule::declarations`] (the sibling field for ordinary properties)
/// keeps every declaration found in the block, in source order, and defers
/// picking the cascade winner to [`cascade_page`] — within one block, origin
/// and specificity are identical for every declaration, so a later
/// `!important` declaration can still lose to an earlier one only if the
/// earlier one is *also* `!important` (CSS Cascading L4 §6.1's "declarations
/// … sorted by … importance, then … order" tie-break chain
/// (<https://www.w3.org/TR/css-cascade-4/#cascade-sort>) applies
/// importance *before* source order). [`cascade_page`] performs that
/// winner-selection logic after parsing, so this field mirrors
/// [`PageRule::declarations`]'s shape instead of pre-empting the cascade.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSizeDeclaration {
    /// The parsed `size` value.
    ///
    /// `pub(crate)`, with the read-only accessor [`Self::value`] the only
    /// path to it from outside the crate — the same narrowing
    /// [`crate::rule::Declaration::value`] uses. [`parse_page_size_value`]
    /// is this field's only producer.
    ///
    /// This does **not** make `PageSize::Named { keyword: None,
    /// orientation: None }` (the state [`PageSize::Named`]'s own doc
    /// documents as parser-unreachable) unconstructible outside the
    /// crate — `Named`'s own fields stay `pub`, and that state remains a
    /// buildable, standalone [`PageSize`] value (see that variant's doc).
    /// What narrows here is one specific reachability path: such a value
    /// can no longer be *attached* to a `PageSizeDeclaration` — a
    /// consumer cannot assign it into this field — so it can no longer
    /// reach [`PageRule::size_declarations`] through this type.
    pub(crate) value: PageSize,
    /// `!important` flag. CSS Paged Media Level 3's "Cascading in the page
    /// context" states declarations in page and margin contexts "cascade
    /// just like declarations in style rule for elements" (cited in full on
    /// [`PageRule::origin`]), so `!important` is spec-meaningful for `size`
    /// too — accepted and retained here even though nothing consumes it yet
    /// (no `size` cascade integration exists — see [`PageRule::size_declarations`]).
    pub important: bool,
}

impl PageSizeDeclaration {
    /// The parsed `size` value — read-only accessor.
    ///
    /// Returns by value: [`PageSize`] is `Copy`, so this follows
    /// [`crate::resolve::ComputedBorder::width`] /
    /// [`crate::resolve::ComputedBorder::style`]'s pattern rather than
    /// [`crate::rule::Declaration::value`]'s (which returns
    /// `&`[`crate::property::PropertyValue`], since that type is not
    /// `Copy`).
    ///
    /// # write 経路が無いことの compile-fail check
    ///
    /// `value` は `pub(crate)` に絞ってある (field doc参照)。
    /// `PageSizeDeclaration` には既に `#[non_exhaustive]` が付いているため、
    /// struct literal / functional-update 経由の fence は `value` 単独の
    /// visibility を discriminate できない — 発生するエラーは常に
    /// non_exhaustive 由来の `E0639` であり、`value` が将来 `pub` に戻っても
    /// compile-fail し続けてしまう ([`crate::resolve::ComputedBorder`] の doc
    /// が指摘する同種の vacuous check と同じ構造)。そのため struct literal
    /// fence は作らず、`PageSizeDeclaration` が `Copy` であることを使う —
    /// `Vec` indexing で取り出した値は borrow を経由しない owned なコピーに
    /// なるので、[`crate::rule::Declaration`] の doc が踏んだ `.clone()`
    /// confound ([`crate::resolve::ComputedBorder`] の doc 参照) はここでも
    /// 発生しない:
    ///
    /// ```compile_fail
    /// use raikiri_style::{Origin, PageSize, RuleTree};
    ///
    /// let mut tree = RuleTree::empty();
    /// tree.add_stylesheet("@page { size: A4 }", Origin::Author);
    /// let mut decl = tree.page_rules[0].size_declarations[0];
    /// decl.value = PageSize::Auto;
    /// ```
    ///
    /// Non-vacuous control (successful compile via the accessor, plus a
    /// direct `important` field read — still `pub`, so a future narrowing
    /// of `important` shows up here first, same discipline as
    /// [`crate::rule::Declaration`]'s doc): [`PageSize`]'s own doc example
    /// exercises both, via `decl.value()` and `decl.important`.
    pub fn value(&self) -> PageSize {
        self.value
    }
}

/// Parsed value of the `marks` descriptor (CSS Paged Media Level 3 §7.2
/// "Crop and Registration Marks: the marks property",
/// <https://www.w3.org/TR/css-page-3/#marks>).
///
/// Grammar (spec verbatim): `none | [ crop || cross ]`. `none` (the
/// descriptor's initial value) is its own top-level alternative, not merely
/// the empty case of the `||` combinator on the right — see
/// [`parse_page_marks_value`] for how the two alternatives are told apart.
/// The `||` combinator on the second alternative means `crop` and `cross`
/// are each independently optional as long as at least one of the two is
/// present, in either source order (`crop cross` and `cross crop` are
/// equally spec-legal, as is either alone).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMarks {
    /// `none` — draw neither crop nor registration marks (initial value).
    None,
    /// `[ crop || cross ]`, with at least one of the two flags set.
    ///
    /// # Invariant: never both `false`
    ///
    /// [`parse_page_marks_value`] never constructs this variant with both
    /// fields `false` — that state is not reachable through this module's
    /// parser (an empty match of neither sub-component is a parse error, the
    /// same invariant [`PageSize::Named`]'s doc documents for its own `||`
    /// alternative).
    Marks {
        /// `crop` was authored — draws short lines at the page corners
        /// marking where the page box should be trimmed.
        crop: bool,
        /// `cross` was authored — draws registration marks used to align
        /// multiple separations/colors.
        cross: bool,
    },
}

/// One `marks:` declaration from an `@page` block, in source order — same
/// shape and rationale as [`PageSizeDeclaration`] (see that type's doc for
/// why this is a `Vec<PageMarksDeclaration>` rather than a single resolved
/// value on [`PageRule`], and for the `pub(crate) value` / `value()`
/// accessor / `pub important` field split).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageMarksDeclaration {
    /// The parsed `marks` value. `pub(crate)`, with [`Self::value`] the only
    /// path to it from outside the crate — mirrors
    /// [`PageSizeDeclaration::value`].
    pub(crate) value: PageMarks,
    /// `!important` flag — spec-meaningful for `marks` for the same reason
    /// covered on [`PageSizeDeclaration::important`], even though nothing
    /// consumes it yet.
    pub important: bool,
}

impl PageMarksDeclaration {
    /// The parsed `marks` value — read-only accessor. Returns by value:
    /// [`PageMarks`] is `Copy`, same shape as [`PageSizeDeclaration::value`].
    pub fn value(&self) -> PageMarks {
        self.value
    }
}

/// Parsed value of the `bleed` descriptor (CSS Paged Media Level 3 §7.3
/// "Bleed Area: the bleed property", <https://www.w3.org/TR/css-page-3/#bleed>).
///
/// Grammar (spec verbatim): `auto | <length>`.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageBleed {
    /// `auto` — the descriptor's initial value. Spec: "Computes to 6pt if
    /// marks has crop and to zero otherwise" — resolving that depends on the
    /// `marks` descriptor's own cascaded value and is used-value work
    /// performed downstream, not at parse time (same layering
    /// [`PageSize::Auto`]'s doc describes for the `size` descriptor's own
    /// UA-defined resolution).
    Auto,
    /// `<length>` — an explicit bleed distance. Unlike `size`'s `<length>`
    /// alternative ("Negative lengths are illegal"), `bleed`'s grammar
    /// explicitly allows negative values: "Values may be negative, but
    /// there may be implementation-specific limits." — so no non-negative
    /// filter is applied here (see [`parse_page_bleed_value`]).
    Length(Length),
}

/// One `bleed:` declaration from an `@page` block, in source order — same
/// shape and rationale as [`PageSizeDeclaration`] (see that type's doc for
/// why this is a `Vec<PageBleedDeclaration>` rather than a single resolved
/// value on [`PageRule`], and for the `pub(crate) value` / `value()`
/// accessor / `pub important` field split).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageBleedDeclaration {
    /// The parsed `bleed` value. `pub(crate)`, with [`Self::value`] the only
    /// path to it from outside the crate — mirrors
    /// [`PageSizeDeclaration::value`].
    pub(crate) value: PageBleed,
    /// `!important` flag — spec-meaningful for `bleed` for the same reason
    /// covered on [`PageSizeDeclaration::important`], even though nothing
    /// consumes it yet.
    pub important: bool,
}

impl PageBleedDeclaration {
    /// The parsed `bleed` value — read-only accessor. Returns by value:
    /// [`PageBleed`] is `Copy`, same shape as [`PageSizeDeclaration::value`].
    pub fn value(&self) -> PageBleed {
        self.value
    }
}

/// One of the sixteen page-margin boxes CSS Paged Media Level 3 §5.1
/// "At-rules for page-margin boxes" names as a margin at-rule
/// (<https://www.w3.org/TR/css-page-3/#margin-at-rules>): `@top-left-corner`,
/// `@top-left`, `@top-center`, `@top-right`, `@top-right-corner`,
/// `@right-top`, `@right-middle`, `@right-bottom`, `@bottom-right-corner`,
/// `@bottom-right`, `@bottom-center`, `@bottom-left`, `@bottom-left-corner`,
/// `@left-bottom`, `@left-middle`, `@left-top`.
///
/// Variant order matches the sixteen boxes' default painting order — CSS
/// Paged Media Level 3 §3.1 "Page Backgrounds and Painting Order"
/// (<https://www.w3.org/TR/css-page-3/#painting>), which lists this exact
/// sequence as the CSS2.1 "tree order" of page-margin boxes and states
/// "Start with `@top-left-corner`, then go clockwise": top edge
/// left-to-right, right edge top-to-bottom, bottom edge right-to-left, left
/// edge bottom-to-top.
///
/// Only *which* margin box a nested at-rule names is recorded here — the
/// sixteen boxes' actual geometry (size and position within the page
/// margin) is used-value / layout work downstream of this parser, not
/// performed by this crate. See [`PageMarginBoxRule`] for the declaration
/// payload this identifies.
///
/// ASCII case-insensitive on parse (`match_ignore_ascii_case!`), matching
/// every other at-rule/keyword production in this module.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PageMarginBoxSlot {
    /// `@top-left-corner`.
    TopLeftCorner,
    /// `@top-left`.
    TopLeft,
    /// `@top-center`.
    TopCenter,
    /// `@top-right`.
    TopRight,
    /// `@top-right-corner`.
    TopRightCorner,
    /// `@right-top`.
    RightTop,
    /// `@right-middle`.
    RightMiddle,
    /// `@right-bottom`.
    RightBottom,
    /// `@bottom-right-corner`.
    BottomRightCorner,
    /// `@bottom-right`.
    BottomRight,
    /// `@bottom-center`.
    BottomCenter,
    /// `@bottom-left`.
    BottomLeft,
    /// `@bottom-left-corner`.
    BottomLeftCorner,
    /// `@left-bottom`.
    LeftBottom,
    /// `@left-middle`.
    LeftMiddle,
    /// `@left-top`.
    LeftTop,
}

/// One margin-box at-rule from an `@page` block body, in source order — e.g.
/// `@top-left { content: counter(page) }` (CSS Paged Media Level 3 §5
/// "Page-Margin Boxes", <https://www.w3.org/TR/css-page-3/#margin-boxes>).
///
/// # Scope: declaration storage only, no cascade / layout
///
/// This stores the declaration list a margin-box at-rule's body contains —
/// most notably `content:` (spec §5.2 "Populating page-margin boxes",
/// <https://www.w3.org/TR/css-page-3/#populating-margin-boxes>), whose value
/// grammar ([`crate::property::ContentComponent`]) already covers `counter()` /
/// `counters()` / quoted strings / `string()` etc.
///
/// # Not filtered against the applicable-property list
///
/// CSS Paged Media Level 3 §5.1 states "The margin at-rules can only
/// contain page-margin properties" — the spec restricts which properties
/// are meaningful in a margin at-rule's body (enumerated in the spec's
/// Appendix A). This parser does not enforce that restriction: the body is
/// parsed as an ordinary declaration list, via the same
/// [`crate::rule::parse_declaration_block`] qualified style rules use (see
/// the `AtRuleParser` impl for [`PageDeclParser`]), so `declarations` can
/// carry any property [`crate::property::parse_value`] recognizes, whether
/// or not it is spec-applicable to a margin context. This mirrors
/// [`PageRule::declarations`], which likewise applies no `@page`-context
/// applicable-property filter — deciding which declarations are
/// context-applicable is downstream, used-value-adjacent work this crate
/// has not wired for either context yet, not something this parse step
/// does.
///
/// Two things are explicitly *not* done here, and are future scope:
///
/// - **Used-value selection across rules.** Multiple margin-box at-rules
///   naming the same [`PageMarginBoxSlot`] (duplicate `@top-left` blocks within
///   one `@page` rule, or the same slot named by several `@page` rules that
///   match one page) remain separate source-ordered entries. The page cascade
///   attaches origin, specificity, and rule order metadata; declaration
///   winner selection and margin-box inheritance remain downstream work.
/// - **Geometry / layout.** The sixteen margin boxes' actual size and
///   position within the page margin is used-value work that consumes this
///   declaration bag; it is not computed by this type.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct PageMarginBoxRule {
    /// Which of the sixteen margin boxes this at-rule names.
    pub slot: PageMarginBoxSlot,
    /// Declarations from the margin at-rule's block body. `pub`, same
    /// surface and shorthand-expansion guarantee as [`PageRule::declarations`]
    /// (see that field's doc for the full "why this stays `pub`" analysis —
    /// it applies unchanged here since both fields exit parsing through the
    /// same shorthand-expansion boundary).
    pub declarations: Vec<Declaration>,
}

/// Parsed `@page` rule — selector + declaration list + source order + origin.
///
/// `source_order` numbers `@page` rules *independently* of style rules; the
/// two rule kinds cascade in different tuple positions in the CSS spec, so a
/// separate 0-indexed counter keeps their bookkeeping decoupled and leaves
/// `StyleRule::source_order` semantics untouched. Cross-kind ordering can be
/// reconstructed by future cascade code if needed.
///
/// Field order (selector → declarations → size_declarations →
/// marks_declarations → bleed_declarations → margin_box_rules →
/// source_order → origin) mirrors [`crate::StyleRule`]'s (selector →
/// declarations → source_order → origin) as closely as the extra
/// `@page`-only fields allow, so both rule kinds present a similar shape to
/// the cascade code. Further future fields (a cascade-origin cache, etc.)
/// are expected to be added later; `#[non_exhaustive]` means that addition
/// stays non-breaking.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct PageRule {
    /// Which pages this rule applies to.
    pub selector: PageSelector,
    /// Declarations from the block body — parsed with the same
    /// [`crate::property::parse_value`] dispatch qualified rules use (via
    /// [`parse_page_declaration_block`]), so unsupported properties are
    /// silently dropped (matching this crate's general unsupported-property
    /// policy).
    ///
    /// Note: `size` / `marks` / `bleed` are `@page`-only descriptors, not
    /// [`PropertyValue`] the general dispatch recognizes — see
    /// [`PageRule::size_declarations`] / [`PageRule::marks_declarations`] /
    /// [`PageRule::bleed_declarations`] for their own dedicated storage.
    /// The margin-box at-rules (`@top-left` etc. per L3 §5) are not
    /// declarations at all (nested at-rules) and so never reach this field
    /// either — see [`PageRule::margin_box_rules`].
    ///
    /// # Why this field (and `RuleTree::page_rules`) is still `pub`
    ///
    /// Still `pub` deliberately, and outside a prior visibility-tightening
    /// pass's approved scope. **This is the canonical, docs.rs-visible
    /// statement of what that leaves open**; [`crate::ruletree::RuleTree::page_rules`]
    /// points here rather than restating it (same discipline as
    /// [`crate::page::PageCascadeResult::declarations`] — spell
    /// `PageRule::declarations` verbatim if you add another pointer site).
    ///
    /// That earlier pass closed the *element* path — the `style_rules` /
    /// `declarations` / `value` fields are now `pub(crate)`, reachable from
    /// outside the crate only through their fully `pub` read-only accessors
    /// (`RuleTree::style_rules()` / `StyleRule::declarations()` /
    /// `Declaration::value()`) — but left the *`@page`* path alone. The
    /// two are closed by different amounts:
    ///
    /// - **Element path**: closed by visibility alone. A consumer only ever
    ///   reaches a read-only `&[StyleRule]` / `&[Declaration]`; no `&mut`
    ///   path to a `Declaration`'s value is public.
    /// - **`@page` path** (this field, and `RuleTree::page_rules`): a
    ///   consumer can still duplicate / remove / reorder an existing
    ///   `Declaration` in place, flip its `important` flag, or clone an
    ///   existing `PageRule` and push back an edited copy (`PageRule` is
    ///   `#[non_exhaustive]`, so a brand-new one cannot be built from a
    ///   struct literal — a seed rule is required). What stays closed is
    ///   narrower: a consumer cannot manufacture a *new* `Declaration`
    ///   carrying a shorthand `PropertyValue` (`margin` / `padding` /
    ///   `border`). That holds for two reasons: (a) `Declaration::value` is
    ///   private, so no struct-literal / functional-update construction is
    ///   possible, and (b) every `Declaration` a consumer could clone came
    ///   out of `parse_declaration_block`, which never emits a shorthand key
    ///   (pinned by `expand_shorthand_into`'s exhaustive match). If either
    ///   (a) or (b) breaks, the `@page` path
    ///   reopens from outside the crate — re-derive this section before
    ///   adding a public constructor to `Declaration`.
    ///
    /// The full mechanical derivation (all four shorthand-expansion call
    /// sites, and why (a)/(b) above are each load-bearing) lives on
    /// [`crate::rule::expand_shorthand_into`]'s doc. That function is
    /// `pub(crate)`, so its doc does not render on docs.rs — this section is
    /// the summary a docs.rs reader can actually reach.
    pub declarations: Vec<Declaration>,
    /// `size:` declarations from the block body, in source order — see
    /// [`PageSizeDeclaration`] for the value shape and why this is a `Vec`
    /// rather than a single resolved value. Parsed by
    /// [`parse_page_declaration_block`] via a dedicated grammar
    /// ([`parse_page_size_value`]), since `size` is an `@page` descriptor
    /// (CSS Paged Media Level 3 §7.1 "Page size: the size property",
    /// <https://www.w3.org/TR/css-page-3/#page-size-prop>), not a
    /// [`PropertyValue`] the general [`crate::property::parse_value`]
    /// dispatch recognizes. Empty when the block declares no `size`.
    ///
    /// Consumed by [`cascade_page`] / [`PageCascadeResult`] using the same
    /// page-selector cascade tuple as ordinary declarations. The resulting
    /// typed value is still converted into a concrete `PageBox` by the
    /// downstream page-layout consumer.
    ///
    /// # `pub` surface — same shape as `declarations`
    ///
    /// This is the same "still `pub`, outside the prior visibility-tightening
    /// pass" surface [`PageRule::declarations`]'s doc analyzes in full — a
    /// consumer can duplicate / remove / reorder an existing
    /// [`PageSizeDeclaration`], flip `important`, or push one back onto a
    /// cloned `PageRule`. [`PageSizeDeclaration::value`] is `pub(crate)`
    /// (see that field's own doc), matching `Declaration::value`, so a
    /// consumer cannot additionally assign a [`PageSize`] the parser itself
    /// would never construct — most notably `PageSize::Named { keyword:
    /// None, orientation: None }`, the state [`PageSize::Named`]'s own doc
    /// documents as parser-unreachable. Nothing in this crate reads
    /// `size_declarations` yet (see the previous paragraph); whichever
    /// change wires a `size` cascade winner can rely on the invariant
    /// holding for every `PageSizeDeclaration` it encounters.
    pub size_declarations: Vec<PageSizeDeclaration>,
    /// `marks:` declarations from the block body, in source order — see
    /// [`PageMarksDeclaration`] for the value shape. Parsed by
    /// [`parse_page_declaration_block`] via a dedicated grammar
    /// ([`parse_page_marks_value`]), since `marks` is an `@page` descriptor
    /// (CSS Paged Media Level 3 §7.2 "Crop and Registration Marks: the marks
    /// property", <https://www.w3.org/TR/css-page-3/#marks>), not a
    /// [`PropertyValue`] the general [`crate::property::parse_value`]
    /// dispatch recognizes. Empty when the block declares no `marks`.
    ///
    /// Consumed by [`cascade_page`] / [`PageCascadeResult`] with the same
    /// origin, specificity, importance, and source-order ordering as `size`.
    /// Shares that field's `pub` surface analysis
    /// ([`PageRule::declarations`]'s doc) and the same
    /// never-construct-the-parser-unreachable-state guarantee
    /// ([`PageMarksDeclaration::value`] is `pub(crate)`, so a consumer
    /// cannot assign a hand-built `PageMarks::Marks { crop: false, cross:
    /// false }` — the state [`PageMarks::Marks`]'s own doc documents as
    /// parser-unreachable — into this field).
    pub marks_declarations: Vec<PageMarksDeclaration>,
    /// `bleed:` declarations from the block body, in source order — see
    /// [`PageBleedDeclaration`] for the value shape. Parsed by
    /// [`parse_page_declaration_block`] via a dedicated grammar
    /// ([`parse_page_bleed_value`]), since `bleed` is an `@page` descriptor
    /// (CSS Paged Media Level 3 §7.3 "Bleed Area: the bleed property",
    /// <https://www.w3.org/TR/css-page-3/#bleed>), not a [`PropertyValue`]
    /// the general [`crate::property::parse_value`] dispatch recognizes.
    /// Empty when the block declares no `bleed`.
    ///
    /// Consumed by [`cascade_page`] / [`PageCascadeResult`] with the same
    /// descriptor cascade ordering as `size` and `marks`. `auto` remains a
    /// typed used-value decision because it depends on the cascaded `marks`
    /// value (see [`PageBleed::Auto`]'s doc). Shares `size_declarations`'s
    /// `pub` surface analysis
    /// ([`PageRule::declarations`]'s doc).
    pub bleed_declarations: Vec<PageBleedDeclaration>,
    /// Margin-box at-rules (`@top-left { … }` etc.) nested in the block
    /// body, in source order — see [`PageMarginBoxRule`] for the value shape
    /// and its doc's "Scope" section for the geometry work deliberately
    /// deferred downstream. [`cascade_page`] preserves matching entries in
    /// [`PageCascadeResult::margin_boxes`]. Parsed by
    /// [`parse_page_declaration_block`] via the [`PageDeclParser`]
    /// `AtRuleParser` impl, since a margin-box at-rule is a nested at-rule
    /// (CSS Paged Media Level 3 §5.1, <https://www.w3.org/TR/css-page-3/#margin-at-rules>),
    /// not a [`Declaration`]. Empty when the block declares no margin-box
    /// at-rule. Shares `bleed_declarations`'s `pub` surface analysis
    /// ([`PageRule::declarations`]'s doc) — [`PageMarginBoxRule::declarations`]
    /// exits parsing through the same shorthand-expansion boundary
    /// [`PageRule::declarations`] does, so the same "cannot construct a
    /// shorthand-carrying `Declaration`" guarantee holds.
    pub margin_box_rules: Vec<PageMarginBoxRule>,
    /// 0-indexed source order among `@page` rules across all
    /// `RuleTree::add_stylesheet` calls.
    pub source_order: u32,
    /// Normal cascade-layer order. Larger values have higher precedence;
    /// `u32::MAX` represents unlayered rules.
    pub layer_order: u32,
    /// Cascade origin this rule was parsed under. See [`Origin`] for the
    /// current 4-variant set (`UserAgent` / `User` / `AuthorPresentationalHint`
    /// / `Author`) — `@page` rules are only ever
    /// parsed via [`crate::ruletree::RuleTree::add_stylesheet`], the same
    /// entry point style rules use, so any [`Origin`] a caller passes
    /// (including [`Origin::User`], whose only production producer today is
    /// consumer-provided `extra_stylesheets` — see
    /// [`Origin`]'s own doc) flows through here unchanged; this field does no
    /// origin-narrowing of its own.
    ///
    /// Two primary sources back the integration; the fragment anchors are the
    /// stable form of each citation:
    ///
    /// - CSS Paged Media Level 3, "Cascading in the page context" —
    ///   "Declarations in page and margin contexts cascade just like
    ///   declarations in style rule for elements", i.e. `@page`
    ///   *participates* in the cascade:
    ///   <https://www.w3.org/TR/css-page-3/#cascading-and-page-context>
    /// - CSS Cascading Level 4, "Cascade Origins" — the per-origin
    ///   ordering mechanism (`!important` reversal) that `@page` rules
    ///   cascade through, reusing [`crate::cascade::cascade_rank`]'s full
    ///   ordering (see that function's doc for the current rank table):
    ///   <https://www.w3.org/TR/css-cascade-4/#cascade-origin>
    ///
    /// The field is populated at parse time so cascade integration never has to
    /// re-index page rules by origin later. The cascade *ordering* itself
    /// remains future work and is not wired here.
    pub origin: Origin,
}
