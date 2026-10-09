//! Compact records the cascade keeps for each candidate declaration.
//!
//! A candidate does not own its declaration's value; a [`ValueRef`] refers to
//! it. The declarations of the active style rules stay in the rule tree, which
//! the cascade borrows throughout. A `style` attribute is stored as a block the
//! second time its source is seen, and that element and every later one with
//! the same source refer to the block; the first keeps its own parse. The
//! other declarations an element brings itself (presentational hints,
//! quirks, inline style, animation) stay in its [`ElementInput`] next to its
//! candidates. [`ValueSource`] resolves the handles and [`ElementCandidates`]
//! pairs one element's candidates with the source of their declarations, so a
//! value is only cloned when a winner is applied.

use std::borrow::Cow;

use crate::PseudoElem;
use crate::error::CascadeError;
use crate::layer::LayerPosition;
use crate::property::{CustomProperty, PropertyKey, PropertyValue};
use crate::rule::Declaration;
use crate::ruletree::Origin;
use crate::specified::CascadePrecedence;

use super::collect::{
    CASCADED_PSEUDO_ELEMENTS, PRESENTATIONAL_HINT_SOURCE_ORDER, PRESENTATIONAL_HINT_SPECIFICITY,
    RankedDecl, Specificity, cascade_rank,
};
use super::limits::{CandidateBudget, bytes_of};
use super::rollback::{Rollback, rollback_kind};
use super::rule_index::IndexedRule;

/// Where a candidate's declaration is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueRef {
    /// Declaration `decl` of the active style rule at position `rule` of the
    /// cascade's rule index.
    Rule { rule: u32, decl: u32 },
    /// Declaration `decl` of the stored `style`-attribute block at position
    /// `block`, which every element with that source text after the first
    /// shares.
    Cached { block: u32, decl: u32 },
    /// The element's own declaration at this position, counted from the
    /// element's first one. Equal positions of two elements refer to two
    /// different declarations.
    Local(u32),
}

/// One candidate declaration of an element or of one of its pseudo-elements:
/// a handle to the declaration, the slot key and rollback kind copied from
/// it, and the precedence the candidate takes from its source.
///
/// The fields are private so that the key and rollback kind always describe
/// the declaration the handle refers to: only [`CandidateSink`] and
/// [`push_rule_candidates`] make candidates, reading each declaration where its
/// handle points. There is no `PartialEq`, because whether two candidates carry
/// the same input depends on the declarations their handles refer to, which
/// [`same_candidates`] compares.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    value: ValueRef,
    key: PropertyKey,
    rollback: Rollback,
    precedence: Precedence,
}

// One candidate is kept per matched declaration of the element being resolved
// and of every input kept for sharing, so its size bounds what the cascade
// holds. Its fields take 30 bytes padded to 32.
const _: () = assert!(
    std::mem::size_of::<Candidate>() <= 32,
    "Candidate grew past 32 bytes: raise the bound together with a cascade memory measurement"
);

impl Candidate {
    /// Where the declaration is kept.
    #[cfg(test)]
    pub(crate) fn value(self) -> ValueRef {
        self.value
    }

    /// The cascade slot of the declaration.
    #[inline]
    pub(crate) fn key(self) -> PropertyKey {
        self.key
    }

    /// How the declaration rolls the cascade back.
    #[inline]
    pub(crate) fn rollback(self) -> Rollback {
        self.rollback
    }

    #[inline]
    pub(crate) fn precedence(self) -> Precedence {
        self.precedence
    }

    /// Whether the declaration is `all: revert-layer`: the `all` property
    /// keeps no other value, and no other value has its slot.
    #[inline]
    pub(crate) fn is_all_revert_layer(self) -> bool {
        self.key == PropertyKey::All
    }
}

/// A custom-property candidate. Its declaration's value is a
/// [`PropertyValue::CustomProperty`], whose case-sensitive name is the slot
/// it competes for.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CustomCandidate {
    value: ValueRef,
    precedence: Precedence,
}

