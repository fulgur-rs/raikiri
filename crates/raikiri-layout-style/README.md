# raikiri-layout-style

Borrowed `ComputedValues` queries implementing Taffy 0.15 `CoreStyle`,
`FlexboxContainerStyle`, `FlexboxItemStyle`, `GridContainerStyle` and
`GridItemStyle`. This crate does not depend on the DOM. `LayoutSession` uses
these views for production Flex/Grid queries (RAIKIRI-713).

`PreparedStyle::new` validates scalar and Grid calc bindings once. The adapter
supplies immutable `AlignedCalc` payloads, identified by `CalcSlot`, and retains
their owners throughout layout. The same expression with another
`non_negative` policy requires another payload. Payloads may be held in
`std::sync::Arc` by a Document owner; this crate borrows them and does not push
an arena from getters. A bare Grid calc breadth has both a min and max slot.
Logical size sources have separate slots when present in the computed values.

`PreparedStyle::prepare` borrows the CV and owns its calc bindings;
`prepare_shared` also retains an `Arc<ComputedValues>`. These forms avoid
self-references when a session owns prepared styles. Preparation allocates;
querying does not. `prepare_with` and `prepare_shared_with` accept the adapter's
existing calc owners and validate their expressions and range policies.
`set_metadata` refreshes box facts without rebuilding bindings or cloning the CV.
`PreparedStyle` is cloneable, while the borrowed
`LayoutStyleView` remains `Copy`.

`LayoutStyleView::new` then borrows that prepared style, `ViewContext`, and
two sparse slices of `PropertyOverride`. It does not scan the entire style.
Getters apply computed values and prepared policy, then pass overrides, then
scoped overrides, and finally project physical values into the requested
layout axes. Later entries within a slice win. Explicit `auto` and `None`
hide the base value; an absent entry exposes it. Resolved lengths have an
explicit tag so percentage edges are not resolved twice.
`OwnedPropertyOverride` keeps any calc owner alive and is projected by
`LayoutStyleView::with_owned_overrides` without cloning an Arc. A resolved
owned length must contain physical pixels.

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
Template tracks and line names return `Some` even for an empty iterator,
matching the current `taffy::Style` implementation. Absent template areas
remain `None`.

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

## DOM integration

The DOM layout session borrows the external cascade and prepares Flex/Grid
containers and all of their projected children before style queries. Hidden
and out-of-flow children are included because Taffy reads their styles before
filtering them. Source values are borrowed; synthetic recipes own shared values.
The production Flex/Grid associated style types use this view. Unit tests wrap
the same prepared views in `CheckedStyle`, compare each consumed value against
an independently converted retained style, and return that retained value.
Core, Block and out-of-flow layout still use the retained style, including the
Grid rows needed by pagination and sizing. `Document::layout_style` continues
to expose the complete converted style.

Document metadata publications refresh preparation before parent cache lookup
and before entering size or edge scopes. Preparation rejects active scopes;
getters neither prepare nor validate. The document retains every shared calc
payload before publishing its raw handle, including payloads from popped
scopes. A successful source bridge releases only the previous bridge's payload
prefix; error and unwind paths preserve it.

Root layout batches source bridging, body corrections and ICB creation until
out-of-flow initialization has consumed document dirtiness. It then synchronizes
and prepares once before the first layout query. The batch rejects scope, cache
and style queries and restores its state on error or unwind; the old calc-owner
prefix is released only after final preparation succeeds. Low-level bridge
callers still prepare eagerly.

The DOM indexes calc owners by exact expression bits and range policy, keeping
the first matching legacy owner. Owned bindings are sorted during preparation
and use binary search during queries. Borrowed binding tables retain their
existing interface. Preparation and getter measurements are separate from
whole-document runtime measurements.
