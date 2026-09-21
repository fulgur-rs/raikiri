use crate::Direction;
use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId, StyleNodeKind};

/// `PseudoClass::Dir` arm of [`super::selector_match::compound_matches`] — resolves the element's
/// **directionality** per HTML Living Standard §3.2.6.4 "The `dir`
/// attribute" (<https://html.spec.whatwg.org/multipage/dom.html#the-directionality>)
/// simplified as documented in the sections below:
///
/// > The directionality of an element \[...\] is either 'ltr' or 'rtl'. To
/// > compute the directionality given an element element, switch on
/// > element's dir attribute state: LTR — Return 'ltr'. RTL — Return 'rtl'.
/// > \[...\] Auto \[...\] Let result be the auto directionality of element.
/// > If result is null, then return 'ltr'. Return result. \[...\] Undefined
/// > \[...\] Otherwise — Return the parent directionality of element.
/// >
/// > To compute the parent directionality given an element element: Let
/// > parentNode be element's parent node. \[...\] If parentNode is an
/// > element, then return the directionality of parentNode. Return 'ltr'.
///
/// i.e. own `dir="ltr"`/`dir="rtl"` wins; own `dir="auto"` uses
/// [`auto_directionality`]'s text scan (falling back to `'ltr'`, *not* the
/// parent's directionality, when the scan finds nothing); otherwise
/// (missing/invalid `dir`) walk up to the nearest ancestor with an explicit
/// `ltr`/`rtl` `dir`; if none exists anywhere (including at the document
/// root, which has no parent element), the default is `'ltr'` — this
/// function is total (`Direction`, not `Option<Direction>`), matching the
/// spec's own "always ltr or rtl, never undetermined" shape.
///
/// CSS Selectors L4 §7.1 <https://www.w3.org/TR/selectors-4/#the-dir-pseudo>
/// (bikeshed source, same fetch as [`Direction`]'s doc) is what motivates
/// consulting ancestors at all rather than just the own attribute (`[dir=C]`
/// would suffice for that): "the directionality of an element inherits so
/// that a child without a dir attribute will have the same directionality
/// as its closest ancestor with a valid dir attribute."
///
/// # Deliberately out of scope
///
/// The quoted `Auto` arm above elides two branches of the referenced "auto
/// directionality" algorithm that this crate does not implement — see
/// [`auto_directionality`]'s own doc for the precise cut and why each is out
/// of scope. Also elided from the `Undefined` arm: the `bdi` element and
/// `input[type=tel]` special cases (raikiri has no notion of
/// element-specific behavior at this layer — `bdi` without an explicit
/// `dir` attribute still falls through to the ancestor walk below, same as
/// any other element with no `dir`) and the shadow-tree host step of
/// "parent directionality" (raikiri has no shadow DOM).
pub(crate) fn resolve_directionality<D: StyleDom, E: StyleElement>(
    dom: &D,
    elem: &E,
    elem_id: StyleNodeId,
    ancestors: &[StyleNodeId],
) -> Direction {
    match own_dir_attribute_state(elem) {
        DirAttributeState::Ltr => return Direction::Ltr,
        DirAttributeState::Rtl => return Direction::Rtl,
        DirAttributeState::Auto => {
            return auto_directionality(dom, elem_id).unwrap_or(Direction::Ltr);
        }
        DirAttributeState::Undefined => {}
    }
    for &ancestor_id in ancestors.iter().rev() {
        // Same "explicit `if let` nesting instead of `.and_then` chain"
        // reason as `effective_language`'s sibling loop — `own_explicit_direction`
        // borrows from `ancestor_elem`, which itself borrows from a `node`
        // temporary that must stay alive across the call.
        if let Some(node) = dom.node(ancestor_id)
            && let Some(ancestor_elem) = node.as_element()
            && let Some(dir) = own_explicit_direction(&ancestor_elem)
        {
            return dir;
        }
    }
    Direction::Ltr
}

/// The `dir` attribute's full enumerated state — HTML LS §3.2.6.4's LTR /
/// RTL / Auto / Undefined states (same fetch as [`resolve_directionality`]'s
/// doc) — including `Auto`, which [`own_explicit_direction`] (the
/// ancestor-walk helper, which only ever needs an explicit `ltr`/`rtl`
/// winner) folds into `None` alongside `Undefined`.
///
/// Attribute keyword matching is ASCII case-insensitive, per HTML's general
/// treatment of enumerated attribute keywords (`the dir attribute is an
/// enumerated attribute with the following keywords and states`, same
/// fetch) — same posture as this crate's other HTML-enumerated-value
/// comparisons (e.g. `elem.tag_name()`'s `eq_ignore_ascii_case` in
/// [`super::selector_match::compound_matches`]). Missing or invalid values (anything other than
/// `ltr`/`rtl`/`auto`) both resolve to `Undefined` per HTML LS's own
/// "missing value default and invalid value default are both the Undefined
/// state".
///
/// # HTML-namespace-only, unlike [`super::lang::effective_language`]'s `lang` reads
///
/// The same fetch continues, immediately after the quoted algorithm:
///
/// > Since the `dir` attribute is only defined for HTML elements, it cannot
/// > be present on elements from other namespaces. Thus, elements from
/// > other namespaces always end up using the parent directionality.
///
/// so a `dir` attribute on a foreign-namespace element (e.g. inline
/// `<svg dir="rtl">`) must resolve to `Undefined` here regardless of its
/// literal value — `elem.namespace_uri().is_none()` is this crate's
/// established "is an HTML element" proxy — the same one
/// `resolve_case_sensitivity`'s doc documents and
/// `resolve_case_sensitivity_non_html_namespace_element_is_case_sensitive`
/// pins for the analogous case-sensitivity divergence.
///
/// This is a *narrower* allowlist than HTML's `lang` step in the same
/// algorithm ("If the node is an HTML element **or an element in the SVG
/// namespace**, and it has a lang in no namespace attribute set" — quoted in
/// full on [`super::lang::effective_language`]'s doc, gated by that function's own
/// [`super::lang::own_html_or_svg_lang_attribute`] helper): both `dir` (here) and `lang`
/// gate on `elem.namespace_uri()`, but `dir` is HTML-only (1-element
/// allowlist — the "only defined for HTML elements" quote above has no SVG
/// carve-out) while `lang` is HTML-**or**-SVG (2-element allowlist). The
/// difference is allowlist *size*, not gate-vs-no-gate — do not widen this
/// function's allowlist to match [`super::lang::own_html_or_svg_lang_attribute`]'s; `dir`
/// genuinely has no SVG exception in the quoted algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirAttributeState {
    /// `dir="ltr"` — HTML LS "LTR" state.
    Ltr,
    /// `dir="rtl"` — HTML LS "RTL" state.
    Rtl,
    /// `dir="auto"` — HTML LS "Auto" state.
    Auto,
    /// Missing, invalid, or on a non-HTML-namespace element — HTML LS
    /// "Undefined" state.
    Undefined,
}

fn own_dir_attribute_state<E: StyleElement>(elem: &E) -> DirAttributeState {
    if elem.namespace_uri().is_some() {
        return DirAttributeState::Undefined;
    }
    match elem.attr("dir") {
        Some(v) if v.eq_ignore_ascii_case("ltr") => DirAttributeState::Ltr,
        Some(v) if v.eq_ignore_ascii_case("rtl") => DirAttributeState::Rtl,
        Some(v) if v.eq_ignore_ascii_case("auto") => DirAttributeState::Auto,
        _ => DirAttributeState::Undefined,
    }
}

/// HTML's `dir` attribute LTR/RTL states only — `Undefined`/`Auto` (missing,
/// invalid, or `auto`) both collapse to `None` here, since the only caller
/// ([`resolve_directionality`]'s ancestor walk) needs exactly "does this
/// ancestor have an explicit winner to inherit", and per HTML LS's "parent
/// directionality" an ancestor's own `Auto`/`Undefined` state is not such a
/// winner (the walk keeps going past it toward that ancestor's own parent).
/// See [`own_dir_attribute_state`]'s doc for the full 4-state read this
/// delegates to, including the namespace gate and case-insensitivity.
pub(crate) fn own_explicit_direction<E: StyleElement>(elem: &E) -> Option<Direction> {
    match own_dir_attribute_state(elem) {
        DirAttributeState::Ltr => Some(Direction::Ltr),
        DirAttributeState::Rtl => Some(Direction::Rtl),
        DirAttributeState::Auto | DirAttributeState::Undefined => None,
    }
}

/// HTML LS §3.2.6.4's "auto directionality" given an element (same fetch as
/// [`resolve_directionality`]'s doc), simplified to the text-content-scan
/// branch only:
///
/// > To compute the auto directionality given an element element: \[...\]
/// > Return the contained text auto directionality of element with
/// > canExcludeRoot set to false.
///
/// i.e. [`contained_text_auto_directionality`] over `element`'s own
/// descendants, `None` iff no descendant text node contains a strong L/AL/R
/// character anywhere ([`resolve_directionality`]'s caller then applies the
/// spec's own `'ltr'` fallback for that case).
///
/// # Deliberately out of scope
///
/// The full algorithm has two branches elided here, both unreachable from
/// this crate's current DOM model:
///
/// - **Form-associated elements' current value** — the spec's first step
///   scans an `input`/`textarea` element's live *value* (not its descendant
///   text) for the same L/AL/R rule. [`StyleElement`] has no notion of a
///   form control's current value (a run-time, potentially
///   script-or-user-mutated state distinct from the parsed DOM tree this
///   crate's style layer operates over) — out of scope for the same reason
///   [`crate::style_dom`] carries no form-control surface at all. Falling
///   through to the descendant-text scan instead happens to coincide with a
///   `textarea`'s *initial* value (its child text node) but not any
///   subsequently-changed value, and always resolves `input` (a void
///   element with no descendant text) to `None` regardless of what the
///   user has typed into it.
/// - **`slot` elements with shadow-tree assigned nodes** — raikiri has no
///   shadow DOM (established elsewhere, e.g. [`resolve_directionality`]'s
///   own "Deliberately out of scope" note on the shadow-tree host step of
///   "parent directionality"), so this branch's precondition never holds.
fn auto_directionality<D: StyleDom>(dom: &D, elem_id: StyleNodeId) -> Option<Direction> {
    contained_text_auto_directionality(dom, elem_id)
}

