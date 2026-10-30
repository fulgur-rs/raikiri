# Public API of the publishable crates

The crates that carry raikiri's public API are `raikiri-html` and `raikiri-dom`
(what fulgur builds on), together with the crates their signatures name:
`raikiri-traits`, `raikiri-style`, `raikiri-layout-geometry`,
`raikiri-layout-style`, `raikiri-svg` and `raikiri-net`. The `raikiri` crate is
for dogfooding and is not part of this surface.

This document records what is decided about that surface. Each section is
added when its decision is made.

## Which names are the API

A *name* is something an outside user can write: a type, function, constant,
module or `pub extern crate` that the crate root reaches, and the variants of an
enum that a `pub use E::*` publishes. `scripts/public-api-lint.sh --surface
CRATE` prints the names of a crate, with the item each re-export comes from, and
`--with-hidden` adds the hidden ones. Methods, fields and trait impls belong to
their type and are not listed. A module and a function can share a path
(`raikiri_style::cascade` is both), and each counts as a name.

| crate | visible names | `#[doc(hidden)]` |
| --- | ---: | ---: |
| `raikiri-html` | 147 | 3 |
| `raikiri-dom` | 74 | 235 |
| `raikiri-traits` | 162 | 85 |
| `raikiri-style` | 600 | 179 |
| `raikiri-svg` | 9 | 9 |
| `raikiri-net` (default features) | 7 | 0 |
| `raikiri-layout-geometry` | 5 | 1 |
| `raikiri-layout-style` | 0 | 65 |

There are three tiers.

1. **Consumer API.** The names `raikiri-html` publishes, and everything their
   signatures reach: the types of `raikiri-dom`, `raikiri-traits`,
   `raikiri-style` and `raikiri-svg` that it re-exports or that a field, a
   parameter or a trait impl names. This is what fulgur builds on, and a change
   to it is what the semver report is for. Of the 147 names of `raikiri-html`,
   66 are declared there (three of them modules), 42 come from `raikiri-dom`,
   23 from `raikiri-style`, 14 from `raikiri-traits` and one,
   `FontCollection`, from shodo. The remaining name is the glob that publishes
   the computed-value types of `raikiri-style` under `computed`.
2. **Supporting API.** The other visible names of `raikiri-dom`,
   `raikiri-traits`, `raikiri-style` and the rest. They are public
   because other crates of this workspace call them, above all the painter in
   `raikiri-paint`, not because a consumer was promised them. A consumer may
   use one, and then it belongs in the first tier: it is named in the list
   below, or re-exported from `raikiri-html`. One that nobody outside the
   workspace uses is hidden before the first publication, and
   `scripts/public-api-lint.sh --first-tier` lists the visible names that no
   consumer reaches, so the second tier is empty of visible names once that
   list is empty. Today it is.
3. **Hidden.** Items marked `#[doc(hidden)]` are public only so that crates of
   this workspace can call them (102 of them are used by `raikiri-paint` today).
   They are outside every promise: documentation leaves them out, and
   `cargo semver-checks` does not count them.

`raikiri-layout-style` is in none of the tiers. No public item of
`raikiri-html` or `raikiri-dom` names one of its types; it is published only
because `raikiri-dom` depends on it, it names Taffy types by design, and every
one of its items is `#[doc(hidden)]`.

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

The run on fulgur-raikiri of fulgur `e66c630` (RAIKIRI-834) found 57 names of
`raikiri-html`, 49 of `raikiri-traits` and one of `raikiri-net`, and none of the
other crates. It uses no hidden name and no path that matches nothing. Its one
glob import, `PageMarginBoxSlot::*`, brings in the variants of a type that
`raikiri-html` publishes. The 50 names outside `raikiri-html` are the ones
fulgur-raikiri takes straight from another crate; they belong to the first
tier, and so do all the names they reach. The list below also holds six names
that fulgur-raikiri does not use but that a consumer needs to set up the
network side: the providers `FileNetworkProvider`, `SystemHttpProvider` and
`UreqHttpProvider`, `HostResolverOverrides` for them,
`ssrf_guard::is_globally_routable`, and `MAX_AUTO_REDIRECT_HOPS`, which the
docs of `NetworkProvider::fetch` give as the redirect cap.