/// Resolves candidate handles: rule handles through the cascade's active
/// rules, cached handles through its stored `style`-attribute blocks, and
/// local handles through one element's own declarations.
#[derive(Clone, Copy)]
pub(crate) struct ValueSource<'a> {
    rules: &'a [IndexedRule<'a>],
    blocks: &'a [Box<[Declaration]>],
    local: &'a [Declaration],
}

impl<'a> ValueSource<'a> {
    pub(crate) fn new(
        rules: &'a [IndexedRule<'a>],
        blocks: &'a [Box<[Declaration]>],
        local: &'a [Declaration],
    ) -> Self {
        Self {
            rules,
            blocks,
            local,
        }
    }

    /// The declaration `value` refers to.
    #[inline]
    pub(crate) fn declaration(self, value: ValueRef) -> &'a Declaration {
        match value {
            ValueRef::Rule { rule, decl } => &self.rules[rule as usize].declarations[decl as usize],
            ValueRef::Cached { block, decl } => &self.blocks[block as usize][decl as usize],
            ValueRef::Local(index) => &self.local[index as usize],
        }
    }

    /// The custom property a custom-property candidate's handle refers to.
    fn custom_property(self, value: ValueRef) -> &'a CustomProperty {
        match &self.declaration(value).value {
            PropertyValue::CustomProperty(custom) => custom,
            // cov:ignore: CandidateSink makes custom-property candidates only from custom-property declarations
            other => unreachable!("a custom-property candidate refers to {other:?}"),
        }
    }
}

/// One element's (or pseudo-element's) candidates in collection order, with
/// the source of their declarations. Winner selection records positions in
/// [`Self::decls`]; a position is only meaningful for the view it came from.
#[derive(Clone, Copy)]
pub(crate) struct ElementCandidates<'a> {
    decls: &'a [Candidate],
    source: ValueSource<'a>,
}

impl ElementCandidates<'static> {
    /// No candidates.
    pub(crate) const EMPTY: Self = Self {
        decls: &[],
        source: ValueSource {
            rules: &[],
            blocks: &[],
            local: &[],
        },
    };
}

impl<'a> ElementCandidates<'a> {
    pub(crate) fn new(decls: &'a [Candidate], source: ValueSource<'a>) -> Self {
        Self { decls, source }
    }

    #[inline]
    pub(crate) fn decls(self) -> &'a [Candidate] {
        self.decls
    }

    /// The declaration of the candidate at `idx`.
    #[inline]
    pub(crate) fn declaration(self, idx: usize) -> &'a Declaration {
        self.source.declaration(self.decls[idx].value)
    }

    /// The value of the candidate at `idx`.
    #[inline]
    pub(crate) fn value(self, idx: usize) -> &'a PropertyValue {
        &self.declaration(idx).value
    }

    /// The candidates `keep` accepts, copied into `scratch` in the same order
    /// and resolving to the same declarations.
    pub(crate) fn filtered<'b>(
        self,
        scratch: &'b mut Vec<Candidate>,
        mut keep: impl FnMut(Candidate) -> bool,
    ) -> ElementCandidates<'b>
    where
        'a: 'b,
    {
        scratch.clear();
        scratch.extend(
            self.decls
                .iter()
                .copied()
                .filter(|&candidate| keep(candidate)),
        );
        ElementCandidates {
            decls: scratch,
            source: self.source,
        }
    }
}

/// The custom-property counterpart of [`ElementCandidates`].
#[derive(Clone, Copy)]
pub(crate) struct CustomCandidates<'a> {
    decls: &'a [CustomCandidate],
    source: ValueSource<'a>,
}

impl CustomCandidates<'static> {
    /// No candidates.
    pub(crate) const EMPTY: Self = Self {
        decls: &[],
        source: ValueSource {
            rules: &[],
            blocks: &[],
            local: &[],
        },
    };
}