/// HTML LS §3.2.6.4's "contained text auto directionality" of an element,
/// with `canExcludeRoot` fixed to `false` (the only value
/// [`auto_directionality`] ever needs — `true` is only used by the
/// shadow-tree `slot` branch this crate does not implement, see that
/// function's doc). Same fetch as [`resolve_directionality`]'s doc:
///
/// > For each node descendant of element's descendants, in tree order: If
/// > any of \[descendant, any ancestor element of descendant that is a
/// > descendant of element\] is one of \[a bdi element, a script element, a
/// > style element, a textarea element, an element whose dir attribute is
/// > not in the Undefined state\], then continue. \[...\] If descendant is
/// > not a Text node, then continue. Let result be the text node
/// > directionality of descendant. If result is not null, then return
/// > result. \[...\] Return null.
///
/// i.e. a depth-first, tree-order walk of `elem_id`'s descendants that skips
/// entire subtrees rooted at a `bdi`/`script`/`style`/`textarea` element or
/// any element with its own non-`Undefined` `dir` (those resolve their own
/// directionality independently and must not contribute text to this
/// element's scan — see [`excluded_from_auto_text_scan`]), scanning every
/// remaining text node in order via [`text_node_first_strong_direction`]
/// until one yields a non-`None` result. `elem_id` itself is never checked
/// against the exclusion set (that is exactly what `canExcludeRoot = false`
/// means) — only its descendants are.
///
/// This crate has no `slot` element / shadow-tree concept, so the spec's
/// "if descendant is a `slot` element whose root is a shadow root" branch
/// (which would otherwise interrupt this walk to consult a shadow host) is
/// omitted; every descendant that isn't excluded is either scanned (if a
/// `Text` node) or recursed into (if an `Element`).
fn contained_text_auto_directionality<D: StyleDom>(
    dom: &D,
    elem_id: StyleNodeId,
) -> Option<Direction> {
    dom.child_ids(elem_id)
        .find_map(|child_id| auto_text_scan_subtree(dom, child_id))
}

/// One node of [`contained_text_auto_directionality`]'s tree-order walk —
/// recurses into element subtrees (unless [`excluded_from_auto_text_scan`]),
/// and applies [`text_node_first_strong_direction`] to text nodes. Comment /
/// processing-instruction / document-fragment / document nodes are none of
/// "text node" or "element node" and so never contribute, matching how
/// [`super::selector_match::matches_empty`] / [`super::html_quirks::is_substantial_node`] treat the same node kinds.
fn auto_text_scan_subtree<D: StyleDom>(dom: &D, node_id: StyleNodeId) -> Option<Direction> {
    let node = dom.node(node_id)?;
    match node.kind() {
        StyleNodeKind::Text => text_node_first_strong_direction(node.text_content().unwrap_or("")),
        StyleNodeKind::Element => {
            // cov:ignore: `kind() == Element` guarantees `as_element()` is
            // `Some` — `StyleNode::as_element`'s own trait doc contract
            // ("Some iff kind() == Element").
            let elem = node.as_element()?;
            if excluded_from_auto_text_scan(&elem) {
                return None;
            }
            dom.child_ids(node_id)
                .find_map(|child_id| auto_text_scan_subtree(dom, child_id))
        }
        StyleNodeKind::Comment
        | StyleNodeKind::ProcessingInstruction
        | StyleNodeKind::DocumentFragment
        | StyleNodeKind::Document => None,
    }
}

/// The exclusion set [`contained_text_auto_directionality`]'s spec quote
/// lists — an element that must not contribute (nor let its own
/// descendants contribute) text to an ancestor's auto-directionality scan,
/// because it resolves its own directionality independently: `bdi`,
/// `script`, `style`, `textarea`, or any element whose own `dir` attribute
/// is not `Undefined` (`ltr`/`rtl`/`auto` all count — an `auto` descendant
/// resolves its *own* text scan rather than leaking its text into the
/// ancestor's).
///
/// The 4 tag names are gated to the HTML namespace
/// (`elem.namespace_uri().is_none()`, this crate's established "is an HTML
/// element" proxy — see [`own_dir_attribute_state`]'s doc) since HTML LS's
/// `bdi`/`script`/`style`/`textarea` dfns denote specifically the HTML
/// elements of those names, not e.g. SVG's own distinct `script`/`style`
/// elements. The `dir`-attribute-state check needs no separate namespace
/// gate — [`own_dir_attribute_state`] already resolves any foreign-namespace
/// element to `Undefined`.
fn excluded_from_auto_text_scan<E: StyleElement>(elem: &E) -> bool {
    if !matches!(own_dir_attribute_state(elem), DirAttributeState::Undefined) {
        return true;
    }
    if elem.namespace_uri().is_some() {
        return false;
    }
    const EXCLUDED_TAGS: &[&str] = &["bdi", "script", "style", "textarea"];
    EXCLUDED_TAGS
        .iter()
        .any(|tag| elem.tag_name().eq_ignore_ascii_case(tag))
}

/// Bidirectional character type, restricted to the 3 *strong* types HTML
/// LS's "text node directionality" algorithm consults — see
/// [`text_node_first_strong_direction`]'s doc for the quoted algorithm and
/// [`strong_bidi_type`]'s doc for how a code point is classified into one
/// of these (or neither).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrongBidiType {
    /// Left-to-Right.
    L,
    /// Arabic Letter.
    Al,
    /// Right-to-Left (non-Arabic).
    R,
}

/// HTML LS §3.2.6.4's "text node directionality" given a `Text` node's data
/// (same fetch as [`resolve_directionality`]'s doc):
///
/// > If text's data does not contain a code point whose bidirectional
/// > character type is L, AL, or R, then return null. Let codePoint be the
/// > first code point in text's data whose bidirectional character type is
/// > L, AL, or R. If codePoint is of bidirectional character type AL or R,
/// > then return 'rtl'. If codePoint is of bidirectional character type L,
/// > then return 'ltr'.
///
/// i.e. scan `data` in order for the first code point [`strong_bidi_type`]
/// classifies as L/AL/R (code points of any other — weak or neutral —
/// bidirectional type, which this crate does not otherwise classify at all,
/// are simply skipped over); `AL`/`R` resolve to `Direction::Rtl`, `L` to
/// `Direction::Ltr`; `None` iff no such code point exists anywhere in
/// `data`.
fn text_node_first_strong_direction(data: &str) -> Option<Direction> {
    data.chars().find_map(|c| match strong_bidi_type(c)? {
        StrongBidiType::L => Some(Direction::Ltr),
        StrongBidiType::Al | StrongBidiType::R => Some(Direction::Rtl),
    })
}

