//! Source ownership deltas, prepared without changing cache or budget state.

use std::collections::BTreeMap;
use std::sync::Arc;
use url::Url;

use super::accounting_work::AccountingWork;
use super::{CachedBackgroundImage, CachedBackgroundSource};

#[cfg(test)]
mod tests;

// A conservative estimate per live identity, including sparse BTree nodes.
// This is an accounting policy, not a standard library layout guarantee.
// Recheck dense, single-entry and empty allocation measurements on toolchain changes.
pub(super) const SOURCE_LEDGER_ENTRY_BYTES: u64 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceAllocation {
    references: usize,
    bytes: u64,
}

#[derive(Default)]
pub(super) struct SourceLedger {
    records: BTreeMap<usize, SourceAllocation>,
}

#[derive(Clone, Copy)]
pub(super) struct SourceChange {
    old: Option<(usize, SourceAllocation)>,
    new: Option<(usize, SourceAllocation)>,
    removed: u64,
    added: u64,
}

impl SourceChange {
    pub(super) fn apply_to(self, current: u64) -> Option<u64> {
        current.checked_sub(self.removed)?.checked_add(self.added)
    }
}

fn identity(source: &CachedBackgroundSource) -> Option<usize> {
    match source {
        CachedBackgroundSource::Svg(document) => Some(Arc::as_ptr(document) as usize),
        CachedBackgroundSource::Raster(_) => None,
    }
}

fn metadata(url: &Url) -> Option<u64> {
    let fixed = std::mem::size_of::<CachedBackgroundImage>()
        + std::mem::size_of::<Url>()
        + 2 * std::mem::size_of::<usize>();
    u64::try_from(fixed)
        .ok()?
        .checked_add(u64::try_from(url.as_str().len()).ok()?)
}

impl SourceLedger {
    pub(super) fn plan(
        &self,
        old: Option<&CachedBackgroundSource>,
        new: &CachedBackgroundSource,
        url: &Url,
        work: &AccountingWork,
    ) -> Option<SourceChange> {
        let old_id = old.and_then(identity);
        let new_id = identity(new);
        let mut change = SourceChange {
            old: None,
            new: None,
            removed: 0,
            added: 0,
        };
        if old_id == new_id {
            return Some(change);
        }
        if let Some(id) = old_id {
            work.record(1);
            let mut record = *self.records.get(&id)?;
            record.references = record.references.checked_sub(1)?;
            change.removed = metadata(url)?;
            if record.references == 0 {
                change.removed = change
                    .removed
                    .checked_add(record.bytes)?
                    .checked_add(SOURCE_LEDGER_ENTRY_BYTES)?;
            }
            change.old = Some((id, record));
        }
        if let CachedBackgroundSource::Svg(document) = new {
            let id = Arc::as_ptr(document) as usize;
            work.record(1);
            let record = match self.records.get(&id) {
                Some(record) => SourceAllocation {
                    references: record.references.checked_add(1)?,
                    bytes: record.bytes,
                },
                None => {
                    let bytes = document.source_only_retained_bytes()?;
                    change.added = bytes.checked_add(SOURCE_LEDGER_ENTRY_BYTES)?;
                    SourceAllocation {
                        references: 1,
                        bytes,
                    }
                }
            };
            change.added = change.added.checked_add(metadata(url)?)?;
            change.new = Some((id, record));
        }
        Some(change)
    }

    // Admission has completed before this method: integer-key updates cannot
    // return a recoverable budget error. Entries still own every recorded Arc.
    pub(super) fn apply(&mut self, change: SourceChange, work: &AccountingWork) {
        if let Some((id, record)) = change.old {
            work.record(1);
            if record.references == 0 {
                self.records.remove(&id);
            } else {
                self.records.insert(id, record);
            }
        }
        if let Some((id, record)) = change.new {
            work.record(1);
            self.records.insert(id, record);
        }
        if self.records.is_empty() {
            // BTreeMap may retain an empty root after its last removal.
            self.records = BTreeMap::new();
        }
    }

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    #[cfg(any(test, feature = "cache-accounting-diagnostics"))]
    pub(super) fn accounted_bytes(&self) -> u64 {
        (self.records.len() as u64).saturating_mul(SOURCE_LEDGER_ENTRY_BYTES)
    }
}
