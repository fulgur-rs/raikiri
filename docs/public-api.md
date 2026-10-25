# Public API of the publishable crates

The crates that carry raikiri's public API are `raikiri-html` and `raikiri-dom`
(what fulgur builds on), together with the crates their signatures name:
`raikiri-traits`, `raikiri-style`, `raikiri-layout-geometry`,
`raikiri-layout-style`, `raikiri-svg` and `raikiri-net`. The `raikiri` crate is
for dogfooding and is not part of this surface.

This document records what is decided about that surface. Each section is
added when its decision is made.

## Which names are the API

A *name* is something an outside user can write: a type, function, constant or
module that the crate root reaches. `scripts/public-api-lint.sh --surface
CRATE` prints the names of a crate, with the item each re-export comes from, and
`--with-hidden` adds the hidden ones. Methods, fields and trait impls belong to
their type and are not listed.

| crate | visible names | `#[doc(hidden)]` |
| --- | ---: | ---: |
| `raikiri-html` | 146 | 3 |
| `raikiri-dom` | 204 | 105 |
| `raikiri-traits` | 247 | 0 |
| `raikiri-style` | 776 | 0 |
| `raikiri-svg` | 18 | 0 |
| `raikiri-net` (default features) | 7 | 0 |
| `raikiri-layout-geometry` | 6 | 0 |
| `raikiri-layout-style` | 65 | 0 |

There are three tiers.

1. **Consumer API.** The names `raikiri-html` publishes, and everything their
   signatures reach: the types of `raikiri-dom`, `raikiri-traits`,
   `raikiri-style` and `raikiri-svg` that it re-exports or that a field, a
   parameter or a trait impl names. This is what fulgur builds on, and a change
   to it is a change every consumer sees. Of the 146 names of `raikiri-html`,
   65 are declared there (three of them modules), 42 come from `raikiri-dom`,
   23 from `raikiri-style`, 14 from `raikiri-traits` and one,
   `FontCollection`, from shodo. The remaining name is the glob that publishes
   the computed-value types of `raikiri-style` under `computed`.
2. **Supporting API.** The other visible names of `raikiri-dom`,
   `raikiri-traits`, `raikiri-style` and the rest. They are public
   because other crates of this workspace call them, above all the painter in
   `raikiri-paint`, not because a consumer was promised them. A consumer may
   use one, and then it belongs in the first tier: it is named in the list
   below, or re-exported from `raikiri-html`. One that nobody outside the
   workspace uses is hidden or made crate-private before the first
   publication.
3. **Hidden.** Items marked `#[doc(hidden)]` are public only so that crates of
   this workspace can call them (65 of them are used by `raikiri-paint` today).
   They are outside every promise: documentation leaves them out, and
   a version number does not count them.

`raikiri-layout-style` is in none of the tiers. No public item of
`raikiri-html` or `raikiri-dom` names one of its types; it is published only
because `raikiri-dom` depends on it, and it names Taffy types by design.

### What fulgur uses

The first tier cannot be finished from this repository, because it depends on
what fulgur calls. `scripts/public-api-lint.sh --usage DIR` reads the Rust
sources under DIR and reports, per crate, which names they reach through `use`
declarations and inline paths, which hidden names, which paths match no name
(renamed or removed), and which glob imports hide the rest. Run it in a fulgur
checkout:

```sh
scripts/public-api-lint.sh --usage /path/to/fulgur/crates/fulgur-raikiri
```

The report is at the level of names: a used type brings all its methods with
it. It reads text, not types: `use raikiri_dom as dom;` is followed, but a path
that a macro builds, or a crate that Cargo renames, is not seen. What it shows is the input for the decision of the second tier: a name
that fulgur uses stays and is documented, a hidden one that it uses is either
promoted or replaced, and a name that nothing outside the workspace uses is
hidden.

### The names of `raikiri-html`

- Parsing: `parse`, `parse_fragment`, `parse_with_sink`, `parse_html`,
  `parse_html_with_limits`, `parse_html_with_resources`, `parse_xhtml`,
  `effective_document_base_url`, `expand_live_stylesheet_imports`,
  `RaikiriTreeSink`, `ParseOptions`, `StylesheetPart`, `StylesheetSource`,
  `UncascadedDocument`, `HtmlDocument`.
