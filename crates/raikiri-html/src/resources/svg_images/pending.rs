//! Completion of checked data-image requests shared by concurrent callers.

use super::super::ProcessDecodeGuard;
use super::{Arc, DecodedImage, Mutex, PoisonError, RenderSvgImages};
use std::sync::Condvar;
use std::thread::{self, ThreadId};

enum Outcome {
    Pending,
    Complete(Option<Arc<DecodedImage>>),
}

pub(super) struct DataFlight {
    owner: ThreadId,
    outcome: Mutex<Outcome>,
    changed: Condvar,
}

impl DataFlight {
    pub(super) fn new() -> Self {
        Self {
            owner: thread::current().id(),
            outcome: Mutex::new(Outcome::Pending),
            changed: Condvar::new(),
        }
    }

    pub(super) fn wait(&self) -> Option<Arc<DecodedImage>> {
        // A registered owner's policy callback cannot wait for itself. This
        // does not guard recursive callbacks before the key was registered.
        if self.owner == thread::current().id() {
            return None;
        }
        let mut outcome = self.outcome.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            match &*outcome {
                Outcome::Complete(image) => return image.clone(),
                Outcome::Pending => {
                    outcome = self
                        .changed
                        .wait(outcome)
                        .unwrap_or_else(PoisonError::into_inner)
                }
            }
        }
    }

    fn complete(&self, image: Option<Arc<DecodedImage>>) {
        *self.outcome.lock().unwrap_or_else(PoisonError::into_inner) = Outcome::Complete(image);
        self.changed.notify_all();
    }
}

pub(super) struct PendingData<'s, 'r, 'a> {
    source: &'s RenderSvgImages<'r, 'a>,
    index: usize,
    flight: Arc<DataFlight>,
    active: bool,
}

impl<'s, 'r, 'a> PendingData<'s, 'r, 'a> {
    pub(super) fn new(
        source: &'s RenderSvgImages<'r, 'a>,
        index: usize,
        flight: Arc<DataFlight>,
    ) -> Self {
        Self {
            source,
            index,
            flight,
            active: true,
        }
    }

    pub(super) fn finish(
        mut self,
        image: Option<Arc<DecodedImage>>,
        permit: ProcessDecodeGuard,
    ) -> Option<Arc<DecodedImage>> {
        // Account completed pixels before releasing the decoder slot. State
        // may be held by a URL request waiting for a slot of its own.
        let (image, pixels) = self.source.reserve_data_pixels(image);
        drop(permit);
        let image = self.source.complete_data(self.index, image, pixels);
        self.flight.complete(image.clone());
        self.active = false;
        image
    }
}

impl Drop for PendingData<'_, '_, '_> {
    fn drop(&mut self) {
        if self.active {
            self.source.complete_data(self.index, None, None);
            self.flight.complete(None);
        }
    }
}
