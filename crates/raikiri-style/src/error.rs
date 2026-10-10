//! Terminal errors from CSS parsing / cascading (moved during Phase B decoupling).
//!
//! Ownership moved from `raikiri-traits::error` to `raikiri-style` as part of
//! the Stylo-pattern decoupling — raikiri-style now owns its cascade error
//! taxonomy directly, and raikiri-traits re-exports the identity back so
//! `RenderError::Cascade(CascadeError)` at the umbrella surface stays stable.
//!
//! **Same responsibility boundary as Stylo/blitz**: Invalid rules / values
//! are silently dropped according to the CSS spec; they are not errors.
//! Errors specific to cssparser / selectors stay inside raikiri-style. This
//! enum exposes only signals for cases where raikiri-style explicitly fails hard:
//! an internal failure, a configured limit the input passes, or an allocation
//! the allocator refuses. CSS implementation details (property / value /
//! source location, etc.) do not leak into the trait layer. Add variants as
//! needed under `#[non_exhaustive]`.
#[non_exhaustive]
#[derive(Debug)]
pub enum CascadeError {
    /// An unrecoverable internal error in raikiri-style (a bug, or input
    /// disallowed by an explicit strict mode). raikiri-style constructs the
    /// detailed log and message internally.
    Internal {
        /// Human-readable failure details constructed inside raikiri-style.
        message: String,
    },
    /// The input needs more of something than a configured limit allows (see
    /// [`crate::CascadeLimits`] and [`crate::RuleTreeLimits`]). The counts
    /// depend only on the document and its stylesheets, so the same input
    /// fails the same way every time.
    LimitExceeded {
        /// Which limit the input passed.
        kind: CascadeLimitKind,
        /// The configured limit.
        limit: u64,
        /// The count the cascade reached when it stopped: the first value
        /// past `limit`, not the input's full need.
        actual: u64,
    },
    /// The allocator refused an allocation of this many bytes.
    ResourceExhausted {
        /// The size of the refused allocation.
        bytes: u64,
    },
}

/// Which of [`crate::CascadeLimits`] or [`crate::RuleTreeLimits`] an input
/// passed.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CascadeLimitKind {
    /// [`crate::CascadeLimits::max_candidates_per_element`].
    CandidatesPerElement,
    /// [`crate::CascadeLimits::max_declarations_visited`].
    DeclarationsVisited,
    /// [`crate::CascadeLimits::max_selector_tests`].
    SelectorTests,
    /// [`crate::CascadeLimits::max_retained_bytes`].
    RetainedBytes,
    /// [`crate::CascadeLimits::max_output_bytes`].
    OutputBytes,
    /// [`crate::RuleTreeLimits::max_rules`].
    StyleRules,
    /// [`crate::RuleTreeLimits::max_selectors`].
    StyleSelectors,
    /// [`crate::RuleTreeLimits::max_declarations`].
    StyleDeclarations,
}

impl std::fmt::Display for CascadeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal { message } => write!(f, "CSS cascade internal error: {message}"),
            Self::LimitExceeded {
                kind,
                limit,
                actual,
            } => write!(
                f,
                "CSS cascade limit exceeded: {kind:?} (limit={limit}, actual={actual})"
            ),
            Self::ResourceExhausted { bytes } => {
                write!(f, "CSS cascade could not allocate {bytes} bytes")
            }
        }
    }
}

impl std::error::Error for CascadeError {}
