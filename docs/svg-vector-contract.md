# SVG vector rendering contract

Status: proposed contract for the SVG work that follows (vector scene,
fonts and CSS, external resources, Krilla output, rendering mode, shared
path). It fixes the supported scope and the boundaries between crates so each
step can be built and reviewed on its own. It is not a promise of full SVG
conformance.

## 1. Where things stand

| Piece | Today |
| --- | --- |
| `raikiri-svg` | Parses a bounded SVG (32 MiB input, 1,000,000 XML nodes, depth 128, 1,000,000 nodes after `use` expansion) into a private `usvg` tree, prepares styles (`styled_source*`), and rasterizes with `resvg` under an output byte limit (32 MiB by default). No backend type is public. |
| Rejected inputs | Any `href` or `src` that is not a fragment (this includes every `<image>` with a URL and `data:` URLs), `xml-stylesheet`, `xml:base`, external CSS `url()`, a filter that holds more elements than D4 allows or asks for more memory or time than its budgets allow, and any `DOCTYPE` other than the inert SVG 1.1 declaration. |
| `<img src=*.svg>`, `background-image`, list markers, `content: url()` | `raikiri-net` keeps the SVG bytes and rasterizes lazily at the size paint asks for (1 pixel per CSS pixel of the drawn box), with a per-source LRU of sizes under a shared byte budget. Consumers receive `DecodedImage` pixels through `ImagePixelSource`. |
| Inline `<svg>` | `Page::inline_svg(fragment)` returns the picture of the root as an `ImageContent`: a vector scene, or pixels where D4 and D6 call for them. The scene is built from the standalone source (viewport, root color, root font and descendant declarations resolved by the cascade) with the fonts of the page. `fulgur-raikiri` draws what it is given. The built-in painter in `raikiri-paint` does not use it and rasterizes the subtree with `raikiri-svg`. |

Two things follow. Vector output exists only for inline SVG, and raikiri
decides there whether a root is drawn as a scene or as pixels. Images and
backgrounds are always pixels.

## 2. Decisions

### D1. Ownership

Raikiri owns SVG resolution: parsing, limits, style resolution, font
selection, resource policy and the conversion to paint commands. Consumers draw
paint commands and do not parse SVG.

- `raikiri-svg` keeps the parser, the limits and the raster path, and gains a
  `scene` builder. `usvg` and `resvg` stay private to it. No `usvg` type
  appears in a public signature of any raikiri crate.
- `fulgur-raikiri` stops depending on `usvg` and `krilla-svg` once its scene
  painter lands. Until then both paths exist side by side (see D12).
- No new crate is introduced, so the set of crates to publish does not grow.

### D2. The vector paint command boundary

The scene is plain data in `raikiri-traits` (module `vector`), next to the
other paint types (`PaintRect`, `PaintClip`, `PaintTransform`, `DecodedImage`).
`raikiri-svg` builds it, `raikiri-net` serves it, `raikiri-html` re-exports it,
and the consumer walks it. Placing it in `raikiri-traits` lets the image trait
name it without a dependency cycle.

Sketch of the vocabulary (names are not final; every public enum and struct is
`#[non_exhaustive]`):

```rust
pub struct VectorImage {
    /// Natural size in CSS pixels: the box the scene was built for.
    pub size: (f32, f32),
    pub root: Group,
}

pub enum Node {
    Group(Group),
    Path(Path),
    Image(RasterImage),
    Text(TextRun),
}

pub struct Group {
    pub transform: Transform,
    pub opacity: f32,
    pub clip: Option<ClipPath>,
    pub children: Vec<Node>,
}

pub struct Path { pub data: PathData, pub fill: Option<Fill>, pub stroke: Option<Stroke> }
pub enum Paint { Color([u8; 4]), LinearGradient(..), RadialGradient(..) }
```

Rules of the boundary:

- Coordinates are CSS pixels in the scene's own space: `(0, 0)` to `size` is
  the viewport box after the root `viewBox` and `preserveAspectRatio` have been
  applied. A consumer places the scene with one affine transform and a clip.
