//! Change snapshots for live document base URL inputs.
use super::Document;
use std::sync::Arc;

/// An opaque snapshot of document identity and base URL inputs.
///
/// Compare snapshots together with the host's document URL when caching
/// [`Document::live_base_url`]. Topology changes, a base element's namespace
/// or null-namespace `href`, full base attribute replacement, and the
/// recorded base invalidate the snapshot. Unrelated topology also invalidates
/// it conservatively. Cloned and restored documents have distinct identities.
#[derive(Debug, Clone)]
pub struct BaseUrlVersion {
    identity: Arc<()>,
    generation: u64,
}
impl PartialEq for BaseUrlVersion {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity) && self.generation == other.generation
    }
}
impl Eq for BaseUrlVersion {}

#[derive(Debug)]
pub(super) struct Tracker {
    version: BaseUrlVersion,
}
impl Default for Tracker {
    fn default() -> Self {
        Self {
            version: BaseUrlVersion {
                identity: Arc::new(()),
                generation: 0,
            },
        }
    }
}
impl Clone for Tracker {
    fn clone(&self) -> Self {
        Self::default()
    }
}
impl Document {
    /// Identity and revision of the inputs to [`Self::live_base_url`] and
    /// the recorded [`Self::base_url`], read without allocation or a tree walk.
    pub fn base_url_version(&self) -> &BaseUrlVersion {
        &self.base_url_inputs.version
    }
    pub(super) fn note_base_url_change(&mut self) {
        if let Some(next) = self.base_url_inputs.version.generation.checked_add(1) {
            self.base_url_inputs.version.generation = next;
        } else {
            // A wrapped generation must never match an older snapshot.
            self.base_url_inputs = Tracker::default();
        }
    }
}
#[cfg(test)]
mod tests;