### The first tier, computed

The first tier is the least set that holds

1. the visible names of `raikiri-html`,
2. the names listed below, which fulgur-raikiri uses from other crates, and
3. every visible item that the signature, a field, a variant, a bound or an
   impl header of an item in the set refers to, in any of the published
   crates, and the methods and impls of those items.

A `#[doc(hidden)]` item is not entered: what only a hidden method or impl names
is outside the promise, so a hidden method cannot make a type part of the API.
`scripts/public-api-lint.sh --first-tier` computes the set and lists the visible
names outside it, one per line, with exit status 1 if there are any. It reports
two more things with the same exit status: a name in the list below that no
crate publishes any more, and a link in the docs of a visible item to a hidden
item, which the rendered docs would show as a dead link (write the bare name
in a code span instead; a `crate::…` path in a plain span is what the
documentation rule keeps for intra-doc links). The report is empty at this point and must be empty
at the first publication. Adding a name that fulgur-raikiri
starts to use means adding it below; a new `pub` item that no consumer reaches
is hidden.

<!-- first-tier-names:begin -->
```text
raikiri_traits::AbortController
raikiri_traits::ConsumerPropertyEvent
raikiri_traits::ConsumerPropertyObserver
raikiri_traits::ConsumerPropertyValue
raikiri_traits::DecodedImage
raikiri_traits::ImageIntrinsicSize
raikiri_traits::ImagePixelSource
raikiri_traits::ImageRasterSize
raikiri_traits::ImageVectorRequest
raikiri_traits::IntrinsicBox
raikiri_traits::LayoutConfig
raikiri_traits::NodeKind
raikiri_traits::PageBox
raikiri_traits::PageDefaults
raikiri_traits::PaintColor
raikiri_traits::PaintTransform
raikiri_traits::RenderError
raikiri_traits::ReplacedResolver
raikiri_traits::ResolveDisposition
raikiri_traits::ResolvedIntrinsic
raikiri_traits::ResolverError
raikiri_traits::ResolverRequest
raikiri_traits::ResourceKind
raikiri_traits::net::Body
raikiri_traits::net::FetchOutcome
raikiri_traits::net::FetchedResource
raikiri_traits::net::MAX_AUTO_REDIRECT_HOPS
raikiri_traits::net::Method
raikiri_traits::net::NetworkError
raikiri_traits::net::NetworkProvider
raikiri_traits::net::Request
raikiri_traits::vector
raikiri_traits::vector::Dash
raikiri_traits::vector::FontFace
raikiri_traits::vector::FontVariation
raikiri_traits::vector::Glyph
raikiri_traits::vector::GradientStop
raikiri_traits::vector::Group
raikiri_traits::vector::LineCap
raikiri_traits::vector::LineJoin
raikiri_traits::vector::Node
raikiri_traits::vector::Paint
raikiri_traits::vector::PaintOrder
raikiri_traits::vector::Path
raikiri_traits::vector::PathSegment
raikiri_traits::vector::RasterImage
raikiri_traits::vector::SpreadMethod
raikiri_traits::vector::Stroke
raikiri_traits::vector::TextRun
raikiri_traits::vector::VectorImage
raikiri_net::FileNetworkProvider
raikiri_net::HostResolverOverrides
raikiri_net::ImageResolver
raikiri_net::SystemHttpProvider
raikiri_net::UreqHttpProvider
raikiri_net::ssrf_guard::is_globally_routable
```
<!-- first-tier-names:end -->

### The names of `raikiri-html`

