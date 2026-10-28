//! Keys for one parse's face/request contexts.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use super::{SvgFontFamily, SvgFontRequest};
use raikiri_traits::vector::FontFace;

/// Address equality and face index match [`FontFace::is_same_face`]. Owning
/// entries keep each indexed allocation alive until the indexes are discarded.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct FaceIdentity {
    address: usize,
    index: u32,
}

impl FaceIdentity {
    pub(super) fn of(face: &FontFace) -> Self {
        Self {
            address: Arc::as_ptr(face.data()) as *const () as usize,
            index: face.index(),
        }
    }
}

enum RequestKey<'a> {
    Borrowed(&'a SvgFontRequest),
    Shared(Arc<SvgFontRequest>),
}

impl RequestKey<'_> {
    fn get(&self) -> &SvgFontRequest {
        match self {
            Self::Borrowed(request) => request,
            Self::Shared(request) => request,
        }
    }
}

impl PartialEq for RequestKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.get(), other.get());
        // request() generates nine positive finite stretches, where bit and
        // float equality agree. This is not Eq for arbitrary public requests.
        a.families == b.families
            && a.weight == b.weight
            && a.stretch.to_bits() == b.stretch.to_bits()
            && a.style == b.style
    }
}
impl Eq for RequestKey<'_> {}

impl Hash for RequestKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let request = self.get();
        request.families.len().hash(state);
        for family in &request.families {
            std::mem::discriminant(family).hash(state);
            if let SvgFontFamily::Named(name) = family {
                name.hash(state);
            }
        }
        request.weight.hash(state);
        request.stretch.to_bits().hash(state);
        std::mem::discriminant(&request.style).hash(state);
    }
}

#[derive(PartialEq, Eq, Hash)]
pub(super) struct ContextKey<'a> {
    face: FaceIdentity,
    request: RequestKey<'a>,
}

impl<'a> ContextKey<'a> {
    pub(super) fn borrowed(face: FaceIdentity, request: &'a SvgFontRequest) -> Self {
        Self {
            face,
            request: RequestKey::Borrowed(request),
        }
    }
}

impl ContextKey<'static> {
    /// Stored keys share the Entry's request. Queries use Borrowed keys and
    /// immutable HashMap covariance, avoiding deep clones or new Arc allocations.
    pub(super) fn shared(face: FaceIdentity, request: Arc<SvgFontRequest>) -> Self {
        Self {
            face,
            request: RequestKey::Shared(request),
        }
    }
}

#[cfg(test)]
#[path = "key/tests.rs"]
mod tests;
