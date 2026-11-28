//! Temporary glyph records indexed by body cell ordinal.

use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex};

use raikiri_dom::{MeasuredTableCell, TableGlyphRecorder};

const SLOT: u64 = 8;
const EMPTY: usize = usize::MAX;
const _: () = assert!(MeasuredTableCell::MAX_RECORD_BYTES <= u8::MAX as usize);

pub(super) struct WindowRecorder {
    first: usize,
    by_source: HashMap<usize, usize>,
    budget: u64,
    batch: Mutex<Batch>,
}

#[derive(Default)]
struct Batch {
    offsets: Vec<usize>,
    data: Vec<u8>,
}

impl fmt::Debug for WindowRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowRecorder")
            .field("first", &self.first)
            .field("cells", &self.by_source.len())
            .finish_non_exhaustive()
    }
}

impl WindowRecorder {
    fn accepts(&self, batch: &Batch, local: usize, len: usize) -> bool {
        let index_end = ((self.first + local) as u64 + 1) * SLOT;
        len <= MeasuredTableCell::MAX_RECORD_BYTES
            && batch.offsets.get(local) == Some(&EMPTY)
            && self.budget - index_end >= batch.data.len() as u64 + 1 + len as u64
    }
}

impl TableGlyphRecorder for WindowRecorder {
    fn can_write(&self, source: usize, len: usize) -> bool {
        self.by_source.get(&source).is_some_and(|&local| {
            self.batch
                .lock()
                .is_ok_and(|batch| self.accepts(&batch, local, len))
        })
    }

    fn write(&self, source: usize, bytes: &[u8]) {
        if let Some(&local) = self.by_source.get(&source)
            && let Ok(mut batch) = self.batch.lock()
            && self.accepts(&batch, local, bytes.len())
        {
            batch.offsets[local] = batch.data.len();
            batch.data.push(bytes.len() as u8);
            batch.data.extend_from_slice(bytes);
        }
    }
}

struct Stream {
    file: BufReader<File>,
    position: Option<u64>,
}

impl Stream {
    fn new(file: File) -> Self {
        Self {
            file: BufReader::new(file),
            position: Some(0),
        }
    }

    fn writer(&mut self, start: u64) -> io::Result<BufWriter<&mut File>> {
        self.position = None;
        self.file.seek(SeekFrom::Start(start))?;
        Ok(BufWriter::new(self.file.get_mut()))
    }

    fn read(&mut self, at: u64, bytes: &mut [u8]) -> io::Result<()> {
        let position = self.position.take();
        if position != Some(at) {
            if let Some(delta) =
                position.and_then(|pos| i64::try_from(i128::from(at) - i128::from(pos)).ok())
            {
                self.file.seek_relative(delta)?;
            } else {
                self.file.seek(SeekFrom::Start(at))?;
            }
        }
        self.file.read_exact(bytes)?;
        self.position = Some(at + bytes.len() as u64);
        Ok(())
    }
}

pub(super) struct GlyphStore {
    index: Stream,
    data: Stream,
    index_end: u64,
    data_end: u64,
    budget: u64,
    failed: bool,
}

impl GlyphStore {
    pub(super) fn new(budget: u64) -> Option<Self> {
        Self::with_files(budget, tempfile::tempfile)
    }

    fn with_files(budget: u64, mut create: impl FnMut() -> io::Result<File>) -> Option<Self> {
        if budget <= SLOT {
            return None;
        }
        Some(Self {
            index: Stream::new(create().ok()?),
            data: Stream::new(create().ok()?),
            index_end: 0,
            data_end: 0,
            budget,
            failed: false,
        })
    }

    pub(super) fn can_record_from(&self, first: usize) -> bool {
        let remaining = self.budget - self.data_end;
        !self.failed && (first as u64) < remaining.saturating_sub(1) / SLOT
    }

    pub(super) fn window(
        &self,
        first: usize,
        mut sources: Vec<usize>,
    ) -> Option<Arc<WindowRecorder>> {
        if !self.can_record_from(first) {
            return None;
        }
        let budget = self.budget - self.data_end;
        let slots = usize::try_from((budget - 1) / SLOT).unwrap_or(usize::MAX);
        sources.truncate(slots.saturating_sub(first));
        if sources.is_empty() {
            return None;
        }
        let count = sources.len();
        Some(Arc::new(WindowRecorder {
            first,
            by_source: sources
                .into_iter()
                .enumerate()
                .map(|(i, id)| (id, i))
                .collect(),
            budget,
            batch: Mutex::new(Batch {
                offsets: vec![EMPTY; count],
                data: Vec::new(),
            }),
        }))
    }

    pub(super) fn append(&mut self, window: &WindowRecorder) {
        let Ok(batch) = window
            .batch
            .lock()
            .map(|mut batch| std::mem::take(&mut *batch))
        else {
            return;
        };
        if !self.can_record_from(window.first) || batch.data.is_empty() {
            return;
        }
        let start = window.first as u64 * SLOT;
        // Chunks commit once, in body order. A failed append is beyond both committed ends.
        if start < self.index_end {
            return;
        }
        let mut data_end = self.data_end;
        let mut index_end = self.index_end;
        let write = (|| -> io::Result<()> {
            let mut data = self.data.writer(data_end)?;
            let mut index = self.index.writer(start)?;
            let mut next = start;
            for (local, &offset) in batch.offsets.iter().enumerate() {
                if offset == EMPTY {
                    continue;
                }
                let at = start + local as u64 * SLOT;
                let len = 1 + usize::from(batch.data[offset]);
                if (self.budget - at - SLOT).saturating_sub(data_end) < len as u64 {
                    continue;
                }
                // Account for holes inside this window without issuing a seek for each missing cell.
                while next < at {
                    index.write_all(&0u64.to_le_bytes())?;
                    next += SLOT;
                }
                index.write_all(&(data_end + 1).to_le_bytes())?;
                data.write_all(&batch.data[offset..offset + len])?;
                next = at + SLOT;
                index_end = next;
                data_end += len as u64;
            }
            data.flush()?;
            index.flush()
        })();
        if write.is_err() {
            self.failed = true;
        } else {
            self.data_end = data_end;
            self.index_end = index_end;
        }
    }

    pub(super) fn read(&mut self, ordinal: usize, bytes: &mut [u8]) -> bool {
        let read = (|| {
            let slot = u64::try_from(ordinal).ok()?.checked_mul(SLOT)?;
            if slot.checked_add(SLOT)? > self.index_end {
                return None;
            }
            let mut word = [0; SLOT as usize];
            self.index.read(slot, &mut word).ok()?;
            let at = u64::from_le_bytes(word).checked_sub(1)?;
            if at.checked_add(1)?.checked_add(bytes.len() as u64)? > self.data_end {
                return None;
            }
            let mut len = [0];
            self.data.read(at, &mut len).ok()?;
            if usize::from(len[0]) != bytes.len() {
                return None;
            }
            self.data.read(at + 1, bytes).ok()
        })();
        read.is_some()
    }
}

#[cfg(test)]
mod tests;