- Parsing: `parse`, `parse_fragment`, `parse_with_sink`, `parse_html`,
  `parse_html_with_limits`, `parse_html_with_resources`, `parse_xhtml`,
  `parse_xhtml_with_resources`, `effective_document_base_url`,
  `expand_live_stylesheet_imports`, `RaikiriTreeSink`, `ParseOptions`,
  `StylesheetPart`, `StylesheetSource`, `UncascadedDocument`, `HtmlDocument`.
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
semver tool and the version number mean by "not public". A new `pub` item in
`raikiri-html`, `raikiri-dom` or the crates their signatures name is part of
the API unless it is hidden, so a change that adds one says why it is needed.

### `#[non_exhaustive]`

It decides whether adding a variant or a field breaks a consumer. The rule is:

1. **An enum is `#[non_exhaustive]`.** A consumer that matches it writes a
   wildcard arm, so a new variant is a compatible change.
2. **A struct whose fields are all public is `#[non_exhaustive]`**, so a field
   can be added; a consumer reads the fields it needs and destructures with
   `..`. A consumer cannot write a struct literal or a `..base` update for such
   a type, so a struct that the consumer builds has `new()`, which takes the
   fields that have no sensible default, and one setter for each other field,
   named after the field and taking its type
   (`Request::new(url, kind).headers(headers)`), and `Default` where an empty
   value exists (`ParseOptions`, `ImageIntrinsicSize`). The fields stay public,
   so `let mut options = ParseOptions::new(); options.base_url = Some(url);`
   works from outside the crate. `ResourceLimits` and `LayoutConfig` with its
   builder are the models.
3. **A value type is exempt**, because its fields or variants are its meaning
   and nobody adds one without changing what the type is. Changing one is a
   breaking release, which the semver report shows. The value types are:
   - identifiers and wrapped names: `NodeId`, `StyleNodeId`, `Tag`,
     `TargetSlotId`, `Symbol`, `Atom`, `AttrValue`, `CounterSymbol`,
     `FontFaceStretch` and `FontFamilyName`;
   - geometry: `Point`, `Size`, `Edges`, `BoxLayout`, `ImageRasterSize`,
     `Sides`, `CornerRadius` and `StartEnd`;
   - colors: `CssColor`;
   - sets that a specification closes, which a source matches without a
     catch-all arm: `Direction`, `DomMutationError` (DOM §4.2.3 names a
     `HierarchyRequestError` and a `NotFoundError` and no other failure),
     `FontFamilyKind`, `SvgFontFamily` and `SvgFontStyle`;
   - the computed values of `raikiri-style`: `ComputedLength`, every other
     `Computed…` type, `VerticalLogicalSize` and the calc helpers
     (`CalcLengthPercentage`, `ComparedPercentage`). The engine converts them
     with exhaustive `match` expressions on purpose: a CSS value that is new
     is then a compile error at each place that has to say what it means for
     layout, where a wildcard arm would make it a silent default. A consumer
     outside the engine reads them far less than the engine does.

`scripts/public-api-lint.sh --exhaustive CRATE` lists the types a crate declares
that a user can still match or build completely, and after this step it lists
the value types and nothing else. `--first-tier` compares that output with the
list below and reports a type that is exhaustive without being listed, and a
listed type that has the attribute now or is gone. A new public type is added
with the attribute from the start; a new value type is added to the list, with
a line above that says which kind it is.