impl<'a> CustomCandidates<'a> {
    pub(crate) fn new(decls: &'a [CustomCandidate], source: ValueSource<'a>) -> Self {
        Self { decls, source }
    }

    pub(crate) fn is_empty(self) -> bool {
        self.decls.is_empty()
    }

    /// Every candidate's custom property with its precedence, in collection
    /// order.
    pub(crate) fn iter(self) -> impl Iterator<Item = (&'a CustomProperty, Precedence)> {
        self.decls.iter().map(move |candidate| {
            (
                self.source.custom_property(candidate.value),
                candidate.precedence,
            )
        })
    }
}

/// Whether two candidate lists carry the same cascade input: equal
/// declarations in the same order, each with the same precedence. Both lists
/// must come from one cascade, so that equal rule handles name one
/// declaration.
pub(crate) fn same_candidates(
    a: Option<ElementCandidates<'_>>,
    b: Option<ElementCandidates<'_>>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.decls.len() == b.decls.len()
                && a.decls.iter().zip(b.decls).all(|(x, y)| {
                    x.key == y.key
                        && x.rollback == y.rollback
                        && x.precedence == y.precedence
                        && same_value((a.source, x.value), (b.source, y.value))
                })
        }
        _ => false,
    }
}

/// The custom-property counterpart of [`same_candidates`].
pub(crate) fn same_custom_candidates(
    a: Option<CustomCandidates<'_>>,
    b: Option<CustomCandidates<'_>>,
) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.decls.len() == b.decls.len()
                && a.decls.iter().zip(b.decls).all(|(x, y)| {
                    x.precedence == y.precedence
                        && same_value((a.source, x.value), (b.source, y.value))
                })
        }
        _ => false,
    }
}