- The scene carries SVG-internal opacity only. The opacity and visibility of
  the host element belong to the page's paint events, so the consumer
  composites them once, around the host box and the scene together. A
  document with no host, such as an SVG image, is the whole picture: its own
  root `opacity` is part of its scene, as it is part of its raster.
- Paths are already flattened to move, line, quadratic, cubic and close
  segments; arcs, basic shapes, `use`, `symbol`, `switch` and markers are
  resolved by the builder.
- The scene never refers to files, URLs or fonts by name. Raster content is
  carried as decoded pixels, and glyphs carry the font data they need (see D9).
  A picture (`RasterImage`) is the shared decoded pixels plus whether scaling
  smooths them. It covers `(0, 0)` to `(width, height)` of the group that
  holds it, one pixel per unit, and that group's transform sizes and places
  it, including the clip of `preserveAspectRatio`. The pixels are shared with
  the page's image cache and not counted in the heap size of the scene.
- Nothing in the scene depends on the consumer's output format. Krilla-specific
  decisions (tiling patterns, shading functions, tagged content) stay in
  `fulgur-raikiri`.

### D3. Initial vector subset

Vector output covers: `svg`, `g`, `defs`, `use`, `symbol`, `switch`, `a`
(as `g`), the basic shapes, `path`, solid colors, `linearGradient` and
`radialGradient` (with `gradientTransform`, `spreadMethod` as far as the output
format allows, and stop opacity), `clipPath` (including `clip-rule` and nested
clips), `opacity`, `fill-opacity`, `stroke-opacity`, the stroke properties
(width, caps, joins, miter limit, dashes), `viewBox`, `preserveAspectRatio`,
`transform`, `visibility`, `display`, `currentColor`, markers (flattened),
`text` (see D9) and `image` with a raster source (see D10).

Paint servers and clips reach the scene already resolved to the user space of
the path or group that uses them (bounding box units are folded into the
gradient transform), so a consumer needs no bounding box. A gradient without
a length paints the color of its last stop, one whose transform cannot be
inverted paints nothing, and a clip path is the union of its shapes filled by
their own rules, intersected with the clip it is chained to.