- Cascade: the ten `build_cascaded*` and `build_rule_tree*` functions,
  `CascadeResult`, `ComputedValues`, `Declaration`, `MediaContext`,
  `MediaType`, `Origin`, `PageCascadeResult`, `PageContextQuery`,
  `PageMarginBoxCascadeResult`, `PageMarginBoxSlot`, `PropertyKey`,
  `PropertyValue`, `StringFetchMode`, `ConsumerPropertyEvent`,
  `ConsumerPropertyGrammar`, `ConsumerPropertyObserver`,
  `ConsumerPropertyRegistration`, `MAX_PAINTED_BACKGROUND_LAYERS`,
  `MINIMAL_UA_CSS` (also under the module `ua`), and the module `computed`
  (the computed-value types of `raikiri-style`).
- Layout: `layout`, `LayoutOptions`, `LayoutConfig`, `LayoutConfigBuilder`,
  `PageDefaults`, `LayoutStatus`, `DocumentLayout`, `DomView`, `Page`,
  `PageGeometry`, `PageMode`, `PageSize`, `PageSizeKeyword`, `PageOrientation`,
  `PageRotation`, `PageBleed`, `PageMarks`, `RootBox`, `NodeId`, `NodeKind`.
- Streaming: `StreamingLayout`, `StreamPage`, `StreamStatus`, `StreamSummary`,
  `PageSink`, `PageCountRunningElement`.
- What a painter draws: `Fragment`, `FragmentKind`, `PaintEvent`, `ClipKind`,
  `OverflowClip`, `RepeatKind`, `GeneratedBox`, `GeneratedKind`,
  `GeneratedImageId`, `LineBox`, `PositionedGlyphRun`, `Glyph`, `FontBlob`,
  `FontId`, `FontRef`, `FontVariation`, `Synthesis`, `Tag`, `TextLineId`,
  `TextShadow`, `RunSource`, `DecorationKind`, `DecorationLine`,
  `DecorationStyle`, `ColumnRule`, `PaintClip`, `PaintInsets`, `PaintRect`,
  `PaintTransform`, `MarginBox`, `MarginBoxBackgroundImage`, `MarginBoxBorder`,
  `MarginBoxRunning`, `MarginBoxText`, `DeferredGlyph`, `DeferredSlot`,
  `DeferredValue`, `PlacedImage`, `PlacedRunningElement`,
  `RunningElementLayout`, `ImageContent`, `RasterImage`, `InlineSvg`,
  `SvgRendering`, `PageSvgFonts`, `Anchor`, `AnchorIndex`, `Link`, and the
  module `image_geometry` (`BackgroundTiles`, `BorderImageGeometry`,
  `BorderImagePart`, `DeviceGrid` and five functions).
- Resources and fonts: `RenderResources`, `RenderSvgImages`, `ResourceLimits`,
  `DEFAULT_MAX_RESOURCE_BYTES`, `DEFAULT_MAX_AGGREGATE_RESOURCE_BYTES`,
  `RenderFonts`, `FontCollection`, `FontCollectionBuilder`,
  `FontCollectionBuildError`, `BundledFont`, `MAX_BUNDLED_FONT_BYTES`.
- Diagnostics: `RenderError`, `RenderWarning`, `WarningKind`.

`scripts/public-api-lint.sh --surface raikiri-html` is the source of this list;
regenerate it when the crate root changes.

## Policy

### `#[doc(hidden)]`

A name that is `pub` only for the benefit of other crates of this workspace
carries `#[doc(hidden)]`. It is then outside the promise, which is what the
version number means by "not public". A new `pub` item in
`raikiri-html`, `raikiri-dom` or the crates their signatures name is part of
the API unless it is hidden, so a change that adds one says why it is needed.

### `#[non_exhaustive]`

It decides whether adding a variant or a field breaks a consumer. Most of the
first tier has it already, and the rule below is the one the existing code
follows:

1. **An enum is `#[non_exhaustive]`.** A consumer that matches it writes a
   wildcard arm, so a new variant is a compatible change. This holds for every
   enum `raikiri-html` declares, 39 of the 40 of `raikiri-traits` and 11 of the
   16 of `raikiri-dom`.