/// Whether two handles refer to equal values. A rule or cached handle names
/// one declaration of the cascade, so equal ones need no comparison. Local
/// handles are numbered per element, so they always compare values.
fn same_value(
    (a_source, a): (ValueSource<'_>, ValueRef),
    (b_source, b): (ValueSource<'_>, ValueRef),
) -> bool {
    (matches!(a, ValueRef::Rule { .. } | ValueRef::Cached { .. }) && a == b)
        || a_source.declaration(a).value == b_source.declaration(b).value
}

/// Collects one element's candidates, and those of its pseudo-elements. The
/// order of the pushes is the order that decides ties between equal
/// precedence, which winner selection and the shorthand-longhand settling
/// after it depend on.
///
/// Every push is counted against the cascade's [`CandidateBudget`] before
/// anything is added, so no candidate list grows past a limit.
pub(crate) struct CandidateSink<'a> {
    decls: &'a mut Vec<Candidate>,
    custom: &'a mut Vec<CustomCandidate>,
    locals: &'a mut Vec<Declaration>,
    /// Position in `locals` of the element's first own declaration.
    local_start: usize,
    budget: &'a mut CandidateBudget,
}

impl<'a> CandidateSink<'a> {
    /// Collects into these vectors, counting against `budget`. The element's
    /// own declarations are appended to `locals` and numbered from its
    /// current end.
    pub(crate) fn new(
        decls: &'a mut Vec<Candidate>,
        custom: &'a mut Vec<CustomCandidate>,
        locals: &'a mut Vec<Declaration>,
        budget: &'a mut CandidateBudget,
    ) -> Self {
        let local_start = locals.len();
        Self {
            decls,
            custom,
            locals,
            local_start,
            budget,
        }
    }

    /// How many more candidates the element may collect.
    pub(crate) fn room(&self) -> usize {
        self.budget.room()
    }

    /// The error for `count` more candidates of the element, more than
    /// [`Self::room`].
    pub(crate) fn past_room(&self, count: usize) -> CascadeError {
        self.budget.past_room(count)
    }

    /// Adds a candidate for every declaration of the active rule at position
    /// `rule` of `rules`; see [`push_rule_candidates`].
    ///
    /// # Errors
    ///
    /// Fails, adding nothing, when the declarations would pass a candidate
    /// limit.
    #[inline]
    pub(crate) fn push_rule(
        &mut self,
        rules: &[IndexedRule<'_>],
        rule: u32,
        precedence: impl Fn(&Declaration) -> Precedence,
    ) -> Result<(), CascadeError> {
        let declarations = rules[rule as usize].declarations;
        self.budget.charge(declarations.len())?;
        push_rule_candidates(
            (&mut *self.decls, &mut *self.custom),
            rule,
            declarations,
            precedence,
        );
        Ok(())
    }

    /// Adds a candidate to `pseudo`, one of the element's pseudo-elements, for
    /// every declaration of the active rule at position `rule` of `rules`;
    /// see [`push_rule_candidates`]. The element and its pseudo-elements
    /// share one candidate budget.
    ///
    /// # Errors
    ///
    /// Fails, adding nothing, when the declarations would pass a candidate
    /// limit.
    #[inline]
    pub(crate) fn push_pseudo_rule(
        &mut self,
        pseudo: &mut PseudoInput,
        rules: &[IndexedRule<'_>],
        rule: u32,
        precedence: impl Fn(&Declaration) -> Precedence,
    ) -> Result<(), CascadeError> {
        let declarations = rules[rule as usize].declarations;
        self.budget.charge(declarations.len())?;
        push_rule_candidates(
            (&mut pseudo.decls, &mut pseudo.custom),
            rule,
            declarations,
            precedence,
        );
        Ok(())
    }

    /// Adds a candidate for every declaration of the stored `style`-attribute
    /// block at position `block` of `blocks`, with the precedence `precedence`
    /// gives it.
    ///
    /// # Errors
    ///
    /// Fails, adding nothing, when the declarations would pass a candidate
    /// limit.
    pub(crate) fn push_cached(
        &mut self,
        blocks: &[Box<[Declaration]>],
        block: u32,
        precedence: impl Fn(&Declaration) -> Precedence,
    ) -> Result<(), CascadeError> {
        let declarations = &blocks[block as usize];
        self.budget.charge(declarations.len())?;
        // The cache checked that every position in a stored block fits in
        // `u32` when it stored the block.
        for (decl, index) in declarations.iter().zip(0u32..) {
            push_candidate(
                (&mut *self.decls, &mut *self.custom),
                ValueRef::Cached { block, decl: index },
                decl,
                precedence(decl),
            );
        }
        Ok(())
    }

    /// Adds a declaration of the element's own, moved when owned and cloned
    /// when borrowed.
    ///
    /// # Errors
    ///
    /// Fails, adding nothing, when the declaration would pass a candidate
    /// limit or the element has more own declarations than a local handle
    /// can number.
    pub(crate) fn push_local(
        &mut self,
        decl: Cow<'_, Declaration>,
        precedence: Precedence,
    ) -> Result<(), CascadeError> {
        let index = u32::try_from(self.locals.len() - self.local_start)
            .map_err(|_| too_many("own declarations of one element"))?;
        self.budget.charge(1)?;
        push_candidate(
            (&mut *self.decls, &mut *self.custom),
            ValueRef::Local(index),
            &decl,
            precedence,
        );
        self.locals.push(decl.into_owned());
        Ok(())
    }

    /// Adds a presentational hint: never important, unlayered, in the author
    /// presentational hint origin at its specificity and source order.
    pub(crate) fn push_hint(&mut self, value: PropertyValue) -> Result<(), CascadeError> {
        self.push_local(
            Cow::Owned(Declaration::new(value, false)),
            Precedence::new(
                Origin::AuthorPresentationalHint,
                false,
                PRESENTATIONAL_HINT_SPECIFICITY,
                PRESENTATIONAL_HINT_SOURCE_ORDER,
                LayerPosition::default(),
            ),
        )
    }
}

// cov:ignore: a handle overflows only past billions of rules, selectors, style blocks or declarations
/// The error for a cascade with more of `what` than a `u32` handle can number.
pub(crate) fn too_many(what: &str) -> CascadeError {
    CascadeError::Internal {
        message: format!("more than u32::MAX {what} in one cascade"),
    }
}

/// Adds a candidate for every one of `declarations`, those of the active rule
/// at position `rule`, with the precedence `precedence` gives it, to `custom`
/// when it declares a custom property and to `decls` otherwise.
fn push_rule_candidates(
    (decls, custom): (&mut Vec<Candidate>, &mut Vec<CustomCandidate>),
    rule: u32,
    declarations: &[Declaration],
    precedence: impl Fn(&Declaration) -> Precedence,
) {
    // `RuleIndex::new` checked that every position fits in `u32`.
    for (decl, index) in declarations.iter().zip(0u32..) {
        push_candidate(
            (&mut *decls, &mut *custom),
            ValueRef::Rule { rule, decl: index },
            decl,
            precedence(decl),
        );
    }
}

/// Adds the candidate of `decl`, found at `value`, to `custom` when it
/// declares a custom property and to `decls` otherwise.
fn push_candidate(
    (decls, custom): (&mut Vec<Candidate>, &mut Vec<CustomCandidate>),
    value: ValueRef,
    decl: &Declaration,
    precedence: Precedence,
) {
    debug_assert_derived_fields(decl);
    if decl.key == PropertyKey::Custom {
        custom.push(CustomCandidate { value, precedence });
    } else {
        decls.push(Candidate {
            value,
            key: decl.key,
            rollback: decl.rollback,
            precedence,
        });
    }
}

/// Checks, in debug builds, that a declaration's derived fields still describe
/// its value. The value only changes through `Declaration::update_value`, which
/// recomputes them; a direct write to `value` would leave them stale. The
/// `Custom` and `All` slots also identify custom properties and
/// `all: revert-layer` without reading the value, so each must be the slot of
/// that value alone.
fn debug_assert_derived_fields(decl: &Declaration) {
    debug_assert_eq!(decl.key, decl.value.key(), "stale declaration key");
    debug_assert_eq!(
        decl.rollback,
        rollback_kind(&decl.value),
        "stale declaration rollback"
    );
    // cov:ignore: the message is built only when the assertion fails, and no
    // other value has this slot
    debug_assert_eq!(
        decl.key == PropertyKey::Custom,
        matches!(decl.value, PropertyValue::CustomProperty(_)),
        "the Custom slot belongs to custom properties alone"
    );
    // cov:ignore: the message is built only when the assertion fails, and no
    // other value has this slot
    debug_assert_eq!(
        decl.key == PropertyKey::All,
        matches!(decl.value, PropertyValue::AllRevertLayer),
        "the All slot belongs to all: revert-layer alone"
    );
}

/// The declarations every element of one cascade can refer to: those of the
/// active style rules and of the stored `style`-attribute blocks.
#[derive(Clone, Copy)]
pub(crate) struct SharedDeclarations<'a> {
    rules: &'a [IndexedRule<'a>],
    blocks: &'a [Box<[Declaration]>],
}

impl<'a> SharedDeclarations<'a> {
    pub(crate) fn new(rules: &'a [IndexedRule<'a>], blocks: &'a [Box<[Declaration]>]) -> Self {
        Self { rules, blocks }
    }

    fn source(self, local: &'a [Declaration]) -> ValueSource<'a> {
        ValueSource::new(self.rules, self.blocks, local)
    }
}

/// One node's cascade input: the candidates of the element and of each of its
/// pseudo-elements, and the element's own declarations, which its local
/// handles number from the first. A node that is not an element has an empty
/// input.
///
/// Everything node-specific the cascade knows about an element — matched
/// rules, inline style, presentational hints, quirks declarations — reaches
/// winner selection only through this input, so two siblings with the same
/// input ([`Self::same_input`]) resolve to the same results.
#[derive(Default)]
pub(crate) struct ElementInput {
    local: Vec<Declaration>,
    decls: Vec<Candidate>,
    custom: Vec<CustomCandidate>,
    /// The candidates of each pseudo-element in [`CASCADED_PSEUDO_ELEMENTS`],
    /// at the same position. They all come from style rules.
    pseudo: [PseudoInput; CASCADED_PSEUDO_ELEMENTS.len()],
}

/// The candidates of one pseudo-element of an element, which
/// [`CandidateSink::push_pseudo_rule`] adds.
#[derive(Default)]
pub(crate) struct PseudoInput {
    decls: Vec<Candidate>,
    custom: Vec<CustomCandidate>,
}

impl ElementInput {
    /// Empties the input, keeping its buffers for the next node.
    pub(crate) fn clear(&mut self) {
        self.local.clear();
        self.decls.clear();
        self.custom.clear();
        for pseudo in &mut self.pseudo {
            pseudo.decls.clear();
            pseudo.custom.clear();
        }
    }

    /// A sink for the element's own candidates, and the pseudo-elements'
    /// inputs, for collecting into this empty input. `budget` starts counting
    /// the element's candidates from zero.
    pub(crate) fn parts<'a>(
        &'a mut self,
        budget: &'a mut CandidateBudget,
    ) -> (
        CandidateSink<'a>,
        &'a mut [PseudoInput; CASCADED_PSEUDO_ELEMENTS.len()],
    ) {
        debug_assert!(self.is_empty(), "collection starts from an empty input");
        budget.start_element();
        (
            CandidateSink::new(&mut self.decls, &mut self.custom, &mut self.local, budget),
            &mut self.pseudo,
        )
    }

    fn is_empty(&self) -> bool {
        self.local.is_empty()
            && self.decls.is_empty()
            && self.custom.is_empty()
            && self
                .pseudo
                .iter()
                .all(|pseudo| pseudo.decls.is_empty() && pseudo.custom.is_empty())
    }

    /// The element's ordinary and custom-property candidates, each `None` when
    /// there are none.
    pub(crate) fn element<'a>(
        &'a self,
        shared: SharedDeclarations<'a>,
    ) -> (Option<ElementCandidates<'a>>, Option<CustomCandidates<'a>>) {
        let source = shared.source(&self.local);
        (
            (!self.decls.is_empty()).then(|| ElementCandidates::new(&self.decls, source)),
            (!self.custom.is_empty()).then(|| CustomCandidates::new(&self.custom, source)),
        )
    }

    /// The ordinary and custom-property candidates of `pseudo`, each `None`
    /// when there are none.
    pub(crate) fn pseudo<'a>(
        &'a self,
        pseudo: PseudoElem,
        shared: SharedDeclarations<'a>,
    ) -> (Option<ElementCandidates<'a>>, Option<CustomCandidates<'a>>) {
        let Some(input) = pseudo_slot(pseudo).map(|slot| &self.pseudo[slot]) else {
            return (None, None);
        };
        let source = shared.source(&[]);
        (
            (!input.decls.is_empty()).then(|| ElementCandidates::new(&input.decls, source)),
            (!input.custom.is_empty()).then(|| CustomCandidates::new(&input.custom, source)),
        )
    }

    /// Whether this input and `other` carry exactly the same cascade input:
    /// equal ordinary, custom-property and per-pseudo-element candidate lists,
    /// compared value by value including origin, importance, specificity and
    /// source order (see [`same_candidates`]). Both inputs must come from the
    /// cascade `shared` belongs to.
    pub(crate) fn same_input(&self, other: &Self, shared: SharedDeclarations<'_>) -> bool {
        let same = |(a_decls, a_custom), (b_decls, b_custom)| {
            same_candidates(a_decls, b_decls) && same_custom_candidates(a_custom, b_custom)
        };
        same(self.element(shared), other.element(shared))
            && CASCADED_PSEUDO_ELEMENTS
                .iter()
                .all(|&pseudo| same(self.pseudo(pseudo, shared), other.pseudo(pseudo, shared)))
    }

    /// The heap bytes the input's buffers hold: their capacities, though not
    /// what the declarations own beyond their inline size.
    pub(crate) fn buffer_bytes(&self) -> usize {
        use std::mem::size_of;
        self.local.capacity() * size_of::<Declaration>()
            + self.decls.capacity() * size_of::<Candidate>()
            + self.custom.capacity() * size_of::<CustomCandidate>()
            + self
                .pseudo
                .iter()
                .map(|pseudo| {
                    pseudo.decls.capacity() * size_of::<Candidate>()
                        + pseudo.custom.capacity() * size_of::<CustomCandidate>()
                })
                .sum::<usize>()
    }
}

