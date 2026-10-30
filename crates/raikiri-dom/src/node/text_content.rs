//! Shared text prefixes with amortized suffix appends and a lazy borrowed view.

use smol_str::{SmolStr, SmolStrBuilder};
use std::fmt;
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

// SmolStr's inline buffer holds up to 23 UTF-8 bytes.
const INLINE_BYTES: usize = 23;

#[derive(Clone)]
pub(crate) struct TextContent(Repr);

#[derive(Clone)]
enum Repr {
    Plain(SmolStr),
    Appended(Arc<PendingText>),
}

struct PendingText {
    prefix: SmolStr,
    suffix: String,
    joined: OnceLock<String>,
}

impl Clone for PendingText {
    fn clone(&self) -> Self {
        Self {
            prefix: self.prefix.clone(),
            suffix: self.suffix.clone(),
            // A writer will invalidate this view immediately. Copy only the
            // suffix when branching from a snapshot, never the joined prefix.
            joined: OnceLock::new(),
        }
    }
}

impl From<SmolStr> for TextContent {
    fn from(value: SmolStr) -> Self {
        Self(Repr::Plain(value))
    }
}

impl TextContent {
    pub(crate) fn len(&self) -> usize {
        match &self.0 {
            Repr::Plain(value) => value.len(),
            Repr::Appended(value) => value.prefix.len() + value.suffix.len(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn as_str(&self) -> &str {
        match &self.0 {
            Repr::Plain(value) => value.as_str(),
            Repr::Appended(value) => value
                .joined
                .get_or_init(|| {
                    let mut joined = String::with_capacity(self.len());
                    joined.push_str(value.prefix.as_str());
                    joined.push_str(&value.suffix);
                    joined
                })
                .as_str(),
        }
    }

    pub(crate) fn append(&mut self, addition: &str) {
        if addition.is_empty() {
            return;
        }
        let length = self
            .len()
            .checked_add(addition.len())
            .expect("text length overflow");
        if self.is_empty() {
            *self = SmolStr::new(addition).into();
            return;
        }
        if length <= INLINE_BYTES {
            let mut joined = SmolStrBuilder::new();
            joined.push_str(self.as_str());
            joined.push_str(addition);
            self.0 = Repr::Plain(joined.finish());
            return;
        }
        match &mut self.0 {
            Repr::Plain(prefix) => {
                self.0 = Repr::Appended(Arc::new(PendingText {
                    prefix: prefix.clone(),
                    suffix: addition.to_owned(),
                    joined: OnceLock::new(),
                }));
            }
            Repr::Appended(value) => {
                let value = Arc::make_mut(value);
                drop(value.joined.take());
                value.suffix.push_str(addition);
            }
        }
    }
}

impl Deref for TextContent {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Debug for TextContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), formatter)
    }
}

#[cfg(test)]
mod tests;