2. **A struct whose fields are all public is `#[non_exhaustive]`**, so a field
   can be added; a consumer reads the fields it needs and destructures with
   `..`. A struct that the consumer builds also has `new()` (or a builder) and
   `Default`, and keeps its fields public, so `let mut limits =
   ResourceLimits::new(); limits.max_resource_bytes = None;` works from outside
   the crate. `ResourceLimits` and `LayoutConfig` with its builder are the
   models.
3. **A plain value type is exempt**: the identifiers, geometry and colors whose
   fields are their meaning (`NodeId`, `Tag`, `Point`, `Size`, `Edges`,
   `Sides`, `CssColor`, `ComputedLength` and the like). Nobody adds a field to
   them without changing what they are, and a literal such as `Size { width,
   height }` is the natural way to write one. Changing one is a breaking
   release.

`scripts/public-api-lint.sh --exhaustive CRATE` lists the types a crate declares
that a user can still match or build completely, so the rules can be checked
against the code. At the time of writing it lists 4 types for `raikiri-html`
(`ParseOptions`, `StylesheetPart`, `StylesheetSource` and `UncascadedDocument`),
25 for `raikiri-dom`, 20 for `raikiri-traits`, 82 for `raikiri-style`, 8 for
`raikiri-svg`, 5 for `raikiri-layout-geometry` and none for `raikiri-net`. Each
list mixes the value types of rule 3 with the types that should get the
attribute; sorting them is part of narrowing the first tier. The types that
get it are changed once, before the first publication, together with the
matching change in fulgur-raikiri (a struct literal becomes a constructor). A
new public type is added with the attribute from the start.

### Features

- A feature is additive: enabling it never removes or changes an item or a
  signature that exists without it.
- A feature exists for a dependency or a facility that most consumers do not
  need (`raikiri-net/http-ureq` pulls `ureq` and the TLS stack), never to
  switch behavior.
- No feature is on by default.
- The public items a feature adds do not name types of the feature's
  dependency in their signatures. `UreqHttpProvider` holds its `ureq::Agent`
  privately, for example.
- `raikiri-dom`'s `logical-snapshot` and `snapshot-serde` are test and
  diagnostic support. They are outside the promise like hidden items.

The lint described next reads the build with the default features, so the rule
on the types a feature adds is kept in review, not by a tool.

### Types of other crates in a signature

A type in a public signature ties every consumer to the version of its crate,
and so does a `pub use` of one. `THIRD_PARTY_ALLOWED` in
`scripts/lib/public_api_lint.py` lists, per published crate, the crates it may
name in a signature, a field, a bound, a trait impl or a re-export (a glob of a
module of another crate counts as naming that crate), and why. The lint fails
for a crate that is not listed, and it counts the other published crates of
this workspace as listed but not the unpublished ones such as `raikiri-paint`.
Today:

| crate | names types of |
| --- | --- |
| `raikiri-html` | `markup5ever` and `tendril` (`RaikiriTreeSink` implements `TreeSink`), `shodo` (`FontCollection`), `url` |
| `raikiri-dom` | `bitflags` (`NodeFlags`), `shodo`, `smol_str`, `url` |
| `raikiri-traits` | `bytes`, `smol_str`, `url` |
| `raikiri-style` | `cssparser`, `selectors`, `precomputed_hash`, `smol_str`, `triomphe` |
| `raikiri-net` | `url` |
| `raikiri-layout-style` | `taffy`, `smol_str` |
| `raikiri-svg`, `raikiri-layout-geometry` | none |

A new major version of any of these is a breaking release of ours. `shodo` is
the one the project also owns: its `FontCollection` is re-exported by
`raikiri-html`, so a shodo release that changes it is a breaking release of
`raikiri-html`.

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

## Gates

- `scripts/public-api-lint.sh` runs in `scripts/gate.sh` and in CI. It fails
  when a public item names a Taffy type, and when a published crate names a
  type of a crate that `THIRD_PARTY_ALLOWED` does not list.
- Every crate that can contain `unsafe` code is checked by the compiler:
  seven publishable crates forbid it and `raikiri-layout-style` denies it
  outside three `Send` impls (the section on soundness above).