/// The position of `pseudo` in [`CASCADED_PSEUDO_ELEMENTS`], if the cascade
/// collects candidates for it.
pub(crate) fn pseudo_slot(pseudo: PseudoElem) -> Option<usize> {
    CASCADED_PSEUDO_ELEMENTS
        .iter()
        .position(|&slot| slot == pseudo)
}

/// Candidates copied out of a cascade with every declaration they refer to,
/// so they outlive the rule tree. Every handle is local.
#[derive(Clone, Debug, Default)]
pub(crate) struct OwnedCandidates {
    local: Vec<Declaration>,
    decls: Vec<Candidate>,
    custom: Vec<CustomCandidate>,
}

impl OwnedCandidates {
    /// Copies `decls` and `custom`, in their order, with the declarations
    /// they refer to. The candidates were counted against the candidate
    /// limits when they were collected, so copying them is not.
    pub(crate) fn copy(
        decls: ElementCandidates<'_>,
        custom: CustomCandidates<'_>,
    ) -> Result<Self, CascadeError> {
        let mut owned = Self::default();
        let mut budget = CandidateBudget::unlimited();
        let mut sink = CandidateSink::new(
            &mut owned.decls,
            &mut owned.custom,
            &mut owned.local,
            &mut budget,
        );
        for candidate in decls.decls {
            sink.push_local(
                Cow::Borrowed(decls.source.declaration(candidate.value)),
                candidate.precedence,
            )?; // cov:ignore: the error branch needs a u32 handle overflow
        }
        for candidate in custom.decls {
            sink.push_local(
                Cow::Borrowed(custom.source.declaration(candidate.value)),
                candidate.precedence,
            )?; // cov:ignore: the error branch needs a u32 handle overflow
        }
        debug_assert_eq!(owned.bytes(), Self::copy_bytes(decls, custom));
        Ok(owned)
    }