<!-- value-types:begin -->
```text
raikiri_dom::DomMutationError
raikiri_dom::Tag
raikiri_layout_geometry::BoxLayout
raikiri_layout_geometry::Edges
raikiri_layout_geometry::Point
raikiri_layout_geometry::Size
raikiri_style::Atom
raikiri_style::AttrValue
raikiri_style::ComputedBorderSpacing
raikiri_style::ComputedColumnWidth
raikiri_style::ComputedFlexBasis
raikiri_style::ComputedGridTemplateTracks
raikiri_style::ComputedGridTrackBreadth
raikiri_style::ComputedGridTrackList
raikiri_style::ComputedGridTrackListComponent
raikiri_style::ComputedGridTrackRepeat
raikiri_style::ComputedGridTrackSize
raikiri_style::ComputedLength
raikiri_style::ComputedLengthPercentage
raikiri_style::ComputedLengthPercentageOrAuto
raikiri_style::ComputedLengthPercentageOrCalc
raikiri_style::ComputedLengthPercentageOrNormal
raikiri_style::ComputedLetterSpacing
raikiri_style::ComputedLineHeight
raikiri_style::ComputedTabSize
raikiri_style::ComputedTextDecorationInset
raikiri_style::ComputedTextDecorationThickness
raikiri_style::ComputedTextIndent
raikiri_style::ComputedTextUnderlineOffset
raikiri_style::ComputedTransformFunction
raikiri_style::CounterSymbol
raikiri_style::CssColor
raikiri_style::Direction
raikiri_style::FontFaceStretch
raikiri_style::FontFamilyKind
raikiri_style::FontFamilyName
raikiri_style::Sides
raikiri_style::StyleNodeId
raikiri_style::VerticalLogicalSize
raikiri_style::computed_api::CalcLengthPercentage
raikiri_style::computed_api::ComparedPercentage
raikiri_style::computed_api::CornerRadius
raikiri_style::property::StartEnd
raikiri_style::resolve::ComputedOverflowClipMargin
raikiri_svg::SvgFontFamily
raikiri_svg::SvgFontStyle
raikiri_traits::ImageRasterSize
raikiri_traits::NodeId
raikiri_traits::Symbol
raikiri_traits::TargetSlotId
```
<!-- value-types:end -->

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
  diagnostic support. They are outside the promise like hidden items; the
  semver report, which enables every feature, may list changes to them.

The semver tool checks with every feature on, which is where additivity shows:
an item that a feature removes or changes is reported as breaking.

The lint described next reads the build with the default features, so the rule
on the types a feature adds is kept in review, not by a tool.

### Types of other crates in a signature

A type in a public signature ties every consumer to the version of its crate,
and so does a `pub use` of one. `THIRD_PARTY_ALLOWED` in
`scripts/lib/public_api_lint.py` lists, per published crate, the crates it may
name in a signature, a field, a bound, a trait impl or a re-export (a glob of a
module of another crate counts as naming that crate), and why. It reads the
items the names stand for, their fields, variants, methods and impls, the
impls of their traits, the types of the crate that their signatures mention
even when no name leads to them, and every other `pub` item that no
`#[doc(hidden)]` covers, so a name the list misses cannot hide an item. The
lint fails for a crate that is not listed, and it counts the other published
crates of this workspace as listed but not the unpublished ones such as
`raikiri-paint`. Today:

| crate | names types of |
| --- | --- |
| `raikiri-html` | `markup5ever` and `tendril` (`RaikiriTreeSink` implements `TreeSink`), `shodo` (`FontCollection`), `url` |
| `raikiri-dom` | `shodo`, `smol_str`, `url` |
| `raikiri-traits` | `bytes`, `smol_str`, `url` |
| `raikiri-style` | `cssparser`, `selectors`, `precomputed_hash`, `smol_str`, `triomphe` |
| `raikiri-net` | `url` |
| `raikiri-layout-style` | none visible (every item is `#[doc(hidden)]`; its entries for `taffy` and `smol_str` apply again when an item is un-hidden) |
| `raikiri-svg`, `raikiri-layout-geometry` | none |

A new major version of any of these is a breaking release of ours. `shodo` is
the one the project also owns: its `FontCollection` is re-exported by
`raikiri-html`, so a shodo release that changes it goes through the order below.

### Versions

