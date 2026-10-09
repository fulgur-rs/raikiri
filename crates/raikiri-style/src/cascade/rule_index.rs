//! Per-cascade rule index used by [`super::collect_cascaded_with_media_context`].
//!
//! Matching every element against every active style rule is quadratic in
//! practice. This module narrows the set of rules tried for one element with
//! two independent, conservative filters, and leaves the actual decision to
//! the ordinary selector matcher:
//!
//! 1. **Buckets keyed by the subject compound.** Each selector of each rule is
//!    filed under exactly one key taken from the compound that must match the
//!    element itself (the rightmost compound, or the originating compound of a
//!    `::before`-style selector): an id if present, else a class, else a type
//!    selector, else the universal bucket. An element only looks at the
//!    buckets for its own id, its own classes, its own tag name, and the
//!    universal bucket.
//! 2. **Ancestor Bloom filter.** While the document walk descends, the tag
//!    name, id, and classes of every element on the current ancestor path are
//!    inserted into a counting Bloom filter. Each selector records the
//!    id/class/type requirements of compounds that must match an *ancestor*
//!    (compounds reached from the subject through child or descendant
//!    combinators only). If any of those hashes is definitely absent from the
//!    filter, the selector cannot match and is skipped.
//!
//! # Correctness contract: superset, never subset
//!
//! Both filters may keep a rule that later fails to match (a false positive),
//! but must never drop a rule that would match. The properties that guarantee
//! this:
//!
//! - Keys and hashes are computed over **ASCII-lowercased** text. Type
//!   selectors match ASCII case-insensitively, and ids/classes match
//!   case-insensitively in quirks mode; folding unconditionally only widens
//!   the candidate set in no-quirks mode.
//! - Keys are hashes, not strings. A hash collision merges two buckets, which
//!   again only adds candidates.
//! - Only simple selectors that appear directly in a compound are used. A
//!   compound matches only if every one of its simple selectors matches, so
//!   each of those is a necessary condition. Selectors nested in `:is()`,
//!   `:where()`, `:not()`, or `:has()` are ignored.
//! - Ancestor requirements stop at the first sibling combinator. A compound
//!   left of `+`/`~` constrains a sibling, not an ancestor.
//! - The Bloom filter mirrors exactly the `ancestor_path` slice that the
//!   matcher uses for child/descendant combinators, and the counting filter's
//!   saturating counters never produce a false "absent".
//!
//! The index is rebuilt on every cascade call. That keeps it correct even if
//! the [`RuleTree`](crate::ruletree::RuleTree) changed after it was parsed, at
//! the cost of one pass over the active rules per cascade.

use std::collections::HashMap;

use selectors::bloom::BloomFilter;
use selectors::parser::{Combinator, Component, Selector, SelectorIter};

use crate::RaikiriSelectorImpl;
use crate::error::CascadeError;
use crate::layer::{LayerOrder, LayerPosition};
use crate::rule::{Declaration, StyleRule};
use crate::style_dom::StyleElement;

/// Upper bound on the ancestor requirements stored per selector. Longer
/// chains only lose filtering power, not correctness.
const MAX_ANCESTOR_HASHES: usize = 4;

/// Namespace seeds keeping tag, id, and class hashes apart.
const TAG_SEED: u32 = 0x811c_9dc5;
const ID_SEED: u32 = 0x6b43_a9b5;
const CLASS_SEED: u32 = 0x2f69_3a1d;

