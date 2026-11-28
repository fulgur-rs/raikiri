use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

const WORD: u64 = 8;
const BATCH: usize = 128;
const BUFFER_SIZE: usize = 4096;

#[derive(Debug)]
struct ReadBuffer {
    bytes: Box<[u8; BUFFER_SIZE]>,
    start: u64,
    len: usize,
}

impl ReadBuffer {
    fn new() -> Self {
        Self {
            bytes: Box::new([0; BUFFER_SIZE]),
            start: 0,
            len: 0,
        }
    }

    fn read(&mut self, file: &mut File, at: u64, dest: &mut [u8], end: u64) -> Option<()> {
        if at.checked_add(dest.len() as u64)? > end || dest.len() > BUFFER_SIZE {
            return None;
        }
        if let Some(offset) = at
            .checked_sub(self.start)
            .and_then(|d| usize::try_from(d).ok())
            && let Some(hit) = self.bytes[..self.len].get(offset..offset.checked_add(dest.len())?)
        {
            dest.copy_from_slice(hit);
            return Some(());
        }

        self.len = 0;
        self.start = at;
        file.seek(SeekFrom::Start(at)).ok()?;
        let limit = (end - at).min(BUFFER_SIZE as u64) as usize;
        while self.len < dest.len() {
            match file.read(&mut self.bytes[self.len..limit]) {
                Ok(0) => return None,
                Ok(n) => self.len += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            }
        }
        dest.copy_from_slice(&self.bytes[..dest.len()]);
        Some(())
    }
}

#[derive(Debug)]
pub(crate) struct Store(Mutex<Files>);

#[derive(Debug)]
struct Files {
    data: File,
    index: File,
    data_limit: u64,
    index_limit: u64,
    committed_end: u64,
    index_end: u64,
    offsets: BTreeMap<u64, u64>,
    pending: Vec<u8>,
    failed: bool,
    index_buffer: ReadBuffer,
    data_buffer: ReadBuffer,
}

impl Store {
    pub(crate) fn new(data: File, index: File, budget: u64) -> Self {
        let index_limit = budget / 4;
        Self(Mutex::new(Files {
            data,
            index,
            data_limit: budget - index_limit,
            index_limit,
            committed_end: 0,
            index_end: 0,
            offsets: BTreeMap::new(),
            pending: Vec::new(),
            failed: false,
            index_buffer: ReadBuffer::new(),
            data_buffer: ReadBuffer::new(),
        }))
    }

    pub(crate) fn can_write(&self, id: usize, len: usize) -> bool {
        self.0
            .lock()
            .is_ok_and(|files| files.available_slot(id, len).is_some())
    }

    pub(crate) fn write(&self, id: usize, bytes: &[u8]) {
        if let Ok(mut files) = self.0.lock() {
            files.write(id, bytes);
        }
    }

    pub(crate) fn read(&self, id: usize, bytes: &mut [u8]) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|mut files| files.read(id, bytes))
            .is_some()
    }
}

impl Files {
    fn slot(&self, id: usize) -> Option<u64> {
        let slot = u64::try_from(id).ok()?.checked_mul(WORD)?;
        (slot.checked_add(WORD)? <= self.index_limit).then_some(slot)
    }

    fn available_slot(&self, id: usize, len: usize) -> Option<u64> {
        let slot = self.slot(id)?;
        let remaining = self.data_limit - self.committed_end - self.pending.len() as u64;
        (!self.failed
            && len <= super::MAX_RECORD
            && remaining >= WORD + len as u64
            && !self.offsets.contains_key(&slot))
        .then_some(slot)
    }

    fn write(&mut self, id: usize, bytes: &[u8]) {
        let Some(slot) = self.available_slot(id, bytes.len()) else {
            return;
        };
        let start = self.committed_end + self.pending.len() as u64;
        let len = bytes.len() as u64;
        // A source may be revisited. Keep its first record without consuming data space.
        if slot < self.index_end {
            let mut offset = [0; WORD as usize];
            if self
                .index_buffer
                .read(&mut self.index, slot, &mut offset, self.index_end)
                .is_none()
            {
                self.failed = true;
                return;
            }
            if offset != [0; WORD as usize] {
                return;
            }
        }
        self.offsets.insert(slot, start + 1);
        self.pending.extend_from_slice(&len.to_le_bytes());
        self.pending.extend_from_slice(bytes);
        if self.offsets.len() == BATCH {
            self.flush(File::write_all);
        }
    }

    fn flush(&mut self, mut write: impl FnMut(&mut File, &[u8]) -> io::Result<()>) {
        if self.failed || self.pending.is_empty() {
            return;
        }
        self.index_buffer.len = 0;
        self.data_buffer.len = 0;
        let result = (|| {
            self.data.seek(SeekFrom::Start(self.committed_end))?;
            write(&mut self.data, &self.pending)?;
            let mut offsets = self.offsets.iter().peekable();
            let mut buffer = [0; BUFFER_SIZE];
            while let Some((&first, &offset)) = offsets.next() {
                buffer[..8].copy_from_slice(&offset.to_le_bytes());
                let mut used = 8;
                while let Some(&(&next, &offset)) = offsets.peek() {
                    let distance = next - first;
                    // Only fill gaps with zeroes beyond the existing index.
                    if distance > (BUFFER_SIZE - 8) as u64
                        || (distance != used as u64 && first + (used as u64) < self.index_end)
                    {
                        break;
                    }
                    let at = distance as usize;
                    buffer[used..at].fill(0);
                    buffer[at..at + 8].copy_from_slice(&offset.to_le_bytes());
                    used = at + 8;
                    offsets.next();
                }
                self.index.seek(SeekFrom::Start(first))?;
                write(&mut self.index, &buffer[..used])?;
                self.index_end = self.index_end.max(first + used as u64);
            }
            Ok::<_, io::Error>(())
        })();
        if result.is_err() {
            // Retain the slot list so partially written offsets are never used.
            self.failed = true;
            return;
        }
        self.committed_end += self.pending.len() as u64;
        self.pending.clear();
        self.offsets.clear();
    }

    fn read(&mut self, id: usize, bytes: &mut [u8]) -> Option<()> {
        let slot = self.slot(id)?;
        self.flush(File::write_all);
        if self.failed && self.offsets.contains_key(&slot) {
            return None;
        }
        let mut word = [0; WORD as usize];
        self.index_buffer
            .read(&mut self.index, slot, &mut word, self.index_end)?;
        let start = u64::from_le_bytes(word).checked_sub(1)?;
        let len = bytes.len() as u64;
        if start.checked_add(WORD)?.checked_add(len)? > self.committed_end {
            return None;
        }
        self.data_buffer
            .read(&mut self.data, start, &mut word, self.committed_end)?;
        if u64::from_le_bytes(word) != len {
            return None;
        }
        self.data_buffer
            .read(&mut self.data, start + WORD, bytes, self.committed_end)
    }
}

#[cfg(test)]
mod tests;