Not in the initial subset: `filter` (drawn by the raster path, see D4),
`mask`, `pattern`, `foreignObject`, animation (SMIL and CSS animations are
ignored), scripting and `a` navigation (an inline `<svg>` link is handled by the
page's link list, not by the scene), and nested SVG documents loaded by
`<image>`. The goal is the common static document, not conformance.

### D4. Fallback

When the builder meets something outside the vector subset that changes the
picture (`filter`, `mask`, `pattern`), it reports that the document cannot be
vector. The caller then uses the existing raster path for that image, so the
output is still correct, only resolution dependent.

A filter is drawn by `resvg` on that path, with every effect it implements. A
document that uses one is never rejected for it, because one filtered image
would otherwise fail the whole render. `resvg` gives a filter a layer as large
as its region (up to five canvases on each side) and keeps the result of every
primitive until the filter is done; a morphology, a convolution or turbulence
also take time that grows with a number the document sets. The output byte
limit bounds neither, so the raster path checks the parsed tree before it
draws, with the transform it draws with, and fails with
`SvgError::FilterLimitExceeded` over a budget: at most eight output-sized
buffers held by one filter, and 2^32 pixel operations over all the filters of
the picture. The filters of one element share the layer of the union of their
regions, and each primitive handles all of it, so each is charged for that
layer. Filters met in masks (however long the chain of masks), patterns, the
text a pattern fills and `feImage` count too. The limit is of the document,
not of the filter syntax, so the same filter can pass at one raster scale and
fail at a larger one. When the raster cannot draw a document that fell back to
it, its error is the one reported, not the scene's.

One more limit applies when the document is parsed, to every path: a
`<filter>` may hold at most 128 elements, its primitives and what they hold.
`usvg` checks each name a primitive uses against all the primitives before it,
so reading a filter takes the square of its size, once for each element that
uses it, and the parser would have to read the filter before any budget above
could look at it. A longer filter fails the parse with the same error and the
`Elements` budget.

A document that holds both a filter and an `<image>` has no picture: only a
scene draws images (the raster path fails with `SvgError::UnsupportedImages`)
and only the raster path draws filters.

The fallback is per image, not per element, so one picture never mixes two
renderings. It is also visible: the page keeps a diagnostic with the reason,
the same way other fallbacks are reported. If the raster path cannot draw the
document either, the image fails like any broken image.

### D5. Entry points

All consumers of SVG go through one resolved-image abstraction that answers
with either pixels or a scene, chosen by D6.

- `ImagePixelSource` keeps `get_decoded_at_size` and gains a default method
  that returns the scene for a URL and a request (`get_vector`, with an
  `ImageVectorRequest` that carries the size of the box). Sources without
  vector support keep the default (`None`), so existing implementations do
  not break. The request will carry the host style inputs of D8 (the
  inherited `color` and the root font) when a consumer needs them; until then
  an image document is built with the default style, as its raster is.
  `RenderResources` answers for the SVG images in its cache. It builds the
  scene once per URL and size with the fonts of the resource set (the same
  faces for all of them), keeps it in the image cache under the same byte
  budgets as the rasters, and remembers an image that the builder sent to the
  raster path so no other box asks again. `None` is a choice of the source,
  never a failure of the image: the caller asks for pixels next.
- `<img>`, `background-image` (including pseudo-elements and margin boxes),
  `list-style-image` and `content: url()` use that method for SVG sources.
- Inline `<svg>` goes through `Page::inline_svg`, which returns an
  `InlineSvg`: the `ImageContent` of the root, its viewport and the host
  opacity that the consumer composites around it. The picture is a scene
  unless the root's rendering (D6) is `raster`, and pixels, at
  `svg_raster_scale`, where D4 or a limit forces them; the policy is raikiri's
  and the consumer draws what it is given. A scene whose stroked glyphs sit
  under a transform other than a rotation, a reflection and a uniform scale
  (`textLength` stretching them) is drawn as pixels too, because a backend
  that strokes in glyph space would distort it. When pixels cannot be had
  either, the scene is returned if there is one and the error otherwise.
- A page exposes the choice, not the mechanism: a replaced fragment is either
  `Raster(pixels)` or `Vector(scene)`. `Page::placed_image`, `placed_marker`
  and `placed_generated_image` return a `PlacedImage` whose `content` is an
  `ImageContent` of one of the two, with the placement of the pixel-only
  `raster_*` methods (which stay as they are). For an image the consumer
  places itself, such as a background layer, `Page::image_content` takes the
  owner element, the URL and the box. The content is a scene when the source
  has one for the box and the owner's rendering (D6) is `vector`; otherwise it
  is pixels asked for at the box times `svg_raster_scale`. The scale applies to
  the pixel-only methods too, where it changes only what a source rasterizes.

### D6. Choosing vector or raster

Vector is the default. Raster is an opt-in for one element, for a whole
document, or for the cases D4 forces.

1. The element that owns the image decides, by the CSS custom property
   `--raikiri-svg-rendering: vector | raster`. It follows the cascade and
   inherits like any custom property, so one declaration can cover a subtree
   or the whole page. The owner is the `<img>` element, the element (or
   pseudo-element, or margin box) that has the `background-image`, the
   `<svg>` root, or the `::before` or `::after` pseudo-element whose `content`
   is the image (what that pseudo-element names itself wins over its
   element).
2. Without a valid value there, the library default applies. The library
   exposes it as `SvgRendering { Vector, Raster }` in the layout options. A
   consumer wrapper, such as `fulgur-raikiri`'s `RenderOptions`, forwards it.
3. An invalid or unknown value is ignored (it behaves as if not declared), the
   same as an invalid declaration of any other property.

Raster resolution is a separate setting: the library option
`svg_raster_scale` (device pixels per CSS pixel, default 1, so existing
output does not change). It is not a CSS property in the first version; it can
be added the same way later if a use appears. A scale that is not finite and
greater than zero is ignored. It applies to the pictures that are drawn as
pixels: those whose rendering is `raster`, and those that D4 sends to the
raster path. A vector scene has no resolution. `SvgDocument::rasterize_with_fonts`
draws that raster: the picture of the CSS pixel grid with every CSS pixel
`svg_raster_scale` pixels wide, text set in the caller's faces. A document with
pictures cannot be rasterized, so it stays a scene whatever its rendering says.

The library reports the choice and does not draw. `LayoutOptions` (and
`StreamingLayout`) take `svg_rendering` and `svg_raster_scale`; a page answers
`Page::svg_rendering(node)`, with `node` the owner, and
`Page::svg_raster_scale()`, and so does the page of a running element. An
unknown value is skipped for the value the element would inherit without it,
and the keywords are not case sensitive. A pseudo-element or a margin box has
no node of its own to ask about; the consumer asks about the element that owns
it until those carry their own answer.

### D7. Sizes, viewports and caching

- The scene is built for a concrete viewport size, in CSS pixels, because
  percentage lengths and `preserveAspectRatio` depend on it. Natural size and
  aspect ratio come from the document and are reported without building a
  scene (as `SvgDocument::intrinsic_size` does).
- A document with a `viewBox` and no size scales to the box that layout gives
  it. `background-size`, `object-fit` and tiling are the consumer's business:
  it places one scene per tile with a transform, and may reuse a scene when
  the tile size repeats.
- Scenes are cached per source, size and host style inputs, in the same LRU and
  byte budget that hold rasters today (the cost of a scene is its estimated
  heap size). Two elements that show the same URL at the same size with
  different `color` or font get different entries, because `currentColor` and
  inherited text properties change the scene. A scene records whether it read
  any host input; one that did not (no `currentColor`, no inherited text) is
  keyed by source and size alone, so the common case keeps one entry.

### D8. Style inputs

- Inline `<svg>`: the host cascade resolves presentation attributes, author
  CSS, root color, root font and descendant declarations, as today. The
  builder takes those resolved inputs.
- SVG used as an image: the document's own `<style>` and attributes apply, as
  today, and the host contributes only the inherited `color` and the root
  font. No page stylesheet reaches an image document.
- `currentColor` resolves to the host `color`. A root that is not visible still
  lets descendants with `visibility: visible` paint.
- Text inherits through the SVG's own tree. The host resolves the root's
  `font-family`, `font-size`, `font-weight` and `font-style`, which reach the
  text like attributes written on the root. `font-stretch`, `font-variant`,
  `font-kerning` and `font-variation-settings` come only from the SVG's own
  attributes and styles. Declarations that the page's stylesheets make on
  elements of an inline SVG reach its text through the same prepared source as
  they reach shapes. External stylesheets and fonts named by the SVG's own CSS
  stay rejected (see D10).

### D9. Text and fonts

SVG text uses the same fonts as the HTML around it. The parser shapes and
positions the text but holds no fonts of its own: its font database stays
empty, and every face it needs is requested from a font source that the caller
supplies (`SvgFontSource` in `raikiri-svg`). The source answers from the
document's font collection, with the same registered faces and the same
generic-family mapping HTML text uses, so SVG and HTML select a face by the
same rules and no second font database exists. Whether system fonts take part
is decided once, for HTML and SVG alike, by the library option that already
governs HTML text.

- The parser resolves the text's own style and lays it out: chunks, `x`, `y`,
  `dx`, `dy` and `rotate` lists, `text-anchor`, `letter-spacing`,
  `word-spacing`, `textLength`, decorations, `textPath`, writing mode, bidi and
  baselines. The collection chooses faces. A request carries the family list
  (with the generic `serif` family appended last, as the parser does), the
  weight, the width and the slant. A character that the selected face lacks is
  asked for again, with that character and the request, and the face the
  collection answers with joins the shaping unless it was used already.
- Text is emitted as glyph runs, so the consumer can keep it selectable and
  embed the font once. Inside a clip path, where only geometry counts, it is
  emitted as outlines. Both are valid scene content.
- A glyph run carries, for each glyph, a reference to the font face (shared,
  not copied per glyph), the glyph id, the characters it stands for and the
  transform that places its outline. The run carries the size, the variation
  settings, the fill and the stroke. It is painted as one shape: the fill
  covers all its glyphs, then the stroke does, or the other way round.
  Underlines, overlines and strike-throughs are paths around the run.
- Faces are selected, never synthesized: there is no synthetic bold or slant.
  The run's variation settings are the ones the parser shaped and flattened the
  glyphs with: what `font-variation-settings` sets, the axes the parser derives
  from the weight, the width and the slant, and the optical size (`opsz`, the
  font size) unless `font-optical-sizing` is `none` or the text sets the axis
  itself. A face without an axis ignores its setting. SVG fonts (`font`
  elements), `tref` and `altGlyph` are not supported. A `pattern` paint on text
  keeps the document on the raster, like a pattern anywhere else.
- The fallback is asked per request: a character that the selected face lacks
  is looked up with the family list, weight, width and slant of the text it
  belongs to, so two texts with different requests do not share an answer.
  A face is the same face for one request only; a face that two requests both
  use is registered once for each, with its own id.
- Text that no face can set is left out, as the raster path leaves it out, and
  a document that is built without a font source has no text in its scene.
- `@font-face` fonts declared by the page are visible to inline SVG through
  `Page::svg_fonts()`, a source built on the collection the page's document
  was laid out with, so SVG and HTML text see the same faces. The faces it
  hands out are shared by all the scenes of that document. `RenderFonts` is
  the source for an SVG without a page: it sees the bundled and, if enabled,
  the system faces, and so does an SVG used as an image, which never sees
  fonts it fetches itself.

### D10. External resources and security

The sandbox that governs HTML governs inline SVG: the same `NetworkProvider`,
the same base directory, the same denial of anything outside it.

`raikiri-svg` opens no file and no connection and decodes no raster format.
What an `<image>` may reference is a decision of the host, made when the
document is parsed (`SvgImageAccess`): `None` (the default of `parse`) allows
fragment references only, `Data` also allows `data:` URLs, and `Any` also
allows URLs, which an `xml:base` would send elsewhere, so a document with one
is rejected under `Any`. A reference outside the access is an error of the
document, as every reference was before.

- Inline `<svg>` may reference raster images by URL. The picture comes from an
  `SvgImageSource` that the caller passes in `SvgResources`. In `raikiri-html`
  that is `RenderResources::svg_images(base_url)`, which asks the same code
  that serves the images of the HTML around it: the URL is resolved against
  the document's base, checked by the resource policy, fetched through the
  page's provider under its response and aggregate byte limits (redirects
  included, each hop checked), and decoded under the per-image and cache
  limits. A `data:` URL goes through the checks of every inline image: the
  policy's scheme and MIME rules, the response limit and the aggregate budget,
  then the decode. A picture that fails any of them is left out of the scene
  and reported as a warning, like a broken image in HTML. External `use`
  targets, `@import` and fonts named by the SVG's own CSS stay rejected.