// Transcribed from `DerivedBidiClass.txt`'s "@missing" default-value
// comments for the code point ranges reserved for right-to-left
// scripts (see this function's doc) — each left edge/right edge here
// is one of those file's own `@missing: START..END; Bidi_Class` lines.
pub(crate) const AL_RANGES: &[(u32, u32)] = &[
    (0x0600, 0x07BF),   // Arabic, Syriac, Arabic Supplement, Thaana
    (0x0860, 0x08FF),   // Syriac Supplement, Arabic Extended-B/-A
    (0xFB50, 0xFDCF),   // Arabic Presentation Forms-A (partial)
    (0xFDF0, 0xFDFF),   // Arabic Presentation Forms-A (partial)
    (0xFE70, 0xFEFF),   // Arabic Presentation Forms-B
    (0x10D00, 0x10D3F), // Hanifi Rohingya
    (0x10EC0, 0x10EFF), // Arabic Extended-C
    (0x10F30, 0x10F6F), // Sogdian
    (0x1EC70, 0x1ECBF), // Indic Siyaq Numbers
    (0x1ED00, 0x1ED4F), // Ottoman Siyaq Numbers
    (0x1EE00, 0x1EEFF), // Arabic Mathematical Alphabetic Symbols
];
// FB1D..FB4F (Hebrew Presentation Forms) is a separate range from the
// main Hebrew block below because it is only *half* of the Alphabetic
// Presentation Forms block — the other half, FB00..FB1C (Latin
// ligatures), is Left_To_Right, not Right_To_Left.
pub(crate) const R_RANGES: &[(u32, u32)] = &[
    (0x0590, 0x05FF),   // Hebrew
    (0x07C0, 0x085F),   // NKo, Samaritan, Mandaic
    (0xFB1D, 0xFB4F),   // Hebrew Presentation Forms
    (0x10800, 0x10CFF), // Cypriot..Old Hungarian
    (0x10D40, 0x10EBF), // Garay, Rumi Numeral Symbols, Yezidi
    (0x10F00, 0x10F2F), // Old Sogdian
    (0x10F70, 0x10FFF), // Old Uyghur..Elymaic
    (0x1E800, 0x1EC6F), // Mende Kikakui..Adlam
    (0x1ECC0, 0x1ECFF), // reserved remainder after Indic Siyaq Numbers (R-default)
    (0x1ED50, 0x1EDFF), // reserved remainder after Ottoman Siyaq Numbers (R-default)
    (0x1EF00, 0x1EFFF), // reserved remainder after Arabic Mathematical Alphabetic Symbols (R-default)
];
// Code points *inside* an `AL_RANGES`/`R_RANGES` span above whose real
// `Bidi_Class` (per `DerivedBidiClass.txt`'s explicit, non-`@missing`
// per-code-point entries, Unicode 17.0.0) is not that span's own
// `AL`/`R` default — see this function's doc, the "known imprecision"
// paragraph, for why this table exists (over-classifying these as
// strong `AL`/`R` can resolve the wrong direction outright, not just
// stop the scan one code point early) and how it was derived. Checked
// before either range table below, so every code point here is
// classified `None` (non-strong) instead of falling into
// `AL_RANGES`/`R_RANGES`'s membership check.
pub(crate) const NON_STRONG_WITHIN_AL_R_RANGES: &[(u32, u32)] = &[
    // Hebrew
    (0x0591, 0x05BD), // NSM
    (0x05BF, 0x05BF), // NSM
    (0x05C1, 0x05C2), // NSM
    (0x05C4, 0x05C5), // NSM
    (0x05C7, 0x05C7), // NSM
    // Arabic, Syriac, Arabic Supplement, Thaana
    (0x0600, 0x0605), // AN
    (0x0606, 0x0607), // ON
    (0x0609, 0x060A), // ET
    (0x060C, 0x060C), // CS
    (0x060E, 0x060F), // ON
    (0x0610, 0x061A), // NSM
    (0x064B, 0x065F), // NSM
    (0x0660, 0x0669), // AN
    (0x066A, 0x066A), // ET
    (0x066B, 0x066C), // AN
    (0x0670, 0x0670), // NSM
    (0x06D6, 0x06DC), // NSM
    (0x06DD, 0x06DD), // AN
    (0x06DE, 0x06DE), // ON
    (0x06DF, 0x06E4), // NSM
    (0x06E7, 0x06E8), // NSM
    (0x06E9, 0x06E9), // ON
    (0x06EA, 0x06ED), // NSM
    (0x06F0, 0x06F9), // EN
    (0x0711, 0x0711), // NSM
    (0x0730, 0x074A), // NSM
    (0x07A6, 0x07B0), // NSM
    // NKo, Samaritan, Mandaic
    (0x07EB, 0x07F3), // NSM
    (0x07F6, 0x07F6), // ON
    (0x07F7, 0x07F9), // ON
    (0x07FD, 0x07FD), // NSM
    (0x0816, 0x0819), // NSM
    (0x081B, 0x0823), // NSM
    (0x0825, 0x0827), // NSM
    (0x0829, 0x082D), // NSM
    (0x0859, 0x085B), // NSM
    // Syriac Supplement, Arabic Extended-B/-A
    (0x0890, 0x0891), // AN
    (0x0897, 0x089F), // NSM
    (0x08CA, 0x08E1), // NSM
    (0x08E2, 0x08E2), // AN
    (0x08E3, 0x08FF), // NSM (this entry's real DerivedBidiClass.txt
    // range continues to U+0902, already covered separately by
    // `NON_STRONG_ALPHABETIC_RANGES`'s Devanagari `(0x0900, 0x0902)`
    // entry below — clipped here to stay inside this span)
    // Hebrew Presentation Forms
    (0xFB1E, 0xFB1E), // NSM
    (0xFB29, 0xFB29), // ES
    // Arabic Presentation Forms-A (partial)
    (0xFBC3, 0xFBD2), // ON
    (0xFD3E, 0xFD3E), // ON
    (0xFD3F, 0xFD3F), // ON
    (0xFD40, 0xFD4F), // ON
    (0xFD90, 0xFD91), // ON
    (0xFDC8, 0xFDCF), // ON
    (0xFDFD, 0xFDFF), // ON
    // Arabic Presentation Forms-B
    (0xFEFF, 0xFEFF), // BN
    // Cypriot..Old Hungarian
    (0x1091F, 0x1091F), // ON
    (0x10A01, 0x10A03), // NSM
    (0x10A05, 0x10A06), // NSM
    (0x10A0C, 0x10A0F), // NSM
    (0x10A38, 0x10A3A), // NSM
    (0x10A3F, 0x10A3F), // NSM
    (0x10AE5, 0x10AE6), // NSM
    (0x10B39, 0x10B3F), // ON
    // Hanifi Rohingya
    (0x10D24, 0x10D27), // NSM
    (0x10D30, 0x10D39), // AN
    // Garay, Rumi Numeral Symbols, Yezidi
    (0x10D40, 0x10D49), // AN
    (0x10D69, 0x10D6D), // NSM
    (0x10D6E, 0x10D6E), // ON
    (0x10E60, 0x10E7E), // AN
    (0x10EAB, 0x10EAC), // NSM
    // Arabic Extended-C
    (0x10ED0, 0x10ED0), // ON
    (0x10ED1, 0x10ED8), // ON
    (0x10EFA, 0x10EFF), // NSM
    // Sogdian
    (0x10F46, 0x10F50), // NSM
    // Old Uyghur..Elymaic
    (0x10F82, 0x10F85), // NSM
    // Mende Kikakui..Adlam
    (0x1E8D0, 0x1E8D6), // NSM
    (0x1E944, 0x1E94A), // NSM
    // Arabic Mathematical Alphabetic Symbols
    (0x1EEF0, 0x1EEF1), // ON
];
// Code points where `Alphabetic=Yes` (the property `char::is_alphabetic`
// implements) but the real `Bidi_Class` is something other than `L` —
// see this function's doc, "`is_alphabetic()` fallback" section, case 1,
// for why this table exists and how it was derived. None of these
// ranges overlaps `AL_RANGES`/`R_RANGES` above (by construction of the
// derivation), so every entry here really does fall through to the
// `is_alphabetic()` branch below absent this check.
pub(crate) const NON_STRONG_ALPHABETIC_RANGES: &[(u32, u32)] = &[
    // Spacing Modifier Letters
    (0x02B9, 0x02BA), // ON
    (0x02C6, 0x02CF), // ON
    (0x02EC, 0x02EC), // ON
    // Combining Diacritical Marks
    (0x0345, 0x0345), // NSM
    (0x0363, 0x036F), // NSM
    // Greek and Coptic
    (0x0374, 0x0374), // ON
    // Devanagari
    (0x0900, 0x0902), // NSM
    (0x093A, 0x093A), // NSM
    (0x0941, 0x0948), // NSM
    (0x0955, 0x0957), // NSM
    (0x0962, 0x0963), // NSM
    // Bengali
    (0x0981, 0x0981), // NSM
    (0x09C1, 0x09C4), // NSM
    (0x09E2, 0x09E3), // NSM
    // Gurmukhi
    (0x0A01, 0x0A02), // NSM
    (0x0A41, 0x0A42), // NSM
    (0x0A47, 0x0A48), // NSM
    (0x0A4B, 0x0A4C), // NSM
    (0x0A51, 0x0A51), // NSM
    (0x0A70, 0x0A71), // NSM
    (0x0A75, 0x0A75), // NSM
    // Gujarati
    (0x0A81, 0x0A82), // NSM
    (0x0AC1, 0x0AC5), // NSM
    (0x0AC7, 0x0AC8), // NSM
    (0x0AE2, 0x0AE3), // NSM
    (0x0AFA, 0x0AFC), // NSM
    // Oriya
    (0x0B01, 0x0B01), // NSM
    (0x0B3F, 0x0B3F), // NSM
    (0x0B41, 0x0B44), // NSM
    (0x0B56, 0x0B56), // NSM
    (0x0B62, 0x0B63), // NSM
    // Tamil
    (0x0B82, 0x0B82), // NSM
    (0x0BC0, 0x0BC0), // NSM
    // Telugu
    (0x0C00, 0x0C00), // NSM
    (0x0C04, 0x0C04), // NSM
    (0x0C3E, 0x0C40), // NSM
    (0x0C46, 0x0C48), // NSM
    (0x0C4A, 0x0C4C), // NSM
    (0x0C55, 0x0C56), // NSM
    (0x0C62, 0x0C63), // NSM
    // Kannada
    (0x0C81, 0x0C81), // NSM
    (0x0CCC, 0x0CCC), // NSM
    (0x0CE2, 0x0CE3), // NSM
    // Malayalam
    (0x0D00, 0x0D01), // NSM
    (0x0D41, 0x0D44), // NSM
    (0x0D62, 0x0D63), // NSM
    // Sinhala
    (0x0D81, 0x0D81), // NSM
    (0x0DD2, 0x0DD4), // NSM
    (0x0DD6, 0x0DD6), // NSM
    // Thai
    (0x0E31, 0x0E31), // NSM
    (0x0E34, 0x0E3A), // NSM
    (0x0E4D, 0x0E4D), // NSM
    // Lao
    (0x0EB1, 0x0EB1), // NSM
    (0x0EB4, 0x0EB9), // NSM
    (0x0EBB, 0x0EBC), // NSM
    (0x0ECD, 0x0ECD), // NSM
    // Tibetan
    (0x0F71, 0x0F7E), // NSM
    (0x0F80, 0x0F83), // NSM
    (0x0F8D, 0x0F97), // NSM
    (0x0F99, 0x0FBC), // NSM
    // Myanmar
    (0x102D, 0x1030), // NSM
    (0x1032, 0x1036), // NSM
    (0x103D, 0x103E), // NSM
    (0x1058, 0x1059), // NSM
    (0x105E, 0x1060), // NSM
    (0x1071, 0x1074), // NSM
    (0x1082, 0x1082), // NSM
    (0x1085, 0x1086), // NSM
    (0x108D, 0x108D), // NSM
    (0x109D, 0x109D), // NSM
    // Tagalog
    (0x1712, 0x1713), // NSM
    // Hanunoo
    (0x1732, 0x1733), // NSM
    // Buhid
    (0x1752, 0x1753), // NSM
    // Tagbanwa
    (0x1772, 0x1773), // NSM
    // Khmer
    (0x17B7, 0x17BD), // NSM
    (0x17C6, 0x17C6), // NSM
    // Mongolian
    (0x1885, 0x1886), // NSM
    (0x18A9, 0x18A9), // NSM
    // Limbu
    (0x1920, 0x1922), // NSM
    (0x1927, 0x1928), // NSM
    (0x1932, 0x1932), // NSM
    // Buginese
    (0x1A17, 0x1A18), // NSM
    (0x1A1B, 0x1A1B), // NSM
    // Tai Tham
    (0x1A56, 0x1A56), // NSM
    (0x1A58, 0x1A5E), // NSM
    (0x1A62, 0x1A62), // NSM
    (0x1A65, 0x1A6C), // NSM
    (0x1A73, 0x1A74), // NSM
    // Combining Diacritical Marks Extended
    (0x1ABF, 0x1AC0), // NSM
    (0x1ACC, 0x1ACE), // NSM
    // Balinese
    (0x1B00, 0x1B03), // NSM
    (0x1B36, 0x1B3A), // NSM
    (0x1B3C, 0x1B3C), // NSM
    (0x1B42, 0x1B42), // NSM
    // Sundanese
    (0x1B80, 0x1B81), // NSM
    (0x1BA2, 0x1BA5), // NSM
    (0x1BA8, 0x1BA9), // NSM
    (0x1BAC, 0x1BAD), // NSM
    // Batak
    (0x1BE8, 0x1BE9), // NSM
    (0x1BED, 0x1BED), // NSM
    (0x1BEF, 0x1BF1), // NSM
    // Lepcha
    (0x1C2C, 0x1C33), // NSM
    (0x1C36, 0x1C36), // NSM
    // Combining Diacritical Marks Supplement
    (0x1DD3, 0x1DF4), // NSM
    // Cyrillic Extended-A
    (0x2DE0, 0x2DFF), // NSM
    // Supplemental Punctuation
    (0x2E2F, 0x2E2F), // ON
    // Cyrillic Extended-B
    (0xA674, 0xA67B), // NSM
    (0xA67F, 0xA67F), // ON
    (0xA69E, 0xA69F), // NSM
    // Modifier Tone Letters
    (0xA717, 0xA71F), // ON
    // Latin Extended-D
    (0xA788, 0xA788), // ON
    // Syloti Nagri
    (0xA802, 0xA802), // NSM
    (0xA80B, 0xA80B), // NSM
    (0xA825, 0xA826), // NSM
    // Saurashtra
    (0xA8C5, 0xA8C5), // NSM
    // Devanagari Extended
    (0xA8FF, 0xA8FF), // NSM
    // Kayah Li
    (0xA926, 0xA92A), // NSM
    // Rejang
    (0xA947, 0xA951), // NSM
    // Javanese
    (0xA980, 0xA982), // NSM
    (0xA9B6, 0xA9B9), // NSM
    (0xA9BC, 0xA9BD), // NSM
    // Myanmar Extended-B
    (0xA9E5, 0xA9E5), // NSM
    // Cham
    (0xAA29, 0xAA2E), // NSM
    (0xAA31, 0xAA32), // NSM
    (0xAA35, 0xAA36), // NSM
    (0xAA43, 0xAA43), // NSM
    (0xAA4C, 0xAA4C), // NSM
    // Myanmar Extended-A
    (0xAA7C, 0xAA7C), // NSM
    // Tai Viet
    (0xAAB0, 0xAAB0), // NSM
    (0xAAB2, 0xAAB4), // NSM
    (0xAAB7, 0xAAB8), // NSM
    (0xAABE, 0xAABE), // NSM
    // Meetei Mayek Extensions
    (0xAAEC, 0xAAED), // NSM
    // Meetei Mayek
    (0xABE5, 0xABE5), // NSM
    (0xABE8, 0xABE8), // NSM
    // Ancient Greek Numbers
    (0x10140, 0x10174), // ON
    // Old Permic
    (0x10376, 0x1037A), // NSM
    // Brahmi
    (0x11001, 0x11001), // NSM
    (0x11038, 0x11045), // NSM
    (0x11073, 0x11074), // NSM
    // Kaithi
    (0x11080, 0x11081), // NSM
    (0x110B3, 0x110B6), // NSM
    (0x110C2, 0x110C2), // NSM
    // Chakma
    (0x11100, 0x11102), // NSM
    (0x11127, 0x1112B), // NSM
    (0x1112D, 0x11132), // NSM
    // Sharada
    (0x11180, 0x11181), // NSM
    (0x111B6, 0x111BE), // NSM
    (0x111CF, 0x111CF), // NSM
    // Khojki
    (0x1122F, 0x11231), // NSM
    (0x11234, 0x11234), // NSM
    (0x11237, 0x11237), // NSM
    (0x1123E, 0x1123E), // NSM
    (0x11241, 0x11241), // NSM
    // Khudawadi
    (0x112DF, 0x112DF), // NSM
    (0x112E3, 0x112E8), // NSM
    // Grantha
    (0x11300, 0x11301), // NSM
    (0x11340, 0x11340), // NSM
    // Tulu-Tigalari
    (0x113BB, 0x113C0), // NSM
    // Newa
    (0x11438, 0x1143F), // NSM
    (0x11443, 0x11444), // NSM
    // Tirhuta
    (0x114B3, 0x114B8), // NSM
    (0x114BA, 0x114BA), // NSM
    (0x114BF, 0x114C0), // NSM
    // Siddham
    (0x115B2, 0x115B5), // NSM
    (0x115BC, 0x115BD), // NSM
    (0x115DC, 0x115DD), // NSM
    // Modi
    (0x11633, 0x1163A), // NSM
    (0x1163D, 0x1163D), // NSM
    (0x11640, 0x11640), // NSM
    // Takri
    (0x116AB, 0x116AB), // NSM
    (0x116AD, 0x116AD), // NSM
    (0x116B0, 0x116B5), // NSM
    // Ahom
    (0x1171D, 0x1171D), // NSM
    (0x1171F, 0x1171F), // NSM
    (0x11722, 0x11725), // NSM
    (0x11727, 0x1172A), // NSM
    // Dogra
    (0x1182F, 0x11837), // NSM
    // Dives Akuru
    (0x1193B, 0x1193C), // NSM
    // Nandinagari
    (0x119D4, 0x119D7), // NSM
    (0x119DA, 0x119DB), // NSM
    // Zanabazar Square
    (0x11A01, 0x11A06), // NSM
    (0x11A09, 0x11A0A), // NSM
    (0x11A35, 0x11A38), // NSM
    (0x11A3B, 0x11A3E), // NSM
    // Soyombo
    (0x11A51, 0x11A56), // NSM
    (0x11A59, 0x11A5B), // NSM
    (0x11A8A, 0x11A96), // NSM
    // Sharada Supplement
    (0x11B60, 0x11B60), // NSM
    (0x11B62, 0x11B64), // NSM
    (0x11B66, 0x11B66), // NSM
    // Bhaiksuki
    (0x11C30, 0x11C36), // NSM
    (0x11C38, 0x11C3D), // NSM
    // Marchen
    (0x11C92, 0x11CA7), // NSM
    (0x11CAA, 0x11CB0), // NSM
    (0x11CB2, 0x11CB3), // NSM
    (0x11CB5, 0x11CB6), // NSM
    // Masaram Gondi
    (0x11D31, 0x11D36), // NSM
    (0x11D3A, 0x11D3A), // NSM
    (0x11D3C, 0x11D3D), // NSM
    (0x11D3F, 0x11D41), // NSM
    (0x11D43, 0x11D43), // NSM
    (0x11D47, 0x11D47), // NSM
    // Gunjala Gondi
    (0x11D90, 0x11D91), // NSM
    (0x11D95, 0x11D95), // NSM
    // Makasar
    (0x11EF3, 0x11EF4), // NSM
    // Kawi
    (0x11F00, 0x11F01), // NSM
    (0x11F36, 0x11F3A), // NSM
    (0x11F40, 0x11F40), // NSM
    // Gurung Khema
    (0x1611E, 0x16129), // NSM
    (0x1612D, 0x1612E), // NSM
    // Miao
    (0x16F4F, 0x16F4F), // NSM
    (0x16F8F, 0x16F92), // NSM
    // Duployan
    (0x1BC9E, 0x1BC9E), // NSM
    // Glagolitic Supplement
    (0x1E000, 0x1E006), // NSM
    (0x1E008, 0x1E018), // NSM
    (0x1E01B, 0x1E021), // NSM
    (0x1E023, 0x1E024), // NSM
    (0x1E026, 0x1E02A), // NSM
    // Cyrillic Extended-D
    (0x1E08F, 0x1E08F), // NSM
    // Tai Yo
    (0x1E6E3, 0x1E6E3), // NSM
    (0x1E6E6, 0x1E6E6), // NSM
    (0x1E6EE, 0x1E6EF), // NSM
    (0x1E6F5, 0x1E6F5), // NSM
];
// Code points where the real `Bidi_Class` (per `DerivedBidiClass.txt`'s
// explicit, non-`@missing` per-code-point entries, Unicode 17.0.0) is `L`,
// but `Alphabetic=Yes` does not hold (`char::is_alphabetic() == false`) —
// see this function's doc, "`is_alphabetic()` fallback" section, case 2,
// for why this table exists and its scope. Restricted to entries whose
// `General_Category` (`DerivedGeneralCategory.txt`, same Unicode version)
// is `Nd` (decimal digit), one of the punctuation categories
// `Pc`/`Pd`/`Pe`/`Pf`/`Pi`/`Po`/`Ps`, or `Mc` (spacing combining mark) —
// plus a single `Mn` (non-spacing mark) entry that is functionally
// identical to the `Mc` ones (noted at its own site below) — see case 2's
// doc for why each of these shapes is included and what is deliberately
// left out. None of these ranges overlaps `AL_RANGES`/`R_RANGES` above
// (verified against this crate's actual `AL_RANGES`/`R_RANGES` tables at
// derivation time); none is `Alphabetic=Yes` under this workspace's
// pinned `char::is_alphabetic` (verified at derivation time) — except
// the three Sharada Vowel Signs Supplement entries noted at their own
// site below, which are `Alphabetic=Yes` under UCD 17.0.0 itself and
// need an entry here for exactly that reason (see their comment). Those
// three are individually-fixed instances of a much larger toolchain-lag
// class this table does not attempt to cover in full — see this
// function's doc, "`is_alphabetic()` fallback" section, case 2's second
// paragraph, for the fuller picture and why the rest is out of scope
// here. None overlaps `NON_STRONG_ALPHABETIC_RANGES` above — true by
// construction for every entry except those same three, since every
// other entry there has `Alphabetic=Yes` and every other entry here
// does not, while the three Sharada entries are instead checked
// directly against that table's actual code points (its `NSM` entries
// for the same Unicode block are 0x11B60, 0x11B62..0x11B64, 0x11B66 —
// disjoint from 0x11B61/0x11B65/0x11B67 here). Also checked directly at
// derivation time for every entry (range-pair overlap, not just
// individual code points, since both tables store ranges). So every
// entry here really would fall through to the final `None` below absent
// this check.
pub(crate) const STRONG_L_NON_ALPHABETIC_RANGES: &[(u32, u32)] = &[
    // Armenian
    (0x055A, 0x055F), // Po
    (0x0589, 0x0589), // Po
    // Devanagari
    (0x0964, 0x0965), // Po
    (0x0966, 0x096F), // Nd
    (0x0970, 0x0970), // Po
    // Bengali
    (0x09E6, 0x09EF), // Nd
    (0x09FD, 0x09FD), // Po
    // Gurmukhi
    (0x0A66, 0x0A6F), // Nd
    (0x0A76, 0x0A76), // Po
    // Gujarati
    (0x0AE6, 0x0AEF), // Nd
    (0x0AF0, 0x0AF0), // Po
    // Oriya
    (0x0B66, 0x0B6F), // Nd
    // Tamil
    (0x0BE6, 0x0BEF), // Nd
    // Telugu
    (0x0C66, 0x0C6F), // Nd
    (0x0C77, 0x0C77), // Po
    // Kannada
    (0x0C84, 0x0C84), // Po
    (0x0CE6, 0x0CEF), // Nd
    // Malayalam
    (0x0D66, 0x0D6F), // Nd
    // Sinhala
    (0x0DE6, 0x0DEF), // Nd
    (0x0DF4, 0x0DF4), // Po
    // Thai
    (0x0E4F, 0x0E4F), // Po
    (0x0E50, 0x0E59), // Nd
    (0x0E5A, 0x0E5B), // Po
    // Lao
    (0x0ED0, 0x0ED9), // Nd
    // Tibetan
    (0x0F04, 0x0F12), // Po
    (0x0F14, 0x0F14), // Po
    (0x0F20, 0x0F29), // Nd
    (0x0F85, 0x0F85), // Po
    (0x0FD0, 0x0FD4), // Po
    (0x0FD9, 0x0FDA), // Po
    // Myanmar
    (0x1040, 0x1049), // Nd
    (0x104A, 0x104F), // Po
    (0x1090, 0x1099), // Nd
    // Georgian
    (0x10FB, 0x10FB), // Po
    // Ethiopic
    (0x1360, 0x1368), // Po
    // Unified Canadian Aboriginal Syllabics
    (0x166E, 0x166E), // Po
    // Runic
    (0x16EB, 0x16ED), // Po
    // Hanunoo
    (0x1735, 0x1736), // Po
    // Khmer
    (0x17D4, 0x17D6), // Po
    (0x17D8, 0x17DA), // Po
    (0x17E0, 0x17E9), // Nd
    // Mongolian
    (0x1810, 0x1819), // Nd
    // Limbu
    (0x1946, 0x194F), // Nd
    // New Tai Lue
    (0x19D0, 0x19D9), // Nd
    // Buginese
    (0x1A1E, 0x1A1F), // Po
    // Tai Tham
    (0x1A80, 0x1A89), // Nd
    (0x1A90, 0x1A99), // Nd
    (0x1AA0, 0x1AA6), // Po
    (0x1AA8, 0x1AAD), // Po
    // Balinese
    (0x1B4E, 0x1B4F), // Po
    (0x1B50, 0x1B59), // Nd
    (0x1B5A, 0x1B60), // Po
    (0x1B7D, 0x1B7F), // Po
    // Sundanese
    (0x1BB0, 0x1BB9), // Nd
    // Batak
    (0x1BFC, 0x1BFF), // Po
    // Lepcha
    (0x1C3B, 0x1C3F), // Po
    (0x1C40, 0x1C49), // Nd
    // Ol Chiki
    (0x1C50, 0x1C59), // Nd
    (0x1C7E, 0x1C7F), // Po
    // Sundanese Supplement
    (0x1CC0, 0x1CC7), // Po
    // Vedic Extensions
    (0x1CD3, 0x1CD3), // Po
    // Tifinagh
    (0x2D70, 0x2D70), // Po
    // Lisu
    (0xA4FE, 0xA4FF), // Po
    // Vai
    (0xA620, 0xA629), // Nd
    // Bamum
    (0xA6F2, 0xA6F7), // Po
    // Saurashtra
    (0xA8CE, 0xA8CF), // Po
    (0xA8D0, 0xA8D9), // Nd
    // Devanagari Extended
    (0xA8F8, 0xA8FA), // Po
    (0xA8FC, 0xA8FC), // Po
    // Kayah Li
    (0xA900, 0xA909), // Nd
    (0xA92E, 0xA92F), // Po
    // Rejang
    (0xA95F, 0xA95F), // Po
    // Javanese
    (0xA9C1, 0xA9CD), // Po
    (0xA9D0, 0xA9D9), // Nd
    (0xA9DE, 0xA9DF), // Po
    // Myanmar Extended-B
    (0xA9F0, 0xA9F9), // Nd
    // Cham
    (0xAA50, 0xAA59), // Nd
    (0xAA5C, 0xAA5F), // Po
    // Tai Viet
    (0xAADE, 0xAADF), // Po
    // Meetei Mayek Extensions
    (0xAAF0, 0xAAF1), // Po
    // Meetei Mayek
    (0xABEB, 0xABEB), // Po
    (0xABF0, 0xABF9), // Nd
    // Aegean Numbers
    (0x10100, 0x10100), // Po
    (0x10102, 0x10102), // Po
    // Ugaritic
    (0x1039F, 0x1039F), // Po
    // Old Persian
    (0x103D0, 0x103D0), // Po
    // Osmanya
    (0x104A0, 0x104A9), // Nd
    // Caucasian Albanian
    (0x1056F, 0x1056F), // Po
    // Brahmi
    (0x11047, 0x1104D), // Po
    (0x11066, 0x1106F), // Nd
    // Kaithi
    (0x110BB, 0x110BC), // Po
    (0x110BE, 0x110C1), // Po
    // Sora Sompeng
    (0x110F0, 0x110F9), // Nd
    // Chakma
    (0x11136, 0x1113F), // Nd
    (0x11140, 0x11143), // Po
    // Mahajani
    (0x11174, 0x11175), // Po
    // Sharada
    (0x111C5, 0x111C8), // Po
    (0x111CD, 0x111CD), // Po
    (0x111D0, 0x111D9), // Nd
    (0x111DB, 0x111DB), // Po
    (0x111DD, 0x111DF), // Po
    // Khojki
    (0x11238, 0x1123D), // Po
    // Multani
    (0x112A9, 0x112A9), // Po
    // Khudawadi
    (0x112F0, 0x112F9), // Nd
    // Tulu-Tigalari
    (0x113D4, 0x113D5), // Po
    (0x113D7, 0x113D8), // Po
    // Newa
    (0x1144B, 0x1144F), // Po
    (0x11450, 0x11459), // Nd
    (0x1145A, 0x1145B), // Po
    (0x1145D, 0x1145D), // Po
    // Tirhuta
    (0x114C6, 0x114C6), // Po
    (0x114D0, 0x114D9), // Nd
    // Siddham
    (0x115C1, 0x115D7), // Po
    // Modi
    (0x11641, 0x11643), // Po
    (0x11650, 0x11659), // Nd
    // Takri
    (0x116B9, 0x116B9), // Po
    (0x116C0, 0x116C9), // Nd
    // Myanmar Extended-C
    (0x116D0, 0x116E3), // Nd
    // Ahom
    (0x11730, 0x11739), // Nd
    (0x1173C, 0x1173E), // Po
    // Dogra
    (0x1183B, 0x1183B), // Po
    // Warang Citi
    (0x118E0, 0x118E9), // Nd
    // Dives Akuru
    (0x11944, 0x11946), // Po
    (0x11950, 0x11959), // Nd
    // Nandinagari
    (0x119E2, 0x119E2), // Po
    // Zanabazar Square
    (0x11A3F, 0x11A46), // Po
    // Soyombo
    (0x11A9A, 0x11A9C), // Po
    (0x11A9E, 0x11AA2), // Po
    // Devanagari Extended-A
    (0x11B00, 0x11B09), // Po
    // Sunuwar
    (0x11BE1, 0x11BE1), // Po
    (0x11BF0, 0x11BF9), // Nd
    // Bhaiksuki
    (0x11C41, 0x11C45), // Po
    (0x11C50, 0x11C59), // Nd
    // Marchen
    (0x11C70, 0x11C71), // Po
    // Masaram Gondi
    (0x11D50, 0x11D59), // Nd
    // Gunjala Gondi
    (0x11DA0, 0x11DA9), // Nd
    // Tolong Siki
    (0x11DE0, 0x11DE9), // Nd
    // Makasar
    (0x11EF7, 0x11EF8), // Po
    // Kawi
    (0x11F43, 0x11F4F), // Po
    (0x11F50, 0x11F59), // Nd
    // Tamil Supplement
    (0x11FFF, 0x11FFF), // Po
    // Cuneiform Numbers and Punctuation
    (0x12470, 0x12474), // Po
    // Cypro-Minoan
    (0x12FF1, 0x12FF2), // Po
    // Gurung Khema
    (0x16130, 0x16139), // Nd
    // Mro
    (0x16A60, 0x16A69), // Nd
    (0x16A6E, 0x16A6F), // Po
    // Tangsa
    (0x16AC0, 0x16AC9), // Nd
    // Bassa Vah
    (0x16AF5, 0x16AF5), // Po
    // Pahawh Hmong
    (0x16B37, 0x16B3B), // Po
    (0x16B44, 0x16B44), // Po
    (0x16B50, 0x16B59), // Nd
    // Kirat Rai
    (0x16D6D, 0x16D6F), // Po
    (0x16D70, 0x16D79), // Nd
    // Medefaidrin
    (0x16E97, 0x16E9A), // Po
    // Duployan
    (0x1BC9F, 0x1BC9F), // Po
    // Sutton SignWriting
    (0x1DA87, 0x1DA8B), // Po
    // Nyiakeng Puachue Hmong
    (0x1E140, 0x1E149), // Nd
    // Wancho
    (0x1E2F0, 0x1E2F9), // Nd
    // Nag Mundari
    (0x1E4F0, 0x1E4F9), // Nd
    // Ol Onal
    (0x1E5F1, 0x1E5FA), // Nd
    (0x1E5FF, 0x1E5FF), // Po
    // The entries above are all `Nd`/punctuation. The entries below are
    // `Mc` (spacing combining mark), plus one `Mn` entry noted at its
    // own site — see this function's doc, case 2, for why these are
    // included here even though the surrounding doc text otherwise
    // talks about "named-script digits and punctuation": most `Mc`
    // code points with an explicit `Bidi_Class=L` entry are vowel signs
    // that are already `Alphabetic=Yes` and so already resolve
    // correctly through the `is_alphabetic()` branch above without
    // needing a table entry at all; these are the residual — viramas,
    // tone marks, and similar combining marks that Unicode does not
    // consider `Alphabetic` (a virama suppresses a vowel rather than
    // representing one) despite carrying `Bidi_Class=L` themselves.
    // Tibetan
    (0x0F3E, 0x0F3F), // Mc
    // Tagalog
    (0x1715, 0x1715), // Mc
    // Hanunoo
    (0x1734, 0x1734), // Mc
    // Balinese
    (0x1B44, 0x1B44), // Mc
    // Sundanese
    (0x1BAA, 0x1BAA), // Mc
    // Batak
    (0x1BF2, 0x1BF3), // Mc
    // Vedic Extensions
    (0x1CE1, 0x1CE1), // Mc
    (0x1CF7, 0x1CF7), // Mc
    // CJK Symbols and Punctuation (Hangul tone marks)
    (0x302E, 0x302F), // Mc
    // Rejang
    (0xA953, 0xA953), // Mc
    // Javanese
    (0xA9C0, 0xA9C0), // Mc
    // Meetei Mayek
    (0xABEC, 0xABEC), // Mc
    // Sharada
    (0x111C0, 0x111C0), // Mc
    // Khojki
    (0x11235, 0x11235), // Mc
    // Grantha
    (0x1134D, 0x1134D), // Mc
    // Tulu-Tigalari
    (0x113CF, 0x113CF), // Mc
    // Takri
    (0x116B6, 0x116B6), // Mc
    // Dives Akuru
    (0x1193D, 0x1193D), // Mc
    // Sharada Vowel Signs Supplement (new in Unicode 17.0.0). Excluded
    // from this workspace's pinned rustc 1.89.0 `char::is_alphabetic()`
    // tables, which predate this block's assignment, even though UCD
    // 17.0.0 marks these `Alphabetic=Yes` like the other vowel signs in
    // this script — so unlike most `Mc` vowel signs, these three still
    // need an explicit entry here rather than relying on the
    // `is_alphabetic()` branch.
    (0x11B61, 0x11B61), // Mc
    (0x11B65, 0x11B65), // Mc
    (0x11B67, 0x11B67), // Mc
    // Bhaiksuki (this one virama is `Mn`, not `Mc`, unlike its
    // counterparts above — same role, different General_Category)
    (0x11C3F, 0x11C3F), // Mn
    // Kawi
    (0x11F41, 0x11F41), // Mc
    // Musical Symbols
    (0x1D165, 0x1D166), // Mc
    (0x1D16D, 0x1D172), // Mc
];

