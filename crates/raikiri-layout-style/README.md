# raikiri-layout-style

Borrowed `ComputedValues` queries implementing Taffy 0.15 `CoreStyle`,
`FlexboxContainerStyle`, `FlexboxItemStyle`, `GridContainerStyle` and
`GridItemStyle`. This crate does not depend on the DOM and is not yet wired
into `LayoutSession` (RAIKIRI-709; integration follows in RAIKIRI-713).

`PreparedStyle::new` validates scalar and Grid calc bindings once. The adapter
supplies immutable `AlignedCalc` payloads, identified by `CalcSlot`, and retains
their owners throughout layout. The same expression with another
`non_negative` policy requires another payload. Payloads may be held in
`std::sync::Arc` by a Document owner; this crate borrows them and does not push
an arena from getters. A bare Grid calc breadth has both a min and max slot.
Logical size sources have separate slots when present in the computed values.

`LayoutStyleView::new` then borrows that prepared style, `ViewContext`, and
two sparse slices of `PropertyOverride`. It does not scan the entire style.
Getters apply computed values and prepared policy, then pass overrides, then
scoped overrides, and finally project physical values into the requested
layout axes. Later entries within a slice win. Explicit `auto` and `None`
hide the base value; an absent entry exposes it. Resolved lengths have an
explicit tag so percentage edges are not resolved twice.

`StyleMetadata` contains adapter-computed facts rather than DOM logic. Supply
flags for actual box identity, replaced elements and sizing-keyword policy;
the default describes an ordinary block box. Neutral/synthetic boxes must
borrow their own appropriately prepared computed values. Natural image size,
ratio and inherited alignment corrections are adapter-supplied overrides.
The view does not implement override push/pop, cache invalidation or ownership
of synthetic styles.

`ViewContext::default()` preserves physical axes for the current flex/grid
bridge. Vertical contexts expose projected getter values for future adapters;
they do not implement vertical flex/grid layout on their own. Algorithms also
need coordinated input/output, overflow, baseline and OOF transformations.
Grid tracks and gaps retain their CSS inline/block meaning rather than being
unconditionally exchanged with physical width/height.

## Calc safety

Taffy lengths are `Copy` and can carry a raw handle without a Rust lifetime.
Keep every prepared payload owner alive until **all** consumers of those
returned lengths have finished resolving them. Do not resolve saved lengths
after dropping their owners. `resolve_calc` is deliberately `unsafe`: callers
must guarantee this crate's handle origin, alignment, liveness and immutable
access. Foreign Taffy calc handles cannot be resolved by this function.
No standalone safe API exposes a raw pointer to a payload.

## Checks

```sh
cargo test -p raikiri-layout-style --locked
cargo clippy -p raikiri-layout-style --all-targets --locked -- -D warnings
cargo test -p raikiri-layout-style --test getter_allocations --locked -- --test-threads=1
```

The allocation test counts successful allocation, zeroed allocation and
reallocation on the querying thread. Its positive control verifies the counter;
initialization, preparation, assertions and destruction stay outside the
measurement. It consumes all Grid iterators and their clones, including long
shared identifiers, across physical and vertical contexts. It does not prove
allocation counts or timing for an integrated full-document layout. Corpus
comparison at actual getter-read time belongs to RAIKIRI-711/713.
