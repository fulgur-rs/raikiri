use super::convert::{FromTaffy, IntoTaffy};
use crate::layout::protocol::OofCandidate;
use raikiri_layout_geometry::Point;

/// The out-of-flow candidates found inside a box, in document order.
///
/// An empty list does not allocate. The list is the library's, wrapped: the
/// candidates are handed to the library and taken back from it as a whole, so
/// the list is moved and never copied. An element is converted when it is
/// pushed to the list or read from it.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct OofCandidates(taffy::OofCandidates);

impl OofCandidates {
    /// An empty list.
    pub(crate) const NONE: Self = Self(taffy::OofCandidates::NONE);

    /// An empty list.
    #[inline]
    pub(crate) const fn new() -> Self {
        Self::NONE
    }

    /// Whether the list holds no candidate.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Add a candidate at the end of the list.
    #[inline]
    pub(crate) fn push(&mut self, candidate: OofCandidate) {
        self.0.push(candidate.into_taffy());
    }

    /// Move all candidates of `other` to the end of this list.
    #[inline]
    pub(crate) fn append(&mut self, other: &mut OofCandidates) {
        self.0.append(&mut other.0);
    }

    /// Take the candidates out of this list, leaving it empty.
    #[inline]
    pub(crate) fn take(&mut self) -> Self {
        Self(self.0.take())
    }

    /// The candidates, in order.
    #[inline]
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = OofCandidate> + '_ {
        self.0
            .iter()
            .map(|&candidate| OofCandidate::from_taffy(candidate))
    }

    /// The candidates, in order, as a vector.
    #[cfg(test)]
    pub(crate) fn to_vec(&self) -> Vec<OofCandidate> {
        self.iter().collect()
    }

    /// Let `edit` change each candidate in place.
    #[inline]
    pub(crate) fn for_each_mut(&mut self, mut edit: impl FnMut(&mut OofCandidate)) {
        for slot in self.0.as_mut_slice() {
            let mut candidate = OofCandidate::from_taffy(*slot);
            edit(&mut candidate);
            *slot = candidate.into_taffy();
        }
    }

    /// Move the static-position areas of every candidate by `offset`.
    ///
    /// This turns areas that are relative to the border box of a child into
    /// areas relative to the border box of its parent as candidates bubble up.
    #[inline]
    pub(crate) fn translate(&mut self, offset: Point<f32>) {
        self.0.translate(offset.into_taffy());
    }
}

impl IntoTaffy for OofCandidates {
    type Taffy = taffy::OofCandidates;

    #[inline]
    fn into_taffy(self) -> taffy::OofCandidates {
        self.0
    }
}

impl FromTaffy for OofCandidates {
    type Taffy = taffy::OofCandidates;

    #[inline]
    fn from_taffy(value: taffy::OofCandidates) -> Self {
        Self(value)
    }
}

/// Out-of-flow candidates ready to be positioned.
///
/// The candidates are converted once, when the batch is collected, into the
/// list the library takes, so that positioning them in whole or in consecutive
/// runs (see [`OofBatch::all`] and [`OofSpan::slice`]) does not convert them
/// again.
#[derive(Debug, Clone)]
pub(crate) struct OofBatch(Vec<taffy::OofCandidate>);

impl OofBatch {
    /// The batch of `candidates`, in order.
    #[inline]
    pub(crate) fn collect(candidates: impl IntoIterator<Item = OofCandidate>) -> Self {
        Self(candidates.into_iter().map(IntoTaffy::into_taffy).collect())
    }

    /// The candidates, in order.
    #[inline]
    pub(crate) fn iter(&self) -> impl ExactSizeIterator<Item = OofCandidate> + '_ {
        self.0
            .iter()
            .map(|&candidate| OofCandidate::from_taffy(candidate))
    }

    /// All candidates of the batch.
    #[inline]
    pub(crate) fn all(&self) -> OofSpan<'_> {
        OofSpan(&self.0)
    }
}

/// A run of consecutive candidates of an [`OofBatch`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct OofSpan<'a>(&'a [taffy::OofCandidate]);

impl<'a> OofSpan<'a> {
    /// The candidates at the positions `range`.
    #[inline]
    pub(crate) fn slice(self, range: std::ops::Range<usize>) -> OofSpan<'a> {
        OofSpan(&self.0[range])
    }

    /// How many candidates the run holds.
    #[inline]
    pub(crate) fn len(self) -> usize {
        self.0.len()
    }

    /// Whether the run holds no candidate.
    #[inline]
    pub(crate) fn is_empty(self) -> bool {
        self.0.is_empty()
    }

    /// The candidates, in order.
    #[inline]
    pub(crate) fn iter(self) -> impl ExactSizeIterator<Item = OofCandidate> + 'a {
        self.0
            .iter()
            .map(|&candidate| OofCandidate::from_taffy(candidate))
    }

    /// The candidates as the library's.
    #[inline]
    pub(super) fn as_taffy(self) -> &'a [taffy::OofCandidate] {
        self.0
    }
}
