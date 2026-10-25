# Public API of the publishable crates

The crates that carry raikiri's public API are `raikiri-html` and `raikiri-dom`
(what fulgur builds on), together with the crates their signatures name:
`raikiri-traits`, `raikiri-style`, `raikiri-layout-geometry`,
`raikiri-layout-style`, `raikiri-svg` and `raikiri-net`. The `raikiri` crate is
for dogfooding and is not part of this surface.

This document records what is decided about that surface. Each section is
added when its decision is made.

## Soundness

The question is whether safe code that uses the public API can cause undefined
behavior. The audit looked at every `unsafe` block and `unsafe impl` of the
workspace and at every public item that takes or returns a raw pointer.

### Where `unsafe` is allowed

The workspace sets `unsafe_code = "deny"`. Beyond that, seven of the eight
publishable crates carry `#![forbid(unsafe_code)]` (`raikiri-traits`,
`raikiri-style`, `raikiri-layout-geometry`, `raikiri-svg`, `raikiri-net`,
`raikiri-dom` and `raikiri-html`), so `unsafe` cannot appear there at all, not
even behind an `#[allow]`.

The eighth, `raikiri-layout-style`, contains the only `unsafe` code of the
production crates: three `unsafe impl Send` for the length types `Dimension`,
`LengthPercentage` and `LengthPercentageAuto` in `src/projection/length.rs`.
Everything else that is `unsafe` is either test or benchmark code (counting
allocators in `raikiri-bench` and in the allocation test of
`raikiri-layout-style`, `std::env::set_var` calls in `raikiri-vrt` tests) or in
`tools/raikiri-js-wasmtime`, which is not a workspace member (the workspace
`exclude`s it) and loads a precompiled module and exports C entry points from
its guest. None of it is published.

To check again:

```sh
git grep -nE 'unsafe (impl|fn|trait|extern)|unsafe \{|allow\(unsafe_code\)' -- '*.rs' ':!crates/*/tests/*' ':!crates/*/benches/*' ':!tools/*'
```

should list those three impls (and their `#[allow]` lines) and nothing else.

### Why the three `Send` impls are sound

The layout library keeps a `calc()` length as a raw pointer in a compact
word, which would make every length `!Send`, and with it every style store and
the whole `Document`. The pointer is not an address. It is the handle of a slot
of one process-wide table (`calc_slab`), made of the slot index and the
generation the slot had when it was issued. Resolving a handle reads only that
table, with an atomic state word that rejects a slot that was freed or reused,
and never dereferences memory that an owner could free. A handle that outlives
its payload therefore resolves to `0.0`, reports no percentage, and is not
live. Moving a length to another thread names the same slot there.

The impls state exactly that and nothing more. Before they were added to the
length types, `Document` carried one blanket `unsafe impl Send`, which also
hid every other field from the compiler's `Send` check; a field with a thread
affinity added later would have been unsound without any warning. Now
`Document` is `Send` by the auto trait, and the compiler rejects a new field
that is not. `Sync` is not claimed anywhere.

### Raw pointers in the public API

Four public items of `raikiri-layout-style` mention `*const ()`: `CalcRef::handle`
(hidden from the docs), `resolve_projected_calc`, `projected_calc_is_live` and
`LayoutStyleView::resolve_calc_handle`. They are safe functions. The pointer is
an opaque word and is never dereferenced, so any value, including one that was
never issued, resolves to nothing (`0.0`, `false` or `None`). `CalcRef` has no
public constructor from a pointer. The tests in
`crates/raikiri-layout-style/tests/stale_calc.rs` use only this safe API and
cover a dropped owner, a reused slot, a foreign handle and lengths moved
between threads.

### Not covered

The audit covers raikiri's own code. It does not cover the `unsafe` inside
dependencies (Taffy, shodo, html5ever, image decoders and so on); their
soundness is theirs.
