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
//! enum exposes only signals for cases where raikiri-style explicitly fails hard.
//!
//! Currently only the `Internal` variant is populated. CSS implementation
//! details (property / value / source location, etc.) do not leak into the
//! trait layer. Add variants as needed under `#[non_exhaustive]`.
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
}

impl std::fmt::Display for CascadeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Internal { message } => write!(f, "CSS cascade internal error: {message}"),
        }
    }
}

impl std::error::Error for CascadeError {}
