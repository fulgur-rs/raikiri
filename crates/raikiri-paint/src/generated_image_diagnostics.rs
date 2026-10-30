//! Optional per-thread measurements of generated-image piece lookup.

use std::cell::Cell;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
/// Work performed while matching generated images to inline pieces.
pub struct ImagePieceWork {
    /// Pieces tested by the legacy first-match search.
    pub predicate_visits: u64,
    /// Pieces visited while building an index.
    pub index_visits: u64,
    /// Images with pixels for which a piece is requested.
    pub image_lookups: u64,
}

thread_local! {
    static ACTIVE: Cell<Option<ImagePieceWork>> = const { Cell::new(None) };
}

/// Start a fresh measurement on this thread.
pub fn begin() {
    ACTIVE.set(Some(ImagePieceWork::default()));
}

/// End this thread's measurement, returning zero when none was active.
pub fn finish() -> ImagePieceWork {
    ACTIVE.replace(None).unwrap_or_default()
}

pub(crate) fn enabled() -> bool {
    ACTIVE.get().is_some()
}

pub(crate) fn index_visit() {
    if let Some(mut work) = ACTIVE.get() {
        work.index_visits += 1;
        ACTIVE.set(Some(work));
    }
}

pub(crate) fn image_lookup() {
    if let Some(mut work) = ACTIVE.get() {
        work.image_lookups += 1;
        ACTIVE.set(Some(work));
    }
}
