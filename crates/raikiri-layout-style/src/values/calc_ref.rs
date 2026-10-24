//! The reference a `calc()` length holds to its payload.
//!
//! This module is the one place of the crate that dereferences a payload.

#![allow(unsafe_code)]

use crate::AlignedCalc;
use std::ptr::NonNull;
use std::sync::Arc;

/// A reference to the payload of a `calc()` length.
///
/// The reference is `Copy` and carries no lifetime: lengths are copied around
/// freely, and the payload they refer to is owned elsewhere. What keeps it
/// valid is a contract on the owner, which this type cannot check:
///
/// - A reference is made only from a payload that is alive and aligned: one
///   that a getter of this crate reads, or one that the adapter retains in an
///   [`Arc`] and passes to [`CalcRef::retained`]. No caller can build one from
///   a raw pointer, so a null or dangling handle cannot be resolved through
///   safe code.
/// - The owner of the payload must stay alive for as long as any copy of the
///   reference is resolved. The adapter that supplies the payloads retains
///   every one of them for the lifetime of its session, before a getter can
///   publish a reference to it.
///
/// A length that a getter returned must therefore not be resolved after the
/// owner of the payload it was read from has been dropped.
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
pub struct CalcRef(NonNull<AlignedCalc>);

impl CalcRef {
    /// A reference to `payload`.
    #[inline]
    pub(crate) fn new(payload: &AlignedCalc) -> Self {
        Self(NonNull::from(payload))
    }

    /// A reference to the payload that `owner` retains.
    ///
    /// The reference does not keep the payload alive: the caller must retain
    /// `owner`, or a clone of it, for as long as the reference, or a length made
    /// from it, can be resolved.
    #[inline]
    pub fn retained(owner: &Arc<AlignedCalc>) -> Self {
        Self::new(owner)
    }

    /// The reference whose handle is `handle`.
    ///
    /// The handle must have been made by [`Self::handle`]; the lengths of this
    /// crate keep it in the layout library's representation.
    #[inline]
    pub(crate) fn from_handle(handle: *const ()) -> Self {
        Self(NonNull::new(handle.cast_mut().cast()).expect("a calc handle is never null"))
    }

    /// The handle of the payload, in the form the layout library keeps.
    #[inline]
    pub(crate) fn handle(self) -> *const () {
        self.0.as_ptr().cast_const().cast()
    }

    /// The address of the payload, which identifies it.
    #[inline]
    pub(crate) fn addr(self) -> usize {
        self.handle().addr()
    }

    #[inline]
    fn payload(&self) -> &AlignedCalc {
        // SAFETY: A reference is made from a live payload, and its owner keeps
        // the payload alive while the reference is resolved (see the type
        // documentation). The payload is never mutated.
        unsafe { self.0.as_ref() }
    }

    /// The payload resolved against the percentage `basis`, with its range
    /// applied.
    #[inline]
    pub fn resolve(self, basis: f32) -> f32 {
        self.payload().resolve(basis)
    }

    /// Whether the payload has a percentage term, which needs a basis to resolve.
    #[inline]
    pub fn has_percentage(self) -> bool {
        self.payload().expression().percent != 0.0
    }
}

/// Resolve the calc handle that the layout library hands back from a length
/// of the projection.
///
/// This is the one way into a payload from a raw pointer, for the library's
/// callback that resolves the lengths of a style it was given.
///
/// # Safety
/// `handle` must be a calc handle of a length that a getter of this crate
/// returned, and the owner of its payload must be alive.
///
/// ```compile_fail,E0133
/// use raikiri_layout_style::resolve_projected_calc;
/// let _ = resolve_projected_calc(std::ptr::null(), 0.0);
/// ```
#[inline]
pub unsafe fn resolve_projected_calc(handle: *const (), basis: f32) -> f32 {
    CalcRef::from_handle(handle).resolve(basis)
}
