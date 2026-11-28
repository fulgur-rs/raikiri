use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::sync::Mutex;

const WORD: u64 = 8;

#[derive(Debug)]
pub(crate) struct Store(Mutex<Files>);

#[derive(Debug)]
struct Files {
    data: File,
    index: File,
    data_limit: u64,
    index_limit: u64,
    committed_end: u64,
    state: State,
}

#[derive(Debug, PartialEq)]
enum State {
    Open,
    Full,
    // A partial index overwrite could otherwise look like a valid offset.
    Failed { slot: u64 },
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
            state: State::Open,
        }))
    }

    pub(crate) fn can_write(&self) -> bool {
        self.0.lock().is_ok_and(|files| files.state == State::Open)
    }

    pub(crate) fn write(&self, id: usize, bytes: &[u8]) {
        if let Ok(mut files) = self.0.lock() {
            files.write(id, bytes, File::write_all);
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

    fn write(
        &mut self,
        id: usize,
        bytes: &[u8],
        mut write: impl FnMut(&mut File, &[u8]) -> io::Result<()>,
    ) {
        if self.state != State::Open {
            return;
        }
        let slot = self.slot(id);
        let len = bytes.len() as u64;
        let end = self
            .committed_end
            .checked_add(WORD)
            .and_then(|at| at.checked_add(len));
        let (Some(slot), Some(end)) = (slot, end.filter(|&end| end <= self.data_limit)) else {
            self.state = State::Full;
            return;
        };

        let start = self.committed_end;
        // Sparse index holes are zero, so offsets are stored one-based.
        let offset = (start + 1).to_le_bytes();
        let result = (|| {
            self.data.seek(SeekFrom::Start(start))?;
            write(&mut self.data, &len.to_le_bytes())?;
            write(&mut self.data, bytes)?;
            self.index.seek(SeekFrom::Start(slot))?;
            write(&mut self.index, &offset)
        })();
        if result.is_ok() {
            self.committed_end = end;
        } else {
            self.state = State::Failed { slot };
        }
    }

    fn read(&mut self, id: usize, bytes: &mut [u8]) -> Option<()> {
        let slot = self.slot(id)?;
        if self.state == (State::Failed { slot }) {
            return None;
        }
        self.index.seek(SeekFrom::Start(slot)).ok()?;
        let mut word = [0; WORD as usize];
        self.index.read_exact(&mut word).ok()?;
        let start = u64::from_le_bytes(word).checked_sub(1)?;
        let len = bytes.len() as u64;
        if start.checked_add(WORD)?.checked_add(len)? > self.committed_end {
            return None;
        }
        self.data.seek(SeekFrom::Start(start)).ok()?;
        self.data.read_exact(&mut word).ok()?;
        if u64::from_le_bytes(word) != len {
            return None;
        }
        self.data.read_exact(bytes).ok()
    }
}

#[cfg(test)]
mod tests;
