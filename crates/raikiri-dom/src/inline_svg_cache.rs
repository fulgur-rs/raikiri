//! Document-owned, bounded memoization for the built-in inline SVG painter.
//!
//! The 32 MiB admission budget charges payload capacity, keys and live object
//! storage. The independent 256-entry bound also bounds deque spare capacity;
//! allocator overhead and caller-held clones are outside this accounting.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::Document;
use crate::inline_svg_style::InlineSvgElementDeclarations;

const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_ENTRIES: usize = 256;

/// Shared straight-alpha pixels used by the built-in SVG painter.
#[derive(Clone, Debug)]
#[doc(hidden)]
pub struct InlineSvgRaster {
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// Shared pixel allocation; consumers must not copy it on a cache hit.
    pub rgba: Arc<Vec<u8>>,
}

#[derive(Debug)]
struct RasterEntry {
    node: usize,
    viewport: (u32, u32),
    style: String,
    /// The declarations of the subtree's elements that the raster was drawn
    /// with, part of the key beside `style`.
    descendants: Arc<[InlineSvgElementDeclarations]>,
    raster: InlineSvgRaster,
    bytes: usize,
}

#[derive(Debug)]
struct SourceEntry {
    node: usize,
    source: Arc<str>,
    bytes: usize,
}

#[derive(Default, Debug)]
struct State {
    sources: VecDeque<SourceEntry>,
    rasters: VecDeque<RasterEntry>,
    bytes: usize,
}

impl State {
    fn make_room(&mut self, bytes: usize) -> bool {
        if bytes > MAX_BYTES {
            return false;
        }
        while self.bytes.saturating_add(bytes) > MAX_BYTES
            || self.sources.len() + self.rasters.len() >= MAX_ENTRIES
        {
            let removed = if let Some(entry) = self.rasters.pop_front() {
                entry.bytes
            } else if let Some(entry) = self.sources.pop_front() {
                entry.bytes
            } else {
                break; // cov:ignore: empty queues imply self.bytes == 0, and incoming bytes were already bounded by MAX_BYTES
            };
            self.bytes -= removed;
        }
        true
    }
}

#[derive(Default, Debug)]
pub(crate) struct InlineSvgCache(Mutex<State>);

// A cloned document owns independent mutable DOM state. Start with no memo.
impl Clone for InlineSvgCache {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl InlineSvgCache {
    /// The memo, locked. A panic while it was held leaves it consistent, so a
    /// poisoned lock is taken over.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn clear(&mut self) {
        *self
            .0
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = State::default();
    }
}

impl Document {
    /// Serializes an SVG subtree once between DOM mutations when it fits the
    /// document's paint memo. Failures and oversized sources are not memoized.
    #[doc(hidden)]
    pub fn cached_svg_subtree(&self, node: usize) -> Result<Option<Arc<str>>, String> {
        let mut cache = self.inline_svg_cache.lock();
        if let Some(entry) = cache.sources.iter().find(|entry| entry.node == node) {
            return Ok(Some(Arc::clone(&entry.source)));
        }
        let Some(source) = self.serialize_svg_subtree(node)? else {
            return Ok(None);
        };
        let source: Arc<str> = source.into();
        let bytes = source
            .len()
            .saturating_add(std::mem::size_of::<SourceEntry>() + 2 * std::mem::size_of::<usize>());
        if cache.make_room(bytes) {
            cache.sources.push_back(SourceEntry {
                node,
                source: Arc::clone(&source),
                bytes,
            });
            cache.bytes += bytes;
        }
        Ok(Some(source))
    }

    /// Finds pixels for an exact viewport and exact resolved style key.
    /// The caller includes every input to its SVG rasterizer in `style` and in
    /// the declarations it recognizes with `descendants`, which is asked about
    /// the declarations of each candidate, the most recently used first, and
    /// says whether they are the ones now in force.
    ///
    /// `descendants` runs without the memo locked, so that a long comparison
    /// does not hold up other paints of the document.
    #[doc(hidden)]
    pub fn cached_inline_svg_raster(
        &self,
        node: usize,
        viewport: (u32, u32),
        style: &str,
        mut descendants: impl FnMut(&[InlineSvgElementDeclarations]) -> bool,
    ) -> Option<InlineSvgRaster> {
        let candidates: Vec<_> = self
            .inline_svg_cache
            .lock()
            .rasters
            .iter()
            .rev()
            .filter(|entry| {
                entry.node == node && entry.viewport == viewport && entry.style == style
            })
            .map(|entry| (Arc::clone(&entry.descendants), entry.raster.clone()))
            .collect();
        let (found, raster) = candidates
            .into_iter()
            .find(|(declarations, _)| descendants(declarations))?;
        // The entry is the most recently used again, unless it was evicted
        // meanwhile.
        let mut cache = self.inline_svg_cache.lock();
        if let Some(at) = cache.rasters.iter().position(|entry| {
            entry.node == node
                && entry.viewport == viewport
                && entry.style == style
                && Arc::ptr_eq(&entry.descendants, &found)
        }) && let Some(entry) = cache.rasters.remove(at)
        {
            cache.rasters.push_back(entry);
        }
        Some(raster)
    }

    /// Keeps successful pixels if their allocation, key and metadata fit the
    /// document's 32 MiB accounting / 256-entry memo. Caller-held clones after eviction
    /// are outside this cache-retained bound. Failures must not be inserted.
    #[doc(hidden)]
    pub fn cache_inline_svg_raster(
        &self,
        node: usize,
        viewport: (u32, u32),
        style: String,
        descendants: Vec<InlineSvgElementDeclarations>,
        raster: InlineSvgRaster,
    ) {
        let bytes = raster
            .rgba
            .capacity()
            .saturating_add(style.capacity())
            .saturating_add(
                descendants
                    .iter()
                    .map(|element| {
                        element
                            .declarations
                            .capacity()
                            .saturating_add(std::mem::size_of::<InlineSvgElementDeclarations>())
                    })
                    .sum::<usize>(),
            )
            .saturating_add(
                std::mem::size_of::<RasterEntry>()
                    + std::mem::size_of::<Vec<u8>>()
                    + 2 * std::mem::size_of::<usize>(),
            );
        let mut cache = self.inline_svg_cache.lock();
        if !cache.make_room(bytes) {
            return;
        }
        if let Some(at) = cache.rasters.iter().position(|entry| {
            entry.node == node
                && entry.viewport == viewport
                && entry.style == style
                && *entry.descendants == *descendants
        }) {
            let previous = cache.rasters.remove(at).expect("the entry was found");
            cache.bytes -= previous.bytes;
        }
        cache.rasters.push_back(RasterEntry {
            node,
            viewport,
            style,
            descendants: descendants.into(),
            raster,
            bytes,
        });
        cache.bytes += bytes;
    }
}

#[cfg(test)]
mod tests;