- A picture is a raster image. An SVG document is never a picture of an SVG,
  however it is named, so one SVG cannot lead to another or back to itself and
  there is no cycle to detect. The parser's `data:` SVG images never reach the
  source either.
- The requests are bounded. One scene asks for at most 256 different sources
  (a repeated URL or `data:` payload counts once) and its pictures hold at
  most 32 MiB of pixels together; the source stops asking once either limit is
  over, because the scene is an error by then. One source makes at most 256
  requests, `data:` pictures included, and requests each URL once for all the
  scenes built with it.
- An SVG loaded as an image is in static mode, as in browsers: it loads nothing
  by itself. Fragment references and `data:` raster images inside it work
  (`Data` access); everything else keeps today's rejection.
- The existing parse limits stay as they are. This contract adds none that
  weakens them.

A document that has pictures cannot be drawn by the raster path or exported as
prepared source: both refuse it (`UnsupportedImages`, `ExternalReference`), so
a consumer that does not draw scenes can never be handed a reference it would
resolve on its own. `Page::inline_svg` and the SVG used as an image keep parsing
with `None` access for now; each moves to `Any` and `Data` together with the
scene consumer that draws its pictures.

### D11. Budgets

The scene builder is bounded the same way the parser is. Initial values, to be
tuned when the builder exists:

| Limit | Initial value |
| --- | --- |
| Scene nodes | 1,000,000 |
| Total path segments | 4,000,000 |
| Group nesting depth | 256 |
| Clip nesting depth | 64 |
| Glyphs in one scene | 1,000,000 |
| Stops in one gradient | 1,024 |
| Gradient stops in one scene | 1,000,000 |
| Different pictures one scene asks for | 256 |
| Pixel bytes the pictures of one scene hold | 32 MiB, the output limit of the raster path |

Exceeding a limit is an error of the scene
(`SvgSceneError::LimitExceeded`), not a signal that the raster is a valid
substitute: `falls_back_to_raster` stays `false`. A consumer that can draw
pixels still may, as inline SVG does (D5), because most of these budgets are
not limits of the raster path, which has its own; when it fails too, the scene's
error is what the caller gets. The cost of a scene is charged to the shared
cache budget, never to an unbounded side allocation.

### D12. Consumers

- `fulgur-raikiri` draws scenes with Krilla paths, fills, strokes, gradients,
  clips, opacity groups and text, and removes `usvg` and `krilla-svg`. It keeps
  drawing pixels for raster images and for the fallback.
- The built-in painter in `raikiri-paint` keeps rasterizing through
  `raikiri-svg` until it has a scene painter. It exists for dogfooding and for
  the WPT reftests, so it must not change rendering while vector support
  arrives; this means the vector path is covered by the consumer's own tests,
  not by the WPT numbers, until a scene painter is added.