    /// Candidates built directly from declarations, for tests that need
    /// values no stylesheet produces.
    #[cfg(test)]
    pub(crate) fn from_declarations(
        declarations: impl IntoIterator<Item = (Declaration, Precedence)>,
    ) -> Self {
        let mut owned = Self::default();
        let mut budget = CandidateBudget::unlimited();
        let mut sink = CandidateSink::new(
            &mut owned.decls,
            &mut owned.custom,
            &mut owned.local,
            &mut budget,
        );
        for (decl, precedence) in declarations {
            sink.push_local(Cow::Owned(decl), precedence)
                .expect("a test pushes few declarations");
        }
        owned
    }

    /// The inline bytes of the copied records: the declarations, which a
    /// result keeping these candidates holds, and the candidates referring
    /// to them. Counted by length, so equal copies count the same.
    pub(crate) fn bytes(&self) -> u64 {
        Self::bytes_for(self.decls.len(), self.custom.len())
    }

    /// The [`Self::bytes`] of the copy [`Self::copy`] makes of `decls` and
    /// `custom`, known before copying them.
    pub(crate) fn copy_bytes(decls: ElementCandidates<'_>, custom: CustomCandidates<'_>) -> u64 {
        Self::bytes_for(decls.decls.len(), custom.decls.len())
    }

