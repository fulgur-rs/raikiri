//! Error accumulation.
//!
//! The macro never stops at the first mistake: every error is collected and
//! emitted as a `compile_error!` next to a best-effort expansion, so a typo
//! in one entry is reported once and does not break the code that uses the
//! other entries (or even the broken entry's own variant and field).

use proc_macro2::TokenStream;

/// Every error found during one expansion, in source order of discovery.
#[derive(Default)]
pub(crate) struct Errors(Option<syn::Error>);

impl Errors {
    /// Records `error`.
    pub(crate) fn push(&mut self, error: syn::Error) {
        match &mut self.0 {
            Some(first) => first.combine(error),
            None => self.0 = Some(error),
        }
    }

    /// Whether nothing has been recorded.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_none()
    }

    /// The recorded errors as `compile_error!` invocations (empty when there
    /// are none).
    pub(crate) fn to_compile_errors(&self) -> TokenStream {
        self.0
            .as_ref()
            .map(syn::Error::to_compile_error)
            .unwrap_or_default()
    }

    /// The recorded errors, one per message.
    #[cfg(test)]
    pub(crate) fn into_vec(self) -> Vec<syn::Error> {
        self.0.map(|e| e.into_iter().collect()).unwrap_or_default()
    }
}