/// ASCII-case-folded FNV-1a followed by a murmur3 finalizer. The finalizer
/// spreads entropy into the low and middle bits, which the Bloom filter uses
/// as its two slot indices.
fn hash_folded(seed: u32, text: &str) -> u32 {
    let mut hash = seed;
    for byte in text.bytes() {
        hash ^= u32::from(byte.to_ascii_lowercase());
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash ^= hash >> 16;
    hash = hash.wrapping_mul(0x85eb_ca6b);
    hash ^= hash >> 13;
    hash = hash.wrapping_mul(0xc2b2_ae35);
    hash ^ (hash >> 16)
}

fn tag_hash(tag: &str) -> u32 {
    hash_folded(TAG_SEED, tag)
}

fn id_hash(id: &str) -> u32 {
    hash_folded(ID_SEED, id)
}

fn class_hash(class: &str) -> u32 {
    hash_folded(CLASS_SEED, class)
}

/// Calls `f` with the tag, id, and class hashes describing `elem`.
fn for_each_element_hash<E: StyleElement>(elem: &E, mut f: impl FnMut(u32)) {
    f(tag_hash(elem.tag_name()));
    if let Some(id) = elem.id() {
        f(id_hash(id));
    }
    elem.for_each_class(&mut |class| f(class_hash(class)));
}

/// Bucket a selector is filed under.
#[derive(Clone, Copy)]
enum BucketKey {
    Id(u32),
    Class(u32),
    Tag(u32),
    Universal,
}

/// Consumes one compound from `iter`, returning its best bucket key and
/// pushing every id/class/type hash it contains to `hashes` (up to `limit`).
fn scan_compound(
    iter: &mut SelectorIter<'_, RaikiriSelectorImpl>,
    hashes: Option<(&mut Vec<u32>, usize)>,
) -> BucketKey {
    let mut id = None;
    let mut class = None;
    let mut tag = None;
    let mut hashes = hashes;
    for component in iter {
        let hash = match component {
            Component::ID(ident) => {
                let hash = id_hash(ident.0.as_str());
                id.get_or_insert(hash);
                hash
            }
            Component::Class(ident) => {
                let hash = class_hash(ident.0.as_str());
                class.get_or_insert(hash);
                hash
            }
            Component::LocalName(local) => {
                let hash = tag_hash(local.name.0.as_str());
                tag.get_or_insert(hash);
                hash
            }
            _ => continue,
        };
        if let Some((out, limit)) = hashes.as_mut()
            && out.len() < *limit
        {
            out.push(hash);
        }
    }
    match (id, class, tag) {
        (Some(hash), ..) => BucketKey::Id(hash),
        (None, Some(hash), _) => BucketKey::Class(hash),
        (None, None, Some(hash)) => BucketKey::Tag(hash),
        (None, None, None) => BucketKey::Universal,
    }
}

/// Bucket key of the compound that must match the element itself, plus the
/// hashes of compounds that must match one of its ancestors.
fn analyze_selector(selector: &Selector<RaikiriSelectorImpl>) -> (BucketKey, Vec<u32>) {
    let mut iter = selector.iter();
    let mut key = scan_compound(&mut iter, None);
    let mut combinator = iter.next_sequence();
    if combinator == Some(Combinator::PseudoElement) {
        // `a::before` matches its originating element `a`; the leading
        // pseudo-element compound carries no element requirement.
        key = scan_compound(&mut iter, None);
        combinator = iter.next_sequence();
    }
    let mut ancestor_hashes = Vec::new();
    while let Some(Combinator::Child | Combinator::Descendant) = combinator {
        if ancestor_hashes.len() >= MAX_ANCESTOR_HASHES {
            break;
        }
        scan_compound(&mut iter, Some((&mut ancestor_hashes, MAX_ANCESTOR_HASHES)));
        combinator = iter.next_sequence();
    }
    (key, ancestor_hashes)
}

/// One active style rule with the per-cascade data that does not depend on
/// the element being matched.
pub(crate) struct IndexedRule<'a> {
    pub(crate) rule: &'a StyleRule,
    /// `rule.declarations`, already expanded when the rule was parsed
    /// ([`crate::rule::expand_shorthand_into`]); the rule tree has no path that
    /// changes a rule after parsing.
    pub(crate) declarations: &'a [Declaration],
    /// The cascade layer position of the rule's declarations under this
    /// cascade's layer order.
    pub(crate) layer: LayerPosition,
    /// Some selector in the list targets the element itself.
    pub(crate) has_element_selector: bool,
    /// Some selector in the list targets a pseudo-element.
    pub(crate) has_pseudo_selector: bool,
}

struct SelectorEntry {
    rule: u32,
    ancestor_hashes: [u32; MAX_ANCESTOR_HASHES],
    ancestor_hash_count: u8,
}

impl SelectorEntry {
    fn ancestor_hashes(&self) -> &[u32] {
        &self.ancestor_hashes[..usize::from(self.ancestor_hash_count)]
    }
}

/// Active style rules in cascade source order, bucketed by subject key.
pub(crate) struct RuleIndex<'a> {
    rules: Vec<IndexedRule<'a>>,
    entries: Vec<SelectorEntry>,
    by_id: HashMap<u32, Vec<u32>>,
    by_class: HashMap<u32, Vec<u32>>,
    by_tag: HashMap<u32, Vec<u32>>,
    universal: Vec<u32>,
}

