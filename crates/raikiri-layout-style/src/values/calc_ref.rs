//! The reference a `calc()` length holds to its payload.

use crate::AlignedCalc;
use crate::calc_slab::lookup;
use std::sync::Arc;

/// A reference to the payload of a `calc()` length.
///
/// The reference is `Copy` and carries no lifetime: lengths are copied around
/// freely, and the payload they refer to is owned elsewhere. It names the
/// payload by a handle that is checked on every use, so it can never reach
/// freed memory:
///
/// - While the payload is alive, [`CalcRef::resolve`] resolves it.
/// - Once the last owner of the payload has dropped it, the reference is
///   stale. A stale reference resolves to `0.0` and has no percentage. It does
///   not resolve a payload that took over the storage: each reuse of a slot
///   changes the generation the handle carries, so a stale reference is
///   recognized until its slot has been reused 2^29 times (2^8 on 32-bit
///   targets), and freed slots are reused in the order they were freed.
///
/// A reference is made only from a payload: one that a getter of this crate
/// reads, or one that the adapter passes to [`CalcRef::retained`]. No caller
/// can build one from a raw pointer, so a null or dangling handle cannot be
/// made through safe code, and resolving a handle never dereferences it.
///
/// A length that a getter returned therefore stays safe to copy and resolve
/// after the owner of the payload it was read from has been dropped, but it
/// no longer resolves to the expression; keep the owner alive for as long as
/// the length is used to get its value.
///
/// ```compile_fail,E0624
/// use raikiri_layout_style::{AlignedCalc, values::CalcRef};
/// # use raikiri_style::property::CalcLengthPercentage;
/// let payload = AlignedCalc::new(CalcLengthPercentage { px: 1.0, percent: 0.0 }, true);
/// let _ = CalcRef::new(&payload);
/// ```
///
/// A raw pointer cannot stand in for a reference:
///
/// ```compile_fail,E0308
/// use raikiri_layout_style::values::Dimension;
/// let _ = Dimension::calc(std::ptr::null());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub struct CalcRef(usize);

impl CalcRef {
    /// A reference to `payload`.
    #[inline]
    pub(crate) fn new(payload: &AlignedCalc) -> Self {
        Self(payload.handle())
    }

    /// A reference to the payload that `owner` retains.
    ///
    /// The reference does not keep the payload alive: it resolves for as long
    /// as `owner`, or a clone of it, is retained, and is stale afterwards.
    #[inline]
    pub fn retained(owner: &Arc<AlignedCalc>) -> Self {
        Self::new(owner)
    }

    /// The reference whose handle is `handle`.
    ///
    /// Any handle is accepted: one that names no live payload gives a stale
    /// reference.
    #[inline]
    pub(crate) fn from_handle(handle: *const ()) -> Self {
        Self(handle.addr())
    }

    /// The handle of the payload, in the form the layout library keeps.
    ///
    /// The handle is an opaque word, not an address: it cannot be dereferenced,
    /// and [`resolve_projected_calc`] resolves it.
    #[doc(hidden)]
    #[inline]
    pub fn handle(self) -> *const () {
        std::ptr::without_provenance(self.0)
    }

    /// The word that identifies the payload.
    #[inline]
    pub(crate) fn addr(self) -> usize {
        self.0
    }

    /// The payload resolved against the percentage `basis`, with its range
    /// applied; `0.0` for a stale reference.
    #[inline]
    pub fn resolve(self, basis: f32) -> f32 {
        lookup(self.0).map_or(0.0, |payload| payload.resolve(basis))
    }

    /// Whether the payload has a percentage term, which needs a basis to
    /// resolve; `false` for a stale reference.
    #[inline]
    pub fn has_percentage(self) -> bool {
        lookup(self.0).is_some_and(|payload| payload.has_percentage())
    }

    /// Whether the payload is still alive, so that the reference resolves to
    /// its expression.
    #[inline]
    pub fn is_live(self) -> bool {
        lookup(self.0).is_some()
    }
}

/// Resolve the calc handle that the layout library hands back from a length
/// of the projection.
///
/// This is the entry for the library's callback that resolves the lengths of
/// a style it was given. Any handle is accepted: one that names no live
/// payload, because its payload was dropped or because it was never issued,
/// resolves to `0.0`.
#[inline]
#[doc(hidden)]
pub fn resolve_projected_calc(handle: *const (), basis: f32) -> f32 {
    CalcRef::from_handle(handle).resolve(basis)
}

/// Whether `handle`, which the layout library handed back from a length of the
/// projection, names a payload that is still alive.
///
/// An adapter that keeps the owners of its payloads for as long as the layout
/// library holds their handles can assert this where it resolves them:
/// [`resolve_projected_calc`] answers `0.0` for a handle that names nothing,
/// and a dropped owner would otherwise go unnoticed.
#[inline]
#[doc(hidden)]
pub fn projected_calc_is_live(handle: *const ()) -> bool {
    CalcRef::from_handle(handle).is_live()
}
