//! parley FontContext Send + Sync, nzv-spike for raikiri-spike-nzv.5
//!
//! Verifies at compile time that `parley::FontContext` (and `parley::LayoutContext`)
//! implement `Send + Sync`, which is required so that a single shared instance can
//! be used across a rayon thread pool in raikiri's parallel paint pipeline
//! (design doc section 12.8 T1 Rayon thread count 1-16 baseline).
//!
//! Approach: a generic `assert_send_sync::<T>()` helper is instantiated for the
//! parley types. If a type is not `Send + Sync`, the compiler rejects this file,
//! so the mere fact that `cargo check -p raikiri-feasibility` succeeds is the
//! evidence.
//!
//! Result (parley 0.10):
//!   - `parley::FontContext:                    Send + Sync` — verified.
//!   - `parley::LayoutContext<[u8; 4]>:         Send + Sync` — verified.
//!   - `parley::LayoutContext<peniko::Brush>:   Send + Sync` — verified.
//!
//! Consequence: raikiri may hold a `FontContext` behind a lightweight
//! `Arc<Mutex<...>>` (or per-worker clones) rather than being forced into a
//! thread-local / per-thread instantiation strategy. The design doc's rayon
//! parallelism assumption for text shaping stands.

/// Compile-time assertion that `T: Send + Sync`.
///
/// Instantiating this function with a concrete type that is not `Send + Sync`
/// produces a compile error, giving us static evidence of thread-safety.
const fn assert_send_sync<T: Send + Sync>() {}

// --- Static assertions (evaluated at compile time via const item init) ---

/// Static assertion: `parley::FontContext: Send + Sync`.
const _ASSERT_FONT_CONTEXT_SEND_SYNC: () = assert_send_sync::<parley::FontContext>();

/// Static assertion: `parley::LayoutContext<[u8; 4]>: Send + Sync`.
///
/// `LayoutContext` is generic over the brush type; raikiri's paint pipeline
/// will use a `peniko::Brush`-shaped value, but for the pure send/sync check
/// any `Send + Sync` brush suffices. `[u8; 4]` (an RGBA color) is a trivially
/// thread-safe stand-in.
const _ASSERT_LAYOUT_CONTEXT_SEND_SYNC: () =
    assert_send_sync::<parley::LayoutContext<[u8; 4]>>();

/// Static assertion: `parley::LayoutContext<peniko::Brush>: Send + Sync`.
///
/// This matches the brush type raikiri actually plans to use downstream in
/// the paint pipeline, so it is worth pinning down explicitly.
const _ASSERT_LAYOUT_CONTEXT_PENIKO_SEND_SYNC: () =
    assert_send_sync::<parley::LayoutContext<peniko::Brush>>();

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime witness that the compile-time asserts above are actually
    /// exercised. The `const _` items are evaluated at crate compile time,
    /// but running a test that also constructs the types and moves a
    /// `FontContext` into a spawned thread gives us a belt-and-braces
    /// behavioural signal in addition to the static bound check.
    #[test]
    fn font_context_and_layout_context_are_send_sync() {
        fn takes_send_sync<T: Send + Sync>(_: &T) {}

        let fcx = parley::FontContext::new();
        let lcx: parley::LayoutContext<peniko::Brush> = parley::LayoutContext::new();

        takes_send_sync(&fcx);
        takes_send_sync(&lcx);

        // Also verify we can move a FontContext into a spawned thread — the
        // most direct behavioural expression of `Send`.
        let handle = std::thread::spawn(move || {
            // Touch the moved value so it isn't optimized away.
            let _moved = fcx;
        });
        handle.join().expect("thread joined cleanly");
    }
}
