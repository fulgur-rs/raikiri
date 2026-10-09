//! Compact records the cascade keeps for each candidate declaration.
//!
//! A candidate does not own its declaration's value; a [`ValueRef`] refers to
//! it. The declarations of the active style rules stay in the rule tree, which
//! the cascade borrows throughout, and the declarations an element brings
//! itself (presentational hints, quirks, inline style, animation) stay in the
//! candidate arena next to the candidates. [`ValueSource`] resolves the
//! handles and [`ElementCandidates`] pairs one element's candidates with the
//! source of their declarations, so a value is only cloned when a winner is
//! applied.

use std::borrow::Cow;

use crate::error::CascadeError;
use crate::layer::LayerPosition;
use crate::property::{CustomProperty, PropertyKey, PropertyValue};
use crate::rule::Declaration;
use crate::ruletree::Origin;
use crate::specified::CascadePrecedence;

use super::collect::{
    PRESENTATIONAL_HINT_SOURCE_ORDER, PRESENTATIONAL_HINT_SPECIFICITY, RankedDecl, Specificity,
    cascade_rank,
};
use super::rollback::{Rollback, rollback_kind};
use super::rule_index::IndexedRule;

/// Where a candidate's declaration is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueRef {
    /// Declaration `decl` of the active style rule at position `rule` of the
    /// cascade's rule index.
    Rule { rule: u32, decl: u32 },
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
/// the declaration the handle refers to: only [`CandidateSink`] makes
/// candidates. There is no `PartialEq`, because whether two candidates carry
/// the same input depends on the declarations their handles refer to, which
/// [`same_candidates`] compares.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    value: ValueRef,
    key: PropertyKey,
    rollback: Rollback,
    precedence: Precedence,
}

// One candidate is kept per matched declaration of every element, so its size
// bounds the candidate arena. Its fields take 30 bytes padded to 32.
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
/// rules, local handles through one element's own declarations.
#[derive(Clone, Copy)]
pub(crate) struct ValueSource<'a> {
    rules: &'a [IndexedRule<'a>],
    local: &'a [Declaration],
}

impl<'a> ValueSource<'a> {
    pub(crate) fn new(rules: &'a [IndexedRule<'a>], local: &'a [Declaration]) -> Self {
        Self { rules, local }
    }

    /// The declaration `value` refers to.
    #[inline]
    pub(crate) fn declaration(self, value: ValueRef) -> &'a Declaration {
        match value {
            ValueRef::Rule { rule, decl } => &self.rules[rule as usize].declarations[decl as usize],
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

/// Whether two handles refer to equal values. A rule handle names one
/// declaration of the cascade, so equal rule handles need no comparison.
/// Local handles are numbered per element, so they always compare values.
fn same_value(
    (a_source, a): (ValueSource<'_>, ValueRef),
    (b_source, b): (ValueSource<'_>, ValueRef),
) -> bool {
    (matches!(a, ValueRef::Rule { .. }) && a == b)
        || a_source.declaration(a).value == b_source.declaration(b).value
}

/// Collects one element's candidates. The order of the pushes is the order
/// that decides ties between equal precedence, which winner selection and
/// the shorthand-longhand settling after it depend on.
pub(crate) struct CandidateSink<'a> {
    decls: &'a mut Vec<Candidate>,
    custom: &'a mut Vec<CustomCandidate>,
    locals: &'a mut Vec<Declaration>,
    /// Position in `locals` of the element's first own declaration.
    local_start: usize,
}

impl<'a> CandidateSink<'a> {
    /// Collects into these vectors. The element's own declarations are
    /// appended to `locals` and numbered from its current end.
    pub(crate) fn new(
        decls: &'a mut Vec<Candidate>,
        custom: &'a mut Vec<CustomCandidate>,
        locals: &'a mut Vec<Declaration>,
    ) -> Self {
        let local_start = locals.len();
        Self {
            decls,
            custom,
            locals,
            local_start,
        }
    }

    /// Adds declaration `index` of the active rule at position `rule`.
    pub(crate) fn push_rule(
        &mut self,
        (rule, index): (u32, u32),
        decl: &Declaration,
        precedence: Precedence,
    ) {
        push_candidate(
            (&mut *self.decls, &mut *self.custom),
            ValueRef::Rule { rule, decl: index },
            decl,
            precedence,
        );
    }

    /// Adds a declaration of the element's own, moved when owned and cloned
    /// when borrowed.
    pub(crate) fn push_local(
        &mut self,
        decl: Cow<'_, Declaration>,
        precedence: Precedence,
    ) -> Result<(), CascadeError> {
        let index = u32::try_from(self.locals.len() - self.local_start)
            .map_err(|_| too_many_own_declarations())?;
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

// cov:ignore: an element's own declarations come from its attributes, which would need billions of declarations to reach this
fn too_many_own_declarations() -> CascadeError {
    CascadeError::Internal {
        message: "an element has more than u32::MAX declarations of its own".to_owned(),
    }
}

/// Adds the candidate of `decl`, found at `value`, to `custom` when it
/// declares a custom property and to `decls` otherwise.
pub(crate) fn push_candidate(
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
    debug_assert_eq!(
        decl.key == PropertyKey::Custom,
        matches!(decl.value, PropertyValue::CustomProperty(_)),
        "the Custom slot belongs to custom properties alone"
    );
    debug_assert_eq!(
        decl.key == PropertyKey::All,
        matches!(decl.value, PropertyValue::AllRevertLayer),
        "the All slot belongs to all: revert-layer alone"
    );
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
    /// they refer to.
    pub(crate) fn copy(
        decls: ElementCandidates<'_>,
        custom: CustomCandidates<'_>,
    ) -> Result<Self, CascadeError> {
        let mut owned = Self::default();
        let mut sink = CandidateSink::new(&mut owned.decls, &mut owned.custom, &mut owned.local);
        for candidate in decls.decls {
            sink.push_local(
                Cow::Borrowed(decls.source.declaration(candidate.value)),
                candidate.precedence,
            )?;
        }
        for candidate in custom.decls {
            sink.push_local(
                Cow::Borrowed(custom.source.declaration(candidate.value)),
                candidate.precedence,
            )?;
        }
        Ok(owned)
    }

    /// Candidates built directly from declarations, for tests that need
    /// values no stylesheet produces.
    #[cfg(test)]
    pub(crate) fn from_declarations(
        declarations: impl IntoIterator<Item = (Declaration, Precedence)>,
    ) -> Self {
        let mut owned = Self::default();
        let mut sink = CandidateSink::new(&mut owned.decls, &mut owned.custom, &mut owned.local);
        for (decl, precedence) in declarations {
            sink.push_local(Cow::Owned(decl), precedence)
                .expect("a test pushes few declarations");
        }
        owned
    }

    pub(crate) fn candidates(&self) -> ElementCandidates<'_> {
        ElementCandidates::new(&self.decls, ValueSource::new(&[], &self.local))
    }

    pub(crate) fn custom_candidates(&self) -> CustomCandidates<'_> {
        CustomCandidates::new(&self.custom, ValueSource::new(&[], &self.local))
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
