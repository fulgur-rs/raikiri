//! Limits on the work of one cascade and the memory of its result, and the
//! counters that enforce them.

use crate::error::{CascadeError, CascadeLimitKind};

/// Limits on the work one cascade does and the memory its result holds.
///
/// Every limit counts something that depends only on the document and its
/// stylesheets, never on the allocator, so an input that passes one fails
/// with [`CascadeError::LimitExceeded`] the same way every time. A memory
/// limit fails before the cascade allocates what it counts; a work limit
/// fails as soon as the work passes it. `None` lifts a limit. The defaults
/// leave ordinary documents far below every limit.
///
/// The limits apply to one cascade run. What they count is listed with each
/// one; in particular the heap that computed values own (strings and lists
/// copied into them) is not counted, and the time of one selector test is
/// not bounded.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CascadeLimits {
    /// The most candidate declarations one element may collect, those of its
    /// pseudo-elements included: one for each declaration of every rule that
    /// matches the element or one of its pseudo-elements, of its `style`
    /// attribute and animation, and of its presentational hints. A `style`
    /// attribute stops being parsed as soon as its declarations would pass
    /// it. This bounds what the cascade holds for the element it resolves.
    ///
    /// Defaults to `Some(65_536)`; elements of ordinary documents collect a
    /// few hundred at most.
    pub max_candidates_per_element: Option<u32>,
    /// The most candidate declarations the whole cascade may collect, summed
    /// over its elements and counted as each element's collection completes.
    ///
    /// Defaults to `Some(2^28)`.
    pub max_declarations_visited: Option<u64>,
    /// The most selector tests the cascade may make. Every rule the rule
    /// index cannot rule out for an element costs one test per selector of
    /// its list in each pass the list needs: one for the element when some
    /// selector targets it, and one for its pseudo-elements when some
    /// selector targets one. A rule that does not match, or has no
    /// declarations, costs its tests without adding candidates. This counts
    /// tests, not their cost: a `:has()` test searches the element's
    /// subtree or following siblings, and rules the index rules out by
    /// ancestor hashes are not counted.
    ///
    /// Defaults to `Some(2^30)`.
    pub max_selector_tests: Option<u64>,
    /// The most bytes of candidate declarations the result may keep, for the
    /// first-line and first-letter styles recomputed from them later. Counted
    /// by the inline size of each kept record, not the heap its values own.
    /// A document with `::first-line` and `::first-letter` styles keeps the
    /// candidates of every element under an element with a `::first-line`
    /// style.
    ///
    /// Defaults to `Some(256 MiB)`.
    pub max_retained_bytes: Option<u64>,
    /// The most bytes the result may hold: the per-node values (computed
    /// values, page values, authored writing modes and flags) for every node
    /// of the document, the computed values of the pseudo-elements, the SVG
    /// paint properties, and the kept candidates. Counted by inline size, not
    /// the heap the values own, such as the strings `var()` substitution
    /// builds or the lists inherited values copy; the per-node part is
    /// counted before it is allocated.
    ///
    /// Defaults to `Some(8 GiB)`. Every node costs about the size of one
    /// [`crate::ComputedValues`], a few kilobytes, and every pseudo-element
    /// with a style as much again, so the default admits a million-node
    /// document whose elements all have `::before` and `::after` styles, as
    /// a universal reset gives them.
    pub max_output_bytes: Option<u64>,
}

impl Default for CascadeLimits {
    fn default() -> Self {
        Self {
            max_candidates_per_element: Some(1 << 16),
            max_declarations_visited: Some(1 << 28),
            max_selector_tests: Some(1 << 30),
            max_retained_bytes: Some(256 << 20),
            max_output_bytes: Some(8 << 30),
        }
    }
}

/// How [`super::cascade_with_options`] runs.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CascadeOptions {
    /// Limits on the cascade's work and on its result's memory.
    pub limits: CascadeLimits,
    /// The viewport as `(width, height)` in CSS px: the basis of the
    /// viewport-percentage lengths (`vw`, `vh`, `vmin`, `vmax`, ...).
    ///
    /// CSS Values 4 §6.1.2 makes them relative to the initial containing
    /// block, which in paged media is the page area of the first page, not
    /// the page box media queries see. `None` uses the media context's
    /// viewport.
    pub viewport: Option<(f32, f32)>,
}

/// A count against one of the limits. It keeps what is left before the
/// limit, so that counting on the cascade's hot paths is one comparison and
/// one subtraction.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Counter {
    /// What can still be added: the limit less the count, or, without a
    /// limit, `u64::MAX` less the count.
    left: u64,
    limit: Option<u64>,
    kind: CascadeLimitKind,
}

impl Counter {
    pub(crate) fn new(limit: Option<u64>, kind: CascadeLimitKind) -> Self {
        Self {
            left: limit.unwrap_or(u64::MAX),
            limit,
            kind,
        }
    }

    /// Adds `amount`, failing instead, and adding nothing, when the count
    /// would pass the limit. Without a limit the count saturates.
    #[inline]
    pub(crate) fn add(&mut self, amount: u64) -> Result<(), CascadeError> {
        if amount > self.left {
            return self.overflow(amount);
        }
        self.left -= amount;
        Ok(())
    }

    /// The count so far.
    pub(crate) fn count(&self) -> u64 {
        self.limit.unwrap_or(u64::MAX) - self.left
    }

    /// What can still be added.
    pub(crate) fn left(&self) -> u64 {
        self.left
    }

    /// Whether adding `amount` would succeed: it fits in what is left, or
    /// there is no limit and the count saturates.
    pub(crate) fn fits(&self, amount: u64) -> bool {
        amount <= self.left || self.limit.is_none()
    }

