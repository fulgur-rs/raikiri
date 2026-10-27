//! Thread-local operation counts for annotation insertion regressions.

use std::cell::Cell;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Work {
    pub parent_entries: usize,
    pub moved_pieces: usize,
    pub end_updates: usize,
    pub fallback_lines: usize,
    pub batch_lines: usize,
    pub inspected: usize,
    pub copied: usize,
}

thread_local! {
    static WORK: Cell<Work> = Cell::new(Work::default());
    static LEGACY: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn record(update: impl FnOnce(&mut Work)) {
    WORK.with(|cell| {
        let mut work = cell.get();
        update(&mut work);
        cell.set(work);
    });
}

pub(super) fn take() -> Work {
    WORK.with(|cell| cell.replace(Work::default()))
}

pub(super) fn legacy_requested() -> bool {
    LEGACY.with(Cell::get)
}

pub(super) fn with_legacy<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            LEGACY.with(|cell| cell.set(self.0));
        }
    }
    let reset = Reset(LEGACY.with(|cell| cell.replace(true)));
    let result = run();
    drop(reset);
    result
}