- `InlineSvg` no longer carries the prepared source: `fulgur-raikiri` moves
  to its `content` in step with this change, and removes `usvg`,
  `krilla-svg` and its own scene building. Every other step lands as an
  additive change first; the removals come last, in the order the public API
  review fixes.

## 3. Build order

Each step is mergeable alone and leaves rendering unchanged unless noted.

1. Geometry, `viewBox` and `transform` to a scene: the `vector` types in
   `raikiri-traits`, the builder for shapes and paths with solid fills and
   strokes, the fallback signal, and tests that compare the scene with the
   raster of the same document.
2. Gradients, clips and opacity.
3. Text, fonts and CSS: font collection input, glyph runs, the inline `<svg>`
   cascade inputs.
4. External resources through the network provider for inline SVG: the
   access modes, the picture node, the source trait and the page's source
   (`RenderSvgImages`). Wiring them into `Page::inline_svg` and into SVG used
   as an image follows with the scene consumers (steps 5 and 7).
5. `fulgur-raikiri` draws scenes with Krilla.
6. The mode switch: `--raikiri-svg-rendering`, the library default and the
   raster scale; the default flips to vector here, once step 5 exists. The
   library side (`SvgRendering`, the layout options and the page queries)
   lands first; each consumer follows it as it moves to scenes.
7. One path for background, `<img>` and inline SVG, and removal of the old
   inline source.

## 4. Out of scope

Filters, masks and patterns in vector form (the raster fallback draws them),
SVG animation, SVG fonts, `foreignObject`, interactive features, and SVG as the
document root of a page.
Each can extend the vocabulary later; the types are `#[non_exhaustive]` for
that reason.

## 5. Risks to watch

- The fallback hides a lot if the subset is too narrow. Each step records how
  many documents in the test corpus fall back, so a drift shows up as a number.
- Glyph runs depend on what the backend exposes. If it exposes outlines only,
  text becomes outlines and loses selectability until the builder reads glyph
  positions itself.
- The scene is public API of crates that are meant to be published. Keep the
  vocabulary small and review each addition as an API change.
