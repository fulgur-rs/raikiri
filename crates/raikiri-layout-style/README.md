# raikiri-layout-style

Borrowed `ComputedValues` queries. `LayoutStyleView` answers each layout
question with a raikiri-owned type (`values`: keywords, lengths, `CalcRef`;
`BoxStyle`: the owned snapshot of a box), and `TaffyStyle` projects the same
view onto the Taffy 0.15 `CoreStyle`, `FlexboxContainerStyle`,
`FlexboxItemStyle`, `GridContainerStyle` and `GridItemStyle` traits. `LeafStyle`
does the same for an owned `BoxStyle` that is measured as a leaf. This crate
does not depend on the DOM. `LayoutSession` uses these views for production
queries.

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
as `taffy::Style` does. Absent template areas remain `None`.

## Calc safety

A length is `Copy` and a `calc()` length carries a `CalcRef`, a handle to its
payload without a Rust lifetime. The handle is not an address. It names a slot
of one process-wide table, together with the generation the slot had when the
handle was issued, and the payload (`AlignedCalc`) owns that slot. Resolving a
handle looks it up in the table and never dereferences memory that an owner
could have freed, so no safe code can read freed memory through a length:

- The owner decides how long a handle resolves. While any owner of the payload
  is alive, `CalcRef::resolve`, `MaybeResolve` and `ResolveOrZero` give the
  expression's value. Once the last owner has dropped the payload, the slot is
  emptied and its generation changes, so every copy of the length that is
  still around is stale. A stale reference resolves to `0.0`, reports no
  percentage (`CalcRef::has_percentage`) and is not live (`CalcRef::is_live`);
  it never resolves another payload that later took over the storage, because
  its generation no longer matches.
- Safe code cannot make a handle up. `CalcRef` has no public constructor from a
  pointer, and `Dimension::calc`, `LengthPercentage::calc` and
  `LengthPercentageAuto::calc` take a `CalcRef` rather than a pointer. A getter
  of this crate makes a reference from a payload it reads, and the adapter makes
  one with `CalcRef::retained` from a payload that it holds in an `Arc`.
- The Taffy projection hands the handle to the layout library as an opaque word
  with its low three bits clear, and the library's callback gives it back to
  `resolve_projected_calc`. That function is safe: a handle that names no live
  payload resolves to `0.0`. `LayoutStyleView::resolve_calc_handle` is the
  stricter entry for tools that compare getter results; it resolves only a
  handle that the view's own payloads contain.
- Resolving takes no lock and allocates nothing: a reader checks the slot's
  state word before and after it reads the value, so a handle that races with
  the reuse of its slot is rejected rather than read torn. Creating and
  dropping a payload take a short lock. A stale handle is recognized until its
  slot has been reused 2^29 times (2^8 on 32-bit targets); freed slots are
  reused in the order they were freed. `projected_calc_is_live` lets an adapter
  assert that a handle still names a payload, and
  `AlignedCalc::resolve_expression` resolves an expression without making one.

The layout library stores a `calc()` handle as a raw pointer, which would make
every length, and every style store and document that holds one, `!Send`.
`Dimension`, `LengthPercentage` and `LengthPercentageAuto` therefore implement
`Send`, in `projection/length.rs`, and these are the only `unsafe` code of the
crate. The claim is local to the length: its pointer is a table handle that
any thread resolves against the same synchronized table, so moving a length
cannot dangle. Because nothing else is asserted, a type that holds a length is
`Send` only if the rest of it is, which the compiler checks (a `Document` that
gained an `Rc` field would stop being `Send`). The safe `CalcRef`, which the
expanded forms (`ExpandedDimension` and the like) carry, stores the handle as a
plain `usize` and is `Send` without any claim. `Sync` is not claimed.

What stays the adapter's job is the value, not the memory: keep every payload
owner alive until all consumers of the returned lengths have finished resolving
them, or a saved length resolves to `0.0` instead of its expression. The
adapter retains every payload for the lifetime of its session before a getter
can publish a reference to it.

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
Every layout algorithm reads its styles through this view: no box keeps a
converted `taffy::Style`. Block flow and the local sizing steps reduce the
view to a transient `BoxStyle` for the length of one call, and a leaf is
measured through `LeafStyle` over that snapshot.

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
