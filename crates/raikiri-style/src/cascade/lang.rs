use crate::style_dom::{StyleDom, StyleElement, StyleNode, StyleNodeId};

/// `PseudoClass::Lang` arm of [`compound_matches`] — CSS Selectors L4 §7.2
/// <https://www.w3.org/TR/selectors-4/#the-lang-pseudo>: "represents an
/// element whose content language is one of the languages listed in its
/// argument" (bikeshed source verbatim, see [`language_range_matches`] doc
/// for the fetch note). `ranges` is empty-or-more per [`crate::PseudoClass::Lang`]
/// grammar (`parse_comma_separated` never actually returns an empty `Vec`
/// for a non-empty `:lang(...)` argument list, but this function does not
/// special-case emptiness — `ranges.iter().any(..)` is vacuously `false` on
/// an empty slice, the same "never matches" outcome an empty argument list
/// should have, so no explicit guard is needed even if that upstream
/// guarantee ever changes).
///
/// [`effective_language`] always resolves to a concrete (possibly empty)
/// content language string — HTML LS §3.2.6.2's "determine the language of
/// a node" algorithm is a total function (its own final "Otherwise" step is
/// exactly [`effective_language`]'s fallback; see that function's doc), so
/// there is no "unresolvable language" case to handle here.
///
/// The empty string is a narrower case than a non-empty content language:
/// it does not match a bare wildcard range, but it does match other ranges, notably the
/// literal empty-string range `:lang("")`. CSS Selectors L4 §7.2, bikeshed
/// source `selectors-4/Overview.bs` `#the-lang-pseudo` (direct raw fetch of
/// `raw.githubusercontent.com/w3c/csswg-drafts/main/selectors-4/Overview.bs`,
/// bypassing WebFetch's truncation on this TR page the same way
/// [`matches_empty`]'s `:empty` doc note does), verbatim: "For this
/// purpose, a wildcard language range (\"*\") does not match elements
/// whose language is not tagged (e.g. `lang=\"\"`), but does match elements
/// whose language is tagged as undetermined (`lang=und`). A language range
/// consisting of an empty string (`:lang(\"\")`) matches (only) elements
/// whose language is not tagged." [`language_range_matches`] itself already
/// enforces both halves of this quote directly: it special-cases an empty
/// `content_language` (needed for `:lang("")`, which is a CSS-level
/// construct rather than a well-formed BCP47 range) by requiring exact
/// string equality against `range`, which yields `false` for a bare `*`
/// range against an empty `content_language` and `true` for `:lang("")`
/// against one — so this function needs no special case of its own and
/// simply delegates every range to it.
pub(crate) fn lang_pseudo_matches<D: StyleDom, E: StyleElement>(
    ranges: &[String],
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> bool {
    let lang = effective_language(dom, elem, ancestors);
    ranges
        .iter()
        .any(|range| language_range_matches(range, &lang))
}

/// Resolves an element's **content language** per HTML Living Standard
/// §3.2.6.2 "The `lang` and `xml:lang` attributes"
/// (<https://html.spec.whatwg.org/multipage/dom.html#the-lang-and-xml:lang-attributes>),
/// simplified to the subset of "determine the
/// language of a node" this crate can express:
///
/// > To determine the language of a node, user agents must use the first
/// > appropriate step in the following list: \[...\] If the node is an HTML
/// > element or an element in the SVG namespace, and it has a lang in no
/// > namespace attribute set — Use the value of that attribute. \[...\] If
/// > the node's parent element is not null — Use the language of that
/// > parent element. Otherwise \[...\] the language of the node is unknown,
/// > and the corresponding language tag is the empty string.
///
/// i.e. own `lang` attribute wins; absent, walk up to the nearest ancestor
/// that has one; absent everywhere (no pragma-set default / protocol-level
/// language either, both out of scope — this crate has no HTTP layer and
/// does not parse `<meta http-equiv=content-language>`), the language is
/// unknown — represented, per the quoted "the corresponding language tag is
/// the empty string" fallback, as `String::new()` (see the "`lang=\"\"`
/// stopping inheritance" section below for how this converges with the
/// explicit-`lang=\"\"` case).
///
/// Like [`own_explicit_direction`]'s `dir` reads, the **own**-attribute
/// step gates on `elem.namespace_uri()` — but a 2-element allowlist (HTML
/// *or* SVG) rather than `dir`'s HTML-only 1-element one, per the quoted
/// step's explicit "an HTML element or an element in the SVG namespace"
/// wording (an earlier version of this
/// function read `lang` unconditionally, which is wrong for any other
/// foreign-namespace element — MathML concretely: `<math lang="ja">` nested
/// under `<html lang="en">` must resolve to `"en"`, not `"ja"`, since MathML
/// is neither HTML nor SVG. [`own_html_or_svg_lang_attribute`] is the gate;
/// see its doc for the allowlist). This only restricts *whose own*
/// attribute counts — the ancestor walk below still applies the same gate
/// per ancestor (a MathML ancestor's `lang` is skipped too, same as its own
/// element case), and a chain that bottoms out with no HTML/SVG element
/// carrying `lang` still resolves to `String::new()`, same as "absent
/// everywhere" below.
///
/// # Deliberately out of scope
///
/// - **`xml:lang` (XML-namespace `lang`)** — the first step in the quoted
///   list, and it *would* take priority over the plain `lang` attribute.
///   Skipped because raikiri does not parse XML/XHTML documents at all yet
///   (`StyleDom::quirks_mode` doc / `resolve_case_sensitivity` doc: "raikiri
///   は現時点で HTML document のみ対象") — there is no XML-namespace
///   attribute surface to read.
///
/// # `lang=""` stopping inheritance
///
/// Per the quoted algorithm, an empty-string `lang` attribute is itself a
/// *found* value ("the primary language is unknown", a distinct terminal
/// state from "no `lang` attribute at all", which keeps walking to the
/// parent). [`StyleElement::attr`] tracks attribute presence independent of
/// value, so [`own_html_or_svg_lang_attribute`] observes an explicit
/// `lang=""` as `Some("")`, not `None` — the `if let Some(lang) = ...`
/// branch below returns immediately for that case (yielding the empty
/// string) rather than falling through to the ancestor walk, matching the
/// quoted algorithm's step order. Callers must still treat this returned
/// empty string as "no content language" for their own purposes if that is
/// what they need (CSS Selectors L4's `:lang()` does — see
/// [`lang_pseudo_matches`]'s doc); [`effective_language`] itself only
/// resolves the language per HTML LS's algorithm, it does not decide what
/// an empty result means to a particular consumer.
///
/// The ancestor-chain-exhausted terminal case below (no `lang` found
/// anywhere) converges on this same empty-string representation, per the
/// quoted algorithm's own final "the corresponding language tag is the
/// empty string" fallback — even though it is reached via a different step
/// (running out of ancestors, not an explicit `lang=""` short-circuit).
fn effective_language<D: StyleDom, E: StyleElement>(
    dom: &D,
    elem: &E,
    ancestors: &[StyleNodeId],
) -> String {
    if let Some(lang) = own_html_or_svg_lang_attribute(elem) {
        return lang.to_owned();
    }
    for &ancestor_id in ancestors.iter().rev() {
        // `node`'s borrow must outlive `ancestor_elem`'s — a `.and_then`
        // chain would try to return a `&str` borrowed from a `node` that
        // drops at the end of the closure, hence the explicit `if let`
        // nesting instead of the more compact combinator chain
        // `effective_language`'s own doc-adjacent sibling functions use
        // where the borrow doesn't need to cross a temporary like this.
        if let Some(node) = dom.node(ancestor_id)
            && let Some(ancestor_elem) = node.as_element()
            && let Some(lang) = own_html_or_svg_lang_attribute(&ancestor_elem)
        {
            return lang.to_owned();
        }
    }
    // Ancestor chain exhausted with no pragma-set default / protocol-level
    // language available (both out of scope, see this function's doc).
    // HTML LS §3.2.6.2's final fallback: "the language of the node is
    // unknown, and the corresponding language tag is the empty string."
    String::new()
}

/// The **own**-attribute half of HTML LS §3.2.6.2's "determine the language
/// of a node" step (quoted in full on [`effective_language`]'s doc), gated
/// to HTML-namespace (`elem.namespace_uri() == None`, this crate's
/// established "is HTML" proxy — see [`own_explicit_direction`]'s doc) or
/// SVG-namespace elements. Any other namespace (MathML concretely, but the
/// gate is namespace-allowlist shaped so it excludes any future foreign
/// namespace equally) returns `None` regardless of whether `lang` is
/// present on the element, so [`effective_language`]'s caller falls through
/// to the ancestor walk exactly as if `lang` were absent — matching the
/// quoted algorithm's own next step ("If the node's parent element is not
/// null — Use the language of that parent element").
fn own_html_or_svg_lang_attribute<E: StyleElement>(elem: &E) -> Option<&str> {
    match elem.namespace_uri() {
        None | Some("http://www.w3.org/2000/svg") => elem.attr("lang"),
        Some(_) => None,
    }
}

/// Does `range` (one comma-separated argument of `:lang(...)`) match
/// `content_language` (the resolved [`effective_language`])? Implements RFC
/// 4647 §3.3.2 "Extended Filtering"
/// (<https://www.rfc-editor.org/rfc/rfc4647.html#section-3.3.2>), which CSS Selectors L4 §7.2 cites verbatim (bikeshed
/// source `selectors-4/Overview.bs`, same fetch as [`Direction`]'s doc —
/// the published TR page truncated before §7.2 for this crate's WebFetch
/// tool):
///
/// > The element's content language matches a language range if its content
/// > language, as represented in BCP 47 syntax, matches the given language
/// > range in an extended filtering operation per \[RFC4647\] (section
/// > 3.3.2).
///
/// RFC 4647 §3.3.2 verbatim (direct fetch of `rfc-editor.org`'s plain-text
/// rendering):
///
/// > 1. Split both the extended language range and the language tag being
/// >    compared into a list of subtags by dividing on the hyphen (%x2D)
/// >    character. Two subtags match if either they are the same when
/// >    compared case-insensitively or the language range's subtag is the
/// >    wildcard '*'.
/// > 2. Begin with the first subtag in each list. If the first subtag in
/// >    the range does not match the first subtag in the tag, the overall
/// >    match fails. Otherwise, move to the next subtag in both the range
/// >    and the tag.
/// > 3. While there are more subtags left in the language range's list:
/// >    A. If the subtag currently being examined in the range is the
/// >       wildcard ('*'), move to the next subtag in the range and
/// >       continue with the loop.
/// >    B. Else, if there are no more subtags in the language tag's list,
/// >       the match fails.
/// >    C. Else, if the current subtag in the range's list matches the
/// >       current subtag in the language tag's list, move to the next
/// >       subtag in both lists and continue with the loop.
/// >    D. Else, if the language tag's subtag is a "singleton" (a single
/// >       letter or digit, which includes the private-use subtag 'x') the
/// >       match fails.
/// >    E. Else, move to the next subtag in the language tag's list and
/// >       continue with the loop.
/// > 4. When the language range's list has no more subtags, the match
/// >    succeeds.
///
/// This function implements exactly the above (`subtags_match` = step 1's
/// per-subtag comparator). Per the CSS quote above, "the matching is
/// performed ASCII case-insensitively", which is exactly RFC4647's own
/// per-subtag rule — no separate case-folding pass needed.
///
/// # BCP47 well-formedness and canonicalization
///
/// CSS Selectors L4 §7.2 additionally requires (bikeshed source, same fetch
/// as above):
///
/// > The \[content language\] and the \[language range\] must be
/// > canonicalized and converted to extlang form as per section 4.5 of
/// > \[RFC5646\] prior to the extended filtering operation; language tags or
/// > ranges that are not valid do not match anything. \[...\] The language
/// > range must be an extended language range according to BCP47. Language
/// > ranges that are not well-formed language tags or which would not be a
/// > well-formed language tag if an initial wildcard character "\*" were
/// > replaced with a valid subtag, do not match anything.
///
/// with an example spelling out that `:lang(åå)` "would not match, because
/// it contain\[s\] non-ASCII characters so is ill-formed", while `:lang(qq)`
/// "could match, even though qq is not a registered language code" — i.e.
/// the bar is grammatical **well-formedness** (RFC 5646 §2.1's ABNF), not
/// full **validity** (well-formed *and* every subtag registered in the IANA
/// Language Subtag Registry, RFC5646's own stricter term — `qq` is
/// well-formed but not valid, and the spec's own example says it can still
/// match).
///
/// [`is_well_formed_language_tag`] and [`is_well_formed_extended_language_range`]
/// implement well-formedness; [`canonicalize_primary_language_subtag`]
/// implements the canonicalization step that matters for matching
/// correctness (deprecated-subtag replacement). Both are applied to `range`
/// and `content_language` before the extended-filtering algorithm below
/// runs. RFC 5646 §4.5 canonicalization (deprecated subtag -> registry
/// `Preferred-Value`) is a genuine *false-negative* source: without it,
/// `language_range_matches("he", "iw")` returns `false` even though `iw` is
/// the deprecated form of `he` and a conformant UA must match `:lang(he)`
/// against `lang="iw"`.
///
/// ## Implemented subset
///
/// - **Well-formedness** is checked per-subtag against the shared
///   length/charset envelope every RFC 5646 §2.1 subtag production other
///   than the primary language subtag reduces to (1 to 8 ASCII letters or
///   digits — see [`is_well_formed_alphanum_subtag`]'s doc for the
///   derivation across `extlang`/`script`/`region`/`variant`/`extension`/
///   `privateuse`), plus a stricter first-subtag check for the `langtag`
///   alternative (ASCII alpha only, length bounds per position) and a
///   separate arm for the top-level `privateuse` alternative used alone
///   (`x-foo`) — see [`is_well_formed_language_tag`]'s doc for both. This is
///   **not** a full position-tracking walk of the `langtag` production:
///   subtag *sequencing* is not validated. Concretely, this implementation
///   does not detect `en-DE-Latn` (region before script), `en-Latn-Cyrl`
///   (two script-shaped subtags), `en-ab1` (a digit where only an
///   extlang/alpha subtag could legally appear), or a bare extension
///   singleton with no following value subtag (`en-a`) as ill-formed — each
///   is accepted because every individual subtag has *some* legal shape,
///   even though the sequence as a whole does not parse under the `langtag`
///   production. What this check *does* still reject beyond the shape
///   envelope: any `langtag`-shaped tag whose first subtag is alpha but
///   shorter than 2 characters — in practice this means 13 of RFC 5646's 26
///   fixed `irregular`/`regular` grandfathered tags (17 `irregular` + 9
///   `regular`) — the `i-*` ones (`i-klingon`, `i-navajo`, ...), whose
///   leading `i` subtag is 1 character. The remaining grandfathered tags
///   (`art-lojban`, `cel-gaulish`, `no-bok`, `no-nyn`, `zh-guoyu`,
///   `zh-hakka`, `zh-min`, `zh-min-nan`, `zh-xiang`, `en-GB-oed`,
///   `sgn-BE-FR`, `sgn-BE-NL`, `sgn-CH-DE`) all have a 2-or-more-character
///   alpha first subtag and so pass this check as ordinary well-formed
///   tags — their special, registration-defined meaning is not recognized
///   (see the canonicalization bullet below for the consequence of that).
/// - **Canonicalization** implements RFC 5646 §4.5 step 3 ("Subtags are
///   replaced by their 'Preferred-Value'"), restricted to the primary
///   language subtag and to a documented table
///   ([`DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS`]) rather than the full IANA
///   registry. Not implemented: extension-subtag reordering (§4.5 step 1),
///   grandfathered/redundant-tag replacement (§4.5 step 2 — the observable
///   consequence, given the well-formedness bullet above: `:lang(hak)`
///   does not match `lang="zh-hakka"`, and `:lang(nb)` does not match
///   `lang="no-bok"`, even though both grandfathered tags pass
///   well-formedness), and the extlang-form `Prefix`-restoration step
///   (needed only when a *primary* language subtag is itself a deprecated
///   extlang subtag — none of this table's entries are).
///
/// A full subtag registry / grammar checker (matching the scope the
/// Unicode Bidi Algorithm gets for `:dir()`'s `auto` value) remains a
/// substantial undertaking beyond this documented subset.
pub(crate) fn language_range_matches(range: &str, content_language: &str) -> bool {
    fn subtags_match(range_subtag: &str, tag_subtag: &str) -> bool {
        range_subtag == "*" || range_subtag.eq_ignore_ascii_case(tag_subtag)
    }
    fn is_singleton(subtag: &str) -> bool {
        subtag.chars().count() == 1
    }

    // `:lang("")` and an untagged content language (HTML LS §3.2.6.2's own
    // "the corresponding language tag is the empty string" fallback,
    // quoted on `effective_language`'s doc) are CSS-level constructs, not
    // BCP47 language tags/ranges — RFC 5646/4647 well-formedness and
    // canonicalization do not apply to either side here. Selectors L4 §7.2
    // (quoted in full on `lang_pseudo_matches`'s doc) instead gives them
    // its own equality rule directly: "A language range consisting of an
    // empty string matches (only) elements whose language is not tagged."
    // This equality check also subsumes "a wildcard language range does
    // not match elements whose language is not tagged" for every range
    // (not just a literal `*`) once `content_language` is empty, since no
    // non-empty range string can equal the empty string.
    if range.is_empty() || content_language.is_empty() {
        return range == content_language;
    }

    let range_subtags: Vec<&str> = range.split('-').collect();
    let tag_subtags: Vec<&str> = content_language.split('-').collect();

    // Selectors L4 §7.2 (quoted above): ill-formed tags/ranges never match.
    if !is_well_formed_extended_language_range(&range_subtags)
        || !is_well_formed_language_tag(&tag_subtags)
    {
        return false;
    }

    // Selectors L4 §7.2 (quoted above): canonicalize before extended
    // filtering. `subtags_match`/the loop below only ever read these
    // through `.eq_ignore_ascii_case`/`== "*"`, so owning `String`s here
    // (needed to overwrite the primary language subtag in place) costs
    // nothing but an allocation per subtag, on a selector-matching path
    // this crate does not treat as hot.
    let mut range_subtags: Vec<String> = range_subtags.into_iter().map(String::from).collect();
    let mut tag_subtags: Vec<String> = tag_subtags.into_iter().map(String::from).collect();
    canonicalize_primary_language_subtag(&mut range_subtags);
    canonicalize_primary_language_subtag(&mut tag_subtags);

    // Step 2: first subtag must match (range's first subtag may itself be
    // `*`, e.g. the bare wildcard range `:lang(*)` — `subtags_match` already
    // handles that).
    if !subtags_match(&range_subtags[0], &tag_subtags[0]) {
        return false;
    }
    let mut ri = 1;
    let mut ti = 1;

    // Step 3.
    while ri < range_subtags.len() {
        let r = &range_subtags[ri];
        if r == "*" {
            ri += 1; // 3.A
            continue;
        }
        let Some(t) = tag_subtags.get(ti) else {
            return false; // 3.B
        };
        if subtags_match(r, t) {
            ri += 1;
            ti += 1; // 3.C
            continue;
        }
        if is_singleton(t) {
            return false; // 3.D
        }
        ti += 1; // 3.E
    }
    true // Step 4.
}

/// Is `subtag` well-formed as any RFC 5646 §2.1 subtag production **other
/// than** the primary language subtag? `extlang` = `3ALPHA`, `script` =
/// `4ALPHA`, `region` = `2ALPHA / 3DIGIT`, `variant` = `5*8alphanum /
/// (DIGIT 3alphanum)`, an extension singleton = 1 alphanumeric character,
/// an extension value subtag = `2*8alphanum`, and a `privateuse`
/// introducer (`"x"`) or value subtag = `1*8alphanum`. Every one of these
/// productions falls inside the same envelope — 1 to 8 ASCII letters or
/// digits — so rather than tracking which specific production a subtag
/// belongs to (a full position-tracking grammar walk, out of scope per
/// [`language_range_matches`]'s doc), this function checks that shared
/// envelope directly.
fn is_well_formed_alphanum_subtag(subtag: &str) -> bool {
    (1..=8).contains(&subtag.len()) && subtag.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Is `subtags` (already split on `-`, guaranteed non-empty by
/// [`language_range_matches`]'s empty-string early return) well-formed as a
/// BCP47 **`Language-Tag`**? RFC 5646 §2.1's top-level production is
/// `Language-Tag = langtag / privateuse / grandfathered`; this function
/// implements the first two alternatives (`grandfathered` is not
/// recognized as its own alternative — see [`language_range_matches`]'s
/// doc for which grandfathered tags this still accepts as ordinary
/// `langtag`s and which it rejects).
///
/// For `langtag`, the primary language subtag (`subtags[0]`) must be pure
/// ASCII alpha, 2 to 8 characters — the union of `language`'s three ABNF
/// alternatives (`2*3ALPHA`, the reserved `4ALPHA`, and `5*8ALPHA`). For the
/// top-level `privateuse` alternative (`"x" 1*("-" (1*8alphanum))`),
/// `subtags[0]` must case-insensitively equal `"x"` and at least one
/// further subtag must be present (the ABNF's `1*`); this is the only
/// signal this function uses to pick between the two alternatives, so it
/// cannot separately model a `langtag`'s own optional trailing
/// `privateuse` extension — that extension's subtags simply pass through
/// [`is_well_formed_alphanum_subtag`] like any other trailing subtag.
/// Every subtag after the first, in either alternative, only needs
/// [`is_well_formed_alphanum_subtag`]'s shared envelope; see that
/// function's doc, and [`language_range_matches`]'s doc for the list of
/// `langtag` subtag *sequences* this does not validate.
fn is_well_formed_language_tag(subtags: &[&str]) -> bool {
    let first = subtags[0];
    let first_ok = if first.eq_ignore_ascii_case("x") {
        subtags.len() >= 2
    } else {
        (2..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic())
    };
    first_ok
        && subtags[1..]
            .iter()
            .all(|s| is_well_formed_alphanum_subtag(s))
}

/// Is `subtags` (already split on `-`, guaranteed non-empty by
/// [`language_range_matches`]'s empty-string early return) well-formed as
/// an RFC 4647 §2.2 "extended language range"?
///
/// > extended-language-range = (1\*8ALPHA / "\*")
/// >                           \*("-" (1\*8alphanum / "\*"))
///
/// The first subtag must be ASCII alpha (1 to 8 characters) or the
/// wildcard `*`; every later subtag must be `*` or satisfy
/// [`is_well_formed_alphanum_subtag`]. This is RFC4647's own range grammar
/// (looser than [`is_well_formed_language_tag`]'s `langtag` production,
/// e.g. it has no per-position script/region/variant distinctions) rather
/// than an implementation of Selectors L4 §7.2's "would not be a
/// well-formed language tag if an initial wildcard \[...\] were replaced
/// with a valid subtag" clause, which would require backtracking over
/// every possible wildcard-to-subtag substitution; see
/// [`language_range_matches`]'s doc for the scope this leaves out.
fn is_well_formed_extended_language_range(subtags: &[&str]) -> bool {
    let first = subtags[0];
    let first_ok = first == "*"
        || ((1..=8).contains(&first.len()) && first.bytes().all(|b| b.is_ascii_alphabetic()));
    first_ok
        && subtags[1..]
            .iter()
            .all(|&s| s == "*" || is_well_formed_alphanum_subtag(s))
}

/// The deprecated **2-letter** (`Type: language`) primary language subtags
/// from the IANA Language Subtag Registry
/// (<https://www.iana.org/assignments/language-subtag-registry/language-subtag-registry>)
/// — every registry record with `Type: language`, a subtag exactly 2
/// letters long, and both a `Deprecated` and a `Preferred-Value` field.
/// This is the complete set of *2-letter* deprecated primary language
/// subtags; the registry additionally lists over 100 deprecated
/// **3-letter** (ISO 639-3) primary language subtags — macrolanguage or
/// orthography mergers such as `ncp` -> `kdz` — which this table
/// deliberately excludes.
const DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS: &[(&str, &str)] = &[
    ("bh", "bih"),
    ("in", "id"),
    ("iw", "he"),
    ("ji", "yi"),
    ("jw", "jv"),
    ("mo", "ro"),
];

/// Canonicalizes `subtags`' primary language subtag (index 0; `subtags` is
/// guaranteed non-empty by [`language_range_matches`]'s empty-string early
/// return) to its registry `Preferred-Value` when it matches (ASCII
/// case-insensitively — subtags are case-insensitive per RFC 5646 §2.1.1)
/// one of [`DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS`], per RFC 5646 §4.5 step 3
/// ("Subtags are replaced by their 'Preferred-Value', if there is one").
/// The replacement is written out in the registry's own lowercase form,
/// which is harmless here because [`language_range_matches`]'s extended
/// filtering comparison is itself ASCII-case-insensitive.
///
/// Only the primary language subtag is ever replaced — this crate's
/// documented subset has no extlang, script, region, variant, extension,
/// or grandfathered/redundant-tag `Preferred-Value` entries, so RFC 5646
/// §4.5's other canonicalization steps are not implemented; see
/// [`language_range_matches`]'s doc for the list.
fn canonicalize_primary_language_subtag(subtags: &mut [String]) {
    for &(deprecated, preferred) in DEPRECATED_PRIMARY_LANGUAGE_SUBTAGS {
        if subtags[0].eq_ignore_ascii_case(deprecated) {
            subtags[0] = preferred.to_string();
            break;
        }
    }
}
