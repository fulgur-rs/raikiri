//! Crate-common "warn observer, or fall back to `eprintln!`" diagnostic
//! channel.
//!
//! # Origin
//!
//! [`crate::fonts::FontWarn`] established the shape first: an
//! `Option<&mut dyn FnMut(&W)>` observer that a caller may supply to consume
//! diagnostic events programmatically, with a default `eprintln!` fallback
//! when `None` so CLI/test use keeps seeing the same output it always has.
//! `crates/raikiri-dom/src/layout.rs`'s `sanitize_finite` silent-clamp site
//! needed the same shape — this module extracts the one part of the pattern
//! that both sites can actually share.
//!
//! # Why a macro, not a generic function (`Observer<W>` was tried and rejected)
//!
//! The obvious generalization is a single generic type + function:
//!
//! ```ignore
//! type Observer<'o, W> = Option<&'o mut dyn FnMut(&W)>;
//! fn emit<W: Display>(observer: &mut Observer<'_, W>, prefix: &str, event: W) { .. }
//! ```
//!
//! This works for [`crate::layout::LayoutWarn`], which is fully owned (no
//! borrowed fields). It does **not** work for [`crate::fonts::FontWarn`],
//! whose observer type is higher-ranked over `FontWarn`'s own borrowed
//! lifetime — `dyn FnMut(&FontWarn<'_>)` desugars to
//! `dyn for<'a> FnMut(&FontWarn<'a>)`, i.e. the closure must accept *any*
//! event lifetime, not just one fixed at the call site. A generic `W` type
//! parameter is monomorphized once per call (e.g. `W = FontWarn<'iteration>`
//! for whatever borrow is live in that loop iteration), which is a strictly
//! narrower, non-higher-ranked type — and `&mut Option<&mut dyn Trait>` is
//! invariant, so there is no implicit coercion from the walker's
//! already-elaborated `for<'a> FnMut(&FontWarn<'a>)` observer down to a
//! single-lifetime instantiation. Unifying `W` against a *family* of types
//! (`FontWarn<'a>` for all `'a`) is exactly what a generic type parameter
//! cannot express in stable Rust without a GAT-shaped helper trait, which
//! would be heavier machinery than this crate-internal pattern warrants.
//!
//! [`emit_warn_via`] sidesteps the whole issue: a `macro_rules!` expands
//! textually *before* type-checking, so it typechecks independently against
//! whatever concrete (possibly higher-ranked) observer type each call site
//! already has. `fonts.rs` keeps its `FontWarnObserver<'o>` alias (borrowed,
//! HRTB) unchanged; `layout.rs` uses a plain non-lifetime `LayoutWarnObserver`
//! (owned event, no HRTB needed). Both route through the same macro, so the
//! "if Some, call it; otherwise eprintln with a prefix" behavior lives in
//! exactly one place, rather than having two independent answers to the
//! same need.
//!
//! # Scope
//!
//! `pub(crate)` only — this is an internal implementation-sharing seam
//! between `fonts.rs` and `layout.rs`, not a crate-external API. No new
//! Cargo dependency, no `raikiri-traits` surface, no public signature
//! anywhere touches this module.

/// Emit a warn-shaped diagnostic event through an `Option<&mut dyn FnMut(&W)>`
/// -shaped observer: call it if `Some`, otherwise `eprintln!` a
/// `"{prefix} warn: {event}"` line (matching the pre-existing `fonts.rs`
/// message shape byte-for-byte) so behavior is unchanged when nothing is
/// wired up to consume the event programmatically.
///
/// Written as a macro (not a generic function) — see the module doc for why
/// a generic `Observer<W>` cannot also serve [`crate::fonts::FontWarn`]'s
/// higher-ranked borrowed observer type.
macro_rules! emit_warn_via {
    ($observer:expr, $prefix:literal, $event:expr) => {{
        let event = $event;
        match $observer.as_mut() {
            Some(cb) => cb(&event),
            None => eprintln!("{} warn: {}", $prefix, event),
        }
    }};
}

pub(crate) use emit_warn_via;