/// Classifies a code point's Unicode Bidi_Class (Unicode Standard Annex #9,
/// the "\[BIDI\]" reference [`text_node_first_strong_direction`]'s HTML LS
/// quote cites) into one of the 3 *strong* types this crate's simplified
/// auto-directionality scan needs — `L`, `Al`, or `R` — or `None` for every
/// other Bidi_Class (the weak and neutral types: European/Arabic numbers,
/// separators, terminators, whitespace, neutral punctuation, combining
/// marks, controls, …), which is exactly the "not strong, keep scanning"
/// outcome [`text_node_first_strong_direction`]'s caller needs for those.
///
/// # Scope: default-block ranges only, not a full per-code-point Bidi_Class table
///
/// The Unicode Character Database's actual `Bidi_Class` property is a
/// complete per-code-point assignment (`DerivedBidiClass.txt`,
/// <https://www.unicode.org/Public/UCD/latest/ucd/extracted/DerivedBidiClass.txt>)
/// — implementing that in full is a multi-thousand-range data table on its
/// own, comparable in scope to the RFC4647/BCP47 canonicalization table
/// [`super::lang::language_range_matches`]'s doc similarly declines to bring in for a
/// different feature. This function instead hard-codes exactly the ranges
/// that same UCD file's own "`@missing`" comments give as the **default**
/// Bidi_Class for every script block the Unicode Standard reserves for
/// right-to-left use (`AL_RANGES` / `R_RANGES` below, transcribed directly
/// from those `@missing` lines, and guarded against the embedded
/// non-default code points inside them by `NON_STRONG_WITHIN_AL_R_RANGES`,
/// see below) — this covers every script block Unicode reserves by default
/// for Arabic-derived (`AL`) or other right-to-left (`R`) use, not just
/// "the major ones". Any code point outside all of those ranges falls back
/// to [`char::is_alphabetic`] (guarded by `NON_STRONG_ALPHABETIC_RANGES`,
/// see below) for `L` (covers Latin, Greek,
/// Cyrillic, CJK, Hangul, Devanagari, and effectively every other
/// left-to-right alphabetic script), then to `STRONG_L_NON_ALPHABETIC_RANGES`
/// (see below) for a further set of code points whose real `Bidi_Class` is
/// `L` despite not being alphabetic (non-Latin decimal digits,
/// script-specific punctuation, and viramas/tone marks that Unicode does
/// not tag `Alphabetic`), or `None` for everything else — which matches
/// the real `Bidi_Class` table for most of those too (their actual classes
/// are typically weak/neutral types like EN, AN, CS, ON, WS, NSM). This is
/// not exhaustive — case 2 of the `is_alphabetic()` fallback section below
/// documents two further residuals left deliberately unguarded beyond
/// `STRONG_L_NON_ALPHABETIC_RANGES`'s fixed `Nd`/punctuation/`Mc`
/// categories: a version-drift-driven one, where a Unicode revision newer
/// than what the pinned toolchain's `is_alphabetic()` was built against
/// assigns real `Bidi_Class=L` to code points this function does not yet
/// special-case (currently on the order of thousands, of which only three
/// are individually listed), and a smaller, toolchain-independent
/// structural one (`So`/`No` symbols and numbers) — see case 2 for both
/// residuals' shape and why each is left unguarded. Three specific code
/// points *are* handled explicitly before
/// reaching any of this, cheaply, without a table: U+200E LEFT-TO-RIGHT
/// MARK and U+200F RIGHT-TO-LEFT MARK
/// (`Cf`, General Punctuation block) and U+061C ARABIC LETTER MARK (`Cf`,
/// Arabic block) all have an explicit (non-`@missing`)
/// `DerivedBidiClass.txt` entry of `L`, `R`, and `AL` respectively — none
/// is alphabetic, so none would resolve correctly through the
/// `is_alphabetic`-based fallback above. U+061C already falls
/// inside `AL_RANGES`'s main Arabic range (U+0600..U+07BF) and needs no
/// special handling; U+200E and U+200F fall inside neither `AL_RANGES` nor
/// `R_RANGES`, so this function checks for those two explicitly, before
/// either range table, at the top of its body.
///
/// A known imprecision from using whole-block ranges rather than the real
/// per-code-point table: a handful of code points *inside* the
/// `AL_RANGES`/`R_RANGES` below (e.g. Arabic-Indic digits, Hebrew
/// punctuation and accents) have a real `Bidi_Class` that is actually a
/// weak/neutral type, not `AL`/`R`. Over-classifying such a code point as
/// strong `AL`/`R` can make the scan stop *and resolve* on it before ever
/// reaching the text's true first strong character, flipping the result
/// outright — not a harmless "one code point early" scan offset with the
/// same eventual outcome. Example: U+0664 ARABIC-INDIC DIGIT FOUR falls
/// inside `AL_RANGES`'s main Arabic span but its real `Bidi_Class` is `AN`
/// (Arabic Number, weak); in `"\u{0664}H"` the spec skips it and resolves
/// on the following Latin `H` (`L`) → `ltr`, whereas over-classifying the
/// digit as strong `AL` would wrongly stop the scan there and resolve
/// `rtl`. `NON_STRONG_WITHIN_AL_R_RANGES` below closes this gap: every
/// explicit (non-`@missing`) `DerivedBidiClass.txt` entry whose range
/// falls inside an `AL_RANGES`/`R_RANGES` span and whose class is not that
/// span's own `AL`/`R` default (74 entries for Unicode 17.0.0 — none of
/// them `L`, only other weak/neutral types: `NSM`, `ON`, `AN`, `EN`, `ET`,
/// `ES`, `CS`, `BN`) — the same "guard the default-block table with the
/// exceptions extracted from the same source file" shape
/// `NON_STRONG_ALPHABETIC_RANGES` below uses for the `is_alphabetic()`
/// fallback. This function checks it before either range table and treats
/// a match as non-strong (`None`), so `AL_RANGES`/`R_RANGES` membership
/// alone no longer over-classifies these code points as strong. Unlike
/// `NON_STRONG_ALPHABETIC_RANGES` (which guards `char::is_alphabetic`, a
/// property computed by a separate, independently-versioned Rust/Unicode
/// table — see its own version-drift caveat below), `AL_RANGES`/`R_RANGES`
/// and this table are both transcribed from the same `DerivedBidiClass.txt`
/// snapshot (Unicode 17.0.0), so there is no comparable cross-source
/// version skew to caveat here.
///
/// # `is_alphabetic()` fallback: two gaps against the real `Bidi_Class`
///
/// Outside `AL_RANGES`/`R_RANGES`, `L` is derived from
/// [`char::is_alphabetic`] — i.e. from Unicode's `Alphabetic` property
/// (`DerivedCoreProperties.txt`), a *different* property than `Bidi_Class`.
/// The two properties agree for the overwhelming majority of code points,
/// but diverge in two ways that can each flip the resolved direction
/// outright:
///
/// 1. **False `L`**: some code points are `Alphabetic=Yes` but their real
///    `Bidi_Class` is not `L` at all — mostly `Mn`/`Mc` combining marks
///    classified `NSM`, plus a smaller set of `Lm` spacing modifier
///    letters and one `Nl` numeral-symbol block (Ancient Greek Numbers,
///    U+10140..U+10174), all classified `ON`. Left unguarded,
///    `is_alphabetic()` would misclassify these as strong `L` and stop the
///    scan right there, even though the text's real first strong character
///    is later. Example: U+0941 DEVANAGARI VOWEL SIGN U is
///    `Other_Alphabetic` (`is_alphabetic() == true`) but `Bidi_Class=NSM`
///    (non-strong); in `"\u{0941}שלום"` the spec skips it and resolves on
///    the following Hebrew (`R`) code point → `rtl`, whereas treating
///    U+0941 itself as strong `L` would wrongly stop the scan there and
///    resolve `ltr`. `NON_STRONG_ALPHABETIC_RANGES` below closes this gap:
///    for Unicode 17.0.0, it is the full set of code points with
///    `Alphabetic=Yes` and a `Bidi_Class` other than `L`, outside
///    `AL_RANGES`/`R_RANGES` (whose own over-classification exceptions are
///    the separate gap `NON_STRONG_WITHIN_AL_R_RANGES` above closes) —
///    derived by intersecting
///    `DerivedCoreProperties.txt`'s `Alphabetic` ranges against
///    `DerivedBidiClass.txt`'s explicit per-code-point entries, both at
///    that same Unicode version. `char::is_alphabetic` itself may track a
///    different Unicode revision than 17.0.0 (this crate does not check
///    Rust's own Unicode table version); any code point added or
///    reclassified between that revision and 17.0.0 is not guaranteed to
///    be covered. This function consults the table before falling back to
///    `is_alphabetic`, and treats a match as non-strong (`None`) rather
///    than `L`.
///
/// 2. **False non-`L`**: the opposite gap — some code points have real
///    `Bidi_Class=L` but `is_alphabetic() == false`, mostly decimal digits
///    of non-Latin scripts (e.g. U+0966..U+096F DEVANAGARI DIGIT
///    ZERO..NINE) plus a smaller set of punctuation (e.g. U+055A..U+055F
///    Armenian punctuation). Left uncorrected, a genuinely-first `L`
///    character in this set would be skipped in favor of a later `AL`/`R`
///    character in the same text, resolving the wrong direction outright
///    — the same failure shape as case 1, just in the opposite direction.
///    `STRONG_L_NON_ALPHABETIC_RANGES` below closes the categorical part
///    of this gap: every explicit (non-`@missing`) `DerivedBidiClass.txt`
///    entry whose `Bidi_Class=L` and whose `General_Category`
///    (`DerivedGeneralCategory.txt`, same Unicode version) is `Nd`
///    (decimal digit), one of the punctuation categories
///    `Pc`/`Pd`/`Pe`/`Pf`/`Pi`/`Po`/`Ps`, or `Mc` (spacing combining mark,
///    plus one functionally-identical `Mn` entry) — 186 entries for
///    Unicode 17.0.0, exhaustive for exactly those categories plus the
///    three Sharada Vowel Signs Supplement code points below, which are
///    added for a different reason covered in the next paragraph, not
///    because a fourth category exists. The first two categories are
///    named-script digits and punctuation; `Mc` needs its own
///    explanation, since most `Mc` code points are vowel signs already
///    covered by `Alphabetic=Yes` — the ones actually needed here are
///    viramas, tone marks, and similar combining marks that Unicode does
///    not consider `Alphabetic` under any Unicode version, despite their
///    real `Bidi_Class=L`. (One member of this same case-2 category,
///    U+200E LEFT-TO-RIGHT MARK, is handled separately — not by this table but
///    by the explicit top-of-function check described above, since that code
///    point was already being singled out for U+200F's sake.)
///
///    A second, unrelated route into this same "real `L`,
///    `is_alphabetic() == false`" shape is version drift, mirroring case
///    1's caveat above: `char::is_alphabetic` may track a Unicode
///    revision older than the 17.0.0 this table's categories are derived
///    against, so a code point that Unicode 17.0.0 assigns both
///    `Bidi_Class=L` and `Alphabetic=Yes` can still fail
///    `is_alphabetic()` under an older-than-17.0.0 toolchain — even
///    though, by rights, it should need no table entry at all and be
///    caught by the plain `c.is_alphabetic()` branch above. The three
///    Sharada Vowel Signs Supplement code points below are exactly this:
///    `Mc`, `Alphabetic=Yes` under UCD 17.0.0, and so already inside this
///    table's `Nd`/punctuation/`Mc` category filter, but listed
///    individually because they are the three code points this table's
///    own derivation verified as affected. They are not the only code
///    points this mechanism affects. Checking this workspace's pinned rustc 1.89.0
///    (`rust-toolchain.toml`) against the same `Bidi_Class=L` ∧
///    `Alphabetic=Yes` (Unicode 17.0.0) intersection finds 4620 such code
///    points failing `char::is_alphabetic()`, not 3 — dominated by CJK
///    Unified Ideographs Extension J alone (4298 code points,
///    U+323B0..U+33479), plus Tangut Ideographic Components and a handful
///    of other letter blocks Unicode 17.0.0 newly assigned (Tai Yo, Beria
///    Erfe, Tolong Siki, Latin Extended-D). Checked directly, not just
///    assumed: of the 4620, exactly 3 are `General_Category` `Mc` — the
///    same three Sharada entries already listed — and none is `Mn`,
///    `Nd`, or one of the punctuation categories; the remaining 4617 are
///    `Lo` (4555), `Lu` (28), `Ll` (26), `Lm` (5), or `Nl` (3) letters
///    and letter-numbers, entirely outside this table's
///    `Nd`/punctuation/`Mc` category filter. So covering them here would
///    mean dropping that filter and turning this into a general
///    "toolchain hasn't caught up to Unicode 17.0.0 yet" table with a
///    completely different, much larger and version-churn-prone scope,
///    not an extension of the current one. Hardcoding thousands of
///    entries for that — one CJK block alone accounts for the
///    overwhelming majority — would be exactly the kind
///    of exhaustive per-code-point enumeration this function's tables
///    otherwise avoid, and it would go silently stale (redundant, not
///    merely unneeded) the moment the pinned toolchain is updated past
///    17.0.0, since `char::is_alphabetic` would then cover these code
///    points on its own. Closing this class of gap for good would mean
///    either updating the pinned rustc toolchain or deriving the
///    `Alphabetic` check directly from UCD data instead of
///    `char::is_alphabetic` — both out of scope for this table. Left as
///    a known, open-ended limitation whose size tracks how far the
///    pinned toolchain lags the newest Unicode data, unlike the closed,
///    version-independent `Nd`/punctuation/`Mc` categories above.
///
///    Separately from version drift, a residual is deliberately left
///    uncovered regardless of toolchain version: explicit `Bidi_Class=L`
///    code points outside `Nd`/punctuation/`Mc` whose real
///    `General_Category` is `So`/`No` (symbols and other numbers). This
///    category is not a clean "symbols, not script text" line the way it
///    might sound — Ethiopic's entire digit series (U+1369..U+137C
///    ETHIOPIC DIGIT ONE..ETHIOPIC NUMBER TEN THOUSAND) is `No`, not `Nd`,
///    because Ethiopic historically has no zero and its "digits" are
///    additive numeral signs rather than positional digits; smaller
///    numeral-adjacent `No` sets exist for Tamil, Bengali, Oriya,
///    Malayalam, Sinhala, and Tibetan half-integers alongside the Braille
///    patterns, circled/parenthesized digits, and squared/circled CJK
///    compatibility symbols that are genuinely symbol-like. Also left out:
///    a handful of `Cf` format characters and `Sk` modifier symbols with
///    real `Bidi_Class=L` that are not `Alphabetic=Yes` (e.g. U+110BD
///    KAITHI NUMBER SIGN / U+110CD KAITHI NUMBER SIGN ABOVE,
///    U+A789..U+A78A MODIFIER LETTER COLON..MODIFIER LETTER SHORT EQUALS
///    SIGN), and the three Private Use Area blocks (`Co`, two of them
///    65534 code points wide, one — the BMP Private Use Area — 6400),
///    which also carry an explicit `Bidi_Class=L` entry each. All of these
///    are excluded by the same category check as the rest of the residual,
///    not specially cased. Unlike the `Mc` viramas this table does cover,
///    the `So`/`No` residual has no single derivation rule that reliably
///    separates "numeral sign genuinely likely to open real text" from
///    "compatibility symbol essentially never seen as the first character
///    of a sentence" — chasing it further would mean enumerating the
///    residual case by case rather than by a `General_Category` rule,
///    which this function's tables otherwise avoid doing. Left as a known,
///    narrowly-scoped gap: it requires one of these residual code points
///    to itself be the text's true first strong character ahead of an
///    unrelated `AL`/`R` character elsewhere in the same string.
///    (Separately, and much larger again: `DerivedBidiClass.txt` also
///    carries a single `@missing: 0000..10FFFF; Left_To_Right` line — the
///    default `Bidi_Class` for every *unassigned* code point in the entire
///    codespace, dwarfing every table on this page combined. That default
///    plays no part in this table's derivation: like
///    `NON_STRONG_ALPHABETIC_RANGES` above, this table only draws from
///    explicit, non-`@missing` per-code-point entries — unlike
///    `AL_RANGES`/`R_RANGES`, which do transcribe `@missing` lines, but
///    only the narrower, per-script-block ones, never this codespace-wide
///    one.)
pub(crate) fn strong_bidi_type(c: char) -> Option<StrongBidiType> {
    let cp = c as u32;
    // U+200E LEFT-TO-RIGHT MARK and U+200F RIGHT-TO-LEFT MARK are explicit
    // per-code-point `DerivedBidiClass.txt` entries (`Cf` General
    // Punctuation, `; L` and `; R` respectively) outside every range in
    // `AL_RANGES`/`R_RANGES` below — see this function's `# Scope` doc
    // section for the full 3-code-point picture including U+061C ARABIC
    // LETTER MARK (which needs no arm here: already `AL_RANGES`-covered).
    // Checked first, before either range table, so neither falls through to
    // the `is_alphabetic` branch below — both are format characters
    // (`is_alphabetic() == false`), which would otherwise return `None` for
    // both instead of their real strong type.
    match cp {
        0x200E => return Some(StrongBidiType::L),
        0x200F => return Some(StrongBidiType::R),
        _ => {}
    }
    if NON_STRONG_WITHIN_AL_R_RANGES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&cp))
    {
        return None;
    }
    if AL_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
        return Some(StrongBidiType::Al);
    }
    if R_RANGES.iter().any(|&(lo, hi)| (lo..=hi).contains(&cp)) {
        return Some(StrongBidiType::R);
    }
    if NON_STRONG_ALPHABETIC_RANGES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&cp))
    {
        return None;
    }
    if c.is_alphabetic() {
        return Some(StrongBidiType::L);
    }
    if STRONG_L_NON_ALPHABETIC_RANGES
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&cp))
    {
        return Some(StrongBidiType::L);
    }
    None
}

