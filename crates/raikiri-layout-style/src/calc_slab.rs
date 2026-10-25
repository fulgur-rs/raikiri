//! The table that gives every `calc()` payload a handle which can be checked.
//!
//! The layout library stores a length as a small `Copy` value and hands its
//! `calc()` handle back to a callback that resolves it, so a handle is one
//! machine word and carries no lifetime. A handle is not an address here: it
//! names a slot of one table, together with the generation the slot had when
//! the handle was issued. Resolving a handle reads only that table, never
//! memory that an owner could have freed. When the payload is dropped its slot
//! is emptied and its generation changes, so every handle that is still around
//! finds an empty slot or another generation and resolves to nothing.
//!
//! Resolving takes no lock, so independent documents laid out on different
//! threads share nothing but read-only cache lines. A slot is written only
//! while it is free, under the lock that creating and dropping payloads take,
//! and a reader checks the slot's state word before and after it reads the
//! value (a sequence lock), so a handle that races with the reuse of its slot
//! is rejected rather than read torn.

use crate::calc::CalcValue;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering, fence};
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

/// The low bits of a handle that the layout library keeps for its own tag; a
/// handle has them clear.
const TAG_BITS: u32 = 3;
const TAG_MASK: usize = (1 << TAG_BITS) - 1;
/// The bits of a handle left for the slot index and the generation.
const IDENTITY_BITS: u32 = usize::BITS - TAG_BITS;
const GENERATION_BITS: u32 = if usize::BITS >= 64 { 29 } else { 8 };
const INDEX_BITS: u32 = IDENTITY_BITS - GENERATION_BITS;
const INDEX_MASK: usize = (1 << INDEX_BITS) - 1;
/// The largest generation. Generation 0 is never issued, so no handle is zero
/// and the all-zero word never names a payload.
const GENERATION_MASK: u64 = (1 << GENERATION_BITS) - 1;
const _: () = assert!(TAG_BITS + INDEX_BITS + GENERATION_BITS == usize::BITS);

/// The state word of a slot: bit 0 is set while the slot holds a payload, bit 1
/// records the payload's range policy, and the rest is the generation.
const LIVE: u64 = 1;
const NON_NEGATIVE: u64 = 2;
const GENERATION_SHIFT: u32 = 2;

/// The slots live in chunks that double in size, so a chunk never moves once
/// it exists and the first holds `1 << FIRST_CHUNK_BITS` slots.
const FIRST_CHUNK_BITS: u32 = 6;
const CHUNKS: usize = (INDEX_BITS - FIRST_CHUNK_BITS + 1) as usize;

static TABLE: Table = Table::new();

/// The slot of a payload in the table, which it frees when dropped.
#[derive(Debug)]
pub(crate) struct Slot(usize);

impl Slot {
    /// Store `value` in a free slot.
    pub(crate) fn new(value: CalcValue) -> Self {
        let handle = TABLE.insert(value);
        Self(handle.expect("the calc table is full"))
    }

    /// The handle of the slot.
    pub(crate) fn handle(&self) -> usize {
        self.0
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        TABLE.remove(self.0);
    }
}

/// The payload that `handle` names, or `None` when it names no live payload:
/// its payload was dropped, or it was never issued.
pub(crate) fn lookup(handle: usize) -> Option<CalcValue> {
    TABLE.get(handle)
}

#[derive(Debug)]
struct Entry {
    state: AtomicU64,
    value: AtomicU64,
}

impl Entry {
    /// A free slot of generation 1.
    fn new() -> Self {
        Self {
            state: AtomicU64::new(1 << GENERATION_SHIFT),
            value: AtomicU64::new(0),
        }
    }
}

#[derive(Debug)]
struct Table {
    chunks: [OnceLock<Box<[Entry]>>; CHUNKS],
    writer: Mutex<Writer>,
}

/// What creating and dropping payloads share: the first slot never used and
/// the freed slots, which are reused in the order they were freed so that a
/// slot's generation advances as slowly as it can.
#[derive(Debug)]
struct Writer {
    next: usize,
    free: VecDeque<usize>,
}

