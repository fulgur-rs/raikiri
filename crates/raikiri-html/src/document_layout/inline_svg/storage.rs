//! Bounded cache-owned XML and metadata, separate from image payload budgets.
//!
//! XML is charged once across root and picture ownership. The charge includes
//! a three-word Arc/header/alignment allowance and actual Vec capacities, not
//! allocator overhead or RSS. Font caches, transient builds and caller-owned
//! images are separate. Only successful picture admission can evict pictures.

use std::mem::size_of;
use std::sync::Arc;

use super::{
    ImageContent, InlineSvgCache, MAX_KEPT_RASTER_BYTES, MAX_KEPT_SCENE_BYTES,
    MAX_KEPT_STORAGE_BYTES, MAX_PICTURES, MAX_PREPARED_ROOTS, PictureKey, PreparedSvg, SvgKey,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct Limits {
    pub(super) roots: usize,
    pub(super) pictures: usize,
    pub(super) storage: usize,
    pub(super) scenes: usize,
    pub(super) rasters: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            roots: MAX_PREPARED_ROOTS,
            pictures: MAX_PICTURES,
            storage: MAX_KEPT_STORAGE_BYTES,
            scenes: MAX_KEPT_SCENE_BYTES,
            rasters: MAX_KEPT_RASTER_BYTES,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Payload {
    Scene(usize),
    Raster(usize),
}

impl Payload {
    fn of(content: &ImageContent) -> Self {
        match content {
            ImageContent::Vector(scene) => Self::Scene(scene.estimated_heap_bytes()),
            ImageContent::Raster(pixels) => Self::Raster(pixels.rgba.len()),
        }
    }
}

#[derive(Debug)]
struct KeptPicture {
    key: PictureKey,
    content: ImageContent,
    payload: Payload,
}

#[derive(Debug)]
pub(super) struct Storage {
    roots: Vec<(SvgKey, PreparedSvg)>,
    pictures: Vec<KeptPicture>,
    limits: Limits,
    scene_bytes: usize,
    raster_bytes: usize,
}

impl Storage {
    pub(super) fn new(limits: Limits) -> Self {
        assert!(limits.roots <= MAX_PREPARED_ROOTS && limits.pictures <= MAX_PICTURES);
        Self {
            roots: Vec::with_capacity(limits.roots),
            pictures: Vec::with_capacity(limits.pictures),
            limits,
            scene_bytes: 0,
            raster_bytes: 0,
        }
    }

    pub(super) fn get(&self, key: &SvgKey) -> Option<PreparedSvg> {
        self.roots
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| value.clone())
    }

    fn canonical(&self, source: &Arc<str>, hash: u64) -> Arc<str> {
        self.roots
            .iter()
            .map(|(_, value)| (&value.source, value.source_hash))
            .chain(
                self.pictures
                    .iter()
                    .map(|value| (&value.key.source, value.key.source_hash)),
            )
            .find(|(stored, stored_hash)| {
                *stored_hash == hash
                    && (Arc::ptr_eq(stored, source) || stored.as_ref() == source.as_ref())
            })
            .map_or_else(|| Arc::clone(source), |(stored, _)| Arc::clone(stored))
    }

    pub(super) fn insert(&mut self, key: SvgKey, mut value: PreparedSvg) -> PreparedSvg {
        value.source = self.canonical(&value.source, value.source_hash);
        // Invalidate an obsolete replacement slot even if the new source is
        // not admitted. Unrelated pictures are never evicted before a build.
        if let Some(index) = self.roots.iter().position(|(stored, _)| *stored == key) {
            self.roots.remove(index);
        }
        if self.limits.roots == 0
            || !self.source_fits_alone(&value.source)
            || self.storage_with(Some(&value.source), false) > self.limits.storage
        {
            return value;
        }
        while self.roots.len() >= self.limits.roots {
            self.roots.remove(0);
        }
        while self.storage_with(Some(&value.source), true) > self.limits.storage {
            let source = source_id(&self.roots[0].1.source);
            self.roots
                .retain(|(_, value)| source_id(&value.source) != source);
        }
        self.roots.push((key, value.clone()));
        value
    }

    fn picture_index(&self, key: &PictureKey) -> Option<usize> {
        self.pictures
            .iter()
            .position(|value| value.key.fingerprint == key.fingerprint && value.key == *key)
    }

    pub(super) fn picture(&self, key: &PictureKey) -> Option<ImageContent> {
        self.picture_index(key)
            .map(|index| self.pictures[index].content.clone())
    }

    pub(super) fn keep(&mut self, mut key: PictureKey, content: &ImageContent) {
        if self.picture_index(&key).is_some() || self.limits.pictures == 0 {
            return;
        }
        key.source = self.canonical(&key.source, key.source_hash);
        let payload = Payload::of(content);
        let fits_payload = match payload {
            Payload::Scene(bytes) => self
                .scene_bytes
                .checked_add(bytes)
                .is_some_and(|sum| sum <= self.limits.scenes),
            Payload::Raster(bytes) => self
                .raster_bytes
                .checked_add(bytes)
                .is_some_and(|sum| sum <= self.limits.rasters),
        };
        // Preserve the existing payload-admission policy. Rejecting a single
        // oversized source or payload cannot evict useful cached pictures.
        if !fits_payload || !self.source_fits_alone(&key.source) {
            return;
        }
        while self.pictures.len() >= self.limits.pictures {
            let old = self.pictures.remove(0);
            self.uncharge(old.payload);
        }
        while self.storage_with(Some(&key.source), true) > self.limits.storage {
            let source = self
                .roots
                .first()
                .map(|(_, value)| source_id(&value.source))
                .unwrap_or_else(|| source_id(&self.pictures[0].key.source));
            self.evict_source(source);
        }
        match payload {
            Payload::Scene(bytes) => self.scene_bytes += bytes,
            Payload::Raster(bytes) => self.raster_bytes += bytes,
        }
        self.pictures.push(KeptPicture {
            key,
            content: content.clone(),
            payload,
        });
    }

    fn uncharge(&mut self, payload: Payload) {
        match payload {
            Payload::Scene(bytes) => self.scene_bytes -= bytes,
            Payload::Raster(bytes) => self.raster_bytes -= bytes,
        }
    }

    fn evict_source(&mut self, source: usize) {
        self.roots
            .retain(|(_, value)| source_id(&value.source) != source);
        let mut index = 0;
        while index < self.pictures.len() {
            if source_id(&self.pictures[index].key.source) == source {
                let old = self.pictures.remove(index);
                self.uncharge(old.payload);
            } else {
                index += 1;
            }
        }
    }

    fn source_fits_alone(&self, source: &str) -> bool {
        self.metadata_bytes()
            .checked_add(source_charge(source))
            .is_some_and(|bytes| bytes <= self.limits.storage)
    }

    fn storage_with(&self, extra: Option<&Arc<str>>, roots: bool) -> usize {
        self.metadata_bytes()
            .saturating_add(self.source_bytes(extra, roots))
    }

    pub(super) fn metadata_bytes(&self) -> usize {
        size_of::<InlineSvgCache>()
            + self.roots.capacity() * size_of::<(SvgKey, PreparedSvg)>()
            + self.pictures.capacity() * size_of::<KeptPicture>()
    }

    fn source_bytes(&self, extra: Option<&Arc<str>>, roots: bool) -> usize {
        // The bounded scratch array is transient stack storage, not retained
        // cache metadata. Sorting makes union accounting O(K log K).
        let mut sources = [(0usize, 0usize); MAX_PREPARED_ROOTS + MAX_PICTURES + 1];
        let mut used = 0;
        if roots {
            for (_, value) in &self.roots {
                sources[used] = (source_id(&value.source), source_charge(&value.source));
                used += 1;
            }
        }
        for value in &self.pictures {
            sources[used] = (
                source_id(&value.key.source),
                source_charge(&value.key.source),
            );
            used += 1;
        }
        if let Some(source) = extra {
            sources[used] = (source_id(source), source_charge(source));
            used += 1;
        }
        sources[..used].sort_unstable_by_key(|&(pointer, _)| pointer);
        let mut previous = None;
        let mut total = 0usize;
        for &(pointer, bytes) in &sources[..used] {
            if previous != Some(pointer) {
                total = total.saturating_add(bytes);
                previous = Some(pointer);
            }
        }
        total
    }

    #[cfg(test)]
    pub(super) fn roots_len(&self) -> usize {
        self.roots.len()
    }

    #[cfg(test)]
    pub(super) fn pictures_len(&self) -> usize {
        self.pictures.len()
    }

    #[cfg(test)]
    pub(super) fn charges(&self) -> (usize, usize, usize, usize) {
        (
            self.metadata_bytes(),
            self.source_bytes(None, true),
            self.scene_bytes,
            self.raster_bytes,
        )
    }

    #[cfg(test)]
    pub(super) fn limits_mut(&mut self) -> &mut Limits {
        &mut self.limits
    }
}

fn source_id(source: &Arc<str>) -> usize {
    Arc::as_ptr(source) as *const () as usize
}

pub(super) fn source_charge(source: &str) -> usize {
    source.len().saturating_add(3 * size_of::<usize>())
}

#[cfg(test)]
mod tests;