    /// Starts counting again from zero.
    pub(crate) fn reset(&mut self) {
        self.left = self.limit.unwrap_or(u64::MAX);
    }

    /// The kind, the limit and the count reached for adding `amount`, which
    /// passes what is left.
    pub(crate) fn exceeding(&self, amount: u64) -> (CascadeLimitKind, u64, u64) {
        (
            self.kind,
            self.limit.unwrap_or(u64::MAX),
            self.count().saturating_add(amount),
        )
    }

    /// The error for adding `amount`, which passes what is left.
    #[cold]
    pub(crate) fn past(&self, amount: u64) -> CascadeError {
        let (kind, limit, actual) = self.exceeding(amount);
        CascadeError::LimitExceeded {
            kind,
            limit,
            actual,
        }
    }

    #[cold]
    #[inline(never)]
    fn overflow(&mut self, amount: u64) -> Result<(), CascadeError> {
        if self.limit.is_some() {
            return Err(self.past(amount));
        }
        self.left = 0;
        Ok(())
    }
}

/// What one walk counted against its limits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WalkCounts {
    /// The most candidates one element collected.
    pub(crate) max_element_candidates: u64,
    pub(crate) declarations_visited: u64,
    pub(crate) selector_tests: u64,
    pub(crate) retained_bytes: u64,
    pub(crate) output_bytes: u64,
}

/// The candidates one cascade has collected, against the candidate limits.
pub(crate) struct CandidateBudget {
    /// Candidates of the element being collected, its pseudo-elements'
    /// included.
    element: Counter,
    /// The most candidates an earlier element collected.
    most: u64,
    /// Candidates of every element collected so far.
    visited: Counter,
}

impl CandidateBudget {
    pub(crate) fn new(limits: &CascadeLimits) -> Self {
        Self::with(
            limits.max_candidates_per_element.map(u64::from),
            limits.max_declarations_visited,
        )
    }

    /// No limits, for copies of candidates that were counted when they were
    /// collected.
    pub(crate) fn unlimited() -> Self {
        Self::with(None, None)
    }

    fn with(per_element: Option<u64>, visited: Option<u64>) -> Self {
        Self {
            element: Counter::new(per_element, CascadeLimitKind::CandidatesPerElement),
            most: 0,
            visited: Counter::new(visited, CascadeLimitKind::DeclarationsVisited),
        }
    }

    /// Starts counting the candidates of another element.
    pub(crate) fn start_element(&mut self) {
        self.most = self.most.max(self.element.count());
        self.element.reset();
    }

    /// The most candidates one element has collected.
    pub(crate) fn max_element(&self) -> u64 {
        self.most.max(self.element.count())
    }

    /// The candidates every element has collected.
    pub(crate) fn visited(&self) -> u64 {
        self.visited.count()
    }

    /// Accounts for `count` more candidates of the current element, failing
    /// before they are added when they would pass the per-element limit.
    #[inline]
    pub(crate) fn charge(&mut self, count: usize) -> Result<(), CascadeError> {
        self.element.add(count as u64)
    }

    /// How many more candidates the current element may collect.
    pub(crate) fn room(&self) -> usize {
        usize::try_from(self.element.left()).unwrap_or(usize::MAX)
    }

    /// The error for `count` more candidates of the current element, more
    /// than [`Self::room`].
    pub(crate) fn past_room(&self, count: usize) -> CascadeError {
        self.element.past(count as u64)
    }

    /// Adds the current element's candidates to those of every element,
    /// failing when they pass the total limit. Counting the total once per
    /// element keeps a single comparison on each push; the element's own
    /// limit bounds what it holds meanwhile.
    pub(crate) fn finish_element(&mut self) -> Result<(), CascadeError> {
        self.visited.add(self.element.count())
    }
}

/// The bytes one cascade's result holds so far, against the result limits.
pub(crate) struct ResultBudget {
    retained: Counter,
    output: Counter,
}

impl ResultBudget {
    pub(crate) fn new(limits: &CascadeLimits) -> Self {
        Self {
            retained: Counter::new(limits.max_retained_bytes, CascadeLimitKind::RetainedBytes),
            output: Counter::new(limits.max_output_bytes, CascadeLimitKind::OutputBytes),
        }
    }

    /// Accounts for `bytes` more of the result, failing before they are
    /// allocated when they would pass the output limit.
    pub(crate) fn output(&mut self, bytes: u64) -> Result<(), CascadeError> {
        self.output.add(bytes)
    }

    /// Accounts for `bytes` more of kept candidates, which are part of the
    /// result too.
    pub(crate) fn retained(&mut self, bytes: u64) -> Result<(), CascadeError> {
        self.retained.add(bytes)?;
        self.output.add(bytes)
    }

    /// Writes the bytes counted so far into `counts`.
    pub(crate) fn count_into(&self, counts: &mut WalkCounts) {
        counts.retained_bytes = self.retained.count();
        counts.output_bytes = self.output.count();
    }
}

/// The inline bytes of `count` values of `T`.
pub(crate) fn bytes_of<T>(count: usize) -> u64 {
    (count as u64).saturating_mul(std::mem::size_of::<T>() as u64)
}

// cov:ignore: allocator failure is nondeterministic; this path fails closed
/// The error for an allocation of `count` values of `T` the allocator refused.
pub(crate) fn exhausted<T>(count: usize) -> CascadeError {
    CascadeError::ResourceExhausted {
        bytes: bytes_of::<T>(count),
    }
}

/// A vector of `len` clones of `value`, or an error when the allocator
/// refuses its buffer.
pub(crate) fn try_filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>, CascadeError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(len)
        .map_err(|_| exhausted::<T>(len))?;
    values.resize(len, value);
    Ok(values)
}

#[cfg(test)]
mod tests;