impl Writer {
    /// The next slot never used, if the index space has one left.
    fn fresh(&mut self) -> Option<usize> {
        (self.next <= INDEX_MASK).then(|| {
            self.next += 1;
            self.next - 1
        })
    }
}

impl Table {
    const fn new() -> Self {
        Self {
            chunks: [const { OnceLock::new() }; CHUNKS],
            writer: Mutex::new(Writer {
                next: 0,
                free: VecDeque::new(),
            }),
        }
    }

    fn writer(&self) -> MutexGuard<'_, Writer> {
        self.writer.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The slot at `index`, if its chunk exists.
    fn entry(&self, index: usize) -> Option<&Entry> {
        let (chunk, offset) = locate(index);
        self.chunks.get(chunk)?.get()?.get(offset)
    }

    /// Store `value` and return the handle of its slot, or `None` when every
    /// slot of the index space is in use.
    fn insert(&self, value: CalcValue) -> Option<usize> {
        let mut writer = self.writer();
        let index = writer.free.pop_front().or_else(|| writer.fresh())?;
        let (chunk, offset) = locate(index);
        let entries = self.chunks[chunk].get_or_init(|| new_chunk(chunk));
        let entry = &entries[offset];
        let free = entry.state.load(Ordering::Relaxed);
        // The slot was freed by a store of its state, which must be visible to
        // a reader before the new value is.
        fence(Ordering::Release);
        entry.value.store(value.to_bits(), Ordering::Relaxed);
        let range = if value.non_negative() {
            NON_NEGATIVE
        } else {
            0
        };
        entry.state.store(free | LIVE | range, Ordering::Release);
        Some(handle(free >> GENERATION_SHIFT, index))
    }

    /// Empty the slot that `handle` names, which the caller owns, and make
    /// every handle issued for it stale.
    fn remove(&self, handle: usize) {
        let mut writer = self.writer();
        let index = (handle >> TAG_BITS) & INDEX_MASK;
        if let Some(entry) = self.entry(index) {
            let state = entry.state.load(Ordering::Relaxed);
            let generation = next_generation(state >> GENERATION_SHIFT);
            entry
                .state
                .store(generation << GENERATION_SHIFT, Ordering::Release);
            writer.free.push_back(index);
        }
    }

    fn get(&self, handle: usize) -> Option<CalcValue> {
        let identity = handle >> TAG_BITS;
        let entry = self.entry(identity & INDEX_MASK)?;
        let issued = (identity >> INDEX_BITS) as u64;
        let before = entry.state.load(Ordering::Acquire);
        let current = before & LIVE != 0 && before >> GENERATION_SHIFT == issued;
        let bits = entry.value.load(Ordering::Relaxed);
        fence(Ordering::Acquire);
        // The slot was not reused while the value was read.
        let stable = entry.state.load(Ordering::Relaxed) == before;
        let untagged = handle & TAG_MASK == 0;
        (current && stable && untagged)
            .then(|| CalcValue::from_bits(bits, before & NON_NEGATIVE != 0))
    }
}

/// The chunk and the offset in it of the slot at `index`: chunk `n` holds
/// `1 << (n + FIRST_CHUNK_BITS)` slots.
fn locate(index: usize) -> (usize, usize) {
    let shifted = index + (1 << FIRST_CHUNK_BITS);
    let chunk = (shifted.ilog2() - FIRST_CHUNK_BITS) as usize;
    (chunk, shifted - (1 << (chunk as u32 + FIRST_CHUNK_BITS)))
}

fn new_chunk(chunk: usize) -> Box<[Entry]> {
    let len = 1_usize << (chunk as u32 + FIRST_CHUNK_BITS);
    (0..len).map(|_| Entry::new()).collect()
}

/// The handle of slot `index` at `generation`.
fn handle(generation: u64, index: usize) -> usize {
    (((generation as usize) << INDEX_BITS) | index) << TAG_BITS
}

fn next_generation(generation: u64) -> u64 {
    if generation == GENERATION_MASK {
        1
    } else {
        generation + 1
    }
}

#[cfg(test)]
mod tests;
