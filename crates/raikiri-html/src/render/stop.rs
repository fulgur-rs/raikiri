//! The ways a stage of a layout run ends the run before it leaves its output.

use super::{PipelineRun, Run};
use raikiri_traits::{LayoutError, RenderError};

/// Why a stage of the run ends it before the stage leaves its output.
pub(super) enum Stop {
    /// The run was cancelled.
    Aborted,
    /// The layout could not resume at the [`super::ResumePoint`] it was given.
    NotResumed,
    Failed(RenderError),
}

impl From<LayoutError> for Stop {
    fn from(error: LayoutError) -> Self {
        match error {
            LayoutError::Aborted => Self::Aborted,
            error => Self::Failed(RenderError::from(error)),
        }
    }
}

impl From<RenderError> for Stop {
    fn from(error: RenderError) -> Self {
        Self::Failed(error)
    }
}

impl Stop {
    /// The end of a run that stopped.
    pub(super) fn into_run(self) -> Result<PipelineRun, RenderError> {
        match self {
            Self::Aborted => Ok(PipelineRun::Aborted),
            Self::NotResumed => Ok(PipelineRun::NotResumed),
            Self::Failed(error) => Err(error),
        }
    }
}

impl Run<'_, '_> {
    /// Stop the run if it was cancelled.
    pub(super) fn check_aborted(&self) -> Result<(), Stop> {
        if (self.is_aborted)() {
            Err(Stop::Aborted)
        } else {
            Ok(())
        }
    }
}