    /// Every copied candidate brings its own declaration.
    fn bytes_for(decls: usize, custom: usize) -> u64 {
        bytes_of::<Declaration>(decls.saturating_add(custom))
            .saturating_add(bytes_of::<Candidate>(decls))
            .saturating_add(bytes_of::<CustomCandidate>(custom))
    }

    pub(crate) fn candidates(&self) -> ElementCandidates<'_> {
        ElementCandidates::new(&self.decls, ValueSource::new(&[], &[], &self.local))
    }

    pub(crate) fn custom_candidates(&self) -> CustomCandidates<'_> {
        CustomCandidates::new(&self.custom, ValueSource::new(&[], &[], &self.local))
    }
}

/// Where one candidate declaration stands in the cascade order, apart from
/// its value: origin and importance, cascade layer, specificity and source
/// order (CSS Cascading 5 §6.1 <https://drafts.csswg.org/css-cascade-5/#cascade-sort>).
///
/// The fields are private so that the rank always agrees with the origin and
/// importance it is derived from. The layer position is stored as its two
/// parts, which keeps the record at 16 bytes instead of the 20 a nested
/// [`LayerPosition`] (with its own padding) would take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Precedence {
    /// Origin plus `!important` precedence from [`cascade_rank`].
    rank: u8,
    origin: Origin,
    /// The importance the candidate takes, which is not always that of its
    /// declaration: animations are never important.
    important: bool,
    layer_attached: bool,
    layer_rank: u32,
    specificity: Specificity,
    source_order: u32,
}