// cov:ignore: pure test-code relocation (no logic changed). patch-coverage's git-diff-based line classifier treats every moved
// line as newly added, and cargo-llvm-cov does not record hits for
// multi-line string-literal continuation lines inside assert!/panic!
// messages even though the containing statement executes in a
// passing test. Verified against every flagged line in this move:
// all are string-literal fragments or trivial format-arg
// expressions inside already-passing tests, none of them
// production code.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::cascade;
    use crate::cascade::test_support::*;
    use crate::ruletree::build_rule_tree;
    use crate::test_dom::TestDoc;

    #[test]
    fn dir_matches_explicit_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
        let p = doc.push_element(0, "p", None);
        doc.set_attr(p, "dir", "ltr");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
    }

    #[test]
    fn dir_matches_explicit_rtl_and_inherits_to_descendant_without_own_dir() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(rtl) { font-family: rtl-font }");
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None); // no dir of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[article].font_family[0].to_string(),
            "rtl-font",
            ":dir(rtl) must match the element with the explicit attribute"
        );
        assert_eq!(
            r.computed[span].font_family[0].to_string(),
            "rtl-font",
            ":dir(rtl) must match a descendant with no dir attribute of its \
             own, inheriting from its dir=\"rtl\" ancestor"
        );
    }

    #[test]
    fn dir_matches_explicit_ltr_and_inherits_to_descendant_without_own_dir() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let section = doc.push_element(0, "section", None);
        doc.set_attr(section, "dir", "rtl"); // grandparent, see doc above
        let article = doc.push_element(section, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None); // no dir of its own

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            ":dir(ltr) must match a descendant with no dir attribute of its \
             own, inheriting from its dir=\"ltr\" ancestor rather than \
             continuing past it to the dir=\"rtl\" grandparent"
        );
    }

    #[test]
    fn dir_defaults_to_ltr_when_no_dir_attribute_anywhere() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { font-family: rtl-font } :dir(ltr) { font-family: ltr-font }",
        );
        let p = doc.push_element(0, "p", None);

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].font_family[0].to_string(), "ltr-font");
    }

    #[test]
    fn dir_auto_scans_own_text_ltr_first_strong_overrides_rtl_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // "Hello" is wrapped in an ordinary (non-excluded) nested <b>, not a
        // direct text child of `span` — exercises the recursive descent
        // into an un-excluded element subtree in
        // `auto_text_scan_subtree`, not just the direct-text-child case.
        let bold = doc.push_element(span, "b", None);
        doc.push_text(bold, "Hello"); // first strong character 'H' is type L

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[article].background_color, RED,
            ":dir(rtl) must still match the article's own explicit dir=\"rtl\""
        );
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "dir=\"auto\" must resolve via the element's own text scan (first \
             strong character 'H' is type L → ltr), not by inheriting the \
             rtl ancestor's directionality"
        );
    }

    #[test]
    fn dir_auto_with_arabic_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}"); // "السلام"

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is Arabic (type AL) → rtl"
        );
    }

    #[test]
    fn dir_auto_with_hebrew_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{05E9}\u{05DC}\u{05D5}\u{05DD}"); // "שלום"

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is Hebrew (type R) → rtl"
        );
    }

    #[test]
    fn dir_auto_with_hebrew_presentation_forms_text_resolves_rtl_despite_ltr_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "\u{FB1D}"); // HEBREW LETTER YOD WITH HIRIQ

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "Hebrew presentation forms character is type R → rtl"
        );
    }

    #[test]
    fn dir_auto_with_leading_ltr_mark_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+200E LEFT-TO-RIGHT MARK, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{200E}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "first strong character is U+200E (type L) → ltr, despite \
             unrelated Arabic text right after it"
        );
    }

    #[test]
    fn dir_auto_with_leading_rtl_mark_before_latin_text_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+200F RIGHT-TO-LEFT MARK, then unrelated Latin (type L) text.
        doc.push_text(span, "\u{200F}Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "first strong character is U+200F (type R) → rtl, despite \
             unrelated Latin text right after it"
        );
    }

    #[test]
    fn dir_auto_skips_non_strong_alphabetic_combining_mark_before_hebrew_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0941 DEVANAGARI VOWEL SIGN U, then "שלום" (Hebrew, type R).
        doc.push_text(span, "\u{0941}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "leading non-strong combining mark must be skipped (not \
             misclassified as strong L) so the scan reaches the Hebrew \
             text and resolves rtl"
        );
    }

    #[test]
    fn dir_auto_skips_non_strong_alphabetic_modifier_letter_before_arabic_resolves_rtl() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "ltr");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+02C6 MODIFIER LETTER CIRCUMFLEX ACCENT, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{02C6}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, BLUE);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "leading non-strong alphabetic modifier letter must be skipped \
             so the scan reaches the Arabic text and resolves rtl"
        );
    }

    #[test]
    fn dir_auto_skips_arabic_indic_digit_before_latin_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0664 ARABIC-INDIC DIGIT FOUR, then Latin "H".
        doc.push_text(span, "\u{0664}H");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Arabic-Indic digit (weak AN, not strong AL) must be \
             skipped so the scan reaches the Latin text and resolves ltr"
        );
    }

    #[test]
    fn dir_auto_with_devanagari_digit_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+0966 DEVANAGARI DIGIT ZERO, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{0966}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Devanagari digit (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall through \
             to the later Arabic text"
        );
    }

    #[test]
    fn dir_auto_with_armenian_punctuation_before_hebrew_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+055A ARMENIAN APOSTROPHE, then "שלום" (Hebrew, type R).
        doc.push_text(span, "\u{055A}\u{05E9}\u{05DC}\u{05D5}\u{05DD}");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Armenian punctuation (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall \
             through to the later Hebrew text"
        );
    }

    #[test]
    fn dir_auto_with_balinese_virama_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+1B44 BALINESE ADEG ADEG, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{1B44}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Balinese virama (strong L, despite not being \
             is_alphabetic()) must resolve ltr on its own, not fall \
             through to the later Arabic text"
        );
    }

    #[test]
    fn dir_auto_with_sharada_vowel_sign_ooe_before_arabic_text_resolves_ltr() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        // U+11B61 SHARADA VOWEL SIGN OOE, then "السلام" (Arabic, type AL).
        doc.push_text(
            span,
            "\u{11B61}\u{0627}\u{0644}\u{0633}\u{0644}\u{0627}\u{0645}",
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "leading Sharada vowel sign (strong L, despite not being \
             is_alphabetic() under this workspace's pinned rustc) must \
             resolve ltr on its own, not fall through to the later Arabic \
             text"
        );
    }

    #[test]
    fn strong_bidi_range_tables_preserve_classification_invariants() {
        fn overlaps(a: (u32, u32), b: (u32, u32)) -> bool {
            a.0 <= b.1 && b.0 <= a.1
        }

        fn assert_table_is_internally_disjoint(name: &str, table: &[(u32, u32)]) {
            for (index, &left) in table.iter().enumerate() {
                assert!(left.0 <= left.1, "{name}[{index}] has an inverted range");
                for (other_index, &right) in table.iter().enumerate().skip(index + 1) {
                    // cov:ignore: the failure-message branch of this `assert!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert!(
                        !overlaps(left, right),
                        "{name}[{index}] {left:#x?} overlaps {name}[{other_index}] {right:#x?}"
                    );
                }
            }
        }

        fn assert_tables_are_disjoint(
            left_name: &str,
            left: &[(u32, u32)],
            right_name: &str,
            right: &[(u32, u32)],
        ) {
            for (left_index, &left_range) in left.iter().enumerate() {
                for (right_index, &right_range) in right.iter().enumerate() {
                    // cov:ignore: the failure-message branch of this `assert!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert!(
                        !overlaps(left_range, right_range),
                        "{left_name}[{left_index}] {left_range:#x?} overlaps \
                         {right_name}[{right_index}] {right_range:#x?}"
                    );
                }
            }
        }

        let tables = [
            ("AL_RANGES", AL_RANGES),
            ("R_RANGES", R_RANGES),
            (
                "NON_STRONG_WITHIN_AL_R_RANGES",
                NON_STRONG_WITHIN_AL_R_RANGES,
            ),
            ("NON_STRONG_ALPHABETIC_RANGES", NON_STRONG_ALPHABETIC_RANGES),
            (
                "STRONG_L_NON_ALPHABETIC_RANGES",
                STRONG_L_NON_ALPHABETIC_RANGES,
            ),
        ];

        for &(name, table) in &tables {
            assert_table_is_internally_disjoint(name, table);
        }
        assert_tables_are_disjoint("AL_RANGES", AL_RANGES, "R_RANGES", R_RANGES);
        assert_tables_are_disjoint(
            "AL_RANGES",
            AL_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "R_RANGES",
            R_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "AL_RANGES",
            AL_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "R_RANGES",
            R_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_WITHIN_AL_R_RANGES",
            NON_STRONG_WITHIN_AL_R_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );
        assert_tables_are_disjoint(
            "NON_STRONG_ALPHABETIC_RANGES",
            NON_STRONG_ALPHABETIC_RANGES,
            "STRONG_L_NON_ALPHABETIC_RANGES",
            STRONG_L_NON_ALPHABETIC_RANGES,
        );

        for &(lo, hi) in NON_STRONG_WITHIN_AL_R_RANGES {
            // cov:ignore: the failure-message branch of this `assert!` only
            // executes for a deliberately broken generated const table;
            // constructing one here would test the fixture instead of the
            // invariant, while the checked-in table is immutable.
            assert!(
                AL_RANGES
                    .iter()
                    .chain(R_RANGES)
                    .any(|&outer| outer.0 <= lo && hi <= outer.1),
                "non-strong exception {lo:#x}..={hi:#x} is outside AL/R defaults"
            );
        }

        for &(lo, hi) in NON_STRONG_ALPHABETIC_RANGES {
            for cp in lo..=hi {
                let character = char::from_u32(cp).expect("range is a Unicode scalar");
                let resolved = strong_bidi_type(character);
                if character.is_alphabetic() {
                    // cov:ignore: the failure-message branch of this `assert_eq!` only
                    // executes for a deliberately broken generated const table;
                    // constructing one here would test the fixture instead of the
                    // invariant, while the checked-in table is immutable.
                    assert_eq!(
                        resolved, None,
                        "alphabetic NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must be excluded from L"
                    );
                }
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    resolved != Some(StrongBidiType::L),
                    "NON_STRONG_ALPHABETIC_RANGES entry U+{cp:04X} must never resolve as L"
                );
            }
        }
        for &(lo, hi) in STRONG_L_NON_ALPHABETIC_RANGES {
            for cp in lo..=hi {
                let character = char::from_u32(cp).expect("range is a Unicode scalar");
                // cov:ignore: the failure-message branch of this `assert!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert!(
                    !character.is_alphabetic(),
                    "STRONG_L_NON_ALPHABETIC_RANGES contains alphabetic U+{cp:04X}"
                );
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(character),
                    Some(StrongBidiType::L),
                    "STRONG_L_NON_ALPHABETIC_RANGES entry U+{cp:04X} must resolve as L"
                );
            }
        }

        for &(lo, hi) in AL_RANGES {
            for cp in lo..=hi {
                let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                    .iter()
                    .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
                {
                    None
                } else {
                    Some(StrongBidiType::Al)
                };
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                    expected,
                    "AL_RANGES classification changed for U+{cp:04X}"
                );
            }
        }
        for &(lo, hi) in R_RANGES {
            for cp in lo..=hi {
                let expected = if NON_STRONG_WITHIN_AL_R_RANGES
                    .iter()
                    .any(|&(excluded_lo, excluded_hi)| (excluded_lo..=excluded_hi).contains(&cp))
                {
                    None
                } else {
                    Some(StrongBidiType::R)
                };
                // cov:ignore: the failure-message branch of this `assert_eq!` only
                // executes for a deliberately broken generated const table;
                // constructing one here would test the fixture instead of the
                // invariant, while the checked-in table is immutable.
                assert_eq!(
                    strong_bidi_type(char::from_u32(cp).expect("range is a Unicode scalar")),
                    expected,
                    "R_RANGES classification changed for U+{cp:04X}"
                );
            }
        }
    }

    #[test]
    fn dir_auto_with_no_strong_directional_text_falls_back_to_ltr_despite_rtl_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None);
        doc.set_attr(span, "dir", "auto");
        doc.push_text(span, "123 456!");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[article].background_color, RED);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, BLUE,
            "no strong L/AL/R character anywhere → 'ltr' fallback, not the \
             rtl ancestor's directionality"
        );
    }

    #[test]
    fn dir_undefined_still_falls_through_to_ancestor_via_background_color() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let article = doc.push_element(0, "article", None);
        doc.set_attr(article, "dir", "rtl");
        let span = doc.push_element(article, "span", None); // no dir attribute at all

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].background_color, RED,
            "missing dir attribute must still fall through to the ancestor's \
             directionality"
        );
    }

    #[test]
    fn auto_directionality_skips_descendant_with_own_dir_attribute() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let inner = doc.push_element(outer, "span", None);
        doc.set_attr(inner, "dir", "rtl");
        doc.push_text(inner, "\u{0627}"); // Arabic alef — must be skipped
        doc.push_text(outer, "Hello"); // outer's own trailing text

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "the dir=\"rtl\" descendant's text must be excluded from the \
             outer element's own auto-directionality scan"
        );
    }

    #[test]
    fn auto_directionality_skips_bdi_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let bdi = doc.push_element(outer, "bdi", None); // no dir attribute
        doc.push_text(bdi, "\u{0627}"); // Arabic alef — must be skipped
        doc.push_text(outer, "Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "a bdi descendant's text must be excluded from the outer \
             element's own auto-directionality scan, regardless of its own \
             dir state"
        );
    }

    #[test]
    fn auto_directionality_skips_script_and_style_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let nested_script = doc.push_element(outer, "script", None);
        doc.push_text(nested_script, "\u{0627}");
        let nested_style = doc.push_element(outer, "style", None);
        doc.push_text(nested_style, "\u{0627}");
        doc.push_text(outer, "Hello");

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "script/style descendant text must be excluded from the outer \
             element's own auto-directionality scan"
        );
    }

    #[test]
    fn auto_directionality_scans_into_foreign_namespace_descendant_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        let svg_text =
            doc.push_element_with_namespace(outer, "text", "http://www.w3.org/2000/svg", &[]);
        doc.push_text(svg_text, "\u{0627}"); // Arabic alef

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, RED,
            "a foreign-namespace descendant's text must still be scanned, \
             not excluded"
        );
    }

    #[test]
    fn auto_directionality_ignores_comment_node_text() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(
            s,
            ":dir(rtl) { background-color: red } :dir(ltr) { background-color: blue }",
        );
        let outer = doc.push_element(0, "div", None);
        doc.set_attr(outer, "dir", "auto");
        doc.push_comment(outer, "\u{0627}"); // Arabic alef inside a comment

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[outer].background_color, BLUE,
            "a comment node's text must never be scanned; with no other \
             text present this must resolve via the 'ltr' no-strong-\
             character fallback"
        );
    }

    #[test]
    fn dir_attribute_on_foreign_namespace_element_is_ignored_falls_through_to_ancestor() {
        let mut doc = TestDoc::new();
        let s = doc.push_element(0, "style", None);
        doc.push_text(s, ":dir(ltr) { font-family: ltr-font }");
        let html = doc.push_element(0, "html", None); // no dir -> default ltr
        let svg = doc.push_element_with_namespace(
            html,
            "svg",
            "http://www.w3.org/2000/svg",
            &[("dir", "rtl")],
        );

        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(
            r.computed[svg].font_family[0].to_string(),
            "ltr-font",
            "dir on a foreign-namespace element must be ignored, not treated \
             as an explicit directionality"
        );
    }

    #[test]
    fn direction_wired_through_cascade_from_inline_style() {
        use crate::property::Direction;
        let cv = cascade_doc("", "p", Some("direction: rtl"));
        assert_eq!(cv.direction, Direction::Rtl);
    }

    #[test]
    fn direction_inherits_from_parent_element() {
        // CSS Writing Modes 4 §2.1: direction は **inherited**.
        use crate::property::Direction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl"));
        let span = doc.push_element(p, "span", None);
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Rtl);
        // cov:ignore: panic-message literal only executed on assertion
        // failure, which doesn't happen while this test passes.
        assert_eq!(
            r.computed[span].direction,
            Direction::Rtl,
            "child should inherit direction from parent (CSS Writing Modes 4 §2.1 Inherited: yes)"
        );
    }

    #[test]
    fn direction_child_own_value_wins_over_inherited() {
        use crate::property::Direction;
        let mut doc = TestDoc::new();
        let p = doc.push_element(0, "p", Some("direction: rtl"));
        let span = doc.push_element(p, "span", Some("direction: ltr"));
        let tree = build_rule_tree(&doc);
        let r = cascade(&doc, &tree).expect("cascade Ok");
        assert_eq!(r.computed[p].direction, Direction::Rtl);
        assert_eq!(r.computed[span].direction, Direction::Ltr);
    }
}