impl<'a> RuleIndex<'a> {
    /// Builds the index. `rules` must already be in the order candidates are
    /// to be pushed (ascending source order); [`Self::candidate_rules`]
    /// returns rule indices in that same order. `layers` is the cascade's
    /// layer order, which gives each rule its layer position.
    ///
    /// Candidates refer to a declaration by the `u32` positions of its rule
    /// and of the declaration in that rule, so this fails when either does
    /// not fit, as it does when there are too many selectors to number.
    pub(crate) fn new(
        rules: impl IntoIterator<Item = &'a StyleRule>,
        layers: &LayerOrder<'_>,
    ) -> Result<Self, CascadeError> {
        let mut index = Self {
            rules: Vec::new(),
            entries: Vec::new(),
            by_id: HashMap::new(),
            by_class: HashMap::new(),
            by_tag: HashMap::new(),
            universal: Vec::new(),
        };
        for rule in rules {
            let rule_idx = u32::try_from(index.rules.len()).map_err(|_| too_many("style rules"))?;
            u32::try_from(rule.declarations.len())
                .map_err(|_| too_many("declarations in one style rule"))?;
            let mut has_element_selector = false;
            let mut has_pseudo_selector = false;
            for selector in rule.selectors.slice() {
                if selector.pseudo_element().is_some() {
                    has_pseudo_selector = true;
                } else {
                    has_element_selector = true;
                }
                let (key, hashes) = analyze_selector(selector);
                let mut ancestor_hashes = [0; MAX_ANCESTOR_HASHES];
                ancestor_hashes[..hashes.len()].copy_from_slice(&hashes);
                let entry_idx =
                    u32::try_from(index.entries.len()).map_err(|_| too_many("selectors"))?;
                index.entries.push(SelectorEntry {
                    rule: rule_idx,
                    ancestor_hashes,
                    ancestor_hash_count: hashes.len() as u8,
                });
                let bucket = match key {
                    BucketKey::Id(hash) => index.by_id.entry(hash).or_default(),
                    BucketKey::Class(hash) => index.by_class.entry(hash).or_default(),
                    BucketKey::Tag(hash) => index.by_tag.entry(hash).or_default(),
                    BucketKey::Universal => &mut index.universal,
                };
                bucket.push(entry_idx);
            }
            index.rules.push(IndexedRule {
                rule,
                declarations: &rule.declarations,
                layer: LayerPosition {
                    attached: false,
                    rank: layers.rank(rule.layer, rule.origin),
                },
                has_element_selector,
                has_pseudo_selector,
            });
        }
        Ok(index)
    }

    pub(crate) fn rule(&self, idx: u32) -> &IndexedRule<'a> {
        &self.rules[idx as usize]
    }

    /// The active rules, at the positions [`Self::candidate_rules`] returns.
    pub(crate) fn rules(&self) -> &[IndexedRule<'a>] {
        &self.rules
    }

    /// Writes to `out` the indices of every rule that might match `elem`
    /// (directly or through one of its pseudo-elements), ascending and
    /// deduplicated. Rules absent from `out` are guaranteed not to match.
    pub(crate) fn candidate_rules<E: StyleElement>(
        &self,
        elem: &E,
        ancestors: &AncestorFilter,
        out: &mut Vec<u32>,
    ) {
        out.clear();
        let mut take = |bucket: Option<&Vec<u32>>| {
            for &entry_idx in bucket.into_iter().flatten() {
                let entry = &self.entries[entry_idx as usize];
                if ancestors.might_contain_all(entry.ancestor_hashes()) {
                    out.push(entry.rule);
                }
            }
        };
        if let Some(id) = elem.id() {
            take(self.by_id.get(&id_hash(id)));
        }
        if !self.by_class.is_empty() {
            elem.for_each_class(&mut |class| take(self.by_class.get(&class_hash(class))));
        }
        take(self.by_tag.get(&tag_hash(elem.tag_name())));
        take(Some(&self.universal));
        out.sort_unstable();
        out.dedup();
    }

    #[cfg(test)]
    pub(crate) fn rule_count(&self) -> usize {
        self.rules.len()
    }
}

// cov:ignore: a stylesheet would need billions of rules, selectors or declarations in one rule to reach this
fn too_many(what: &str) -> CascadeError {
    CascadeError::Internal {
        message: format!("more than u32::MAX {what} in one cascade"),
    }
}

/// Counting Bloom filter over the tag/id/class hashes of the elements on the
/// current ancestor path. Kept in lockstep with the walk's `ancestor_path`:
/// [`Self::push`] whenever an element is appended to it, [`Self::truncate`]
/// with the same depth whenever it is truncated.
pub(crate) struct AncestorFilter {
    bloom: Box<BloomFilter>,
    /// Hashes inserted for every ancestor, flattened.
    hashes: Vec<u32>,
    /// Start offset in `hashes` of each ancestor's group.
    frames: Vec<usize>,
}

impl AncestorFilter {
    pub(crate) fn new() -> Self {
        Self {
            bloom: Box::new(BloomFilter::new()),
            hashes: Vec::new(),
            frames: Vec::new(),
        }
    }

    pub(crate) fn push<E: StyleElement>(&mut self, elem: &E) {
        self.frames.push(self.hashes.len());
        for_each_element_hash(elem, |hash| {
            self.bloom.insert_hash(hash);
            self.hashes.push(hash);
        });
    }

    pub(crate) fn truncate(&mut self, depth: usize) {
        if depth >= self.frames.len() {
            return;
        }
        let start = self.frames[depth];
        for &hash in &self.hashes[start..] {
            self.bloom.remove_hash(hash);
        }
        self.hashes.truncate(start);
        self.frames.truncate(depth);
    }

    #[cfg(test)]
    pub(crate) fn depth(&self) -> usize {
        self.frames.len()
    }

    fn might_contain_all(&self, hashes: &[u32]) -> bool {
        hashes
            .iter()
            .all(|&hash| self.bloom.might_contain_hash(hash))
    }
}

#[cfg(test)]
mod tests;