const _: () = assert!(
    std::mem::size_of::<Precedence>() <= 16,
    "Precedence grew past 16 bytes, which candidate records count on"
);

impl Precedence {
    pub(crate) fn new(
        origin: Origin,
        important: bool,
        specificity: Specificity,
        source_order: u32,
        layer: LayerPosition,
    ) -> Self {
        Self {
            rank: cascade_rank(origin, important),
            origin,
            important,
            layer_attached: layer.attached,
            layer_rank: layer.rank,
            specificity,
            source_order,
        }
    }

    #[cfg(test)]
    pub(crate) fn origin(self) -> Origin {
        self.origin
    }

    #[cfg(test)]
    pub(crate) fn important(self) -> bool {
        self.important
    }

    pub(crate) fn layer(self) -> LayerPosition {
        LayerPosition {
            attached: self.layer_attached,
            rank: self.layer_rank,
        }
    }

    #[cfg(test)]
    pub(crate) fn specificity(self) -> Specificity {
        self.specificity
    }

    #[cfg(test)]
    pub(crate) fn source_order(self) -> u32 {
        self.source_order
    }

    /// The ranking winner selection compares, for the candidate found at
    /// `idx` of the candidates being ranked.
    pub(crate) fn ranked(self, idx: usize) -> RankedDecl {
        RankedDecl {
            rank: self.rank,
            layer_priority: self.layer().priority(self.important),
            specificity: self.specificity,
            source_order: self.source_order,
            idx,
        }
    }

    /// What [`super::rollback::select_layered_winner`] inspects about the
    /// candidate at `idx` that rolls the cascade back as `rollback` says.
    pub(crate) fn layered(
        self,
        idx: usize,
        rollback: Rollback,
    ) -> (CascadePrecedence, Origin, LayerPosition, bool, Rollback) {
        let ranked = self.ranked(idx);
        (
            (
                ranked.rank,
                ranked.layer_priority,
                ranked.specificity,
                ranked.source_order,
                idx,
            ),
            self.origin,
            self.layer(),
            self.important,
            rollback,
        )
    }
}

#[cfg(test)]
mod tests;