All workspace crates share one version, and the development series advances by
patch even for a breaking change (`docs/releasing.md`). Until the first
publication that is the whole promise: pin the raikiri crates to one exact
version together. The semver report tells a reviewer, and the order below tells
the consumers, what a version breaks. At the first publication the report
becomes strict (`scripts/semver-checks.sh --strict`) and release-plz starts
checking semver too (`semver_check = true`).

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
- `scripts/semver-checks.sh` runs `cargo semver-checks` over the published
  crates except `raikiri-layout-style`, against the merge base with the base
  branch; a crate the base does not have yet is left out. In CI the `semver`
  job runs it on the pull requests that change more than documentation (in a
  stack, on the top one) and shows the findings in its summary and as a
  warning. It does not fail the job while the crates are at 0.0.x, and a fault
  of the tooling itself is a warning too, so no report is not a clean report:
  look for the job. cargo-semver-checks is pinned to 0.49.0, which
  reads the rustdoc JSON format of the pinned toolchain; move
  `SEMVER_CHECKS_VERSION` together with `KNOWN_FORMAT_VERSIONS` when the
  toolchain moves (a test keeps the script and the workflow in step).

## Order of the breaking changes

The changes below alter what a consumer of `raikiri-html` or `raikiri-dom`
writes. Each one is cheap while the only consumer is fulgur-raikiri and no
release carries this API yet, and a breaking release once one does. The order is
chosen so that a change that adds types comes before the ones that annotate
or remove them, and so that fulgur-raikiri is adapted once per step.

| # | Change | Issue | State | What changes for a consumer |
| ---: | --- | --- | --- | --- |
| 1 | XHTML is parsed by xml5ever | RAIKIRI-797 | done | `XhtmlParseError` is gone, `parse_xhtml` fails with the shared `ParseError`, and malformed XHTML is repaired, not rejected |
| 2 | SVG is drawn from a vector scene | RAIKIRI-9 (510 to 519) | done | `InlineSvg`, `ImageContent` and the SVG resources of `RenderResources` carry the scene contract (`docs/svg-vector-contract.md`) |
| 3 | XHTML joins `StreamingLayout` | RAIKIRI-798 | done | `StreamingLayout::new_xhtml` is added; the caller picks the syntax and nothing is sniffed from the markup |
| 4 | `Page::inline_svg` returns a scene | RAIKIRI-830 | done | `InlineSvg::source` is replaced by `InlineSvg::content`, an `ImageContent` that raikiri chooses (a scene, or pixels where the contract calls for them), and `Page::inline_svg` fails with `SvgSceneError`; the usvg tree path of the painter goes away |
| 5 | Narrow the first tier to what fulgur uses | RAIKIRI-834 | done | 310 items and 10 modules that no consumer reaches are `#[doc(hidden)]` (`raikiri-layout-style` entirely), and `--first-tier` reports nothing |
| 6 | `#[non_exhaustive]` and constructors | RAIKIRI-835 | done | the option and result types (`ParseOptions`, `Request`, `FetchedResource`, `DecodedImage`, `ResolvedIntrinsic`, `RenderWarning`, `PolicyViolation`, `ImageIntrinsicSize` and a few small ones) are built with `new()` and setters instead of a struct literal, and `ParseOptions` has `Default`; the diagnostic and keyword enums need a wildcard arm |
| 7 | First publication | RAIKIRI-795 | open | from here a breaking change is a version bump, and the semver report is strict |

Reasons for the order:

- Step 3 decided the shape of the streaming entry point, and step 4 decides the
  shape of `InlineSvg`; both add or replace public types, so they come before
  the steps that decide which types stay and how they are marked. Step 4 is
  separate work, but after step 7 it would cost a breaking release.
- Step 5 starts from the output of `scripts/public-api-lint.sh --usage` on
  fulgur-raikiri (the first run is recorded on RAIKIRI-834) and removes names.
  It comes before step 6 so that step 6 annotates only the types that remain.
- Step 6 added the attribute to the types that `--exhaustive` listed and that
  are not value types, which changed how a consumer builds them. It is last so
  that it ran once, over the final set of types, with one matching change in
  fulgur-raikiri.
- The `shodo` re-export (`FontCollection`) is not a step: a shodo release that
  changes it is a breaking release of `raikiri-html` and goes through this
  table when it happens.

Not before the first publication: the `Cache` and adapter crate split
(S10). It moves code between crates without changing a public signature, which
a `--dump` of the crates before and after shows, so it can follow the
publication as a compatible release. A new crate needs its name reserved first,
which can be done at any time.
