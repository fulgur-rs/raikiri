use super::*;

/// Establish a minimal inline formatting context for qualifying block containers.
/// Without `<br>`, this makes a single non-wrapping line (see "Implementation").
/// With at least one `<br>` whose display is not none, forced breaks alone
/// create at least one more (potentially visible) line than there are breaks
/// (see "`<br>` forced break"). A leading, trailing, or repeated `<br>` can
/// have a zero-height line, so fewer lines may be visible. Conversely, a
/// container with `<br>` also enables `flex_wrap: Wrap`: independent size-based
/// wrapping can add an unbounded number of lines (see that section and Non-goals).
///
/// CSS 2.1 §9.4.2 <https://www.w3.org/TR/CSS21/visuren.html#inline-formatting>:
/// "a block container either contains only block-level boxes or
/// establishes an inline formatting context and thus contains only
/// inline-level boxes." A node qualifies here when its computed display
/// ([`DisplayValue`]) is [`DisplayValue::Block`] or
/// [`DisplayValue::InlineBlock`] (both create a block container for their
/// contents). CSS Display 3 §2 calls `inline-block` an "inline flow-root",
/// which establishes an independent formatting context, as distinct from a
/// plain `block`'s "block flow" (see [`DisplayValue::InlineBlock`]). This pass
/// only cares that the content area is a block container, not this distinction.
/// The node must also have **at least two** in-document children, all of them
/// inline-level: [`NodeKind::Text`] children or [`NodeKind::Element`] children
/// whose [`DisplayValue`] is [`DisplayValue::Inline`] or
/// [`DisplayValue::InlineBlock`]. Ignore [`DisplayValue::None`] children
/// for both the count and disqualification.
/// Count every [`NodeKind::Text`] child, including whitespace-only text.
/// HTML parsing often creates these nodes from indentation between sibling tags.
/// Under CSS 2.1 §9.2.1.1, text is ordinary inline-level content regardless
/// of its characters: `white-space` collapsing affects rendering, not the
/// formatting context. Thus a container with just one substantive inline child
/// may already have two qualifying children because of parser whitespace.
/// Keep the plain block path when there is only one inline-level child: there
/// is no other box to place beside it, and taffy's block layout puts that
/// child in the same position as a one-item flex row with
/// `align-items: flex-start`. Both size to its margin box and need no
/// cross-axis handling. There is no behavioral gap worth disturbing the block path.
///
/// A plain [`DisplayValue::Inline`] node with only direct text children does
/// not qualify. Unlike `block` or `inline-block`, a non-replaced `inline` box
/// does not create a block container for its contents under CSS 2.1 §9.2.1.1.
/// The contents flow as ordinary inline-level content in the *same* inline
/// formatting context as the box. As an exception, create a synthetic flex
/// line root if it has a nested inline element child: taffy's block path would
/// stack that child's descendants vertically. This is a minimal line-box
/// bridge per nested wrapper, not DOM flattening.
///
/// Also leave a container unchanged if it has any in-document block-level,
/// flex, or grid child. Mixed block-level and inline-level content requires
/// anonymous block boxes around inline-level runs under CSS 2.1 §9.2.1.1;
/// this minimal pass does not implement them.
///
/// # Implementation
///
/// Taffy has no inline layout mode, so implement the line box as a one-line,
/// non-wrapping flex container: [`Display::Flex`] + `flex_direction: Row`
/// (taffy's default) + `flex_wrap: NoWrap` (also the default) lays out its
/// participating children left to right on one line. This matches CSS 2.1
/// §9.4.2: boxes are "laid out horizontally, one after the other, beginning at
/// the top of a containing block". Set `flex_direction` and `flex_wrap`
/// explicitly rather than trusting defaults: `bridge_flex` may have copied
/// author `flex-direction` / `flex-wrap` declarations that were inert when
/// this node was a block container. They become active when it turns into a
/// flex container and must be overridden.
///
/// `align_items: Baseline` is used only when a direct, in-flow `inline-block`
/// or inline replaced image (`<img>`) participates. This implements the
/// supported atomic/replaced baseline slice; it does not make the synthetic
/// flex wrapper a full inline formatting context. Text-only roots and plain
/// inline wrappers keep `FlexStart`. `taffy_impl.rs` reports text baselines for the
/// inline-block path; replaced images without a baseline use Taffy's
/// bottom-edge fallback. `vertical-align: top` / `bottom` remains a child
/// `align_self` override.
///
/// Supported `vertical-align` shifts are accumulated at the containing line
/// root from in-flow inline descendants. Nested offsets add along each inline
/// path; traversal stops at atomic inline-blocks and nested synthetic roots so
/// an extent is counted once rather than padding every wrapper. The root's
/// synthetic leading/trailing is added to authored padding.
///
/// Reset the container's `justify_content` to `None` (taffy's default,
/// equivalent to CSS `normal`) for the same reason. An author
/// `justify-content` copied by `bridge_alignment` was inert on a block
/// container but, on a flex container, changes main-axis item placement
/// (e.g. space-between). CSS 2.1 inline formatting has no equivalent way to
/// redistribute multiple boxes across a line box; resetting the property
/// disables that effect. Also reset `gap` (`row-gap` / `column-gap`) to `0`:
/// CSS 2.1 §9.4.2 does not insert authored gaps between inline-level boxes,
/// so disable the author values copied by `bridge_gap`.
///
/// Reset `align_content`, also copied by `bridge_alignment`, to
/// `Some(FlexStart)`. This flex-only property distributes spare cross-axis
/// space between multiple flex lines and has no CSS inline counterpart.
/// Otherwise, with an explicit `height`, the default `Stretch` can distribute
/// extra space into a zero-height `<br>` line and create a visible gap.
///
/// Give every participating child `flex_grow: 0.0` / `flex_shrink: 0.0` /
/// `flex_basis: auto` / `align_self: None` (or a `Top` / `Bottom` override),
/// overriding author values copied by `bridge_flex` / `bridge_alignment` that
/// have likewise become active. Set `flex_grow` / `flex_shrink` to `0` because,
/// without line wrapping, content wider than the container should overflow
/// rather than shrink. The default `flex-shrink: 1` can compress a text child's
/// box below its shaped content, detaching the box from the glyph run already
/// shaped by [`preshape_text`]. The glyphs will not shrink with the box; they
/// may overflow and overlap their neighbor, worse than the former stacked
/// block rendering. Restore `flex_basis: auto` for the same reason: an author
/// `flex-basis` directly sets the main size regardless of `flex_grow` or
/// `flex_shrink`, so `flex_shrink: 0` alone cannot protect the box. A
/// content-based basis keeps the box at least as wide as its shaped content.
/// Reset `align_self` to `None` (= `auto`) to fall back to the alignment chosen
/// on the container (`auto` falls back to the parent's `align-items`; see
/// `bridge_alignment`). Convert `vertical-align: top` / `bottom` explicitly
/// to `FlexStart` / `FlexEnd`, respectively.
///
/// # `<br>` forced break
///
/// HTML LS §the-br-element
/// (<https://html.spec.whatwg.org/multipage/text-level-semantics.html#the-br-element>)
/// defines `<br>` as "a line break". The HTML LS rendering section
/// (§phrasing-content-3) uses `br { display-outside: newline; }`, but this
/// is illustrative notation outside CSS Display 4's `<display-outside>`
/// production (`block | inline | run-in`), not a CSS declaration raikiri-style
/// can consume. Instead, this pass identifies `<br>` by tag_name, just as
/// [`find_body`] directly compares `tag_name() == Some("body")`. No new
/// [`NodeKind`] variant is needed; it remains `NodeKind::Element`.
///
/// If a qualifying container has at least one participating `<br>` child
/// whose display is not none, switch its `flex_wrap` from `NoWrap` (taffy's
/// default) to `Wrap` and set only that `<br>` child's `flex_basis` to `100%`
/// instead of `auto`. Other participating children still get `flex_grow: 0` /
/// `flex_shrink: 0`.
///
/// Taffy's flex line-packing algorithm (CSS Flexbox 1 §9.2 "Line Length
/// Determination" <https://www.w3.org/TR/css-flexbox-1/#algo-line-break>)
/// then breaks naturally at `<br>`. In a multi-line container (where
/// `flex-wrap` is not `nowrap`), it adds items one by one until an item
/// would exceed the main size, then moves that item to the next line.
/// The exception is an item first on a still-empty line: it stays even if
/// too wide. Since `<br>` has a hypothetical main size of 100% of the
/// container's width, it cannot fit after another item and moves to a new
/// line, but stays put if it starts an empty line. The height (cross size,
/// which `flex_basis` does not control in a row) of the line occupied by
/// `<br>` alone comes from [`compute_leaf_layout`](taffy::compute_leaf_layout).
/// `<br>` is a childless leaf and has no
/// [`Node::text_layout`](crate::node::Node::text_layout), so measurement
/// always returns `0` (see the leaf branch of this module's
/// `LayoutPartialTree::compute_child_layout` in `taffy_impl.rs`). The line
/// occupied by `<br>` is full width but zero height; the next line follows
/// the previous one without a gap. Runs before and after `<br>` thus occupy
/// different visible lines, while `<br>` itself occupies no visible height,
/// matching the visual effect of a CSS 2.1 forced line break.
///
/// This mechanism also enables `flex_wrap: Wrap` only for containers with
/// `<br>`. Containers without it retain `flex_wrap: NoWrap`, but those with
/// it can also undergo size-based wrapping, normally a Non-goal, on either
/// side of `<br>`: taffy wraps whenever inline-level items exceed the
/// available width, independently of the break. This difference applies
/// only to containers with `<br>`. Existing no-wrap regression tests (e.g.
/// `establish_minimal_line_boxes_upgrades_qualifying_container_to_flex_row`)
/// exercise containers without `<br>` and remain unaffected.
///
/// # Non-goals (this pass)
///
/// - No size-based line wrapping: the qualifying container itself establishes
///   exactly one line box, regardless of available width, unless it contains
///   `<br>` whose display is not none. See "`<br>` forced break" for the
///   number of lines with `<br>`: these are forced breaks, not size-based
///   wrapping. This only describes the container's line boxes; a participating
///   text child's shaped glyph run may itself wrap across lines. See the last
///   bullet: [`preshape_text`] independently soft-wraps each text node using
///   the page width.
/// - `<br>` itself always measures `0×0`, so this does not reproduce the
///   line-height-sized empty lines of a real browser. CSS 2.1 §10.8.1
///   <https://www.w3.org/TR/CSS21/visudet.html#strut> specifies a "strut":
///   a zero-width virtual inline box at the start of each line box, carrying
///   the font and line-height of the element establishing that line. It gives
///   even empty lines a minimum line-height. This pass does not synthesize a
///   strut in its line boxes (flex lines); only measurement of `<br>` via
///   [`compute_leaf_layout`](taffy::compute_leaf_layout) determines its
///   line's cross size. This is a consequence of [`establish_minimal_line_boxes`]
///   building all line boxes without struts, even single non-wrapping ones,
///   not two separate special cases. Two behaviors follow:
///   (a) If `<br>` is the first participating item on its line (at the start
///   of a container or the second or later `<br>` in `<br><br>`), the
///   line-packing exception that an empty line accepts its first item keeps
///   it there. Its zero height creates no visible empty line. This pass does
///   not reproduce the empty-line height a real browser creates at the start
///   of `<p><br>text</p>` or between consecutive `<br>` elements.
///   (b) A trailing `<br>` with no following inline-level content likewise
///   adds only a zero-height line, not visible space. Both effects have the
///   same cause: measuring a childless `<br>` leaf with no text layout returns
///   `0`, rather than representing separate special cases.
/// - Do not flatten nested inline elements into an ancestor's DOM line box.
///   If a nested wrapper has an inline element child, make that wrapper a
///   minimal flex line root; lay out its children independently within it.
/// - After this pass (after `apply_computed_to_style`, later in
///   `layout_single_page`), [`preshape_text`] shapes and soft-wraps each text
///   node against the full page width. This is unrelated to the width that
///   this pass eventually gives the node beside its siblings on a line box.
///   Matching text shaping to the actual available inline space is follow-up work.
/// - Handle `text-align: center` through this pass and
///   [`realign_text_after_layout`]. A qualifying container sets
///   `justify_content` to `Center` to center the entire line box; other
///   values retain `None` (`Right` / `Justify` flex handling is outside this
///   task). For a lone Text child in a non-qualifying block container,
///   `realign_text_after_layout` centers it in parley instead. See its docs.
fn text_align_to_parley(v: TextAlign) -> Alignment {
    match v {
        TextAlign::Start => Alignment::Start,
        TextAlign::End => Alignment::End,
        TextAlign::Left => Alignment::Left,
        TextAlign::Right => Alignment::Right,
        TextAlign::Center => Alignment::Center,
        TextAlign::Justify => Alignment::Justify,
        // CSS Text 3 §6.1: `justify-all` justifies every line, including the last.
        // Parley has no equivalent, so degrade to `Justify` (which leaves
        // the last line start-aligned; this difference is documented).
        TextAlign::JustifyAll => Alignment::Justify,
        // `MatchParent` should be resolved before reaching the computed layer
        // (see `raikiri_style::computed::ComputedValues::text_align`).
        // Defensively fall back to `Start` (the initial value).
        TextAlign::MatchParent => Alignment::Start,
        // Fail-closed fallback for future `#[non_exhaustive]` variants.
        _ => Alignment::Start,
    }
}

/// After taffy's `compute_root_layout`, re-align each Text parley `Layout`
/// using its containing block width.
///
/// # Why not align during preshape?
///
/// [`preshape_text`] runs before taffy and only knows `page_box.width`.
/// Naively passing `Center` there centers text in a narrow containing block
/// against the page width, causing a large offset. After taffy, use the
/// resolved parent box width (`unrounded_layout.size.width`) as the containing
/// width for `break_all_lines(Some(w))` + `align(...)`. This bakes an offset
/// based on the correct width into the glyph run. Skip `Start`, which is
/// width-independent and already correct from preshape.
///
/// # Exclude children of flex line boxes
///
/// When the parent was converted to flex by [`establish_minimal_line_boxes`]
/// (`IS_INLINE_ROOT`), its Text is a flex item on the line box. The container's
/// `justify_content: Center` handles centering. Aligning each parley piece
/// against the parent width instead would center all pieces independently
/// and make them overlap, so explicitly skip them. Aligning against each
/// piece's own box width would only give an offset-zero no-op, but skipping
/// makes the intent clearer.
///
/// # Known limitations
///
/// - Use the parent's `size.width` directly as containing width. Do not
///   subtract padding or borders (only px lengths should be subtracted;
///   this is not yet supported, so centered text in a narrow padded box is
///   slightly off).
/// - A re-break can change the number of lines (long text in a narrow box),
///   changing text height without updating the taffy box; sibling y positions
///   remain stale. The main centering target, short single-line text, keeps
///   the same height. Fully reconciling long wrapped text needs a redesign
///   of the inline formatting context.
/// - Resolve logical `Start` / `End` to physical alignment using the node's
///   `cv.direction`. Parley's public API cannot accept a base direction, so
///   resolve to `Left` / `Right` instead of relying on parley's `Start` / `End`.
///   The `dir=rtl` attribute is outside scope (only CSS `direction` is used;
///   mapping the dir attribute to direction belongs to Epic 3).
/// - Resolve `%` `text-indent` against the parent's border-box width (without
///   subtracting its content-box edges, the same approximation as above).
///   Indent for a flex line-box child is unsupported because it would diverge
///   from the measured box width.
///
/// Find the block-container ancestor defining tab-stop metrics (CSS Text 3 §4.2).
/// Climb through `Inline` / `Contents`. Return None on encountering a
/// non-block-container such as `Flex` / `Grid`, or reaching the root; the
/// caller falls back to its own font as a fail-safe.
fn nearest_block_container(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> Option<usize> {
    let mut cur = parent_of.get(idx).copied().flatten();
    while let Some(a) = cur {
        if doc.nodes[a].kind() != NodeKind::Element {
            cur = parent_of.get(a).copied().flatten();
            continue;
        }
        match cascade.computed[a].display {
            DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::ListItem => {
                return Some(a);
            }
            DisplayValue::Inline | DisplayValue::Contents => {
                cur = parent_of.get(a).copied().flatten();
            }
            _ => return None,
        }
    }
    None
}

/// The CSS-collapsible whitespace set (space / tab / LF / FF / CR).
/// `char::is_whitespace` also includes NBSP, which must not collapse, so do
/// not use it here (an approximation of CSS Text 3 §4.1).
fn is_collapsible_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// Whether a sibling contributes preceding content in a block (for the
/// first-line check in CSS Text 3 §8.1).
/// - Text: ignore whitespace-only text under collapsing white-space modes;
///   preserved whitespace occupies a line and therefore counts. Always
///   ignore empty strings.
/// - `br` always counts (a forced break puts following content on a later line).
/// - Ignore `display: none`; count replaced elements (e.g. `img`).
///   Otherwise, ignore childless nodes and count nodes with children.
///   This deliberately over-excludes empty inlines, as documented.
fn is_block_content(doc: &Document, cascade: &CascadeResult, sib: usize) -> bool {
    match &doc.nodes[sib].data {
        crate::node::NodeData::Text(t) => {
            if t.text_content.is_empty() {
                return false;
            }
            match cascade.computed[sib].white_space {
                WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine => {
                    !t.text_content.chars().all(is_collapsible_ws)
                }
                _ => true,
            }
        }
        crate::node::NodeData::Element(_) => {
            let tag = doc.nodes[sib].tag_name().unwrap_or("");
            if tag.eq_ignore_ascii_case("br") {
                return true;
            }
            if cascade.computed[sib].display == DisplayValue::None {
                return false;
            }
            if doc.nodes[sib].children.is_empty() {
                return matches!(
                    tag.to_ascii_lowercase().as_str(),
                    "img"
                        | "input"
                        | "video"
                        | "canvas"
                        | "iframe"
                        | "embed"
                        | "object"
                        | "textarea"
                        | "select"
                        | "button"
                        | "hr"
                );
            }
            true
        }
        _ => false,
    }
}

/// Whether this Text node starts the first formatted line of its block
/// (CSS Text 3 §8.1: without each-line, `text-indent` only affects the first).
/// Per-Text-node preshape cannot equate "first line of the node" with "first
/// line of the block" (after `<br>`, across inline fragments, or among nested
/// blocks, as pinned by length-002). Climb to the nearest block-container
/// ancestor and scan preceding siblings; return false if any has content.
/// Return true when no block ancestor exists (fail-safe, preserving prior behavior).
/// Maps computed hanging/each-line flags plus node line position to parley
/// [`IndentOptions`] (CSS Text 3 §8.1).
///
/// Returns `None` when the node must not indent at all. Position semantics:
/// - [`LineStart::MidLine`] (mid-line inline split): never indent — parley
///   cannot know the layout starts mid-line, so any amount would shift
///   already-placed content.
/// - [`LineStart::BlockStart`]: the layout's first line is the block's first
///   line — pass the flags through unchanged (parley resolves first/wrap/
///   hard-break lines itself, including hanging+each-line combined via XOR).
/// - [`LineStart::AfterBreak`]: every layout line is a non-first block line.
///   basic skips; each-line and combined pass through unchanged (exact per
///   parley scope-line semantics); hanging-only maps to
///   `{ each_line: true, hanging: false }`, which is exact for single-line
///   and post-hard-break lines — soft-wrapped continuations inside the node
///   are missed (no parley option indents all lines unconditionally).
///   Documented approximation; the common test shape (short lines) is exact.
fn indent_options_for_node(
    hanging: bool,
    each_line: bool,
    start: LineStart,
) -> Option<IndentOptions> {
    match (hanging, each_line, start) {
        (_, _, LineStart::MidLine) => None,
        (false, false, LineStart::BlockStart) => Some(IndentOptions::default()),
        (false, false, LineStart::AfterBreak) => None,
        (false, true, _) => Some(IndentOptions {
            each_line: true,
            hanging: false,
        }),
        (true, false, LineStart::BlockStart) => Some(IndentOptions {
            each_line: false,
            hanging: true,
        }),
        (true, false, LineStart::AfterBreak) => Some(IndentOptions {
            each_line: true,
            hanging: false,
        }),
        (true, true, _) => Some(IndentOptions {
            each_line: true,
            hanging: true,
        }),
    }
}

/// The position at the start of a line (for leading trim).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LineStart {
    /// Start of a block.
    BlockStart,
    /// Immediately after `<br>`, a preserved `\n`, or a preceding block.
    AfterBreak,
    /// Any other position (mid-line).
    MidLine,
}

/// Whether a Text node contains a preserved forced break (`\n`).
/// Pre / PreWrap / BreakSpaces / PreLine preserve it. In Normal / Nowrap,
/// `\n` becomes a space instead of a break.
fn has_preserved_break(cascade: &CascadeResult, idx: usize, text: &str) -> bool {
    if !text.contains('\n') {
        return false;
    }
    !matches!(
        cascade.computed[idx].white_space,
        WhiteSpace::Normal | WhiteSpace::Nowrap
    )
}

/// Determine line-start position by scanning backward (for leading trim).
/// Scan preceding siblings while climbing toward the block ancestor:
/// - Skip whitespace-only text in collapsing modes, empty text, and `display: none`.
/// - Preserved `\n` in text, `br`, or a preceding block: [`LineStart::AfterBreak`].
/// - Other content: [`LineStart::MidLine`].
/// - Reach the block start (or root): [`LineStart::BlockStart`].
fn line_start_pos(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> LineStart {
    let block = nearest_block_container(doc, cascade, parent_of, idx);
    let mut cur = idx;
    loop {
        let Some(p) = parent_of.get(cur).copied().flatten() else {
            return LineStart::BlockStart;
        };
        if let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == cur) {
            for &sib in doc.nodes[p].children[..pos].iter().rev() {
                match &doc.nodes[sib].data {
                    crate::node::NodeData::Text(t) => {
                        if has_preserved_break(cascade, sib, &t.text_content) {
                            return LineStart::AfterBreak;
                        }
                        if is_block_content(doc, cascade, sib) {
                            return LineStart::MidLine;
                        }
                    }
                    crate::node::NodeData::Element(_) => {
                        let tag = doc.nodes[sib].tag_name().unwrap_or("");
                        if tag.eq_ignore_ascii_case("br") {
                            return LineStart::AfterBreak;
                        }
                        match cascade.computed[sib].display {
                            DisplayValue::Block
                            | DisplayValue::InlineBlock
                            | DisplayValue::ListItem => {
                                return LineStart::AfterBreak;
                            }
                            DisplayValue::None => {}
                            _ => {
                                if is_block_content(doc, cascade, sib) {
                                    return LineStart::MidLine;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if Some(p) == block {
            return LineStart::BlockStart;
        }
        cur = p;
    }
}

/// Whether the following position is a block end, `<br>`, preserved `\n`,
/// or a following block (for trailing trim). Scan forward, symmetrically
/// to [`line_start_pos`].
fn trail_trim(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
) -> bool {
    let block = nearest_block_container(doc, cascade, parent_of, idx);
    let mut cur = idx;
    loop {
        let Some(p) = parent_of.get(cur).copied().flatten() else {
            return true;
        };
        if let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == cur) {
            for &sib in doc.nodes[p].children[pos + 1..].iter() {
                match &doc.nodes[sib].data {
                    crate::node::NodeData::Text(t) => {
                        if has_preserved_break(cascade, sib, &t.text_content) {
                            return true;
                        }
                        if is_block_content(doc, cascade, sib) {
                            return false;
                        }
                    }
                    crate::node::NodeData::Element(_) => {
                        let tag = doc.nodes[sib].tag_name().unwrap_or("");
                        if tag.eq_ignore_ascii_case("br") {
                            return true;
                        }
                        match cascade.computed[sib].display {
                            DisplayValue::Block
                            | DisplayValue::InlineBlock
                            | DisplayValue::ListItem => {
                                return true;
                            }
                            DisplayValue::None => {}
                            _ => {
                                if is_block_content(doc, cascade, sib) {
                                    return false;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        if Some(p) == block {
            return true;
        }
        cur = p;
    }
}

/// white-space phase 1 collapsing (CSS Text 3 §4.1).
///
/// - Pre / PreWrap / BreakSpaces: unchanged (expand tabs in [`preshape_text`]).
/// - Normal / Nowrap: convert `\t \n \f \r` to spaces, collapse runs, and trim line edges.
/// - PreLine: preserve `\n` (forced break); otherwise follow Normal.
/// - Trim the leading run when `start != MidLine` and the trailing run
///   when `trim_end` is true.
///
/// Out of scope (documented): CJK space removal around segment breaks,
/// PreLine hanging trailing spaces, and interaction with `overflow-wrap`.
fn collapse_ws(
    text: &str,
    ws: WhiteSpace,
    start: LineStart,
    trim_end: bool,
) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
            return Cow::Borrowed(text);
        }
        _ => {}
    }
    let keep_nl = ws == WhiteSpace::PreLine;
    // Fast path: borrow the string if it needs no transformation.
    let needs = text.chars().any(|c| match c {
        '\t' | '\x0C' | '\r' => true,
        '\n' => !keep_nl,
        ' ' => true,
        _ => false,
    });
    if !needs {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut pending_space = false;
    let mut at_start = true;
    // Leading run: drop at BlockStart / AfterBreak; keep one space at MidLine.
    let mut drop_leading = start != LineStart::MidLine;
    for c in text.chars() {
        if c == '\n' && keep_nl {
            // PreLine preserved break: flush pending space and insert a newline.
            // Treat the next run as line-leading (CSS Text 3 §4.1.2).
            // Hanging space before the break is unsupported; emit one space.
            if pending_space && !at_start {
                out.push(' ');
            }
            out.push('\n');
            pending_space = false;
            at_start = true;
            drop_leading = true;
            continue;
        }
        let is_ws = matches!(c, '\t' | '\x0C' | '\r' | '\n' | ' ');
        if is_ws {
            if !at_start || !drop_leading {
                pending_space = true;
            }
            continue;
        }
        if pending_space && (!at_start || !drop_leading) {
            out.push(' ');
        }
        pending_space = false;
        at_start = false;
        out.push(c);
    }
    // Trailing run: drop it at `trim_end` (block end / before break),
    // otherwise keep one space.
    if pending_space && !trim_end {
        out.push(' ');
    }
    Cow::Owned(out)
}

/// Prepare font-metric `text-indent: ch` values before Taffy measures leaves.
///
/// Parley first shapes against the page width, but Taffy may later ask a text
/// leaf for an intrinsic size at a narrower containing width. The prepared
/// metric and flags live on `TextData`; `taffy_impl` reapplies them for each
/// width probe and rebreaks the existing layout before returning its height.
pub(crate) fn prepare_text_indent_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
) {
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &child in &doc.nodes[idx].children.clone() {
            if child < parent_of.len() {
                parent_of[child] = Some(idx);
            }
        }
    }
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for idx in 0..doc.nodes.len() {
        let Some(text) = doc.nodes[idx].data.as_text_mut() else {
            continue;
        };
        text.text_indent_px = None;
        text.text_indent_hanging = false;
        text.text_indent_each_line = false;
        text.text_indent_rebreak = false;
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IN_IFC_SUBTREE | NodeFlags::IS_IFC_ROOT)
        {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        if doc.nodes[idx].is_inline_svg_content() {
            continue;
        }
        let cv = &cascade.computed[idx];
        let Some(parent_idx) = parent_of[idx] else {
            continue; // cov:ignore: in-document text nodes always have a parent.
        };
        if cv.text_indent_ch_factor.is_none() {
            continue;
        }
        let needs_line_break_property = matches!(
            cv.word_break,
            WordBreak::BreakAll | WordBreak::KeepAll | WordBreak::BreakWord
        ) || !matches!(cv.overflow_wrap, OverflowWrap::Normal)
            || matches!(cv.white_space, WhiteSpace::BreakSpaces);
        let has_forced_break = doc.nodes[parent_idx]
            .children
            .iter()
            .any(|&child| is_forced_line_break(doc, child, cascade));
        let parent_is_inline_root = doc.nodes[parent_idx]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT);
        let has_modifier = cv.text_indent_hanging || cv.text_indent_each_line;
        let parent_is_block_container = matches!(
            cascade.computed[parent_idx].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        );
        let has_other_in_document_child = has_modifier
            && doc.nodes[parent_idx]
                .children
                .iter()
                .any(|&child| child != idx && doc.nodes[child].is_in_document());
        if (has_modifier && (!parent_is_block_container || has_other_in_document_child))
            || (parent_is_inline_root
                && (has_modifier || (!needs_line_break_property && has_forced_break)))
            || (cascade.computed[parent_idx].width_ch.is_some()
                && (has_modifier || !needs_line_break_property))
        {
            // Modifier line scope and a `ch` containing width still use the
            // established post-Taffy path for inline roots and width-aware
            // boxes. The pre-Taffy bridge is limited to direct, finite-width
            // text runs where its measured line break is unambiguous.
            continue;
        }
        let layout_max_advance = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(text) => text
                .text_layout
                .as_ref()
                .map(|layout| layout.layout_max_advance()),
            _ => None, // cov:ignore: the preceding as_text_mut guard makes this arm unreachable.
        };
        let preserve_wide_body_run = layout_max_advance
            .is_some_and(|advance| advance > max_advance + 0.01)
            && is_leading_body_text(doc, Some(parent_idx), idx)
            && !needs_line_break_property
            && matches!(cv.text_align, TextAlign::Start | TextAlign::Left);
        if preserve_wide_body_run {
            // Match `realign_text_after_layout`: this deliberately keeps the
            // page-width run untouched, including its indent metadata.
            continue;
        }
        let Some(options) = indent_options_for_node(
            cv.text_indent_hanging,
            cv.text_indent_each_line,
            line_start_pos(doc, cascade, &parent_of, idx),
        ) else {
            continue;
        };
        let Some(indent) = measured_text_indent_px(cv, fonts, layout_cx, &mut ch_probes) else {
            continue; // cov:ignore: only computed ch provenance reaches this point.
        };
        let hanging = options.hanging;
        let each_line = options.each_line;
        // Rebreak direct metric-backed runs before Taffy so their line count
        // contributes to the leaf height. Guarded modifier and `width: ch`
        // shapes remain on the post-Taffy path above.
        let rebreak =
            !matches!(cv.white_space, WhiteSpace::Nowrap) && cv.text_wrap != TextWrapMode::Nowrap;
        let text = doc.nodes[idx]
            .data
            .as_text_mut()
            .expect("text node remains text during layout preparation");
        text.text_indent_px = Some(indent);
        text.text_indent_hanging = hanging;
        text.text_indent_each_line = each_line;
        text.text_indent_rebreak = rebreak;
        if let Some(layout) = text.text_layout.as_mut() {
            layout.set_text_indent(indent, options);
            if rebreak && max_advance.is_finite() && max_advance > 0.0 {
                layout.break_all_lines(Some(max_advance));
            }
        } // cov:ignore: the no-layout branch is defensive for pre-shaped callers.
    }
}

pub(crate) fn prepare_ch_box_values_before_taffy(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) {
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let mut measure = |provenance: &Option<ChLengthProvenance>| {
            provenance.as_ref().map(|provenance| {
                measured_ch_length_px(
                    provenance.factor,
                    Some(&provenance.font),
                    cv,
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                )
            })
        };
        let width = measure(&cv.width_ch).map(|value| value.max(0.0));
        let height = measure(&cv.height_ch).map(|value| value.max(0.0));
        let padding = (
            measure(&cv.padding_ch.top).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.right).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.bottom).map(|value| value.max(0.0)),
            measure(&cv.padding_ch.left).map(|value| value.max(0.0)),
        );
        let margin = (
            measure(&cv.margin_ch.top),
            measure(&cv.margin_ch.right),
            measure(&cv.margin_ch.bottom),
            measure(&cv.margin_ch.left),
        );
        let style = &mut doc.nodes[idx].style;
        if let Some(width) = width {
            style.size.width = Dimension::length(width);
        }
        if let Some(height) = height {
            style.size.height = Dimension::length(height);
        }
        if let Some(top) = padding.0 {
            style.padding.top = LengthPercentage::length(top);
        }
        if let Some(right) = padding.1 {
            style.padding.right = LengthPercentage::length(right);
        }
        if let Some(bottom) = padding.2 {
            style.padding.bottom = LengthPercentage::length(bottom);
        }
        if let Some(left) = padding.3 {
            style.padding.left = LengthPercentage::length(left);
        }
        if let Some(top) = margin.0 {
            style.margin.top = LengthPercentageAuto::length(top);
        }
        if let Some(right) = margin.1 {
            style.margin.right = LengthPercentageAuto::length(right);
        }
        if let Some(bottom) = margin.2 {
            style.margin.bottom = LengthPercentageAuto::length(bottom);
        }
        if let Some(left) = margin.3 {
            style.margin.left = LengthPercentageAuto::length(left);
        }
    }
}

// A helper for the replaced-only post-pass below: mixed inline rows must retain
// Taffy's computed text/replaced baselines rather than being reflowed as image
// and whitespace runs.
fn inline_root_has_unhandled_baseline_content(
    doc: &Document,
    parent_id: usize,
    cascade: &CascadeResult,
) -> bool {
    doc.nodes[parent_id].children.iter().any(|&child| {
        let child_cv = &cascade.computed[child];
        match &doc.nodes[child].data {
            NodeData::Text(text) => {
                let has_text = !text.text_content.is_empty();
                let has_preserved_break = has_text
                    && !matches!(
                        child_cv.white_space,
                        WhiteSpace::Normal | WhiteSpace::Nowrap
                    )
                    && text
                        .text_content
                        .chars()
                        .any(|ch| matches!(ch, '\n' | '\r' | '\x0C'));
                let has_non_whitespace = has_text
                    && !text
                        .text_content
                        .chars()
                        .all(|ch| is_collapsible_ws(ch) || ch == '\u{00A0}');
                has_preserved_break || has_non_whitespace
            }
            NodeData::Element(_) => doc.nodes[child].tag_name() != Some("img"),
            _ => false,
        }
    })
}

fn relative_position_x_offset(
    cascade: &CascadeResult,
    child_id: usize,
    containing_width: f32,
) -> f32 {
    let child_cv = &cascade.computed[child_id];
    if !matches!(
        child_cv.position,
        PositionValue::Relative | PositionValue::Sticky
    ) {
        return 0.0;
    }
    let left =
        super::page::used_computed_length_percentage_or_auto(child_cv.left, containing_width);
    let right =
        super::page::used_computed_length_percentage_or_auto(child_cv.right, containing_width);
    left.or_else(|| right.map(|value| -value)).unwrap_or(0.0)
}

fn relative_position_y_offset(
    cascade: &CascadeResult,
    child_id: usize,
    containing_height: f32,
) -> f32 {
    let child_cv = &cascade.computed[child_id];
    if !matches!(
        child_cv.position,
        PositionValue::Relative | PositionValue::Sticky
    ) {
        return 0.0;
    }
    let top = super::page::used_computed_length_percentage_or_auto(child_cv.top, containing_height);
    let bottom =
        super::page::used_computed_length_percentage_or_auto(child_cv.bottom, containing_height);
    top.or_else(|| bottom.map(|value| -value)).unwrap_or(0.0)
}

// cov:ignore: exercised by resource-enabled ignored WPT reftests; default coverage has no sparse asset run.
pub(crate) fn realign_inline_replaced_children(doc: &mut Document, cascade: &CascadeResult) {
    for parent_id in 0..doc.nodes.len() {
        if !doc.nodes[parent_id]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
        {
            continue;
        }
        let has_replaced_child = doc.nodes[parent_id].children.iter().any(|&child| {
            doc.nodes[child].is_in_document() && doc.nodes[child].tag_name() == Some("img")
        });
        if !has_replaced_child {
            continue;
        }
        let baseline_aligned =
            doc.nodes[parent_id].style.align_items == Some(TaffyAlignItems::BASELINE);
        if baseline_aligned && inline_root_has_unhandled_baseline_content(doc, parent_id, cascade) {
            // Taffy's positions must survive on mixed rows and rows with
            // explicit breaks that already have line-specific baseline geometry.
            continue;
        }
        let parent_width = doc.nodes[parent_id].unrounded_layout.size.width.max(0.0);
        if !parent_width.is_finite() || parent_width <= 0.0 {
            continue;
        }
        let line_height = line_height_px(&cascade.computed[parent_id]).max(0.0);
        if line_height <= 0.0 || !line_height.is_finite() {
            continue;
        }
        let mut x = 0.0_f32;
        let mut y = 0.0_f32;
        let mut line_height_used = line_height;
        let mut baseline_for_line = None;
        // Keep NBSP-connected inline content together. U+00A0 is Glue in
        // UAX 14 and forbids breaks before and after, so CSS line breaking
        // must not split an image plus NBSP plus image run across lines.
        // Group maximal runs of images linked by NBSP-containing whitespace
        // and wrap each group atomically. Collapsible-only whitespace stays
        // breakable and keeps the existing drop-at-wrap behavior.
        let children = doc.nodes[parent_id].children.clone();
        const KIND_SKIP: u8 = 0;
        const KIND_IMG: u8 = 1;
        const KIND_GLUE: u8 = 2;
        const KIND_BREAKABLE: u8 = 3;
        const KIND_BR: u8 = 4;
        let mut kinds: Vec<u8> = Vec::with_capacity(children.len());
        let mut widths: Vec<f32> = Vec::with_capacity(children.len());
        let mut heights: Vec<f32> = Vec::with_capacity(children.len());
        let mut collapsibles: Vec<bool> = Vec::with_capacity(children.len());
        for &child in &children {
            if !doc.nodes[child].is_in_document() {
                kinds.push(KIND_SKIP);
                widths.push(0.0);
                heights.push(0.0);
                collapsibles.push(false);
                continue;
            }
            if doc.nodes[child].tag_name() == Some("br") {
                kinds.push(KIND_BR);
                widths.push(0.0);
                heights.push(0.0);
                collapsibles.push(false);
                continue;
            }
            let is_inline_whitespace = matches!(
                &doc.nodes[child].data,
                NodeData::Text(text)
                    if !text.text_content.is_empty()
                        && text
                            .text_content
                            .chars()
                            .all(|ch| is_collapsible_ws(ch) || ch == '\u{00A0}')
            );
            let is_collapsible_whitespace = is_inline_whitespace
                && matches!(
                    cascade.computed[child].white_space,
                    WhiteSpace::Normal | WhiteSpace::Nowrap | WhiteSpace::PreLine
                )
                && matches!(
                    &doc.nodes[child].data,
                    NodeData::Text(text)
                        if text.text_content.chars().all(is_collapsible_ws)
                );
            if is_inline_whitespace {
                let width =
                    doc.nodes[child].unrounded_layout.size.width.max(
                        style_dimension_length(doc.nodes[child].style.size.width).unwrap_or(0.0),
                    );
                if width <= 0.0 || !width.is_finite() {
                    kinds.push(KIND_SKIP);
                    widths.push(0.0);
                    heights.push(0.0);
                    collapsibles.push(false);
                    continue;
                }
                let has_nbsp = matches!(
                    &doc.nodes[child].data,
                    NodeData::Text(text) if text.text_content.contains('\u{00A0}')
                );
                if has_nbsp {
                    kinds.push(KIND_GLUE);
                } else {
                    kinds.push(KIND_BREAKABLE);
                }
                widths.push(width);
                heights.push(0.0);
                collapsibles.push(is_collapsible_whitespace);
                continue;
            }
            if doc.nodes[child].tag_name() == Some("img") {
                let width = doc.nodes[child].unrounded_layout.size.width.max(0.0);
                let height = doc.nodes[child].unrounded_layout.size.height.max(0.0);
                if width <= 0.0 || !width.is_finite() {
                    kinds.push(KIND_SKIP);
                    widths.push(0.0);
                    heights.push(0.0);
                    collapsibles.push(false);
                    continue;
                }
                kinds.push(KIND_IMG);
                widths.push(width);
                heights.push(height);
                collapsibles.push(false);
                continue;
            }
            kinds.push(KIND_SKIP);
            widths.push(0.0);
            heights.push(0.0);
            collapsibles.push(false);
        }
        // Partition participating positions into wrap-atomic groups. Images
        // join the open group only through an adjacent NBSP glue node, so
        // back-to-back images without NBSP between them stay separable and
        // keep the existing wrapping behavior. Each collapsible-only or
        // preserved-space run without NBSP forms its own single-item group.
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut group_for_pos: Vec<Option<usize>> = vec![None; children.len()];
        let mut current: Vec<usize> = Vec::new();
        for pos in 0..children.len() {
            match kinds[pos] {
                KIND_IMG => {
                    let glued = current.last().is_some_and(|&last| kinds[last] == KIND_GLUE);
                    if current.is_empty() || !glued {
                        if !current.is_empty() {
                            let gid = groups.len();
                            for &p in &current {
                                group_for_pos[p] = Some(gid);
                            }
                            groups.push(std::mem::take(&mut current));
                        }
                        current.push(pos);
                    } else {
                        current.push(pos);
                    }
                }
                KIND_GLUE => {
                    current.push(pos);
                }
                KIND_BREAKABLE => {
                    if !current.is_empty() {
                        let gid = groups.len();
                        for &p in &current {
                            group_for_pos[p] = Some(gid);
                        }
                        groups.push(std::mem::take(&mut current));
                    }
                    let gid = groups.len();
                    groups.push(vec![pos]);
                    group_for_pos[pos] = Some(gid);
                }
                _ => {
                    if !current.is_empty() {
                        let gid = groups.len();
                        for &p in &current {
                            group_for_pos[p] = Some(gid);
                        }
                        groups.push(std::mem::take(&mut current));
                    }
                }
            }
        }
        if !current.is_empty() {
            let gid = groups.len();
            for &p in &current {
                group_for_pos[p] = Some(gid);
            }
            groups.push(current);
        }
        let group_widths: Vec<f32> = groups
            .iter()
            .map(|group| group.iter().map(|&p| widths[p]).sum())
            .collect();
        for pos in 0..children.len() {
            let child = children[pos];
            let kind = kinds[pos];
            if kind == KIND_SKIP {
                continue;
            }
            if kind == KIND_BR {
                x = 0.0;
                y += line_height_used;
                line_height_used = line_height;
                continue;
            }
            let width = widths[pos];
            let height = heights[pos];
            let is_collapsible_whitespace = collapsibles[pos];
            let gid = group_for_pos[pos].expect("participating position has a group");
            let group = &groups[gid];
            let is_first_in_group = group.first().is_some_and(|&first| first == pos);
            let is_glue_group = group.len() > 1;
            if is_glue_group {
                // A multi-item group holds at least one NBSP glue node by
                // construction, so the whole run moves as one unit. Only the
                // first item tests the group width. Later items skip the wrap
                // test so the run can overflow the line rather than split.
                if is_first_in_group {
                    let group_width = group_widths[gid];
                    if x > 0.0 && x + group_width > parent_width + 0.01 {
                        x = 0.0;
                        y += line_height_used;
                        line_height_used = line_height;
                        baseline_for_line = None;
                    }
                }
            } else {
                if x > 0.0 && x + width > parent_width + 0.01 {
                    x = 0.0;
                    y += line_height_used;
                    line_height_used = line_height;
                    baseline_for_line = None;
                    if is_collapsible_whitespace {
                        continue;
                    }
                }
                if is_collapsible_whitespace && x == 0.0 {
                    // CSS-collapsible leading whitespace is removed at a new line.
                    // U+00A0 is not collapsible and keeps its measured advance.
                    continue;
                }
            }
            let taffy_y = doc.nodes[child].unrounded_layout.location.y;
            let relative_offset_y = relative_position_y_offset(
                cascade,
                child,
                doc.nodes[parent_id].unrounded_layout.size.height.max(0.0),
            );
            let flow_taffy_y = taffy_y - relative_offset_y;
            let child_y = if !baseline_aligned {
                y + relative_offset_y
            } else if doc.nodes[child].style.align_self == Some(TaffyAlignItems::FLEX_START) {
                flow_taffy_y + y + relative_offset_y
            } else if doc.nodes[child].style.align_self == Some(TaffyAlignItems::FLEX_END) {
                if y == 0.0 {
                    flow_taffy_y + relative_offset_y
                } else {
                    y + line_height_used - height + relative_offset_y
                }
            } else {
                let baseline_offset = *baseline_for_line.get_or_insert(if y == 0.0 {
                    flow_taffy_y + height
                } else {
                    height
                });
                y + baseline_offset - height + relative_offset_y
            };
            let relative_offset_x = relative_position_x_offset(
                cascade,
                child,
                doc.nodes[parent_id].unrounded_layout.size.width.max(0.0),
            );
            doc.nodes[child].unrounded_layout.location.x = x + relative_offset_x;
            doc.nodes[child].unrounded_layout.location.y = child_y;
            x += width;
            line_height_used = line_height_used.max(height);
        }
        let used_height = if x > 0.0 || y == 0.0 {
            y + line_height_used
        } else {
            y
        };
        let taffy_height = doc.nodes[parent_id].unrounded_layout.size.height;
        doc.nodes[parent_id].unrounded_layout.size.height = if baseline_aligned {
            taffy_height.max(used_height)
        } else {
            used_height
        };
    }
}

// Text indentation is otherwise applied to TextData layouts, so an empty
// inline block has no text layout to receive the first-line offset. Keep this
// post-layout bridge narrow: only one empty inline-block in a horizontal LTR
// block, with optional collapsible whitespace, no modifiers or forced breaks,
// and normal-flow positioning can be offset without reflowing a line.
pub(crate) fn realign_single_empty_inline_block_indent(
    doc: &mut Document,
    cascade: &CascadeResult,
) {
    for parent_id in 0..doc.nodes.len() {
        let cv = &cascade.computed[parent_id];
        if !matches!(cv.display, DisplayValue::Block | DisplayValue::InlineBlock) {
            continue;
        }
        if cv.direction != Direction::Ltr
            || cv.writing_mode != WritingMode::HorizontalTb
            || cv.text_indent_hanging
            || cv.text_indent_each_line
            || cv.text_indent_ch_factor.is_some()
            || !matches!(
                cv.text_align,
                TextAlign::Start | TextAlign::Left | TextAlign::Right
            )
        {
            continue;
        }

        let mut inline_block = None;
        let mut unsupported_content = false;
        for &child in &doc.nodes[parent_id].children {
            if !doc.nodes[child].is_in_document() {
                continue;
            }
            if is_forced_line_break(doc, child, cascade) {
                unsupported_content = true;
                break;
            }
            match &doc.nodes[child].data {
                NodeData::Text(text)
                    if matches!(
                        cascade.computed[child].white_space,
                        WhiteSpace::Normal | WhiteSpace::Nowrap
                    ) && text.text_content.chars().all(is_css_white_space) => {}
                NodeData::Element(_)
                    if cascade.computed[child].display == DisplayValue::InlineBlock
                        && cascade.computed[child].float == FloatValue::None
                        && matches!(
                            cascade.computed[child].position,
                            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                        )
                        && doc.nodes[child].children.is_empty() =>
                {
                    if inline_block.replace(child).is_some() {
                        unsupported_content = true;
                        break;
                    }
                }
                _ => {
                    unsupported_content = true;
                    break;
                }
            }
        }
        if unsupported_content {
            continue;
        }
        let Some(inline_block) = inline_block else {
            continue;
        };

        let content_width = doc.nodes[parent_id].unrounded_layout.content_box_width();
        if !content_width.is_finite() || content_width < 0.0 {
            continue;
        }
        let indent = bounded_text_indent_amount(cv.text_indent, content_width, None);
        if !indent.is_finite() || indent == 0.0 {
            continue;
        }
        doc.nodes[inline_block].unrounded_layout.location.x += indent;
    }
}

/// Find the nearest synthetic inline formatting root for a text node.
fn inline_root_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_id: usize,
) -> Option<usize> {
    let mut ancestor = parent_of[text_id];
    while let Some(id) = ancestor {
        if doc.nodes[id].flags.contains(NodeFlags::IS_INLINE_ROOT) {
            return Some(id);
        }
        ancestor = parent_of[id];
    }
    None
}

/// Return a node's horizontal position in the document coordinate space.
fn absolute_layout_x(doc: &Document, parent_of: &[Option<usize>], mut id: usize) -> f32 {
    let mut x = 0.0;
    while let Some(parent) = parent_of[id] {
        x += doc.nodes[id].unrounded_layout.location.x;
        id = parent;
    }
    x
}

pub(crate) fn realign_text_after_layout(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
) {
    // Parent map: the arena has no parent pointers, so derive them from children.
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children.clone() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for (idx, parent) in parent_of.iter().enumerate() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if doc.nodes[idx].is_inline_svg_content() {
            continue;
        }
        if !doc.nodes[idx].is_in_document() {
            continue;
        }
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IN_IFC_SUBTREE | NodeFlags::IS_IFC_ROOT)
        {
            continue;
        }
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        let mut ancestor = *parent;
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        let mut inside_multicol = false;
        // cov:ignore: multicol fragment detection is exercised by the ignored foundation WPT run.
        while let Some(ancestor_id) = ancestor {
            if !matches!(
                cascade.computed[ancestor_id].column_count,
                ColumnCountValue::Auto
            ) || !matches!(
                cascade.computed[ancestor_id].column_width,
                ComputedColumnWidth::Auto
            ) {
                inside_multicol = true;
                break;
            }
            ancestor = parent_of[ancestor_id];
        }
        // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
        if inside_multicol
            // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
            && matches!(
                &doc.nodes[idx].data,
                NodeData::Text(text) if text.multicol_fragments.is_some()
            )
        // cov:ignore: fragment-aware text alignment is exercised by the ignored foundation WPT run.
        {
            // Fragment ranges are already width-constrained and their line
            // offsets are consumed by the multicol paint path. Nested inline
            // text still needs the ordinary post-Taffy alignment pass.
            continue;
        }
        // Resolve logical `start` / `end` to physical alignment using this
        // element's `direction` (CSS Text 3 §6.1). Parley's `Start` / `End`
        // follow content-inferred bidi direction and misalign RTL content
        // (observed in the text-align-end-001 regression).
        // Skip only `Start` + LTR (preshape already gets it right, avoiding
        // a wrapping regression caused by re-breaking).
        let cv = &cascade.computed[idx];
        let parley_align = match text_align_to_parley(cv.text_align) {
            Alignment::Start if cv.direction == Direction::Rtl => Alignment::Right,
            Alignment::End if cv.direction == Direction::Rtl => Alignment::Left,
            a => a,
        };
        // `text-justify: none` disables justification (CSS Text 3 §6.2):
        // combined with `text-align: justify` it falls back to the start
        // edge (direction-aware).
        let mut align = parley_align;
        if cv.text_justify == TextJustify::None && align == Alignment::Justify {
            align = match cv.direction {
                Direction::Rtl => Alignment::Right,
                _ => Alignment::Start,
            };
        }
        // `text-indent` (CSS Text 3 §8.1): length plus hanging/each-line
        // flags, resolved against the containing block width for `%`
        // (unknown at preshape time — same width-basis problem as align).
        // The indent applies per node position (see `indent_options_for_node`
        // doc): MidLine never, BlockStart always, AfterBreak per flags.
        let nonzero_indent = match cv.text_indent {
            ComputedTextIndent::Px(px) => px != 0.0,
            ComputedTextIndent::Percent(p) => p != 0.0,
            ComputedTextIndent::Calc(calc) => calc.px != 0.0 || calc.percent != 0.0,
        };
        let indent_options = if nonzero_indent {
            indent_options_for_node(
                cv.text_indent_hanging,
                cv.text_indent_each_line,
                line_start_pos(doc, cascade, &parent_of, idx),
            )
        } else {
            None
        };
        let measured_indent = measured_text_indent_px(cv, fonts, layout_cx, &mut ch_probes);
        // Preshape breaks at page width, not the width of a narrow container
        // (measured in slice 1b: length-001's three-line article stays on
        // one line). Re-breaking every node regressed the pinned nowrap-001
        // test, so that experiment was reverted (it needs a separate full
        // baseline review; see the preceding comment). Only re-break nodes
        // with an indent or non-Start alignment.
        // All non-flex text re-breaks against the containing width here
        // preshape only knows the page width, so
        // narrow-container wrapping would otherwise never apply. `nowrap`
        // nodes are handled by the branch below without re-breaking.
        // `Start` alignment after re-break is a no-op offset-wise; only the
        // wrap points change.
        // `Justify` / `End` also need the correct containing width.
        // This goal chiefly targets `Center`, but fixes their width too.
        let Some(parent_idx) = *parent else {
            continue;
        };
        // Defer flex line-box children to container-side justification (see docs).
        // Skip indent too: taffy measured the flex item's box without it,
        // so indenting here would separate the glyph run from its box
        // (a documented limitation).
        let needs_line_break_property = matches!(
            cv.word_break,
            WordBreak::BreakAll | WordBreak::KeepAll | WordBreak::BreakWord
        ) || !matches!(cv.overflow_wrap, OverflowWrap::Normal)
            || matches!(cv.white_space, WhiteSpace::BreakSpaces);
        let has_forced_break = doc.nodes[parent_idx]
            .children
            .iter()
            .any(|&child| is_forced_line_break(doc, child, cascade));
        if doc.nodes[parent_idx]
            .flags
            .contains(NodeFlags::IS_INLINE_ROOT)
            && !inside_multicol
            && !needs_line_break_property
            && (cv.text_indent_ch_factor.is_none()
                || cv.text_indent_hanging
                || cv.text_indent_each_line
                || has_forced_break)
        {
            continue;
        }
        let containing_width = doc.nodes[parent_idx].unrounded_layout.size.width;
        if !containing_width.is_finite() || containing_width <= 0.0 {
            continue;
        }
        let preserve_wide_body_run = !inside_multicol
            && is_leading_body_text(doc, Some(parent_idx), idx)
            && matches!(cv.text_align, TextAlign::Start | TextAlign::Left);
        let inline_continuation = inline_root_for_text(doc, &parent_of, idx).and_then(|root| {
            let root_cv = &cascade.computed[root];
            if root == parent_idx
                || cv.direction != Direction::Ltr
                || root_cv.direction != Direction::Ltr
                || root_cv.writing_mode != WritingMode::HorizontalTb
            {
                return None;
            }
            let root_x = absolute_layout_x(doc, &parent_of, root);
            let node_x = absolute_layout_x(doc, &parent_of, idx);
            let prefix = node_x - root_x;
            let width = doc.nodes[root].unrounded_layout.size.width;
            if !prefix.is_finite() || !width.is_finite() || prefix <= 0.0 || width <= prefix {
                return None;
            }
            Some((width - prefix, root_x - node_x))
        });
        let Some(layout) = doc.nodes[idx]
            .data
            .as_text_mut()
            .and_then(|t| t.text_layout.as_mut())
        else {
            continue;
        };
        if let Some((available, continuation_offset)) = inline_continuation {
            let should_rebreak = available.is_finite()
                && available > 0.0
                && available + f32::EPSILON < layout.width();
            if should_rebreak {
                layout.break_all_lines(Some(available));
                layout.align(Alignment::Start, AlignmentOptions::default());
                let line_count = layout.len();
                if let NodeData::Text(text) = &mut doc.nodes[idx].data {
                    text.text_line_offsets = Some(
                        (0..line_count)
                            .map(|line| if line == 0 { 0.0 } else { continuation_offset })
                            .collect(),
                    );
                }
                continue;
            }
        }
        // A direct body text run may intentionally overflow the page content
        // width when the paged containing block carries a margin.  Preserve
        // the page-width shaping used by the paged bridge instead of
        // re-breaking a one-line run to the narrower body box.
        if preserve_wide_body_run
            && layout.width() > containing_width + 0.01
            && !needs_line_break_property
        {
            continue;
        }
        // No-wrap (`white-space: nowrap` or `text-wrap: nowrap`): preshape
        // already broke without a width cap, so re-breaking here would wrap.
        // Apply indent and align onto the preshaped single line instead.
        // Single-line Justify is a parley no-op (last line excluded), which
        // matches `text-align: justify` under nowrap.
        if cv.white_space == WhiteSpace::Nowrap || cv.text_wrap == TextWrapMode::Nowrap {
            if let Some(options) = indent_options {
                let amount =
                    bounded_text_indent_amount(cv.text_indent, containing_width, measured_indent);
                layout.set_text_indent(amount, options);
            }
            layout.align(align, AlignmentOptions::default());
            continue;
        }
        if let Some(options) = indent_options {
            let amount =
                bounded_text_indent_amount(cv.text_indent, containing_width, measured_indent);
            layout.set_text_indent(amount, options);
        }
        layout.break_all_lines(Some(containing_width));
        // `text-align-last` (CSS Text 3 §6.1): applying an explicit last-line
        // value is outside scope. Parley's `align(Justify)` deliberately omits
        // the last line (`BreakReason::None`, observed in alignment.rs), so
        // its public API cannot reproduce last-line-only justification or
        // alignment, even on a single line. Keep `text_align_last` cascade
        // wiring (parse/wire/inherit) for future parley support. Unresolved
        // `MatchParent` is also future work: ignoring it is correct, rather
        // than approximating it as `auto` on the reader side.
        layout.align(align, AlignmentOptions::default());
    }

    // Rebreaking text after Taffy can increase an inline child's line count.
    // Propagate that used height through auto-sized ancestors so multicolumn
    // item backgrounds and the enclosing block cover the rebroken run.
    // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text || !doc.nodes[idx].is_in_document() {
            continue;
        }
        if matches!(
            &doc.nodes[idx].data,
            NodeData::Text(text) if text.multicol_fragments.is_some()
        ) {
            continue;
        }
        let Some(text_height) = doc.nodes[idx].text_layout().map(|layout| layout.height()) else {
            continue;
        };
        if !text_height.is_finite() || text_height <= 0.0 {
            continue;
        }
        doc.nodes[idx].unrounded_layout.size.height =
            doc.nodes[idx].unrounded_layout.size.height.max(text_height);
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }

    // Taffy does not include floated descendants in an auto-height
    // containing block's used height. Propagate the bottom edge of supported
    // left/right floats through auto-height ancestors, including the
    // containing-block border. This keeps following flow from moving upward
    // after a fragmented flex item with a float descendant.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element
            || !doc.nodes[idx].is_in_document()
            || !matches!(
                cascade.computed[idx].float,
                FloatValue::Left | FloatValue::Right
            )
        {
            continue;
        }
        let mut child = idx;
        while let Some(parent) = parent_of[child] {
            if matches!(
                cascade.computed[parent].height,
                ComputedLengthPercentageOrAuto::Auto
            ) {
                let child_bottom = doc.nodes[child].unrounded_layout.location.y
                    + doc.nodes[child].unrounded_layout.size.height
                    + cascade.computed[parent].border.bottom.width().px();
                doc.nodes[parent].unrounded_layout.size.height = doc.nodes[parent]
                    .unrounded_layout
                    .size
                    .height
                    .max(child_bottom);
            }
            child = parent;
        }
    }
}
/// Collapse adjacent block margins inside a non-replaced inline box.
///
/// CSS 2.1 §9.2.1.1 splits an inline containing block around block-level
/// children. Whitespace-only text between those children is part of the
/// anonymous inline fragments, not an intervening block. Taffy's block path
/// already gives the children the right vertical order, but it does not
/// perform this block-in-inline margin collapse; keeping both margins would
/// add one extra line-height-sized gap between two adjacent block children.
/// Keep this pass narrow: only direct children of a plain `inline` wrapper and
/// only resolvable absolute margins are rewritten.
pub(crate) fn collapse_block_in_inline_margins(
    doc: &mut Document,
    idx: usize,
    cascade: &CascadeResult,
) {
    if cascade.computed[idx].display != DisplayValue::Inline {
        return;
    }

    let mut previous_block = None;
    for &child in &doc.nodes[idx].children.clone() {
        if !doc.nodes[child].is_in_document() {
            continue;
        }
        match doc.nodes[child].kind() {
            NodeKind::Text => {
                // Whitespace around a block child belongs to the anonymous
                // inline fragments and must not interrupt sibling margin
                // collapse. Any visible text does interrupt that sequence.
                if !text_of(doc, child).is_some_and(|text| text.chars().all(is_css_white_space)) {
                    previous_block = None;
                }
            }
            NodeKind::Element => {
                let display = cascade.computed[child].display;
                if display == DisplayValue::None {
                    continue;
                }
                if is_inline_element_box(cascade, child) {
                    previous_block = None;
                    continue;
                }
                if let Some(previous) = previous_block {
                    collapse_adjacent_block_margins(doc, previous, child);
                }
                previous_block = Some(child);
            }
            _ => previous_block = None,
        }
    }
}

fn collapse_adjacent_block_margins(doc: &mut Document, previous: usize, next: usize) {
    let previous_margin = doc.nodes[previous].style.margin.bottom.into_raw();
    let next_margin = doc.nodes[next].style.margin.top.into_raw();
    if previous_margin.tag() != CompactLength::LENGTH_TAG
        || next_margin.tag() != CompactLength::LENGTH_TAG
    {
        return;
    }
    let previous_margin = previous_margin.value();
    let next_margin = next_margin.value();
    if !previous_margin.is_finite() || !next_margin.is_finite() {
        return;
    }
    let collapsed = if previous_margin >= 0.0 && next_margin >= 0.0 {
        previous_margin.max(next_margin)
    } else if previous_margin <= 0.0 && next_margin <= 0.0 {
        previous_margin.min(next_margin)
    } else {
        previous_margin + next_margin
    };
    if collapsed.is_finite() {
        doc.nodes[previous].style.margin.bottom = LengthPercentageAuto::length(collapsed);
        doc.nodes[next].style.margin.top = LengthPercentageAuto::length(0.0);
    }
}

/// Return the nearest block-like inline formatting context for a node.
///
/// The nested-inline bridge must not be enabled by an autospace pair in an
/// unrelated sibling block. Low-level DOM tests do not install the HTML UA
/// stylesheet, so if no block-like ancestor is visible the highest reachable
/// ancestor is used as the conservative context fallback.
fn inline_formatting_context_root(
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    node_id: usize,
) -> usize {
    let mut current = node_id;
    let mut fallback = node_id;
    while let Some(parent) = parent_of.get(current).copied().flatten() {
        fallback = parent;
        if matches!(
            cascade.computed[parent].display,
            DisplayValue::Block
                | DisplayValue::InlineBlock
                | DisplayValue::Flex
                | DisplayValue::InlineFlex
                | DisplayValue::Grid
                | DisplayValue::InlineGrid
                | DisplayValue::Table
                | DisplayValue::InlineTable
                | DisplayValue::TableRow
                | DisplayValue::TableCell
                | DisplayValue::TableCaption
        ) {
            return parent;
        }
        current = parent;
    }
    fallback
}

/// Return whether one inline formatting context contains an actual autospace
/// boundary. This reuses the same edge discovery and `text-autospace` option
/// handling as `preshape_text`, instead of combining character classes from
/// unrelated text nodes or ignoring custom values and language gating.
fn inline_context_has_autospace_candidate(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    context_root: usize,
) -> bool {
    for (idx, node) in doc.nodes.iter().enumerate() {
        if node.kind() != NodeKind::Text
            || !node.is_in_document()
            || inline_formatting_context_root(cascade, parent_of, idx) != context_root
            || cascade.computed[idx].text_autospace == TextAutospace::NoAutospace
            || has_vertical_writing_mode(cascade, parent_of, idx)
        {
            continue;
        }
        let NodeData::Text(text) = &node.data else {
            // cov:ignore: NodeKind::Text always carries NodeData::Text; this is a defensive match arm.
            continue;
        };
        let language = effective_language_for_text(doc, parent_of, idx);
        let before = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, -1);
        let after = autospace_adjacent_edge_char(doc, cascade, parent_of, idx, 1);
        if !text_autospace_boxes_with_edges(
            &text.text_content,
            cascade.computed[idx].text_autospace,
            &language,
            1.0,
            before,
            after,
        )
        .is_empty()
        {
            return true;
        }
    }
    false
}

/// Vertical line-box extent contributed by a supported `vertical-align` value.
/// Positive raised offsets extend the line's ascent; negative lowered offsets
/// extend its descent. The paint-side shift is still applied to glyphs.
/// spec: <https://www.w3.org/TR/CSS21/visudet.html#propdef-vertical-align>
fn vertical_align_linebox_extent(
    vertical_align: VerticalAlign,
    parent_font_size_px: f32,
) -> (f32, f32) {
    let shift = match vertical_align {
        VerticalAlign::Sub => parent_font_size_px / 5.0,
        VerticalAlign::Super => -(parent_font_size_px / 3.0),
        VerticalAlign::Length(Length::Px(px)) => -px,
        _ => 0.0,
    };
    if !shift.is_finite() {
        return (0.0, 0.0);
    }
    if shift < 0.0 {
        (-shift, 0.0)
    } else {
        (0.0, shift)
    }
}

#[derive(Default)]
struct SyntheticLineboxExtents {
    leading: f32,
    trailing: f32,
}

/// Accumulate vertical-align extents from an in-flow inline subtree into one
/// containing synthetic line root. Inline offsets compose along a descendant
/// path. Atomic inline-blocks and nested synthetic roots own their own content
/// line boxes, so stop after accounting for their outer shift.
fn collect_inline_linebox_extents(
    doc: &Document,
    cascade: &CascadeResult,
    synthetic_line_roots: &[bool],
    idx: usize,
    parent_font_size_px: f32,
    ancestor_shift_px: f32,
    extents: &mut SyntheticLineboxExtents,
) {
    if doc.nodes[idx].kind() != NodeKind::Element || !doc.nodes[idx].is_in_document() {
        return;
    }
    let computed = &cascade.computed[idx];
    if computed.display == DisplayValue::None
        || matches!(
            computed.position,
            PositionValue::Absolute | PositionValue::Fixed
        )
        || !matches!(
            computed.display,
            DisplayValue::Inline | DisplayValue::InlineBlock
        )
    {
        return;
    }

    let (leading, trailing) =
        vertical_align_linebox_extent(computed.vertical_align, parent_font_size_px);
    let shifted_baseline = ancestor_shift_px + trailing - leading;
    extents.leading = extents.leading.max(-shifted_baseline);
    extents.trailing = extents.trailing.max(shifted_baseline);

    if computed.display == DisplayValue::InlineBlock
        || synthetic_line_roots.get(idx).copied().unwrap_or(false)
    {
        return;
    }
    let descendant_parent_font_size_px = computed.font_size.px();
    for &child in &doc.nodes[idx].children {
        collect_inline_linebox_extents(
            doc,
            cascade,
            synthetic_line_roots,
            child,
            descendant_parent_font_size_px,
            shifted_baseline,
            extents,
        );
    }
}

/// Add synthetic line-box leading/trailing to authored padding without losing
/// a percentage component. Taffy's calc callback already resolves this mixed
/// value for bridged CSS lengths.
fn padding_with_linebox_extent(
    doc: &mut Document,
    value: ComputedLengthPercentage,
    extra: f32,
    site: &'static str,
) -> LengthPercentage {
    if extra <= 0.0 {
        return computed_length_percentage_to_taffy_length_percentage(
            value,
            site,
            &mut doc.layout_warnings,
        );
    }
    let extra = sanitize_taffy(extra, site, &mut doc.layout_warnings);
    match value {
        ComputedLengthPercentage::Px(px) => {
            LengthPercentage::length(sanitize_taffy(px + extra, site, &mut doc.layout_warnings))
        }
        ComputedLengthPercentage::Percent(percent) => {
            let percent = sanitize_taffy(percent, site, &mut doc.layout_warnings);
            let value = CalcLengthPercentage { percent, px: extra };
            doc.calc_values.push(std::sync::Arc::new(value));
            let pointer = doc
                .calc_values
                .last()
                .map(|value| (&**value) as *const _ as *const ())
                .expect("calc value was just pushed");
            LengthPercentage::calc(pointer)
        }
    }
}

pub(crate) fn establish_minimal_line_boxes(doc: &mut Document, cascade: &CascadeResult) {
    let mut parent_of = vec![None; doc.nodes.len()];
    for parent in 0..doc.nodes.len() {
        for &child in &doc.nodes[parent].children {
            if child < parent_of.len() {
                parent_of[child] = Some(parent);
            }
        }
    }
    let mut autospace_candidates_by_root = HashMap::new();
    let mut synthetic_line_roots = vec![false; doc.nodes.len()];
    let mut multicol_roots = vec![false; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE)
        {
            // Laid out by the shodo inline engine, not as a parley line box.
            continue;
        }
        // A multicolumn container establishes fragmentainers rather than the
        // synthetic single flex line used by ordinary inline roots.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        if !matches!(cascade.computed[idx].column_count, ColumnCountValue::Auto)
            // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
            || !matches!(
                cascade.computed[idx].column_width,
                ComputedColumnWidth::Auto
            )
        // cov:ignore: multicol fragmentainer roots are exercised by the ignored foundation WPT run.
        {
            multicol_roots[idx] = true;
            continue;
        }
        let has_autospace_candidate = if cascade.computed[idx].display == DisplayValue::Inline {
            let context_root = inline_formatting_context_root(cascade, &parent_of, idx);
            *autospace_candidates_by_root
                .entry(context_root)
                .or_insert_with(|| {
                    inline_context_has_autospace_candidate(doc, cascade, &parent_of, context_root)
                })
        } else {
            false
        };
        synthetic_line_roots[idx] =
            qualifies_for_minimal_line_box(doc, idx, cascade, has_autospace_candidate);
    }
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IS_IFC_ROOT | NodeFlags::IN_IFC_SUBTREE)
        {
            // Laid out by the shodo inline engine, not as a parley line box.
            doc.nodes[idx].flags.remove(NodeFlags::IS_INLINE_ROOT);
            continue;
        }
        if multicol_roots[idx] {
            doc.nodes[idx].flags.remove(NodeFlags::IS_INLINE_ROOT);
            continue;
        }
        collapse_block_in_inline_margins(doc, idx, cascade);
        let qualifies = synthetic_line_roots[idx];
        doc.nodes[idx]
            .flags
            .set(NodeFlags::IS_INLINE_ROOT, qualifies);
        if !qualifies {
            continue;
        }
        // This includes display:none children: taffy excludes them entirely
        // from the layout tree with Display::None. Checking their
        // flex_grow/flex_shrink is redundant but harmless.
        let participating_children: Vec<usize> = doc.nodes[idx]
            .children
            .iter()
            .copied()
            .filter(|&c| doc.nodes[c].is_in_document())
            .collect();
        // Does this container have any `<br>` whose display is not none?
        // This is exactly the condition in this function's "`<br>` forced break"
        // docs. A display:none `<br>` makes no box and must not count:
        // counting it would turn on `flex_wrap: Wrap` for a visually break-free
        // container, unexpectedly enabling size-based wrapping (see docs).
        let has_forced_break = participating_children
            .iter()
            .any(|&c| is_forced_line_break(doc, c, cascade));
        let container_cv = &cascade.computed[idx];
        let has_ch_indent = container_cv.text_indent_ch_factor.is_some()
            && !container_cv.text_indent_hanging
            && !container_cv.text_indent_each_line;
        // The synthetic flex bridge has no full inline formatting context, so
        // text-only roots and plain inline wrappers keep FlexStart. Preserve
        // Taffy's baseline fallback for direct in-flow inline-block and
        // replaced inline-image participants.
        let baseline_aligns_supported_boxes = participating_children.iter().any(|&child| {
            let child_cv = &cascade.computed[child];
            let is_baseline_box = child_cv.display == DisplayValue::InlineBlock
                || (child_cv.display == DisplayValue::Inline
                    && doc.nodes[child].tag_name() == Some("img"));
            is_baseline_box
                && !matches!(
                    child_cv.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
        });
        let mut linebox_extents = SyntheticLineboxExtents::default();
        for &child in &participating_children {
            collect_inline_linebox_extents(
                doc,
                cascade,
                &synthetic_line_roots,
                child,
                container_cv.font_size.px(),
                0.0,
                &mut linebox_extents,
            );
        }
        let linebox_leading = linebox_extents.leading;
        let linebox_trailing = linebox_extents.trailing;
        let linebox_padding_top = (linebox_leading > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.top,
                linebox_leading,
                "line-box-leading",
            )
        });
        let linebox_padding_bottom = (linebox_trailing > 0.0).then(|| {
            padding_with_linebox_extent(
                doc,
                container_cv.padding.bottom,
                linebox_trailing,
                "line-box-trailing",
            )
        });
        {
            let style = &mut doc.nodes[idx].style;
            if let Some(padding) = linebox_padding_top {
                style.padding.top = padding;
            }
            if let Some(padding) = linebox_padding_bottom {
                style.padding.bottom = padding;
            }
            style.display = Display::Flex;
            style.flex_direction = TaffyFlexDirection::Row;
            style.flex_wrap = if has_forced_break || has_ch_indent {
                TaffyFlexWrap::Wrap
            } else {
                TaffyFlexWrap::NoWrap
            };
            style.align_items = Some(if baseline_aligns_supported_boxes {
                TaffyAlignItems::BASELINE
            } else {
                TaffyAlignItems::FLEX_START
            });
            style.align_content = Some(TaffyAlignContent::FLEX_START);
            // Center the whole line box for `text-align: center` via flex
            // main-axis alignment, not parley. Keep other values at `None`:
            // flex handling of `Right` and others is future work.
            style.justify_content = if cascade.computed[idx].text_align == TextAlign::Center {
                Some(TaffyAlignContent::CENTER)
            } else {
                None
            };
            style.gap = Size {
                width: LengthPercentage::length(0.0),
                height: LengthPercentage::length(0.0),
            };
        }
        for c in participating_children {
            let is_break = is_forced_line_break(doc, c, cascade);
            let strip_inline_padding = cascade.computed[c].display == DisplayValue::Inline
                && has_line_box_edge_aligned_descendant(doc, c, cascade);
            let child_style = &mut doc.nodes[c].style;
            if strip_inline_padding {
                // The wrapper is bridged as a block box, so its block-axis
                // padding would move a nested top/bottom-aligned descendant
                // away from the outer line edge. For this narrow edge-aligned
                // slice, remove that padding from the wrapper's block layout.
                child_style.padding.top = LengthPercentage::length(0.0);
                child_style.padding.bottom = LengthPercentage::length(0.0);
            }
            child_style.flex_grow = 0.0;
            child_style.flex_shrink = 0.0;
            child_style.flex_basis = if is_break {
                Dimension::percent(1.0)
            } else {
                Dimension::auto()
            };
            child_style.align_self = match cascade.computed[c].vertical_align {
                VerticalAlign::Top => Some(TaffyAlignSelf::FLEX_START),
                VerticalAlign::Bottom => Some(TaffyAlignSelf::FLEX_END),
                _ => None,
            };
        }
    }
}

/// Whether `idx`, an in-document node, should act as a "`<br>` forced break"
/// in [`establish_minimal_line_boxes`] (see that function's section).
///
/// Check for `NodeKind::Element` with `tag_name() == Some("br")` and computed
/// `display` other than [`DisplayValue::None`]. This direct tag_name comparison
/// follows [`find_body`], which compares `tag_name() == Some("body")`. Identifying
/// `<br>` needs no new [`NodeKind`] variant.
fn is_forced_line_break(doc: &Document, idx: usize, cascade: &CascadeResult) -> bool {
    doc.nodes[idx].kind() == NodeKind::Element
        && doc.nodes[idx].tag_name() == Some("br")
        && cascade.computed[idx].display != DisplayValue::None
}

/// Return whether an inline wrapper contains a descendant whose aligned
/// subtree is explicitly pinned to a line-box edge.
///
/// The minimal bridge may keep a nested inline wrapper on taffy's block path
/// when it has no inline element child. A wrapper with
/// `padding-top`/`padding-bottom` would therefore move both its text and the
/// aligned descendant, unlike the CSS line-box model. The caller uses this
/// predicate only for the focused `top`/`bottom` slice.
fn has_line_box_edge_aligned_descendant(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
) -> bool {
    let mut stack = doc.nodes[idx].children.clone();
    while let Some(child) = stack.pop() {
        if doc.nodes[child].kind() == NodeKind::Element {
            if matches!(
                cascade.computed[child].vertical_align,
                VerticalAlign::Top | VerticalAlign::Bottom
            ) {
                return true;
            }
            stack.extend(doc.nodes[child].children.iter().copied());
        }
    }
    false
}

/// Whether `idx` (a [`NodeKind::Element`]) qualifies for the minimal line-box
/// handling in [`establish_minimal_line_boxes`]. See that function's docs
/// for the qualifying condition and rationale.
fn qualifies_for_minimal_line_box(
    doc: &Document,
    idx: usize,
    cascade: &CascadeResult,
    has_autospace_candidate: bool,
) -> bool {
    // Deliberately read the pre-bridge `DisplayValue`, not the post-
    // `bridge_display` `taffy::Display`: by this second pass, `Block`,
    // `Inline`, and `InlineBlock` have all collapsed to `Display::Block`.
    // Exclude plain `Inline` with only direct text children; qualify only
    // wrappers with a nested inline element child (see module docs).
    // Qualify table types (`Table` / `InlineTable` / `TableRow` /
    // `TableCell` / `TableCaption`) only with all-inline children; see
    // the match-arm note. The table dispatch in `taffy_impl.rs` checks
    // `IS_INLINE_ROOT` to bypass the table engine.
    let is_plain_inline = has_autospace_candidate
        && cascade.computed[idx].display == DisplayValue::Inline
        && cascade.computed[idx].text_autospace != TextAutospace::NoAutospace;
    if cascade.computed[idx].display == DisplayValue::Inline && !is_plain_inline {
        return false;
    }
    let has_inline_element_child = is_plain_inline
        && doc.nodes[idx].children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element && is_inline_element_box(cascade, child)
        });
    match cascade.computed[idx].display {
        DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::Inline => {}
        // Table-internal boxes whose children are ALL inline-level get no
        // anonymous fixup from the table engine (it only collects rows and
        // cells) — without this they stack vertically in taffy's block
        // path. A pure-inline table/row/cell/caption is exactly one
        // anonymous cell's content (css-tables-3 §2.2.1 fixup with a single
        // run), so flowing it inline here is equivalent. Boxes with any
        // cell/row (or other block-level) child keep the table path via the
        // disqualify arm below. Column groups/columns never have renderable
        // children and stay disqualified.
        DisplayValue::Table
        | DisplayValue::InlineTable
        | DisplayValue::TableRow
        | DisplayValue::TableCell
        | DisplayValue::TableCaption => {}
        _ => return false,
    }
    let mut inline_level_count = 0usize;
    for &c in &doc.nodes[idx].children {
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => inline_level_count += 1,
            NodeKind::Element => match cascade.computed[c].display {
                DisplayValue::Inline | DisplayValue::InlineBlock => inline_level_count += 1,
                DisplayValue::None => {}
                // Block-level / flex / grid children, as well as future
                // non_exhaustive DisplayValue variants, disqualify the parent:
                // mixed block-level and inline-level content is out of scope
                // (see module docs).
                _ => return false,
            },
            // Comment / ProcessingInstruction / DocumentFragment /
            // Document should never satisfy `is_in_document()` (see
            // `NodeData` docs); fail closed here without assuming that invariant.
            // cov:ignore: unreachable — Comment/ProcessingInstruction/DocumentFragment/Document nodes always have IS_IN_DOCUMENT cleared (see document.rs's mark_in_document_flags invariant), filtered by the is_in_document() guard above; kept as a defensive fallback rather than relying on that invariant here.
            _ => return false,
        }
    }
    if is_plain_inline {
        // A plain inline with a nested inline element needs a synthetic line
        // container so nested descendants do not take the block path and
        // stack vertically. Keep direct text-only inline containers on the
        // historical path; their content already participates in the parent
        // line box and existing layout behavior remains unchanged.
        has_inline_element_child && inline_level_count >= 1
    } else {
        inline_level_count >= 2
    }
}

/// Minimum valid `font-weight` passed to parley.
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#font-weight-prop> specifies
/// `<number [1,1000]>`. This sink (`parley::FontWeight`) is distinct from
/// [`MAX_TAFFY_MAGNITUDE`] / [`MAX_FONT_SIZE_PX`], so use the spec range
/// rather than a shared bound: limits depend on the sink. Unlike skrifa /
/// CSSWG issues, this site needs no engineering measurement.
const MIN_FONT_WEIGHT: f32 = 1.0;

/// Maximum valid `font-weight` passed to parley (CSS Fonts 4 §2.2).
const MAX_FONT_WEIGHT: f32 = 1000.0;

/// Fallback for non-finite (`NaN`) `font-weight`.
///
/// CSS Fonts 4 §2.2 "Font weight: the font-weight property"
/// <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> assigns
/// `normal` this computed value, matching `ComputedValues::initial().
/// font_weight` (see `crates/raikiri-style/src/computed.rs`).
///
/// [`sanitize_finite`] maps NaN to `0.0` at length sinks because the spec's
/// initial padding/margin is geometric zero or because infinite-precision
/// `0 * inf = NaN` should evaluate to zero. Neither justification applies
/// to font-weight. `0.0` is outside `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]`;
/// using it for NaN would leave the sanitized value outside the sink's valid
/// range. Font-weight therefore has its own fallback.
const FALLBACK_FONT_WEIGHT: f32 = 400.0;

/// Sanitize non-finite (`NaN` / `±Inf`) or out-of-range
/// `[MIN_FONT_WEIGHT, MAX_FONT_WEIGHT]` `font-weight` immediately before
/// passing it to `parley::FontWeight::new`.
///
/// # Why guard here?
///
/// Guard non-finite / out-of-range f32 values at the sink where they are used
/// (target context), not at parse time (specified layer) or during resolution
/// (computed layer). The inherited root argument of
/// [`raikiri_style::page::cascade_page`] (raikiri-style), direct construction
/// of `ComputedValues`, and `raikiri_style::cascade::resolve_relative_weight`
/// all belong to the resolve/computed layer. This policy expressly forbids
/// adding guards there: "do not sanitize public computed-layer surfaces".
/// Placing this function in [`preshape_text`], the sole caller of
/// `parley::FontWeight::new`, matches the sink-adjacent guards at the other
/// five sites (four taffy bridge helpers plus the font-size guard in
/// `preshape_text`); this is site 6.
///
/// # Why use [`FALLBACK_FONT_WEIGHT`] (`400.0`) rather than `0.0` for NaN?
///
/// Do not reuse [`sanitize_finite`] directly, for two reasons:
///
/// 1. **Different NaN fallback.** [`sanitize_finite`] unconditionally maps
///    NaN to `0.0`, but its rationale (see its docs) does not apply to
///    font-weight. Zero is outside the valid range. CSS Fonts 4 §2.2
///    "Font weight: the font-weight property"
///    <https://www.w3.org/TR/css-fonts-4/#valdef-font-weight-normal> gives
///    `normal` the computed value `400.0`; using it follows the policy that
///    fallbacks and limits vary by sink.
/// 2. **Cost of changing the signature.** Adding a `nan_fallback` parameter
///    could unify the functions in theory, but it would add an irrelevant
///    argument to the existing length-related call sites (four via
///    `sanitize_taffy` plus one direct font-size call) and all their tests.
///    A small separate function keeps the diff proportional to the single
///    font-weight sink in scope.
///
/// # Do not change the producer (`resolve_relative_weight`)
///
/// Do not alter `resolve_relative_weight`'s asymmetric handling of `Bolder`,
/// `Lighter`, and `-Inf` (see its docs). Changing resolution is forbidden by
/// this policy; this sink guard only ensures that whatever the resolver
/// produces ends up finite and within the valid range.
fn sanitize_font_weight(v: f32, diag: &mut Vec<LayoutWarn>) -> f32 {
    let clamped = if v.is_nan() {
        FALLBACK_FONT_WEIGHT
    } else {
        v.clamp(MIN_FONT_WEIGHT, MAX_FONT_WEIGHT)
    };
    if clamped != v {
        push_layout_warn(
            diag,
            LayoutWarn::NonFiniteClamped {
                site: "font-weight",
                raw: v,
                clamped,
            },
        );
    }
    clamped
}

/// `raikiri_style::property::FontStyle` (`Normal | Italic | Oblique`,
/// `#[non_exhaustive]`) → parley's `FontStyle` (re-exported from the
/// `parlance` crate: `Normal | Italic | Oblique(Option<f32>)`, CSS Fonts 4
/// §2.4 <https://www.w3.org/TR/css-fonts-4/#font-style-prop>).
///
/// `Oblique` has no `<angle>` payload on the raikiri-style side (bare
/// keyword only — see `raikiri_style::property::FontStyle` doc's "Scope
/// carving" section), so it maps to parley's `Oblique(None)`, which per
/// parley's own doc uses the engine-specific default oblique angle. The
/// wildcard arm exists purely for `StyleFontStyle`'s `#[non_exhaustive]`
/// forward-compat contract (a downstream match must tolerate variants
/// added to the source enum later, e.g. a future `<angle>`-bearing
/// oblique) and is unreachable with the variant set that exists today.
fn font_style_to_parley(v: StyleFontStyle) -> FontStyle {
    match v {
        StyleFontStyle::Normal => FontStyle::Normal,
        StyleFontStyle::Italic => FontStyle::Italic,
        StyleFontStyle::Oblique => FontStyle::Oblique(None),
        // cov:ignore: unreachable while StyleFontStyle is
        // Normal|Italic|Oblique only; required for its #[non_exhaustive]
        // contract (see doc above).
        _ => FontStyle::Normal,
    }
}

/// Sanitize [`ComputedLineHeight`] (`cascade.computed[idx].line_height`;
/// all [`ComputedValues`] fields are `pub`, so values may be non-finite)
/// immediately before parley's two numeric sinks (`ComputedLineHeight::Number`
/// and `Length`). Pass `Normal` through; it has no numeric value.
///
/// The asymmetric `[0.0, MAX]` clamp follows CSS Inline 3 §5.1,
/// "Line Spacing: the line-height property"
/// (<https://www.w3.org/TR/css-inline-3/#propdef-line-height>):
/// `normal | <number [0,∞]> | <length-percentage [0,∞]>`. As at existing
/// `font-size` / `font-weight` sanitization sites, negative values are outside
/// the grammar's valid range.
///
/// # `+Inf` **hangs** by a different route than at other sites
///
/// Measured by pushing `StyleProperty::LineHeight` directly to `RangedBuilder`
/// and bounding a worker thread with `recv_timeout`:
/// `parley::LineHeight::Absolute(f32::INFINITY)` and
/// `FontSizeRelative(f32::INFINITY)` both hang in `break_all_lines`.
/// The mechanism differs from the font-size hang, in which
/// `next_x <= max_advance` stays false with `next_x = +Inf` (see
/// [`MAX_FONT_SIZE_PX`]). In `parley-0.10.0/src/layout/line_break.rs`,
/// `BreakerState::add_line_height` calculates `running_line_height =
/// running_line_height.max(height)`. When height is `+Inf`, the running
/// height also becomes `+Inf`, so `running_line_height > line_max_height`
/// (with default `line_max_height = f32::MAX`) remains true. The
/// `max_height_exceeded` branch does not advance, causing the hang.
///
/// `NaN` does **not** hang: Rust's `f32::max` discards NaN and returns the
/// other operand (not IEEE 754 total-order behavior), so the running height
/// remains finite and advances. This matches the asymmetry at font-size site
/// 5: `NaN`, `-Inf`, and huge finite values do not hang there (see
/// `parley_break_all_lines_completes_for_nan_neg_inf_and_huge_finite_font_size`).
///
/// `FontSizeRelative` can also overflow in `value * font_size` (see
/// [`MAX_LINE_HEIGHT_NUMBER`]); [`MAX_LINE_HEIGHT_NUMBER`] caps it to prevent
/// that overflow. `Absolute` does no multiplication, so even `f32::MAX` alone
/// does not overflow (`f32::MAX > f32::MAX` is false). Reuse
/// [`MAX_FONT_SIZE_PX`] as its limit not to prevent huge finite values but
/// to clamp already non-finite input to a finite value. A value this large
/// is typographically unreasonable for line-height (px), just as for font-size.
pub(crate) fn sanitize_line_height(
    v: ComputedLineHeight,
    diag: &mut Vec<LayoutWarn>,
) -> ComputedLineHeight {
    match v {
        ComputedLineHeight::Normal => ComputedLineHeight::Normal,
        ComputedLineHeight::Number(n) => ComputedLineHeight::Number(sanitize_finite(
            n,
            0.0,
            MAX_LINE_HEIGHT_NUMBER,
            "line-height (number)",
            diag,
        )),
        ComputedLineHeight::Length(len) => {
            ComputedLineHeight::Length(ComputedLength(sanitize_finite(
                len.px(),
                0.0,
                MAX_FONT_SIZE_PX,
                "line-height (length)",
                diag,
            )))
        }
    }
}

/// [`ComputedLineHeight`] (`Normal | Number | Length`) → parley's
/// [`LineHeight`] (`MetricsRelative | FontSizeRelative | Absolute`,
/// `parley-0.10.0/src/style/mod.rs`).
///
/// The caller must make the value finite with [`sanitize_line_height`]
/// first. This function maps values but does not sanitize them, just as
/// [`font_style_to_parley`] separates mapping from sanitization.
///
/// # Value mapping (reason for each arm)
///
/// - [`ComputedLineHeight::Normal`] → `LineHeight::MetricsRelative(1.0)`:
///   parley's own `Default` (`impl Default for LineHeight` in
///   `parley-0.10.0/src/style/mod.rs`). This preserves the old behavior
///   that relied on the parley default before line-height was wired.
/// - [`ComputedLineHeight::Number`] → `LineHeight::FontSizeRelative`:
///   parley computes `FontSizeRelative(value) * font_size` (the `push_run`
///   code in `parley-0.10.0/src/layout/data.rs`). This `font_size` is the
///   **element's own** computed font-size (`StyleProperty::FontSize`) pushed
///   to the same `RangedBuilder` by `preshape_text`. That matches CSS Inline
///   3 §5.1: a child multiplies the unitless number by its own font-size.
/// - [`ComputedLineHeight::Length`] → `LineHeight::Absolute`: pass the px
///   value already absolutized by the computed layer (see
///   [`ComputedLineHeight::Length`]). Parley uses
///   `LineHeight::Absolute(value) => value` (`data.rs`), with no second resolve.
///
/// [`ComputedLineHeight`] intentionally lacks `#[non_exhaustive]` (see the
/// [`raikiri_style::resolve`] module docs). Unlike [`font_style_to_parley`],
/// this match has no wildcard arm: adding a variant should fail compilation.
fn line_height_to_parley(v: ComputedLineHeight) -> LineHeight {
    match v {
        ComputedLineHeight::Normal => LineHeight::MetricsRelative(1.0),
        ComputedLineHeight::Number(n) => LineHeight::FontSizeRelative(n),
        ComputedLineHeight::Length(len) => LineHeight::Absolute(len.px()),
    }
}

/// Pre-shape every Text node with parley and store the result in `Node.text_layout`.
///
/// Assume the caller (`layout_single_page`) has cleared every
/// `Node.text_layout = None` first (for safe re-entry).
/// Use the font stack, size, weight, style, and line-height from
/// `cascade.computed[idx]` (already inherited from the parent).
/// `max_advance` is the wrap boundary, usually `page_box.width`.
///
/// # Why wire `line_height` before taffy's `compute_root_layout`?
///
/// This function runs **before** taffy's `compute_root_layout` within
/// `layout_single_page` (see `text_align` below). Properties that require
/// taffy's resolved width cannot be handled this early, but `line_height`
/// does not: its dependency goes in the opposite direction, so this order
/// is **necessary**.
///
/// - During shaping (`builder.build(&text)`, before `break_all_lines`),
///   parley computes `RunMetrics.line_height` in `push_run` in
///   `parley-0.10.0/src/layout/data.rs`. Its inputs are the font metrics
///   (ascent / descent / leading obtained directly from the shaped font),
///   this element's computed `font_size` already pushed by this function,
///   and the `StyleProperty::LineHeight` pushed here. It does not depend on
///   the containing block width or available space resolved by taffy
///   (the corresponding arm in `parley-0.10.0/src/resolve/mod.rs` only
///   multiplies by `device pixel scale`).
/// - Conversely, the shaped `Layout::height()` already includes line height.
///   `compute_child_layout` in `taffy_impl.rs` supplies it *to* taffy as
///   a leaf's intrinsic size. Line height thus constructs an input to taffy,
///   rather than requiring taffy's output. This reverses the dependency of
///   `text_align`, which requires taffy's width for alignment (see below).
///   Running here is exactly the right time, not too early.
///
/// # Unused `ComputedValues` fields
///
/// `cascade.computed[idx]` also contains `direction` / `text_align`, neither
/// of which this function reads:
///
/// - `direction`: no API accepts it. `RangedBuilder` / `TreeBuilder` expose
///   no public base-direction argument, and parley's internal bidi resolver
///   always receives `None` for its base level. It infers direction from
///   paragraph text using the Unicode Bidirectional Algorithm P2/P3
///   first-strong-character heuristic, defaulting to LTR when no strong
///   directional character exists (Unicode Standard Annex #9,
///   <https://www.unicode.org/reports/tr9/>). There is currently no API to
///   pass `cv.direction` explicitly; the default is not hard-coded LTR.
/// - `text_align`: this function deliberately fixes its final
///   `layout.align(...)` at `Alignment::Start` without reading
///   `cv.text_align`. After taffy, [`realign_text_after_layout`] re-breaks
///   and aligns using the resolved containing block width. A direct enum
///   mapping here would only have `max_advance` (usually `page_box.width`),
///   wrongly offsetting `Center` / `Right` / `End` / `Justify` inside narrow
///   containers against the page width. `Start` does not depend on width.
///   Moreover, `parley::Alignment::Start` / `End` resolve physical direction
///   from the layout's bidi analysis. Without wiring `direction`, wiring
///   only `text_align` would still resolve `Start` / `End` from the text's
///   inferred direction. These are one coupled gap, not two independent ones:
///   parley's public API does not expose direction wiring (see above).
///   On flex-converted line boxes (`IS_INLINE_ROOT`), the container handles
///   centering through `justify_content`, not parley (see
///   [`establish_minimal_line_boxes`]).
///
/// # Infallible
///
/// This formerly returned `Result<(), LayoutError>`. Its only `Err` case
/// was `LayoutError::Internal` when `cv.font_size` was a specified-layer
/// `Length` other than `Px`. Since `font_size` became [`ComputedLength`] (px),
/// that match and error became unreachable. The function is `pub(crate)`,
/// so narrowing the return type has no external effect.
///
/// Preserve the previous per-text-node expansion for inline contexts whose
/// shared line cursor and wrap positions are not available to pre-shaping.
/// Also returns the byte length each tab was replaced by (in source order) so
/// offsets into `text` can be remapped.
fn expand_tabs_locally(
    text: &str,
    tab_size: ComputedTabSize,
    space_advance: f32,
) -> (String, Vec<usize>) {
    let stop = match tab_size {
        ComputedTabSize::Number(number) if number > 0.0 && number.is_finite() => number,
        ComputedTabSize::Length(length)
            if length.px().is_finite() && length.px() >= 0.0 && space_advance > 0.0 =>
        {
            length.px() / space_advance
        }
        _ => 0.0,
    };
    let mut output = String::with_capacity(text.len());
    let mut tab_lens = Vec::new();
    if stop <= 0.0 {
        output.extend(text.chars().filter(|&character| character != '\t'));
        tab_lens.resize(text.matches('\t').count(), 0);
        return (output, tab_lens);
    }
    let mut column = 0.0_f32;
    for character in text.chars() {
        match character {
            '\n' => {
                column = 0.0;
                output.push(character);
            }
            '\t' => {
                let next = ((column / stop).floor() + 1.0) * stop;
                let count = (next.round() - column.round()).max(0.0) as usize;
                column = next;
                output.extend(std::iter::repeat_n(' ', count));
                tab_lens.push(count);
            }
            _ => {
                column += 1.0;
                output.push(character);
            }
        }
    }
    (output, tab_lens)
}

/// Move inline-box offsets computed on `original` onto the text produced by a
/// rewrite that replaced its tabs (in source order) by `tab_lens` bytes and
/// kept every other character. Autospace boundaries are detected before tabs
/// are rewritten so a tab still separates its neighbors even when it expands
/// to nothing.
fn remap_boxes_through_tab_rewrite(original: &str, tab_lens: &[usize], boxes: &mut [InlineBox]) {
    let tabs: Vec<usize> = original.match_indices('\t').map(|(at, _)| at).collect();
    debug_assert_eq!(tabs.len(), tab_lens.len());
    for inline_box in boxes {
        let shift: isize = tabs
            .iter()
            .zip(tab_lens)
            .take_while(|&(&at, _)| at < inline_box.index)
            .map(|(_, &len)| len as isize - 1)
            .sum();
        inline_box.index = inline_box.index.saturating_add_signed(shift);
    }
}

const LINE_BREAK_NBSP_INLINE_BOX_ID_BASE: u64 = 1 << 62;
const LINE_BREAK_NBSP_INLINE_BOX_ID_LIMIT: u64 = 1 << 63;

/// Advance-only box for a trailing `word-space-transform` separator that faces
/// inline content (CSS Text 4, issue um59.29).
///
/// A trailing U+200B at a text-node edge becomes a trailing ASCII space (or
/// U+3000) after phase-1 collapsing, but Parley `width()` excludes trailing
/// whitespace while `full_width()` includes it. Taffy measures `width()`, so
/// the inter-node advance would be lost. Stripping that trailing run and
/// re-adding it as an `InFlow` box keeps the advance in `width()` while
/// preserving the soft-wrap opportunity: Parley always allows a break after
/// an inline box, unlike an NBSP migration which forbids it.
const WORD_SPACE_EDGE_INLINE_BOX_ID_BASE: u64 = 1 << 61;

/// Replace U+00A0 in Parley's input with a nonpainting advance box.
///
/// Existing inline-box offsets are based on the source string and must move
/// left by two UTF-8 bytes for every preceding NBSP that is removed.
fn replace_nbsp_with_inline_boxes(
    text: &str,
    advance: f32,
    existing_boxes: &[InlineBox],
) -> Option<(String, Vec<InlineBox>)> {
    if !advance.is_finite() || advance <= 0.0 {
        return None;
    }

    let nbsp_offsets: Vec<usize> = text
        .match_indices('\u{00A0}')
        .map(|(offset, _)| offset)
        .collect();
    if nbsp_offsets.is_empty() {
        return Some((text.to_owned(), existing_boxes.to_vec()));
    }

    let id_count = u64::try_from(nbsp_offsets.len()).ok()?;
    let last_id = LINE_BREAK_NBSP_INLINE_BOX_ID_BASE.checked_add(id_count.checked_sub(1)?)?;
    if last_id >= LINE_BREAK_NBSP_INLINE_BOX_ID_LIMIT
        || existing_boxes.iter().any(|inline_box| {
            (LINE_BREAK_NBSP_INLINE_BOX_ID_BASE..LINE_BREAK_NBSP_INLINE_BOX_ID_LIMIT)
                .contains(&inline_box.id)
        })
    {
        return None;
    }

    let mut output = String::with_capacity(text.len());
    let mut adapter_boxes = Vec::with_capacity(nbsp_offsets.len());
    for (source_index, character) in text.char_indices() {
        if character == '\u{00A0}' {
            let ordinal = u64::try_from(adapter_boxes.len()).ok()?;
            adapter_boxes.push(InlineBox {
                id: LINE_BREAK_NBSP_INLINE_BOX_ID_BASE.checked_add(ordinal)?,
                kind: InlineBoxKind::InFlow,
                index: output.len(),
                width: advance,
                height: 0.0,
            });
        } else {
            debug_assert!(source_index <= text.len());
            output.push(character);
        }
    }

    let mut ordered_boxes = Vec::with_capacity(existing_boxes.len() + adapter_boxes.len());
    for (order, inline_box) in existing_boxes.iter().enumerate() {
        let source_index = inline_box.index;
        if source_index > text.len() || !text.is_char_boundary(source_index) {
            return None;
        }
        let removed_bytes = nbsp_offsets
            .partition_point(|&offset| offset < source_index)
            .checked_mul('\u{00A0}'.len_utf8())?;
        let output_index = source_index.checked_sub(removed_bytes)?;
        let mut inline_box = inline_box.clone();
        inline_box.index = output_index;
        // At a shared output index, the original source boundary determines
        // whether a box was before, between, or after removed NBSP characters.
        ordered_boxes.push((output_index, source_index, 0_u8, order, inline_box));
    }
    for (order, (source_index, inline_box)) in
        nbsp_offsets.iter().copied().zip(adapter_boxes).enumerate()
    {
        let output_index = inline_box.index;
        ordered_boxes.push((output_index, source_index, 1_u8, order, inline_box));
    }
    ordered_boxes.sort_by_key(|(output_index, source_index, kind, order, _)| {
        (*output_index, *source_index, *kind, *order)
    });
    let boxes = ordered_boxes
        .into_iter()
        .map(|(_, _, _, _, inline_box)| inline_box)
        .collect();
    Some((output, boxes))
}

/// Resolve `tab-size` to an absolute stop interval in CSS pixels.
fn tab_stop_advance(tab_size: ComputedTabSize, block_space_advance: f32) -> f32 {
    match tab_size {
        ComputedTabSize::Number(number)
            if number.is_finite() && number >= 0.0 && block_space_advance.is_finite() =>
        {
            (number * block_space_advance).clamp(0.0, MAX_TAFFY_MAGNITUDE)
        }
        ComputedTabSize::Length(length) if length.px().is_finite() && length.px() >= 0.0 => {
            length.px().min(MAX_TAFFY_MAGNITUDE)
        }
        _ => 0.0,
    }
}

/// Byte ranges of tab replacement spaces and the word spacing each needs.
type TabSpacingRanges = Vec<(std::ops::Range<usize>, f32)>;

/// Replace each tab by an invisible U+0020 with ranged WordSpacing so its
/// advance exactly reaches the next stop. Keeping a normal glyph run preserves
/// the text's font metrics and line height; the ranged spacing supplies the
/// block-container-derived physical width when the inline font differs.
///
/// Also returns the byte length each tab was replaced by (in source order).
pub(crate) fn replace_tabs_with_styled_spaces(
    text: &str,
    interval: f32,
    space_base_advance: f32,
    mut measure_segment: impl FnMut(&str) -> f32,
) -> (String, TabSpacingRanges, Vec<usize>) {
    if !text.contains('\t') {
        return (text.to_owned(), Vec::new(), Vec::new());
    }
    let mut tab_lens = Vec::new();
    let mut output = String::with_capacity(text.len());
    let mut spacing_ranges = Vec::new();
    let mut segment = String::new();
    let mut cursor = 0.0_f32;
    let mut flush_segment = |segment: &mut String, output: &mut String, cursor: &mut f32| {
        if segment.is_empty() {
            return;
        }
        let measured = measure_segment(segment);
        if measured.is_finite() && measured > 0.0 {
            *cursor = (*cursor + measured).min(MAX_TAFFY_MAGNITUDE);
        }
        output.push_str(segment);
        segment.clear();
    };
    for character in text.chars() {
        match character {
            '\t' => {
                flush_segment(&mut segment, &mut output, &mut cursor);
                let before = output.len();
                if interval > 0.0 && interval.is_finite() {
                    let next = ((cursor / interval).floor() + 1.0) * interval;
                    let gap = (next - cursor).max(0.0);
                    if gap > 0.0 {
                        let start = output.len();
                        output.push(' ');
                        let word_spacing = (gap - space_base_advance)
                            .clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE);
                        spacing_ranges.push((start..output.len(), word_spacing));
                    }
                    cursor = next.min(MAX_TAFFY_MAGNITUDE);
                }
                tab_lens.push(output.len() - before);
            }
            '\n' => {
                flush_segment(&mut segment, &mut output, &mut cursor);
                output.push(character);
                cursor = 0.0;
            }
            _ => segment.push(character),
        }
    }
    flush_segment(&mut segment, &mut output, &mut cursor);
    (output, spacing_ranges, tab_lens)
}

/// Measure one probe glyph/character advance (px) with Parley.
///
/// The caller chooses the sample because tabs use U+0020 while `ch` uses the
/// U+0030 zero glyph. Non-finite, zero, or implausibly large results fall back
/// to `font_size * 0.5` as a fail-safe.
///
/// [`Layout::width`]: parley::Layout::width
fn probe_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        sample,
        family_str,
        font_size_px,
        font_weight,
        font_style,
        false,
    )
}

fn probe_ch_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    probe_text_advance_inner(
        fonts,
        layout_cx,
        "0",
        family_str,
        font_size_px,
        font_weight,
        font_style,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn probe_text_advance_inner(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    allow_zero: bool,
) -> f32 {
    let mut warnings: Vec<LayoutWarn> = Vec::new();
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(font_weight, &mut warnings);
    let family = FontFamily::from(family_str);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, true);
    builder.push_default(StyleProperty::FontFamily(family));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    let w = layout.width();
    let has_glyph_run = allow_zero
        && layout
            .lines()
            .flat_map(|line| line.items())
            .any(|item| match item {
                PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
                _ => false, // cov:ignore: this helper shapes plain text and cannot create non-glyph items.
            });
    if w.is_finite() && (w > 0.0 || has_glyph_run) && w <= size * 4.0 {
        w
    } else {
        size * 0.5
    }
}

#[derive(Clone, Copy)]
struct TextProbeStyle<'a> {
    family_str: &'a str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
    letter_spacing: f32,
    word_spacing: f32,
}

fn probe_text_full_width(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    sample: &str,
    style: TextProbeStyle<'_>,
) -> f32 {
    let mut warnings = Vec::new();
    let size = sanitize_finite(
        style.font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    );
    let weight = sanitize_font_weight(style.font_weight, &mut warnings);
    let mut builder = layout_cx.ranged_builder(fonts, sample, 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(
        style.family_str,
    )));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
        style.font_style,
    )));
    builder.push_default(StyleProperty::LetterSpacing(style.letter_spacing));
    builder.push_default(StyleProperty::WordSpacing(style.word_spacing));
    let mut layout: Layout<()> = builder.build(sample);
    layout.break_all_lines(None);
    layout.full_width()
}

/// Return whether the requested face maps both the space and zero glyphs.
///
/// CSS's first-available `ch` face must be usable for the space and must also
/// provide U+0030, whose advance supplies the metric. A mapped zero-advance
/// glyph is valid; cmap presence is the only coverage check here.
fn family_candidate_has_ch_glyphs(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Return whether any face in a generic family maps both required glyphs.
fn generic_family_has_ch_glyphs(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_ch_glyphs = false;
    query.matches_with(|font| {
        has_ch_glyphs = font.charmap().is_some_and(|charmap| {
            charmap.map(0x20_u32).is_some_and(|glyph| glyph != 0)
                && charmap.map(0x30_u32).is_some_and(|glyph| glyph != 0)
        });
        if has_ch_glyphs {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_ch_glyphs
}

/// Measure CSS `ch` from the selected font's U+0030 advance.
///
/// Uses the same font-selection and glyph-coverage checks as the layout sink.
/// If no selected face provides a usable zero glyph, this returns the
/// deterministic `0.5em` fallback used by style resolution.
///
/// [`raikiri_style::ChFontKey`] preserves the declaring element's family,
/// size, weight, and style so callers can resolve authored `ch` provenance
/// after font-face registration.
pub fn measure_ch_advance_for_font_key(
    fonts: &mut FontContext,
    key: &raikiri_style::ChFontKey,
) -> f32 {
    let family = key
        .family
        .iter()
        .map(|family| family.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let mut layout_cx = LayoutContext::<()>::new();
    probe_ch_text_advance(
        fonts,
        &mut layout_cx,
        &family,
        key.size.px(),
        key.weight,
        key.style,
    )
}

/// Probe U+0030 using the first remaining family that can supply the glyph.
///
/// Named faces are checked through Fontique cmap metadata. An unregistered
/// name is skipped so a later available family can win; when a generic family
/// is reached, Parley receives the remaining authored list so its fallback
/// resolver preserves generic and later-family order. A valid zero-advance
/// mapped glyph remains accepted by `probe_ch_zero_advance`.
fn probe_ch_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        if raw_candidate.is_empty() {
            // cov:ignore: candidates were already filtered for emptiness.
            continue;
        }
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            None
        } else {
            let lower_name = name.to_ascii_lowercase();
            parley::fontique::GenericFamily::parse(&lower_name)
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        let has_glyphs =
            registered && family_candidate_has_ch_glyphs(fonts, name, font_weight, font_style);
        if let Some(generic) = generic {
            // An unresolved named face may represent an @font-face source
            // that the WPT loader could not activate (for example a missing
            // or malformed web-font resource). Do not silently replace that unavailable
            // face with a generic metric; the style-layer fallback is the
            // deterministic 0.5em result for this no-face case.
            if saw_unregistered_named
                || !generic_family_has_ch_glyphs(fonts, generic, font_weight, font_style)
            {
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if has_glyphs {
            return probe_ch_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    // No remaining family advertises U+0030. Do not shape the rejected list
    // again: Parley may emit a .notdef run whose advance would masquerade as
    // a valid zero-width glyph. Match the style-layer fallback instead.
    let mut warnings = Vec::new();
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut warnings,
    ) * 0.5
}

/// Return whether a named face maps U+6C34, the character used by CSS `ic`.
fn family_candidate_has_ic_glyph(
    fonts: &mut FontContext,
    family: &str,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families([family]);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_glyph = false;
    query.matches_with(|font| {
        has_glyph = font
            .charmap()
            .is_some_and(|charmap| charmap.map(0x6C34_u32).is_some_and(|glyph| glyph != 0));
        if has_glyph {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_glyph
}

/// Return whether a generic family has any face mapping U+6C34.
fn generic_family_has_ic_glyph(
    fonts: &mut FontContext,
    generic: parley::fontique::GenericFamily,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> bool {
    use parley::fontique::{Attributes, FontWeight, FontWidth, QueryStatus};

    let families: Vec<_> = fonts.collection.generic_families(generic).collect();
    if families.is_empty() {
        // cov:ignore: generic-family availability is platform-dependent and may be empty.
        return false;
    }
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut query = fonts.collection.query(&mut fonts.source_cache);
    query.set_families(families);
    query.set_attributes(Attributes::new(
        FontWidth::default(),
        font_style_to_parley(font_style),
        FontWeight::new(weight),
    ));
    let mut has_glyph = false;
    query.matches_with(|font| {
        has_glyph = font
            .charmap()
            .is_some_and(|charmap| charmap.map(0x6C34_u32).is_some_and(|glyph| glyph != 0));
        if has_glyph {
            QueryStatus::Stop
        } else {
            QueryStatus::Continue
        }
    });
    has_glyph
}

/// Measure U+6C34 using the first available family that maps it.
///
/// CSS `ic` uses the ideographic advance from a mapped face. Do not accept a
/// `.notdef` advance when a family lacks U+6C34, and preserve a genuine zero
/// advance. If no candidate maps the character, use the specified 1em fallback.
///
/// The computed family list currently does not retain `@font-face`
/// `unicode-range` provenance for this metric. Such alias-specific filtering
/// remains a known limitation; ordinary registered-family and generic fallback
/// selection is still checked against cmap metadata here.
fn probe_ic_text_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let candidates: Vec<&str> = family_str
        .split(',')
        .map(str::trim)
        .filter(|candidate| !candidate.is_empty())
        .collect();
    let mut saw_unregistered_named = false;
    for (index, raw_candidate) in candidates.iter().copied().enumerate() {
        let was_quoted = raw_candidate.starts_with('"') || raw_candidate.starts_with('\'');
        let name = raw_candidate
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                raw_candidate
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .unwrap_or(raw_candidate)
            .trim();
        let generic = if was_quoted {
            // Quoted generic-looking names are ordinary family names.
            None
        } else {
            parley::fontique::GenericFamily::parse(&name.to_ascii_lowercase())
        };
        let registered = fonts.collection.family_id(name).is_some();
        if !registered && generic.is_none() {
            saw_unregistered_named = true;
        }
        if let Some(generic) = generic {
            if saw_unregistered_named
                || !generic_family_has_ic_glyph(fonts, generic, font_weight, font_style)
            {
                // An earlier unavailable family prevents this generic fallback.
                continue;
            }
            let remaining_families = candidates[index..].join(", ");
            return probe_ic_zero_advance(
                fonts,
                layout_cx,
                &remaining_families,
                font_size_px,
                font_weight,
                font_style,
            );
        }
        if registered && family_candidate_has_ic_glyph(fonts, name, font_weight, font_style) {
            return probe_ic_zero_advance(
                fonts,
                layout_cx,
                raw_candidate,
                font_size_px,
                font_weight,
                font_style,
            );
        }
    }
    sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut Vec::new(),
    )
}

fn probe_ic_zero_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    family_str: &str,
    font_size_px: f32,
    font_weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let size = sanitize_finite(
        font_size_px,
        0.0,
        MAX_FONT_SIZE_PX,
        "font-size",
        &mut Vec::new(),
    );
    let weight = sanitize_font_weight(font_weight, &mut Vec::new());
    let mut builder = layout_cx.ranged_builder(fonts, "水", 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::from(family_str)));
    builder.push_default(StyleProperty::FontSize(size));
    builder.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
    builder.push_default(StyleProperty::FontStyle(font_style_to_parley(font_style)));
    let mut layout: Layout<()> = builder.build("水");
    layout.break_all_lines(None);
    let has_glyph = layout
        .lines()
        .flat_map(|line| line.items())
        .any(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => run.glyphs().any(|glyph| glyph.id != 0),
            // cov:ignore: a plain single-character metric layout is expected to contain only glyph runs.
            _ => false,
        });
    let width = layout.full_width();
    // Keep the same 4em malformed-font guard used by the existing `ch`
    // metric probe while accepting zero as a valid mapped advance.
    if has_glyph && width.is_finite() && width >= 0.0 && width <= size * 4.0 {
        width
    } else {
        // cov:ignore: malformed or missing-glyph metric fallback is defensive.
        size
    }
}

/// Measure the `ch` advance needed by a computed `text-indent` value.
///
/// The cache key mirrors the text shaping face selection used by
/// [`probe_text_advance`], and the caller supplies the same font context that
/// shaped the document's text. The caller supplies the authored factor only
/// for `ch` values; this function returns the clamped used px value.
/// `probe_ch_text_advance`, cached per font selection in `probes`.
fn cached_ch_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
    family: &str,
    size_px: f32,
    weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    let style = match font_style {
        StyleFontStyle::Normal => 0,
        StyleFontStyle::Italic => 1,
        StyleFontStyle::Oblique => 2,
        _ => 0,
    };
    let key = (
        family.to_owned(),
        size_px.to_bits(),
        weight.to_bits(),
        style,
    );
    *probes.entry(key).or_insert_with(|| {
        probe_ch_text_advance(fonts, layout_cx, family, size_px, weight, font_style)
    })
}

/// The `ch` advance for a spacing value: measured with the font that declared
/// it when `source` records one, otherwise with the given shaping font.
#[allow(clippy::too_many_arguments)]
fn spacing_ch_advance(
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
    source: Option<&raikiri_style::ChFontKey>,
    family: &str,
    size_px: f32,
    weight: f32,
    font_style: StyleFontStyle,
) -> f32 {
    match source {
        Some(key) => {
            let family = key
                .family
                .iter()
                .map(|family| family.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            cached_ch_advance(
                fonts,
                layout_cx,
                probes,
                &family,
                key.size.px(),
                key.weight,
                key.style,
            )
        }
        None => cached_ch_advance(
            fonts, layout_cx, probes, family, size_px, weight, font_style,
        ),
    }
}

fn measured_ch_length_px(
    factor: f32,
    source: Option<&raikiri_style::ChFontKey>,
    cv: &ComputedValues,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
) -> f32 {
    let family_atoms = source.map(|key| &key.family).unwrap_or(&cv.font_family);
    let family = family_atoms
        .iter()
        .map(|family| family.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let size = source.map_or(cv.font_size, |key| key.size);
    let weight = source.map_or(cv.font_weight, |key| key.weight);
    let font_style = source.map_or(cv.font_style, |key| key.style);
    let advance = cached_ch_advance(
        fonts,
        layout_cx,
        probes,
        &family,
        size.px(),
        weight,
        font_style,
    );
    let used = factor * advance;
    if used.is_nan() {
        0.0
    } else {
        used.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

fn measured_text_indent_px(
    cv: &ComputedValues,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    probes: &mut HashMap<(String, u32, u32, u8), f32>,
) -> Option<f32> {
    let factor = cv.text_indent_ch_factor?;
    let measured = measured_ch_length_px(
        factor,
        cv.text_indent_ch_font.as_ref(),
        cv,
        fonts,
        layout_cx,
        probes,
    ) + cv.text_indent_ch_offset;
    Some(if measured.is_nan() {
        0.0
    } else {
        measured.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    })
}

pub(crate) fn bounded_text_indent_amount(
    value: ComputedTextIndent,
    containing_width: f32,
    measured: Option<f32>,
) -> f32 {
    let raw = match value {
        ComputedTextIndent::Px(px) => measured.unwrap_or(px),
        ComputedTextIndent::Percent(percent) => containing_width * percent / 100.0,
        // A measured `ch` calc already folds its absolute part into `measured`,
        // so only the percentage term is added on top.
        ComputedTextIndent::Calc(calc) => match measured {
            Some(measured) => measured + containing_width * calc.percent / 100.0,
            None => calc.px + containing_width * calc.percent / 100.0,
        },
    };
    if raw.is_nan() {
        0.0
    } else {
        raw.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
    }
}

/// [`ComputedLength`]: raikiri_style::ComputedLength
/// [`ComputedLineHeight`]: raikiri_style::ComputedLineHeight
/// CSS white-space characters (CSS Text 3 §4.1.1): space, tab, LF, FF, CR.
fn is_css_white_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\x0C' | '\r')
}

/// Zero-width space (U+200B): a segment break adjacent to it is removed
/// without leaving a space (CSS Text 3 §4.1.2 rule 1).
const ZERO_WIDTH_SPACE: char = '\u{200B}';

/// Hangul blocks for the CSS Text 3 §4.1.2 Hangul carve-out (a break with
/// Hangul on either side is NOT removed even between wide characters):
/// Jamo, Compatibility Jamo, Extended-A/B, Syllables. Ranges are stable
/// since Unicode 2.0 and need no data tables.
fn is_hangul_for_break(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'
            | '\u{3130}'..='\u{318F}'
            | '\u{A960}'..='\u{A97F}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{D7B0}'..='\u{D7FF}'
    )
}

/// East Asian Width map for segment-break decisions (CSS Text 3 §4.1.2
/// rule 2: removal when both sides are Fullwidth/Wide/Halfwidth).
/// Loaded once per process from ICU compiled data (same provider the
/// parley dependency already links; no new data pulled in).
fn east_asian_width_map()
-> icu_properties::CodePointMapDataBorrowed<'static, icu_properties::props::EastAsianWidth> {
    icu_properties::CodePointMapDataBorrowed::<icu_properties::props::EastAsianWidth>::new()
}

/// Whether `c` is a default-ignorable code point for segment-break
/// neighbor selection, excluding U+200B whose explicit rule is handled by
/// the caller.
fn is_segment_break_ignorable(c: char) -> bool {
    // U+200B has an explicit segment-break rule and must remain visible to
    // the caller; other default-ignorable code points (variation selectors,
    // soft hyphen, LRM, etc.) do not participate in the EAW neighbor test.
    c != ZERO_WIDTH_SPACE
        && icu_properties::CodePointSetData::new::<
            icu_properties::props::DefaultIgnorableCodePoint,
        >()
        .contains(c)
}

/// Whether `c` counts as wide for break removal: East Asian Width
/// Fullwidth/Wide/Halfwidth (Ambiguous excluded) and not Hangul (which
/// the spec carves out even when wide, e.g. Hangul syllables).
fn is_wide_for_break(c: char) -> bool {
    use icu_properties::props::EastAsianWidth;
    let ea = east_asian_width_map().get(c);
    (ea == EastAsianWidth::Fullwidth
        || ea == EastAsianWidth::Wide
        || ea == EastAsianWidth::Halfwidth)
        && !is_hangul_for_break(c)
}

/// Deepest edge text char inside element `elem` (`dir` -1: last,
/// +1: first), skipping `display:none` subtrees and empty text.
/// Returns `None` when the element holds no text (e.g. empty inline or
/// image): callers treat that as narrow, never as a boundary.
fn deep_edge_char(doc: &Document, cascade: &CascadeResult, elem: usize, dir: i8) -> Option<char> {
    let kids = &doc.nodes[elem].children;
    let range: Box<dyn Iterator<Item = usize>> = if dir < 0 {
        Box::new((0..kids.len()).rev())
    } else {
        Box::new(0..kids.len())
    };
    for i in range {
        let c = kids[i];
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => {
                let t = text_of(doc, c)?;
                if t.is_empty() {
                    continue;
                }
                return if dir < 0 {
                    t.chars().next_back()
                } else {
                    t.chars().next()
                };
            }
            NodeKind::Element => {
                if cascade.computed[c].display == DisplayValue::None {
                    continue;
                }
                if let Some(ch) = deep_edge_char(doc, cascade, c, dir) {
                    return Some(ch);
                }
            }
            _ => continue,
        }
    }
    None
}

/// Directly neighboring char of text node `idx` (`dir` -1: char before,
/// +1: char after) in reading order: in-sibling text edge (including
/// collapsible spaces — adjacency to a space keeps it), else the edge
/// text of a neighboring element (deep), else ascending through inline
/// ancestors like [`has_inline_adjacent`]. `None` at a block boundary or
/// when no text is found (treated as narrow, never as a break).
fn edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let p = parent_of[node]?;
        let kids = &doc.nodes[p].children;
        let pos = kids.iter().position(|&c| c == node)?;
        let mut i = pos as isize + dir as isize;
        while i >= 0 && (i as usize) < kids.len() {
            let sib = kids[i as usize];
            i += dir as isize;
            if !doc.nodes[sib].is_in_document() {
                continue;
            }
            match doc.nodes[sib].kind() {
                NodeKind::Text => {
                    let t = text_of(doc, sib)?;
                    if t.is_empty() {
                        continue;
                    }
                    return if dir < 0 {
                        t.chars().next_back()
                    } else {
                        t.chars().next()
                    };
                }
                NodeKind::Element => {
                    if cascade.computed[sib].display == DisplayValue::None {
                        continue;
                    }
                    if let Some(ch) = deep_edge_char(doc, cascade, sib, dir) {
                        return Some(ch);
                    }
                    continue;
                }
                _ => continue,
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return None;
    }
}

/// Check one side of a text node for a neighboring whitespace-only node that
/// contains a segment break. Unlike `whitespace_run_has_break`, this keeps
/// the direction so a leading space prefix is not associated with a later,
/// unrelated break in the same inline sequence.
fn whitespace_edge_has_break(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let Some(parent) = parent_of[idx] else {
        return false;
    };
    let Some(pos) = doc.nodes[parent].children.iter().position(|&c| c == idx) else {
        return false;
    };
    let mut i = pos as isize + dir as isize;
    while i >= 0 && (i as usize) < doc.nodes[parent].children.len() {
        let sibling = doc.nodes[parent].children[i as usize];
        i += dir as isize;
        if !doc.nodes[sibling].is_in_document() {
            continue;
        }
        if doc.nodes[sibling].kind() == NodeKind::Element
            && cascade.computed[sibling].display == DisplayValue::None
        {
            continue;
        }
        if doc.nodes[sibling].kind() != NodeKind::Text {
            break;
        }
        let Some(sibling_text) = text_of(doc, sibling) else {
            break;
        };
        if !sibling_text.chars().all(is_css_white_space) {
            break;
        }
        if sibling_text.contains(['\n', '\r']) {
            return true;
        }
    }
    false
}

/// Remove whitespace runs whose segment break is removable because of the
/// significant characters on both sides. The ordinary collapse pass still
/// handles all other runs; this narrow pre-pass only prevents it from
/// turning a removable CJK break into a space after the parser combines the
/// surrounding indentation and character into one text node.
fn remove_removable_segment_break_runs(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    s: &str,
) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < chars.len() {
        if !is_css_white_space(chars[i]) {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && is_css_white_space(chars[i]) {
            i += 1;
        }
        let end = i;
        let has_break = chars[start..end]
            .iter()
            .any(|&ch| matches!(ch, '\n' | '\r'))
            || (start == 0 && whitespace_edge_has_break(doc, cascade, parent_of, idx, -1))
            || (end == chars.len() && whitespace_edge_has_break(doc, cascade, parent_of, idx, 1));
        if !has_break {
            out.extend(chars[start..end].iter().copied());
            continue;
        }
        let significant =
            |ch: &&char| !is_css_white_space(**ch) && !is_segment_break_ignorable(**ch);
        let before = chars[..start]
            .iter()
            .rev()
            .find(significant)
            .copied()
            .or_else(|| edge_non_whitespace_char(doc, cascade, parent_of, idx, -1));
        let after = chars[end..]
            .iter()
            .find(significant)
            .copied()
            .or_else(|| edge_non_whitespace_char(doc, cascade, parent_of, idx, 1));
        let removable = before == Some(ZERO_WIDTH_SPACE)
            || after == Some(ZERO_WIDTH_SPACE)
            || matches!((before, after), (Some(a), Some(b)) if is_wide_for_break(a) && is_wide_for_break(b));
        if !removable {
            out.extend(chars[start..end].iter().copied());
        }
    }
    out
}

/// Whether the contiguous text-node whitespace run around `idx` contains
/// a segment break. This handles the common HTML-tokenizer shape where a
/// run's spaces and character-reference LF tokens become separate siblings.
/// A mixed text node is intentionally not crossed: its own collapse pass has
/// already seen the complete text and must remain the owner of that run.
fn whitespace_run_has_break(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    text: &str,
) -> bool {
    if text.contains(['\n', '\r']) {
        return true;
    }
    let Some(parent) = parent_of[idx] else {
        return false;
    };
    let Some(pos) = doc.nodes[parent].children.iter().position(|&c| c == idx) else {
        return false;
    };
    for dir in [-1isize, 1] {
        let mut i = pos as isize + dir;
        while i >= 0 && (i as usize) < doc.nodes[parent].children.len() {
            let sibling = doc.nodes[parent].children[i as usize];
            i += dir;
            if !doc.nodes[sibling].is_in_document() {
                continue;
            }
            if doc.nodes[sibling].kind() == NodeKind::Element
                && cascade.computed[sibling].display == DisplayValue::None
            {
                continue;
            }
            if doc.nodes[sibling].kind() != NodeKind::Text {
                break;
            }
            let Some(sibling_text) = text_of(doc, sibling) else {
                break;
            };
            if !sibling_text.chars().all(is_css_white_space) {
                break;
            }
            if sibling_text.contains(['\n', '\r']) {
                return true;
            }
        }
    }
    false
}

/// Find the nearest non-CSS-whitespace character at an element edge.
///
/// This is deliberately different from `deep_edge_char`: segment-break
/// transformation has to look through the whole collapsible run, not merely
/// at the first LF/space text node in that run. The HTML tokenizer commonly
/// creates one text node per character reference, so a run such as
/// `一些\n\n\n中文` is represented by several adjacent whitespace nodes.
fn deep_edge_non_whitespace_char(
    doc: &Document,
    cascade: &CascadeResult,
    elem: usize,
    dir: i8,
) -> Option<char> {
    let kids = &doc.nodes[elem].children;
    let range: Box<dyn Iterator<Item = usize>> = if dir < 0 {
        Box::new((0..kids.len()).rev())
    } else {
        Box::new(0..kids.len())
    };
    for i in range {
        let c = kids[i];
        if !doc.nodes[c].is_in_document() {
            continue;
        }
        match doc.nodes[c].kind() {
            NodeKind::Text => {
                let Some(t) = text_of(doc, c) else {
                    continue;
                };
                let edge = if dir < 0 {
                    t.chars()
                        .rev()
                        .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                } else {
                    t.chars()
                        .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                };
                if edge.is_some() {
                    return edge;
                }
            }
            NodeKind::Element => {
                if cascade.computed[c].display == DisplayValue::None {
                    continue;
                }
                if let Some(ch) = deep_edge_non_whitespace_char(doc, cascade, c, dir) {
                    return Some(ch);
                }
            }
            _ => continue,
        }
    }
    None
}

/// Directly neighboring non-whitespace character of a text node in reading
/// order. CSS-whitespace-only text nodes are skipped so callers can evaluate
/// one segment-break run even when the parser split it into many nodes.
fn edge_non_whitespace_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let p = parent_of[node]?;
        let kids = &doc.nodes[p].children;
        let pos = kids.iter().position(|&c| c == node)?;
        let mut i = pos as isize + dir as isize;
        while i >= 0 && (i as usize) < kids.len() {
            let sib = kids[i as usize];
            i += dir as isize;
            if !doc.nodes[sib].is_in_document() {
                continue;
            }
            match doc.nodes[sib].kind() {
                NodeKind::Text => {
                    let Some(t) = text_of(doc, sib) else {
                        continue;
                    };
                    let edge = if dir < 0 {
                        t.chars()
                            .rev()
                            .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                    } else {
                        t.chars()
                            .find(|&ch| !is_css_white_space(ch) && !is_segment_break_ignorable(ch))
                    };
                    if edge.is_some() {
                        return edge;
                    }
                }
                NodeKind::Element => {
                    if cascade.computed[sib].display == DisplayValue::None {
                        continue;
                    }
                    if let Some(ch) = deep_edge_non_whitespace_char(doc, cascade, sib, dir) {
                        return Some(ch);
                    }
                }
                _ => continue,
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return None;
    }
}

/// Whether `idx` generates an inline-level box for white-space trimming.
///
/// Text nodes are inline-level; elements follow their computed display
/// (`inline`, `inline-block`, `inline-table`). `contents` is treated as
/// inline (conservative: a wrong keep preserves old behavior, a wrong drop
/// would regress). `<br>` counts as a block boundary (leading/trailing
/// spaces around a forced break collapse).
fn is_inline_for_trim(doc: &Document, cascade: &CascadeResult, idx: usize) -> bool {
    if doc.nodes[idx].kind() == NodeKind::Text {
        return true;
    }
    // Absolutely/fixed-positioned boxes are out of the inline flow and cannot
    // keep a collapsible space at the boundary of the following text.
    if matches!(
        cascade.computed[idx].position,
        PositionValue::Absolute | PositionValue::Fixed
    ) {
        return false;
    }
    if doc.nodes[idx].tag_name() == Some("br") {
        return false;
    }
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Whether the element's own box is inline-level (strict: `contents` and
/// `<br>` excluded — used for ancestor ascent, where a `contents` wrapper
/// must not stop the search but a real inline box does not continue it...
/// actually ascent continues through ALL inline ancestors; this helper
/// reports whether `idx` (an element) generates an inline-level box,
/// `<br>` included as inline for ascent since content inside... `<br>` has
/// no children, so it never matters here).
pub(crate) fn is_inline_element_box(cascade: &CascadeResult, idx: usize) -> bool {
    matches!(
        cascade.computed[idx].display,
        DisplayValue::Inline
            | DisplayValue::InlineBlock
            | DisplayValue::InlineTable
            | DisplayValue::Contents
    )
}

/// Nearest significant sibling of the node holding text `idx` in `dir`
/// (-1: preceding, +1: following), skipping `display:none` boxes and
/// whitespace-only text nodes (both generate nothing trimmable-against).
/// Returns the sibling id, or the ancestor to ascend to when the parent's
/// edge is reached without finding one (handled by the caller via
/// `inline_beyond_block_edge`).
fn significant_sibling(
    doc: &Document,
    cascade: &CascadeResult,
    parent: usize,
    pos: usize,
    dir: i8,
) -> Option<usize> {
    let kids = &doc.nodes[parent].children;
    let mut i = pos as isize + dir as isize;
    while i >= 0 && (i as usize) < kids.len() {
        let sib = kids[i as usize];
        i += dir as isize;
        if !doc.nodes[sib].is_in_document() {
            continue;
        }
        if doc.nodes[sib].kind() == NodeKind::Element
            && (cascade.computed[sib].display == DisplayValue::None
                || matches!(
                    cascade.computed[sib].position,
                    PositionValue::Absolute | PositionValue::Fixed
                ))
        {
            continue;
        }
        if doc.nodes[sib].kind() == NodeKind::Text
            && text_of(doc, sib).is_some_and(|t| t.chars().all(is_css_white_space))
        {
            continue;
        }
        return Some(sib);
    }
    None
}

/// Whether inline-level content precedes/follows text node `idx`
/// (`dir` -1/+1) for collapsible-space trimming (CSS Text 3 §4.1.2 phases
/// II–III): spaces at a block boundary are removed; between inlines kept.
/// Ascends through inline ancestors so `x<span> a</span>` keeps its space.
fn has_inline_adjacent(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let mut node = idx;
    loop {
        let p = match parent_of[node] {
            Some(p) => p,
            None => return false,
        };
        let kids = &doc.nodes[p].children;
        let pos = match kids.iter().position(|&c| c == node) {
            Some(pos) => pos,
            None => return false,
        };
        if let Some(sib) = significant_sibling(doc, cascade, p, pos, dir) {
            return is_inline_for_trim(doc, cascade, sib);
        }
        // Parent edge: ascend iff the parent itself is inline-level.
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            node = p;
            continue;
        }
        return false;
    }
}

/// Find the nearest in-flow inline character on one side of a text node.
///
/// Text nodes are shaped independently, but autospace applies across ordinary
/// inline-element boundaries. Whitespace, block-level boxes, atomic inline
/// boxes, and isolation boundaries stop the search rather than being skipped;
/// default-ignorable code points such as variation selectors are looked past.
pub(crate) fn autospace_adjacent_edge_char(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> Option<char> {
    let mut node = idx;
    loop {
        let parent = parent_of[node]?;
        let children = &doc.nodes[parent].children;
        let position = children.iter().position(|&child| child == node)?;
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(children[..position].iter().rev())
        } else {
            Box::new(children[position + 1..].iter())
        };
        for &sibling in siblings {
            match boundary_inline_edge(doc, cascade, sibling, dir, is_segment_break_ignorable) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return None,
                InlineEdge::Char(character) => return Some(character),
            }
        }
        if doc.nodes[parent].kind() == NodeKind::Element
            && is_inline_element_box(cascade, parent)
            && !is_shaping_isolation_boundary(doc, parent)
            && !boundary_shaping_box_breaks(cascade, parent)
        {
            node = parent;
            continue;
        }
        return None;
    }
}

/// Whether a character belongs to a script whose joining behavior must
/// continue across inline box boundaries. The CSS Text boundary-shaping cases covered
/// here exercise Arabic, N'Ko, and Mongolian; a zero-width joiner at an
/// inline boundary lets Parley retain the same joining context when the DOM
/// stores each styled text node in a separate layout.
fn is_boundary_shaping_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x0600..=0x06ff
            | 0x0750..=0x077f
            | 0x07c0..=0x07ff
            | 0x08a0..=0x08ff
            | 0x1800..=0x18af
            | 0xfb50..=0xfdff
            | 0xfe70..=0xfeff
    )
}

/// Add the zero-width joiners needed to preserve joining-script context at
/// inline box boundaries while each DOM text node is shaped independently.
///
/// Only the character on each edge matters: a space, an authored ZWNJ, or a
/// non-joining script at that edge already ends the joining context, so no
/// joiner is added there even when the rest of the run is joining script.
fn add_boundary_shaping_joiners(mut text: String, before: bool, after: bool) -> String {
    if after
        && text
            .chars()
            .next_back()
            .is_some_and(is_boundary_shaping_char)
    {
        text.push('\u{200d}');
    }
    if before && text.chars().next().is_some_and(is_boundary_shaping_char) {
        text.insert(0, '\u{200d}');
    }
    text
}

/// Whether an element creates a bidi isolation boundary for shaping.
///
/// The current style bridge does not expose `unicode-bidi` in computed values,
/// so cover the HTML isolation forms used by the WPT boundary-shaping tests:
/// `<bdi>` and an inline element with `dir="auto"`.
fn is_shaping_isolation_boundary(doc: &Document, node_id: usize) -> bool {
    doc.nodes[node_id].tag_name() == Some("bdi")
        || doc.nodes[node_id]
            .attribute("dir")
            .is_some_and(|value| value.eq_ignore_ascii_case("auto"))
}

/// Whether a boundary box prevents joining across its inline content.
///
/// Nonzero physical inline-edge margins, padding, and borders are shaping
/// boundaries; outline and text decoration are not. The current style bridge
/// normalizes writing mode to horizontal-tb, so the inline edges are left and
/// right. Atomic inline-level boxes also cannot share a shaping context with
/// their siblings.
fn boundary_shaping_box_breaks(cascade: &CascadeResult, node_id: usize) -> bool {
    let cv = &cascade.computed[node_id];
    if !matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents) {
        return true;
    }
    let nonzero_length_percentage = |value: ComputedLengthPercentage| match value {
        ComputedLengthPercentage::Px(px) | ComputedLengthPercentage::Percent(px) => {
            px.abs() > f32::EPSILON
        }
    };
    let nonzero_length_percentage_or_auto = |value: ComputedLengthPercentageOrAuto| match value {
        ComputedLengthPercentageOrAuto::Px(px) | ComputedLengthPercentageOrAuto::Percent(px) => {
            px.abs() > f32::EPSILON
        }
        ComputedLengthPercentageOrAuto::Calc(_) => true,
        ComputedLengthPercentageOrAuto::Auto => false,
    };
    [cv.margin.left, cv.margin.right]
        .into_iter()
        .any(nonzero_length_percentage_or_auto)
        || [cv.padding.left, cv.padding.right]
            .into_iter()
            .any(nonzero_length_percentage)
        || [cv.border.left.width().px(), cv.border.right.width().px()]
            .into_iter()
            .any(|width| width.abs() > f32::EPSILON)
}

/// Whether a joining-script shaping context may cross the selected inline
/// boundary: the first rendered character beyond the boundary must itself be
/// able to join (a joining-script character or an authored ZWJ), and no
/// box-model, atomic, line-break, or bidi-isolation boundary may intervene.
fn boundary_shaping_adjacent(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    dir: i8,
) -> bool {
    let mut node = idx;
    loop {
        let p = match parent_of[node] {
            Some(p) => p,
            None => return false,
        };
        let kids = &doc.nodes[p].children;
        let pos = match kids.iter().position(|&c| c == node) {
            Some(pos) => pos,
            None => return false,
        };
        let siblings: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
            Box::new(kids[..pos].iter().rev())
        } else {
            Box::new(kids[pos + 1..].iter())
        };
        for &sib in siblings {
            match boundary_inline_edge(doc, cascade, sib, dir, |_| false) {
                InlineEdge::Empty => continue,
                InlineEdge::Break => return false,
                InlineEdge::Char(ch) => {
                    return ch == '\u{200d}' || is_boundary_shaping_char(ch);
                }
            }
        }
        if doc.nodes[p].kind() == NodeKind::Element && is_inline_element_box(cascade, p) {
            if is_shaping_isolation_boundary(doc, p) || boundary_shaping_box_breaks(cascade, p) {
                return false;
            }
            node = p;
            continue;
        }
        return false;
    }
}

/// What a subtree contributes at the edge facing a shaping boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InlineEdge {
    /// The subtree renders nothing inline; look further along the boundary.
    Empty,
    /// The subtree ends the joining context before any character.
    Break,
    /// The first rendered character met from the boundary side.
    Char(char),
}

/// Walk into `idx` from the side facing the boundary (`dir` +1 enters from
/// the start, -1 from the end) and report the first character it renders,
/// ignoring characters for which `skip` returns true. Out-of-flow and `display:none` boxes render nothing inline; atomic
/// inlines, `<br>`, bidi isolates, and inline boxes with nonzero inline-edge
/// margin/border/padding break the context.
fn boundary_inline_edge(
    doc: &Document,
    cascade: &CascadeResult,
    idx: usize,
    dir: i8,
    skip: fn(char) -> bool,
) -> InlineEdge {
    let node = &doc.nodes[idx];
    if !node.is_in_document() {
        return InlineEdge::Empty;
    }
    if let Some(text) = text_of(doc, idx) {
        let edge = if dir < 0 {
            text.chars().rev().find(|&ch| !skip(ch))
        } else {
            text.chars().find(|&ch| !skip(ch))
        };
        return edge.map_or(InlineEdge::Empty, InlineEdge::Char);
    }
    // Comments and processing instructions never carry the in-document flag,
    // so any remaining node here is an element.
    let cv = &cascade.computed[idx];
    if cv.display == DisplayValue::None
        || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
    {
        return InlineEdge::Empty;
    }
    if !is_inline_for_trim(doc, cascade, idx)
        || is_shaping_isolation_boundary(doc, idx)
        || boundary_shaping_box_breaks(cascade, idx)
    {
        return InlineEdge::Break;
    }
    let children = &node.children;
    let ordered: Box<dyn Iterator<Item = &usize>> = if dir < 0 {
        Box::new(children.iter().rev())
    } else {
        Box::new(children.iter())
    };
    for &child in ordered {
        match boundary_inline_edge(doc, cascade, child, dir, skip) {
            InlineEdge::Empty => continue,
            edge => return edge,
        }
    }
    InlineEdge::Empty
}

/// Raw text content of a text node.
fn text_of(doc: &Document, idx: usize) -> Option<&str> {
    match &doc.nodes[idx].data {
        crate::node::NodeData::Text(t) => Some(t.text_content.as_str()),
        _ => None,
    }
}

/// Collapse `text` per its computed `white-space` (CSS Text 3 §4.1) for
/// shaping: segment breaks/tabs become spaces (except `pre` family),
/// collapsible runs merge, and spaces at block boundaries are removed
/// (sibling context via `parent_of`, built once per preshape pass).
/// `pre`/`pre-wrap`/`break-spaces` pass through untouched; `pre-line`
/// keeps newlines but collapses spaces with boundary trimming.
/// Result of [`collapse_text_for_shaping`]: shaped text plus whether a
/// kept trailing space was stripped for forward migration.
pub(crate) struct CollapsedText {
    /// Text to shape (leading kept spaces already NBSP-ified in place;
    /// trailing kept spaces stripped — see `migrate_count`).
    pub(crate) text: String,
    /// A collapsible trailing space was removed while inline content
    /// follows: the caller prepends one NBSP to the next shaped text
    /// (parley keeps leading NBSP, trims trailing — so spaces only ever
    /// migrate forward, never stay trailing).
    pub(crate) migrate_count: u32,
}

fn has_authored_non_whitespace_text(doc: &Document, idx: usize) -> bool {
    text_of(doc, idx).is_some_and(|text| text.chars().any(|ch| !is_css_white_space(ch)))
        || doc.nodes[idx]
            .children
            .iter()
            .any(|&child| has_authored_non_whitespace_text(doc, child))
}

// cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
fn preserves_inline_whitespace_item(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    fallback_width: f32,
) -> bool {
    let Some(parent) = parent_of.get(idx).copied().flatten() else {
        return false;
    };
    if multicol_metrics_for_node(cascade, parent_of, parent, fallback_width).is_some()
        || !matches!(
            cascade.computed[parent].display,
            DisplayValue::Block | DisplayValue::InlineBlock
        )
    {
        return false;
    }
    let Some(pos) = doc.nodes[parent]
        .children
        .iter()
        .position(|&child| child == idx)
    else {
        return false;
    };
    let has_inline_element = |children: &[usize]| {
        children.iter().any(|&child| {
            doc.nodes[child].kind() == NodeKind::Element
                && is_inline_element_box(cascade, child)
                && cascade.computed[child].display != DisplayValue::None
                && (has_authored_non_whitespace_text(doc, child)
                    // Replaced inline content has no text descendants.
                    || doc.nodes[child].tag_name() == Some("img"))
        })
    };
    has_inline_element(&doc.nodes[parent].children[..pos])
        && has_inline_element(&doc.nodes[parent].children[pos + 1..])
}

pub(crate) fn collapse_text_for_shaping(
    doc: &Document,
    cascade: &CascadeResult,
    parent_of: &[Option<usize>],
    idx: usize,
    text: &str,
    ws: WhiteSpace,
) -> CollapsedText {
    match ws {
        WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces => {
            return CollapsedText {
                text: text.to_string(),
                migrate_count: 0,
            };
        }
        _ => {}
    }
    // Flex/grid containers drop whitespace-only text children outright
    // (CSS Flexbox 1 §4 / CSS Grid 1 §5.2 anonymous-item rules: collapsed
    // runs vanish instead of becoming zero-size items). Without this the
    // migration below would inject NBSPs between flex items (e.g. WPT
    // flexbox_flex-0-0's newline-separated spans), shifting them apart.
    // Content-bearing text (anonymous flex/grid items) keeps flowing below.
    if text.chars().all(is_css_white_space)
        && let Some(p) = parent_of[idx]
        && doc.nodes[p].kind() == NodeKind::Element
        && matches!(
            cascade.computed[p].display,
            DisplayValue::Flex | DisplayValue::Grid
        )
    {
        return CollapsedText {
            text: String::new(),
            migrate_count: 0,
        };
    }
    // `matches!` carries its own wildcard arm, so this stays exhaustive
    // against the non-exhaustive `WhiteSpace` (cross-crate match without a
    // visible wildcard would not compile).
    // Phase I: CR/CRLF become LF; tabs/FF become spaces. Line feeds are
    // KEPT here (even for `normal`/`nowrap`): an interior single `\n`
    // carries East Asian Width / adjacency context that only the shaper
    // resolves correctly (CSS Text 3 §4.1.2 segment-break rules — parley
    // implements them; a blanket `\n`→space conversion breaks e.g. WPT
    // segment-break-transformation-rules-001 fullwidth/fullwidth). Runs of
    // 2+ line feeds collapse to one space below (multi-break runs never
    // reach the shaper as breaks; WPT removable-2 pins this), while a
    // single interior break passes through. Edge breaks are decided after
    // the boundary flags are known.
    let mut s = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            s.push('\n');
        } else if c == '\t' || c == '\x0C' {
            s.push(' ');
        } else {
            s.push(c);
        }
    }
    // Line-feed runs: `pre-line` keeps them verbatim for the shaper
    // (paragraph breaks survive); `normal`/`nowrap` collapse every maximal
    // run to one space here. The single-vs-wide decision for a lone break
    // between inlines happens in the all-whitespace arm below with East
    // Asian Width data; single-node content keeps the space approximation
    // it always had (multi-break collapse is pinned by WPT removable-2).
    let is_pre_line = matches!(ws, WhiteSpace::PreLine);
    // Lone-break optimized path (`normal`/`nowrap` only): an all-whitespace
    // node holding a segment-break run decides with direct-neighbor context
    // instead of the blanket conversion below. CSS Text 3 §4.1.2: a break
    // next to a zero-width space vanishes; next to a collapsible space the
    // space
    // survives (migrate); in a multi-break run one space survives; between
    // two wide (Fullwidth/Wide/Halfwidth, non-Hangul) chars it vanishes;
    // otherwise one space survives. `pre-line` keeps its feeds for the
    // shaper via the normal pipeline (dedupe carve-out above). Mixed text
    // nodes are handled by the removable-run pre-pass below.
    if !is_pre_line
        && s.chars().all(is_css_white_space)
        && whitespace_run_has_break(doc, cascade, parent_of, idx, text)
    {
        // A segment-break run may be split into several whitespace text
        // nodes by the HTML tokenizer. Let only its first node decide the
        // result; otherwise each LF would independently migrate a space.
        if edge_char(doc, cascade, parent_of, idx, -1).is_some_and(is_css_white_space) {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        let before = has_inline_adjacent(doc, cascade, parent_of, idx, -1);
        let after = has_inline_adjacent(doc, cascade, parent_of, idx, 1);
        if !before || !after {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        // Skip all CSS whitespace nodes surrounding this run before applying
        // the segment-break transformation rules. In particular, removable-3
        // and removable-4 put ordinary spaces around every LF; those spaces
        // are part of the same sequence and must not migrate before the wide
        // character test runs.
        let p = edge_non_whitespace_char(doc, cascade, parent_of, idx, -1);
        let n = edge_non_whitespace_char(doc, cascade, parent_of, idx, 1);
        if p == Some(ZERO_WIDTH_SPACE) || n == Some(ZERO_WIDTH_SPACE) {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        if let (Some(a), Some(b)) = (p, n)
            && is_wide_for_break(a)
            && is_wide_for_break(b)
        {
            return CollapsedText {
                text: String::new(),
                migrate_count: 0,
            };
        }
        return CollapsedText {
            text: String::new(),
            migrate_count: 1,
        };
    }

    let s = if is_pre_line {
        s
    } else {
        remove_removable_segment_break_runs(doc, cascade, parent_of, idx, &s)
    };
    let chars_vec: Vec<char> = s.chars().collect();
    let mut merged = String::with_capacity(s.len());
    let mut in_spaces = false;
    let mut i = 0usize;
    while i < chars_vec.len() {
        let c = chars_vec[i];
        if c == ' ' {
            if !in_spaces {
                merged.push(' ');
            }
            in_spaces = true;
            i += 1;
        } else if c == '\n' {
            let mut j = i;
            while j < chars_vec.len() && chars_vec[j] == '\n' {
                j += 1;
            }
            if is_pre_line {
                for &k in &chars_vec[i..j] {
                    merged.push(k);
                }
                in_spaces = false;
            } else {
                merged.push(' ');
                in_spaces = true;
            }
            i = j;
        } else {
            in_spaces = false;
            merged.push(c);
            i += 1;
        }
    }
    // Extended dedupe: an all-whitespace node whose previous relevant
    // sibling's raw text ENDS with whitespace collapses away — runs
    // spanning nodes (spaces, breaks, or mixed) keep the single space the
    // earlier node migrates. Skipped for `pre-line` around line feeds (a
    // following break must survive to shape the paragraph); pure-space
    // runs still dedupe.
    // NBSP-bearing nodes never dedupe: NBSPs don't collapse, so each
    // migrating NBSP node adds its own space to the pending count.
    if merged.chars().all(is_css_white_space)
        && !merged.contains('\u{00A0}')
        && let Some(p) = parent_of[idx]
        && let Some(pos) = doc.nodes[p].children.iter().position(|&c| c == idx)
    {
        let raw_has_break = text_of(doc, idx).is_some_and(|t| t.contains('\n'));
        for &prev in doc.nodes[p].children[..pos].iter().rev() {
            if !doc.nodes[prev].is_in_document() {
                continue;
            }
            if doc.nodes[prev].kind() == NodeKind::Element
                && cascade.computed[prev].display == DisplayValue::None
            {
                continue;
            }
            if doc.nodes[prev].kind() == NodeKind::Text
                && let Some(t) = text_of(doc, prev)
            {
                let ends_ws = t.chars().next_back().is_some_and(is_css_white_space);
                if !ends_ws {
                    break;
                }
                if is_pre_line && (raw_has_break || t.contains('\n')) {
                    break;
                }
                // Run continues here: the earlier node migrates the
                // single surviving space.
                return CollapsedText {
                    text: String::new(),
                    migrate_count: 0,
                };
            }
            break;
        }
    }
    let before = has_inline_adjacent(doc, cascade, parent_of, idx, -1);
    let after = has_inline_adjacent(doc, cascade, parent_of, idx, 1);
    // Lone-space arm covers plain spaces AND lone NBSPs (a `&nbsp;` entity
    // parses to its own text node, which parley would otherwise trim to
    // zero when shaped alone — the same fate as a lone collapsible space,
    // so it migrates identically; WPT unremovable-* pins the outcome).
    if merged.chars().all(|c| c == ' ' || c == '\u{00A0}') {
        // Fully collapsed: a lone boundary space vanishes; between inlines
        // the survivors migrate forward (shaped alone even NBSP trims to
        // zero — parley keeps only leading non-collapsible runs followed
        // by more content, measured directly). Count = NBSPs plus one for
        // a collapsible run (runs already merged to one above; NBSPs never
        // collapse, so each is preserved).
        if before && after {
            let count = merged.chars().filter(|&c| c == '\u{00A0}').count() as u32
                + merged.contains(' ') as u32;
            return CollapsedText {
                text: String::new(),
                migrate_count: count,
            };
        }
        return CollapsedText {
            text: String::new(),
            migrate_count: 0,
        };
    }
    // Edge line feeds (single — runs already collapsed above): dropped
    // at block edges, converted to a space toward inline content (a lone
    // survivor is indistinguishable from a collapsed space downstream;
    // wide-char pairs across nodes stay space-approximated — symmetric
    // pairs are unaffected either way).
    // Known limitation for pre-line, intentional for now, see
    // raikiri-spike-6r1x.12: an edge line feed facing inline content is
    // degraded to a space even under pre-line, where the spec preserves
    // segment breaks as forced breaks. Single-node interior breaks stay
    // preserved, so X newline Y as one node keeps two lines while the
    // same text split across sibling nodes collapses to one line with a
    // migrated or NBSP space. Keeping the edge break in place alone does
    // not restore the break: measured split geometries keep the two-line
    // block height but misplace the second run to the right of the first
    // instead of at the next line start, and an interior break plus a
    // following sibling misplaces that sibling the same way. Correct
    // rendering needs cross-node forced-break handling at the line-box
    // level, analogous to the br path with wrap plus a full-width break
    // item, combined with stripping the edge break from shaping so it is
    // counted once, not twice. That is beyond a minimal per-node fix, so
    // the space degradation stays until that line-box work lands. Do not
    // add a test pinning the collapsed single-line value as correct.
    let mut out = merged;
    if !before {
        out = out.trim_start_matches([' ', '\n']).to_string();
    } else if out.starts_with('\n') {
        out.replace_range(..1, " ");
    }
    if !after {
        out = out.trim_end_matches([' ', '\n']).to_string();
    } else if out.ends_with('\n') {
        out.pop();
        out.push(' ');
    }
    let trailing_kept = after && out.ends_with(' ');
    if after {
        // Trailing survivor migrates forward (see `migrate_count`); strip
        // it here so shaping never sees a trimmable edge space.
        out = out.trim_end_matches(' ').to_string();
    }
    // Kept leading spaces become NBSP in place: parley keeps a leading
    // NBSP followed by content (measured), while a plain leading space
    // would trim. NBSP forfeits a soft-wrap opportunity at that spot;
    // acceptable since the alternative (today) is losing the space.
    if out.starts_with(' ') {
        out.replace_range(..1, "\u{00A0}");
    }
    CollapsedText {
        text: out,
        migrate_count: trailing_kept as u32,
    }
}

/// A coarse CSS Text 4 autospace class.  The style layer preserves the
/// complete `text-autospace` value; this layout slice only needs the boundary
/// classes to create non-painting in-flow advances.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TextAutospaceClass {
    Ideograph,
    Letter,
    Numeric,
    Other,
}

fn text_autospace_class(c: char) -> TextAutospaceClass {
    use icu_properties::props::{GeneralCategoryGroup, Script};

    let gc =
        icu_properties::CodePointMapDataBorrowed::<icu_properties::props::GeneralCategory>::new()
            .get(c);
    let eaw = east_asian_width_map().get(c);
    let wide = matches!(
        eaw,
        icu_properties::props::EastAsianWidth::Fullwidth
            | icu_properties::props::EastAsianWidth::Wide
    );
    // CSS Text's ideographic set includes Han and the Japanese kana/stroke
    // ranges. Punctuation in the shared ranges is not an ideograph.
    let ideograph =
        (icu_properties::CodePointMapDataBorrowed::<icu_properties::props::Script>::new().get(c)
            == Script::Han
            || matches!(c, '\u{3041}'..='\u{30ff}' | '\u{31c0}'..='\u{31ff}'))
            && !GeneralCategoryGroup::Punctuation.contains(gc);
    if ideograph {
        return TextAutospaceClass::Ideograph;
    }
    if !wide
        && (GeneralCategoryGroup::Letter.contains(gc) || GeneralCategoryGroup::Mark.contains(gc))
    {
        return TextAutospaceClass::Letter;
    }
    if !wide && GeneralCategoryGroup::Number.contains(gc) {
        return TextAutospaceClass::Numeric;
    }
    TextAutospaceClass::Other
}

fn text_autospace_ascii_punctuation(c: char) -> bool {
    // CSS Text 4's `punctuation` class is language-sensitive. Chromium's
    // current behavior (and the zh WPT slice) inserts around these ASCII
    // punctuation marks, but not around fullwidth/CJK punctuation.
    matches!(c, '!' | '#' | ':' | ';' | '?')
}

#[cfg(test)]
pub(crate) fn text_autospace_boxes(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_edges(text, value, language, font_size, None, None)
}

fn text_autospace_boxes_with_edges(
    text: &str,
    value: TextAutospace,
    language: &str,
    font_size: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    text_autospace_boxes_with_width(
        text,
        value,
        language,
        (font_size * 0.125).max(0.0),
        before,
        after,
    )
}

fn text_autospace_boxes_with_width(
    text: &str,
    value: TextAutospace,
    language: &str,
    width: f32,
    before: Option<char>,
    after: Option<char>,
) -> Vec<InlineBox> {
    let (ideograph_alpha, ideograph_numeric, punctuation) = match value {
        TextAutospace::Normal | TextAutospace::Auto => {
            (true, true, language_matches(language, "zh"))
        }
        TextAutospace::NoAutospace => (false, false, false),
        TextAutospace::Custom {
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
            ..
        } => (
            ideograph_alpha,
            ideograph_numeric,
            punctuation && language_matches(language, "zh"),
        ),
        // `TextAutospace` is non-exhaustive so downstream crates remain
        // source-compatible when the style layer gains another keyword.
        _ => (true, true, language_matches(language, "zh")), // cov:ignore: no future non-exhaustive variant exists in the pinned style crate.
    };
    if !ideograph_alpha && !ideograph_numeric && !punctuation {
        return Vec::new();
    }
    if !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }

    let mut previous =
        before.and_then(|c| (!is_segment_break_ignorable(c)).then(|| (c, text_autospace_class(c))));
    let mut last = None;
    let mut boxes = Vec::new();
    for (index, c) in text.char_indices() {
        // Default-ignorable characters, including variation selectors, do not
        // break the neighboring-character test. The VS WPT expects `国` + VS
        // + `A` to use the same autospace boundary as `国A`.
        if is_segment_break_ignorable(c) {
            continue;
        }
        let class = text_autospace_class(c);
        if let Some((previous_char, previous_class)) = previous
            && text_autospace_pair_needs_box(
                previous_char,
                previous_class,
                c,
                class,
                ideograph_alpha,
                ideograph_numeric,
                punctuation,
            )
        {
            boxes.push(InlineBox {
                // Reserve the high ID range for autospace boxes so paint can
                // distinguish their inline baseline semantics from tabs.
                id: u64::MAX - boxes.len() as u64,
                kind: InlineBoxKind::InFlow,
                index,
                width,
                height: 0.0,
            });
        }
        previous = Some((c, class));
        last = Some((c, class));
    }
    if let (Some((previous_char, previous_class)), Some(next)) = (last, after)
        && !is_segment_break_ignorable(next)
        && text_autospace_pair_needs_box(
            previous_char,
            previous_class,
            next,
            text_autospace_class(next),
            ideograph_alpha,
            ideograph_numeric,
            punctuation,
        )
    {
        boxes.push(InlineBox {
            // Reserve the high ID range for autospace boxes so paint can
            // distinguish their inline baseline semantics from tabs.
            id: u64::MAX - boxes.len() as u64,
            kind: InlineBoxKind::InFlow,
            index: text.len(),
            width,
            height: 0.0,
        });
    }
    boxes
}

fn text_autospace_pair_needs_box(
    previous_char: char,
    previous_class: TextAutospaceClass,
    current_char: char,
    current_class: TextAutospaceClass,
    ideograph_alpha: bool,
    ideograph_numeric: bool,
    punctuation: bool,
) -> bool {
    let class_boundary = match (previous_class, current_class) {
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Letter)
        | (TextAutospaceClass::Letter, TextAutospaceClass::Ideograph) => ideograph_alpha,
        (TextAutospaceClass::Ideograph, TextAutospaceClass::Numeric)
        | (TextAutospaceClass::Numeric, TextAutospaceClass::Ideograph) => ideograph_numeric,
        _ => false,
    };
    let punctuation_boundary = punctuation
        && ((text_autospace_ascii_punctuation(previous_char)
            && current_class == TextAutospaceClass::Ideograph)
            || (previous_class == TextAutospaceClass::Ideograph
                && text_autospace_ascii_punctuation(current_char)));
    class_boundary || punctuation_boundary
}

fn effective_language_for_text(
    doc: &Document,
    parent_of: &[Option<usize>],
    text_idx: usize,
) -> String {
    let mut current = parent_of[text_idx];
    while let Some(idx) = current {
        if let crate::node::NodeData::Element(element) = &doc.nodes[idx].data
            && let Some(attr) = element.attributes.iter().find(|attr| {
                (attr.namespace.is_none() && attr.local.eq_ignore_ascii_case("lang"))
                    || (attr.namespace.as_deref() == Some("http://www.w3.org/XML/1998/namespace")
                        && attr.local == "lang")
            })
        {
            return attr.value.trim().to_ascii_lowercase();
        }
        current = parent_of[idx];
    }
    String::new()
}

fn language_matches(language: &str, primary: &str) -> bool {
    language == primary
        || language
            .strip_prefix(primary)
            .is_some_and(|rest| rest.starts_with('-'))
}

/// Turkic dotted-I casing applies to the Latin writing system, not an
/// explicitly different script such as `tr-Cyrl`.
fn uses_turkic_case_tailoring(language: &str) -> bool {
    if !language_matches(language, "tr") && !language_matches(language, "az") {
        return false;
    }
    // In a BCP 47 tag the optional script subtag follows the primary language.
    language.split('-').nth(1).is_none_or(|subtag| {
        subtag.len() != 4
            || !subtag.bytes().all(|byte| byte.is_ascii_alphabetic())
            || subtag.eq_ignore_ascii_case("Latn")
    })
}

fn text_transform_has_full_width(transform: TextTransform) -> bool {
    matches!(
        transform,
        TextTransform::FullWidth
            | TextTransform::CapitalizeFullWidth
            | TextTransform::UppercaseFullWidth
            | TextTransform::LowercaseFullWidth
            | TextTransform::FullWidthFullSizeKana
            | TextTransform::CapitalizeFullWidthFullSizeKana
            | TextTransform::UppercaseFullWidthFullSizeKana
            | TextTransform::LowercaseFullWidthFullSizeKana
    )
}

/// Apply the supported CSS `text-transform` values after whitespace collapsing
/// and before shaping. The operation is performed on each text node, preserving
/// the existing layout and paint ownership model.
fn apply_text_transform(text: &str, transform: TextTransform, language: &str) -> String {
    let (case, full_width, full_size_kana) = match transform {
        TextTransform::None => (None, false, false),
        TextTransform::Capitalize => (Some(TextTransform::Capitalize), false, false),
        TextTransform::Uppercase => (Some(TextTransform::Uppercase), false, false),
        TextTransform::Lowercase => (Some(TextTransform::Lowercase), false, false),
        TextTransform::FullWidth => (None, true, false),
        TextTransform::FullSizeKana => (None, false, true),
        TextTransform::CapitalizeFullWidth => (Some(TextTransform::Capitalize), true, false),
        TextTransform::UppercaseFullWidth => (Some(TextTransform::Uppercase), true, false),
        TextTransform::LowercaseFullWidth => (Some(TextTransform::Lowercase), true, false),
        TextTransform::CapitalizeFullSizeKana => (Some(TextTransform::Capitalize), false, true),
        TextTransform::UppercaseFullSizeKana => (Some(TextTransform::Uppercase), false, true),
        TextTransform::LowercaseFullSizeKana => (Some(TextTransform::Lowercase), false, true),
        TextTransform::FullWidthFullSizeKana => (None, true, true),
        TextTransform::CapitalizeFullWidthFullSizeKana => {
            (Some(TextTransform::Capitalize), true, true)
        }
        TextTransform::UppercaseFullWidthFullSizeKana => {
            (Some(TextTransform::Uppercase), true, true)
        }
        TextTransform::LowercaseFullWidthFullSizeKana => {
            (Some(TextTransform::Lowercase), true, true)
        }
        _ => (None, false, false),
    };
    let mut cased = String::with_capacity(text.len());
    let mut in_word = false;
    let turkic_case = uses_turkic_case_tailoring(language);
    for c in text.chars() {
        match case {
            Some(TextTransform::Uppercase) => {
                if turkic_case {
                    match c {
                        'i' => cased.push('İ'),
                        'ı' => cased.push('I'),
                        _ => cased.extend(c.to_uppercase()),
                    }
                } else {
                    cased.extend(c.to_uppercase());
                }
            }
            Some(TextTransform::Lowercase) => {
                if turkic_case {
                    match c {
                        'I' => cased.push('ı'),
                        'İ' => cased.push('i'),
                        _ => cased.extend(c.to_lowercase()),
                    }
                } else {
                    cased.extend(c.to_lowercase());
                }
            }
            Some(TextTransform::Capitalize) => {
                if c.is_alphanumeric() {
                    let can_titlecase = !(0x24D0..=0x24E9).contains(&(c as u32));
                    if !in_word && can_titlecase {
                        cased.extend(c.to_uppercase());
                    } else {
                        cased.push(c);
                    }
                    in_word = true;
                } else {
                    cased.push(c);
                    in_word = false;
                }
            }
            _ => cased.push(c),
        }
    }
    if matches!(case, Some(TextTransform::Lowercase)) && turkic_case {
        tailor_turkic_combining_dot(&mut cased);
    }
    if matches!(case, Some(TextTransform::Capitalize)) && language_matches(language, "nl") {
        tailor_dutch_ij(&mut cased);
    }
    if matches!(case, Some(TextTransform::Uppercase)) && language_matches(language, "el") {
        strip_greek_tonos(&mut cased);
    }
    cased
        .chars()
        .map(|c| {
            let c = if full_size_kana {
                full_size_kana_char(c)
            } else {
                c
            };
            if full_width { full_width_char(c) } else { c }
        })
        .collect()
}

fn tailor_turkic_combining_dot(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let combining_dot = char::from_u32(0x0307).unwrap();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == 'ı' && chars.get(index + 1) == Some(&combining_dot) {
            out.push('i');
            index += 2;
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }
    *text = out;
}

fn tailor_dutch_ij(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut at_word_start = true;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if at_word_start && c == 'I' && chars.get(index + 1) == Some(&'j') {
            out.push('I');
            out.push('J');
            at_word_start = false;
            index += 2;
            continue;
        }
        out.push(c);
        at_word_start = !c.is_alphanumeric();
        index += 1;
    }
    *text = out;
}

fn strip_greek_tonos(text: &mut String) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_alphabetic() {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        let mut end = index;
        while end < chars.len()
            && (chars[end].is_alphabetic() || ('\u{0300}'..='\u{036f}').contains(&chars[end]))
        {
            end += 1;
        }
        let letter_count = chars[index..end]
            .iter()
            .filter(|c| c.is_alphabetic())
            .count();
        let keep_tonos = letter_count == 1;
        let mut word_index = index;
        while word_index < end {
            let c = chars[word_index];
            if !keep_tonos
                && let Some(&next) = chars.get(word_index + 1)
                && matches!(c, 'Ά' | 'Έ' | 'Ή' | 'Ί' | 'Ό' | 'Ύ' | 'Ώ')
                && matches!(next, 'Ι' | 'Υ')
            {
                out.push(greek_tonos_removed(c));
                out.push(greek_diaeresis(next));
                word_index += 2;
            } else if !keep_tonos
                && let (Some(&diaeresis), Some(&acute)) =
                    (chars.get(word_index + 1), chars.get(word_index + 2))
                && matches!(c, 'Ι' | 'Υ')
                && diaeresis == '\u{0308}'
                && acute == '\u{0301}'
            {
                out.push(greek_diaeresis(c));
                word_index += 3;
            } else if !keep_tonos && c == '\u{0301}' {
                word_index += 1;
            } else {
                out.push(if keep_tonos {
                    c
                } else {
                    greek_tonos_removed(c)
                });
                word_index += 1;
            }
        }
        index = end;
    }
    *text = out;
}

fn greek_tonos_removed(c: char) -> char {
    match c {
        'Ά' => 'Α',
        'Έ' => 'Ε',
        'Ή' => 'Η',
        'Ί' => 'Ι',
        'Ό' => 'Ο',
        'Ύ' => 'Υ',
        'Ώ' => 'Ω',
        _ => c,
    }
}

fn greek_diaeresis(c: char) -> char {
    match c {
        'Ι' => 'Ϊ',
        'Υ' => 'Ϋ',
        _ => c,
    }
}

fn full_size_kana_char(c: char) -> char {
    match c {
        'ぁ' => 'あ',
        'ぃ' => 'い',
        'ぅ' => 'う',
        'ぇ' => 'え',
        'ぉ' => 'お',
        'ゕ' => 'か',
        'ゖ' => 'け',
        'っ' => 'つ',
        'ゃ' => 'や',
        'ゅ' => 'ゆ',
        'ょ' => 'よ',
        'ゎ' => 'わ',
        'ァ' => 'ア',
        'ィ' => 'イ',
        'ゥ' => 'ウ',
        'ェ' => 'エ',
        'ォ' => 'オ',
        'ヵ' => 'カ',
        'ㇰ' => 'ク',
        'ヶ' => 'ケ',
        'ㇱ' => 'シ',
        'ㇲ' => 'ス',
        'ッ' => 'ツ',
        'ㇳ' => 'ト',
        'ㇴ' => 'ヌ',
        'ㇵ' => 'ハ',
        'ㇶ' => 'ヒ',
        'ㇷ' => 'フ',
        'ㇸ' => 'ヘ',
        'ㇹ' => 'ホ',
        'ㇺ' => 'ム',
        'ャ' => 'ヤ',
        'ュ' => 'ユ',
        'ョ' => 'ヨ',
        'ㇻ' => 'ラ',
        'ㇼ' => 'リ',
        'ㇽ' => 'ル',
        'ㇾ' => 'レ',
        'ㇿ' => 'ロ',
        'ヮ' => 'ワ',
        'ｧ' => 'ｱ',
        'ｨ' => 'ｲ',
        'ｩ' => 'ｳ',
        'ｪ' => 'ｴ',
        'ｫ' => 'ｵ',
        'ｯ' => 'ﾂ',
        'ｬ' => 'ﾔ',
        'ｭ' => 'ﾕ',
        'ｮ' => 'ﾖ',
        '\u{1B132}' => '\u{3053}',
        '\u{1B150}' => '\u{3090}',
        '\u{1B151}' => '\u{3091}',
        '\u{1B152}' => '\u{3092}',
        '\u{1B155}' => '\u{30B3}',
        '\u{1B164}' => '\u{30F0}',
        '\u{1B165}' => '\u{30F1}',
        '\u{1B166}' => '\u{30F2}',
        '\u{1B167}' => '\u{30F3}',
        _ => c,
    }
}

fn full_width_char(c: char) -> char {
    // Unicode compatibility decompositions: invert <wide>, apply <narrow>.
    // ASCII has an arithmetic mapping; the remaining entries need a table.
    match c {
        ' ' => '\u{3000}',
        '!'..='~' => char::from_u32(c as u32 + 0xFEE0).unwrap_or(c),
        '\u{A2}' => '\u{FFE0}',
        '\u{A3}' => '\u{FFE1}',
        '\u{A5}' => '\u{FFE5}',
        '\u{A6}' => '\u{FFE4}',
        '\u{AC}' => '\u{FFE2}',
        '\u{AF}' => '\u{FFE3}',
        '\u{20A9}' => '\u{FFE6}',
        '\u{2985}' => '\u{FF5F}',
        '\u{2986}' => '\u{FF60}',
        '\u{FF61}' => '\u{3002}',
        '\u{FF62}' => '\u{300C}',
        '\u{FF63}' => '\u{300D}',
        '\u{FF64}' => '\u{3001}',
        '\u{FF65}' => '\u{30FB}',
        '\u{FF66}' => '\u{30F2}',
        '\u{FF67}' => '\u{30A1}',
        '\u{FF68}' => '\u{30A3}',
        '\u{FF69}' => '\u{30A5}',
        '\u{FF6A}' => '\u{30A7}',
        '\u{FF6B}' => '\u{30A9}',
        '\u{FF6C}' => '\u{30E3}',
        '\u{FF6D}' => '\u{30E5}',
        '\u{FF6E}' => '\u{30E7}',
        '\u{FF6F}' => '\u{30C3}',
        '\u{FF70}' => '\u{30FC}',
        '\u{FF71}' => '\u{30A2}',
        '\u{FF72}' => '\u{30A4}',
        '\u{FF73}' => '\u{30A6}',
        '\u{FF74}' => '\u{30A8}',
        '\u{FF75}' => '\u{30AA}',
        '\u{FF76}' => '\u{30AB}',
        '\u{FF77}' => '\u{30AD}',
        '\u{FF78}' => '\u{30AF}',
        '\u{FF79}' => '\u{30B1}',
        '\u{FF7A}' => '\u{30B3}',
        '\u{FF7B}' => '\u{30B5}',
        '\u{FF7C}' => '\u{30B7}',
        '\u{FF7D}' => '\u{30B9}',
        '\u{FF7E}' => '\u{30BB}',
        '\u{FF7F}' => '\u{30BD}',
        '\u{FF80}' => '\u{30BF}',
        '\u{FF81}' => '\u{30C1}',
        '\u{FF82}' => '\u{30C4}',
        '\u{FF83}' => '\u{30C6}',
        '\u{FF84}' => '\u{30C8}',
        '\u{FF85}' => '\u{30CA}',
        '\u{FF86}' => '\u{30CB}',
        '\u{FF87}' => '\u{30CC}',
        '\u{FF88}' => '\u{30CD}',
        '\u{FF89}' => '\u{30CE}',
        '\u{FF8A}' => '\u{30CF}',
        '\u{FF8B}' => '\u{30D2}',
        '\u{FF8C}' => '\u{30D5}',
        '\u{FF8D}' => '\u{30D8}',
        '\u{FF8E}' => '\u{30DB}',
        '\u{FF8F}' => '\u{30DE}',
        '\u{FF90}' => '\u{30DF}',
        '\u{FF91}' => '\u{30E0}',
        '\u{FF92}' => '\u{30E1}',
        '\u{FF93}' => '\u{30E2}',
        '\u{FF94}' => '\u{30E4}',
        '\u{FF95}' => '\u{30E6}',
        '\u{FF96}' => '\u{30E8}',
        '\u{FF97}' => '\u{30E9}',
        '\u{FF98}' => '\u{30EA}',
        '\u{FF99}' => '\u{30EB}',
        '\u{FF9A}' => '\u{30EC}',
        '\u{FF9B}' => '\u{30ED}',
        '\u{FF9C}' => '\u{30EF}',
        '\u{FF9D}' => '\u{30F3}',
        '\u{FF9E}' => '\u{3099}',
        '\u{FF9F}' => '\u{309A}',
        '\u{FFA0}' => '\u{3164}',
        '\u{FFA1}' => '\u{3131}',
        '\u{FFA2}' => '\u{3132}',
        '\u{FFA3}' => '\u{3133}',
        '\u{FFA4}' => '\u{3134}',
        '\u{FFA5}' => '\u{3135}',
        '\u{FFA6}' => '\u{3136}',
        '\u{FFA7}' => '\u{3137}',
        '\u{FFA8}' => '\u{3138}',
        '\u{FFA9}' => '\u{3139}',
        '\u{FFAA}' => '\u{313A}',
        '\u{FFAB}' => '\u{313B}',
        '\u{FFAC}' => '\u{313C}',
        '\u{FFAD}' => '\u{313D}',
        '\u{FFAE}' => '\u{313E}',
        '\u{FFAF}' => '\u{313F}',
        '\u{FFB0}' => '\u{3140}',
        '\u{FFB1}' => '\u{3141}',
        '\u{FFB2}' => '\u{3142}',
        '\u{FFB3}' => '\u{3143}',
        '\u{FFB4}' => '\u{3144}',
        '\u{FFB5}' => '\u{3145}',
        '\u{FFB6}' => '\u{3146}',
        '\u{FFB7}' => '\u{3147}',
        '\u{FFB8}' => '\u{3148}',
        '\u{FFB9}' => '\u{3149}',
        '\u{FFBA}' => '\u{314A}',
        '\u{FFBB}' => '\u{314B}',
        '\u{FFBC}' => '\u{314C}',
        '\u{FFBD}' => '\u{314D}',
        '\u{FFBE}' => '\u{314E}',
        '\u{FFC2}' => '\u{314F}',
        '\u{FFC3}' => '\u{3150}',
        '\u{FFC4}' => '\u{3151}',
        '\u{FFC5}' => '\u{3152}',
        '\u{FFC6}' => '\u{3153}',
        '\u{FFC7}' => '\u{3154}',
        '\u{FFCA}' => '\u{3155}',
        '\u{FFCB}' => '\u{3156}',
        '\u{FFCC}' => '\u{3157}',
        '\u{FFCD}' => '\u{3158}',
        '\u{FFCE}' => '\u{3159}',
        '\u{FFCF}' => '\u{315A}',
        '\u{FFD2}' => '\u{315B}',
        '\u{FFD3}' => '\u{315C}',
        '\u{FFD4}' => '\u{315D}',
        '\u{FFD5}' => '\u{315E}',
        '\u{FFD6}' => '\u{315F}',
        '\u{FFD7}' => '\u{3160}',
        '\u{FFDA}' => '\u{3161}',
        '\u{FFDB}' => '\u{3162}',
        '\u{FFDC}' => '\u{3163}',
        '\u{FFE8}' => '\u{2502}',
        '\u{FFE9}' => '\u{2190}',
        '\u{FFEA}' => '\u{2191}',
        '\u{FFEB}' => '\u{2192}',
        '\u{FFEC}' => '\u{2193}',
        '\u{FFED}' => '\u{25A0}',
        '\u{FFEE}' => '\u{25CB}',
        _ => c,
    }
}

fn is_leading_body_text(document: &Document, body_id: Option<usize>, node_id: usize) -> bool {
    let Some(body_id) = body_id else {
        return false;
    };
    for &child_id in &document.nodes[body_id].children {
        if child_id == node_id {
            return true;
        }
        let Some(child) = document.get_node(child_id) else {
            continue;
        };
        match child.kind() {
            NodeKind::Text => {
                if let crate::node::NodeData::Text(text) = &child.data
                    && !text.text_content.trim().is_empty()
                {
                    return false;
                }
            }
            NodeKind::Element
                if child.is_display_none() || child.is_non_rendered_html_element() => {}
            _ => return false,
        }
    }
    false
}

fn parley_word_break(value: WordBreak, _line_break: LineBreak) -> ParleyWordBreak {
    match value {
        WordBreak::BreakAll => ParleyWordBreak::BreakAll,
        WordBreak::KeepAll => ParleyWordBreak::KeepAll,
        // `manual`, `auto-phrase`, and the deprecated `break-word` do not
        // have a direct Parley word-break mode. `break-word` gets its
        // emergency wrapping behavior from `parley_overflow_wrap` below.
        WordBreak::Normal | WordBreak::Manual | WordBreak::AutoPhrase | WordBreak::BreakWord => {
            ParleyWordBreak::Normal
        }
        _ => ParleyWordBreak::Normal,
    }
}

fn has_visible_non_nbsp_character(text: &str) -> bool {
    text.chars()
        .any(|character| character != '\u{00A0}' && !is_css_white_space(character))
}

// The callback overrides every analyzed UAX line boundary. Keep control
// classes without exact WPT coverage on the per-job fallback path.
fn line_break_class_map()
-> icu_properties::CodePointMapDataBorrowed<'static, icu_properties::props::LineBreak> {
    icu_properties::CodePointMapDataBorrowed::<icu_properties::props::LineBreak>::new()
}

fn has_unverified_anywhere_control(text: &str) -> bool {
    use icu_properties::props::LineBreak as UnicodeLineBreak;

    let classes = line_break_class_map();
    text.chars().any(|character| {
        matches!(
            classes.get(character),
            UnicodeLineBreak::Glue
                | UnicodeLineBreak::WordJoiner
                | UnicodeLineBreak::ZWSpace
                | UnicodeLineBreak::ZWJ
                | UnicodeLineBreak::CombiningMark
        ) && !matches!(
            character,
            '\u{00A0}' // verified WPT -006/-010; replaced by the Raikiri adapter
                | '\u{2060}' // verified WPT overrides-uax-behavior-001
                | '\u{FEFF}' // verified WPT overrides-uax-behavior-002
                | '\u{200B}' // verified WPT overrides-uax-behavior-003
                | '\u{180E}' // verified WPT overrides-uax-behavior-010
                | '\u{034F}' // verified WPT overrides-uax-behavior-012
                | '\u{200D}' // verified WPT overrides-uax-behavior-015
        )
    })
}

fn pre_wrap_anywhere_subset(text: &str) -> bool {
    text.matches('\u{00A0}').count() == 1
        && text
            .chars()
            .all(|character| !is_css_white_space(character) || character == ' ')
        && !text.starts_with(' ')
        && !text.ends_with(' ')
        && !text.contains("  ")
        && has_visible_non_nbsp_character(text)
}

fn should_enable_anywhere_override(
    line_break: LineBreak,
    white_space: WhiteSpace,
    nowrap: bool,
    text: &str,
) -> bool {
    if line_break != LineBreak::Anywhere
        || nowrap
        || !has_visible_non_nbsp_character(text)
        || has_unverified_anywhere_control(text)
    {
        return false;
    }

    match white_space {
        WhiteSpace::Normal | WhiteSpace::PreLine => true,
        WhiteSpace::PreWrap => pre_wrap_anywhere_subset(text),
        _ => false,
    }
}

fn should_enable_anywhere_callback(
    line_break: LineBreak,
    white_space: WhiteSpace,
    nowrap: bool,
    text: &str,
    nbsp_adapter_enabled: bool,
) -> bool {
    should_enable_anywhere_override(line_break, white_space, nowrap, text)
        && (!text.contains('\u{00A0}') || nbsp_adapter_enabled)
}

fn should_enable_nbsp_adapter(
    line_break: LineBreak,
    white_space: WhiteSpace,
    nowrap: bool,
    text: &str,
    has_authored_tab: bool,
    has_inline_root_ancestor: bool,
) -> bool {
    !(has_inline_root_ancestor || (white_space == WhiteSpace::PreWrap && has_authored_tab))
        && text.contains('\u{00A0}')
        && should_enable_anywhere_override(line_break, white_space, nowrap, text)
}

fn has_inline_root_ancestor(doc: &Document, parent_of: &[Option<usize>], node: usize) -> bool {
    let mut ancestor = parent_of[node];
    while let Some(index) = ancestor {
        if doc.nodes[index].flags.contains(NodeFlags::IS_INLINE_ROOT) {
            return true;
        }
        ancestor = parent_of[index];
    }
    false
}

fn line_break_anywhere_override(context: parley::LineBreakContext) -> Option<bool> {
    // A mandatory newline already ends the current line. Do not create a
    // second soft opportunity immediately before it.
    Some(context.after != '\n')
}

static LINE_BREAK_ANYWHERE_OVERRIDE: &parley::LineBreakOverrideFn =
    &(line_break_anywhere_override as fn(parley::LineBreakContext) -> Option<bool>);

fn parley_line_break_override(enabled: bool) -> Option<&'static parley::LineBreakOverrideFn> {
    enabled.then_some(LINE_BREAK_ANYWHERE_OVERRIDE)
}

fn parley_overflow_wrap(
    word_break: WordBreak,
    _line_break: LineBreak,
    value: OverflowWrap,
) -> ParleyOverflowWrap {
    if matches!(word_break, WordBreak::BreakWord) {
        return ParleyOverflowWrap::BreakWord;
    }
    match value {
        OverflowWrap::Normal => ParleyOverflowWrap::Normal,
        OverflowWrap::Anywhere => ParleyOverflowWrap::Anywhere,
        OverflowWrap::BreakWord => ParleyOverflowWrap::BreakWord,
        _ => ParleyOverflowWrap::Normal,
    }
}

fn parley_text_wrap_mode(value: TextWrapMode, nowrap: bool) -> ParleyTextWrapMode {
    if nowrap || matches!(value, TextWrapMode::Nowrap) {
        ParleyTextWrapMode::NoWrap
    } else {
        ParleyTextWrapMode::Wrap
    }
}

pub(crate) fn preshape_text(
    doc: &mut Document,
    cascade: &CascadeResult,
    fonts: &mut FontContext,
    layout_cx: &mut LayoutContext<()>,
    max_advance: f32,
    page_width: f32,
) {
    for node in &mut doc.nodes {
        if let Some(text) = node.data.as_text_mut() {
            text.snap_glyph_x_to_1_64 = false;
        }
    }
    // Cheap threshold: for tiny DOMs sequential is faster than rayon overhead.
    // Collect eligible Text nodes first to avoid borrowing `doc.nodes` mutably
    // across parallel tasks (which would require Sync on Document). Owned jobs
    // are Send and avoid sharing &Document across threads.
    struct Job {
        idx: usize,
        had_tab: bool,
        text: String,
        family_str: String,
        font_size_raw: f32,
        font_weight_raw: f32,
        font_style: StyleFontStyle,
        line_height_raw: ComputedLineHeight,
        letter_spacing_raw: f32,
        // Preserve authored `ch` so the shaping font's `0` advance can replace
        // the style-layer fallback before Parley lays out the text.
        letter_spacing_ch_factor: Option<f32>,
        // Absolute part of a `ch`-bearing calc, added to the measured advance.
        letter_spacing_ch_offset: f32,
        // Font that declared an inherited `ch` letter-spacing.
        letter_spacing_ch_font: Option<raikiri_style::ChFontKey>,
        // Style-layer fallback in CSS px; replaced with a measured `ch`
        // advance when the authored-unit marker below is present.
        word_spacing_raw: f32,
        // Preserve the authored unit because `ComputedLength` alone loses it.
        word_spacing_ch_factor: Option<f32>,
        word_spacing_ch_offset: f32,
        word_spacing_ch_font: Option<raikiri_style::ChFontKey>,
        tab_size: ComputedTabSize,
        white_space: WhiteSpace,
        word_break: WordBreak,
        line_break: LineBreak,
        line_break_override_enabled: bool,
        has_inline_root_ancestor: bool,
        overflow_wrap: OverflowWrap,
        text_wrap_mode: TextWrapMode,
        autospace_boxes: Vec<InlineBox>,
        // Soft wrapping suppressed (`white-space: nowrap` or
        // `text-wrap: nowrap`).
        nowrap: bool,
        max_advance: f32,
        // tab-stop metrics use the block-container ancestor's font and spacing.
        metrics_family: String,
        metrics_size: f32,
        metrics_weight: f32,
        metrics_style: StyleFontStyle,
        metrics_letter_spacing_raw: f32,
        metrics_letter_spacing_ch_factor: Option<f32>,
        metrics_letter_spacing_ch_offset: f32,
        metrics_letter_spacing_ch_font: Option<raikiri_style::ChFontKey>,
        metrics_word_spacing_raw: f32,
        metrics_word_spacing_ch_factor: Option<f32>,
        metrics_word_spacing_ch_offset: f32,
        metrics_word_spacing_ch_font: Option<raikiri_style::ChFontKey>,
        simple_pre_block: bool,
        // Preserve the metric quantization used by a simple pre block when
        // one inline wrapper contains the only text run and a terminal
        // preserved newline is the sole sibling.
        simple_preserved_run: bool,
        tab_spacing_ranges: Vec<(std::ops::Range<usize>, f32)>,
        // Trailing `word-space-transform` separators stripped from `text`
        // because Parley `width()` excludes them (see
        // `WORD_SPACE_EDGE_INLINE_BOX_ID_BASE`). Re-added as one wrapping
        // advance box after `ch`/tab/NBSP resolution so the measured advance
        // uses the resolved spacing.
        word_space_edge_spaces: u32,
        word_space_edge_ideos: u32,
    }

    // The in-flow inline `<wbr>` under `word-space-transform: space` or
    // `ideographic-space` becomes an advance-bearing inline item. The element
    // has no Text node to shape; measuring its own font's space keeps the
    // following run in place without changing the authored DOM or making a
    // text node. This covers single-line spacing, not general `<wbr>` wrapping.
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].tag_name() != Some("wbr") || !doc.nodes[idx].is_in_document() {
            continue;
        }
        let cv = &cascade.computed[idx];
        if cv.display != DisplayValue::Inline
            || matches!(cv.position, PositionValue::Absolute | PositionValue::Fixed)
        {
            continue;
        }
        let space = match cv.word_space_transform {
            WordSpaceTransform::Space if text_transform_has_full_width(cv.text_transform) => {
                "\u{3000}"
            }
            WordSpaceTransform::Space => " ",
            WordSpaceTransform::IdeographicSpace => "\u{3000}",
            _ => continue,
        };
        let family = cv
            .font_family
            .iter()
            .map(|family| family.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        // Match the glyph inserted for U+200B, including full-width transforms.
        let advance = probe_text_full_width(
            fonts,
            layout_cx,
            space,
            TextProbeStyle {
                family_str: &family,
                font_size_px: cv.font_size.px(),
                font_weight: cv.font_weight,
                font_style: cv.font_style,
                letter_spacing: cv.letter_spacing.px(),
                word_spacing: cv.word_spacing.px(),
            },
        );
        doc.nodes[idx].style.size.width = Dimension::length(advance);
    }

    // Shared parent map for simple pre-block eligibility and whitespace
    // boundary trimming (the arena has no stored parent pointers).
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in doc.nodes[idx].children.clone().iter() {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
    fn family_str_of(cv: &ComputedValues) -> String {
        cv.font_family
            .iter()
            .map(|family| family.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
    let mut jobs: Vec<Job> = Vec::with_capacity(doc.nodes.len() / 2);
    // CSS Text 4 `ic` probes are shared by text nodes with the same font
    // selection, just like the existing `ch` probe cache below.
    let mut ic_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    // Outstanding forward-migrated spaces (counted: collapsible space
    // runs collapse to one via dedupe, but NBSPs never collapse so each
    // migrating NBSP node adds one).
    let mut migrate_pending: u32 = 0;
    let mut migrate_pending_full_width = false;
    let body_id = (0..doc.nodes.len()).find(|&idx| doc.nodes[idx].tag_name() == Some("body"));
    let has_out_of_flow_ancestor = |mut parent: Option<usize>| {
        while let Some(id) = parent {
            if matches!(
                cascade.computed[id].position,
                PositionValue::Absolute | PositionValue::Fixed
            ) {
                return true;
            }
            parent = parent_of[id];
        }
        false
    };
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Text {
            continue;
        }
        if !doc.nodes[idx].is_in_document() || doc.nodes[idx].is_inline_svg_content() {
            continue;
        }
        if doc.nodes[idx]
            .flags
            .intersects(NodeFlags::IN_IFC_SUBTREE | NodeFlags::IS_IFC_ROOT)
        {
            continue;
        }
        let raw: String = match &doc.nodes[idx].data {
            crate::node::NodeData::Text(t) if !t.text_content.is_empty() => {
                t.text_content.as_str().to_string()
            }
            _ => continue,
        };

        // CSS Text 3 §4.1: collapse segment breaks/runs and trim boundary
        // spaces before shaping. Fully-collapsed text keeps `text_layout`
        // empty (`None`, same as empty source text above): shaping `""`
        // would still produce a strut line, but collapsed-away text must
        // contribute zero size. Paint skips `None` layouts.
        // `display:none` text is skipped entirely (neither shaped nor
        // migrating): it generates no boxes, so it must not consume a
        // pending migrated space.
        if cascade.computed[idx].display == DisplayValue::None {
            continue;
        }
        let cv = &cascade.computed[idx];
        let language = effective_language_for_text(doc, &parent_of, idx);
        let ws = cv.white_space;
        let collapsed = collapse_text_for_shaping(doc, cascade, &parent_of, idx, &raw, ws);
        if std::env::var("COLLAPSE_DBG2").is_ok() {
            eprintln!(
                "C2 raw={:?} out={:?} mig={}",
                raw, collapsed.text, collapsed.migrate_count
            );
        }
        // A space migrated by an EARLIER node lands here (never this
        // node's own trailing space, which belongs after it).
        let pending_in = migrate_pending;
        let mut text = collapsed.text;
        if text.is_empty() {
            // Whitespace separators between inline children become flex-item
            // boundaries in the multicol projection, so they must not migrate
            // into the next child run (table-cell references have the same
            // boundary behavior).
            // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
            let whitespace_boundary = parent_of[idx].is_some_and(|parent| {
                multicol_metrics_for_node(cascade, &parent_of, parent, max_advance).is_some()
            });
            // cov:ignore: whitespace boundary migration is exercised by the ignored foundation WPT run.
            if whitespace_boundary {
                migrate_pending = 0;
                migrate_pending_full_width = false;
                continue;
            }
            // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
            let inline_space_count =
                // cov:ignore: preserved inline whitespace sizing is exercised by the ignored foundation WPT run.
                if preserves_inline_whitespace_item(doc, cascade, &parent_of, idx, max_advance) {
                    let nbsp_count = raw.chars().filter(|&ch| ch == '\u{00a0}').count() as u32;
                    if nbsp_count > 0 {
                        nbsp_count
                    } else if raw.chars().any(is_css_white_space) {
                        1
                    } else {
                        0
                    }
                } else {
                    0
                };
            // cov:ignore: preserved inline whitespace sizing is exercised by the ignored foundation WPT run.
            if inline_space_count > 0 {
                let family = family_str_of(cv);
                let space_advance = probe_text_full_width(
                    fonts,
                    layout_cx,
                    " ",
                    TextProbeStyle {
                        family_str: &family,
                        font_size_px: cv.font_size.px(),
                        font_weight: cv.font_weight,
                        font_style: cv.font_style,
                        letter_spacing: cv.letter_spacing.px(),
                        word_spacing: cv.word_spacing.px(),
                    },
                );
                doc.nodes[idx].style.size.width =
                    Dimension::length(space_advance * inline_space_count as f32);
                migrate_pending = 0;
                migrate_pending_full_width = false;
                continue;
            }
            // A collapsed space styled with `full-width` must remain owned by
            // that inline run: attaching its transformed U+3000 to the
            // preceding job avoids changing the line metrics of the following
            // text node while retaining the source style's transform.
            if collapsed.migrate_count > 0
                && pending_in == 0
                && text_transform_has_full_width(cv.text_transform)
                && has_inline_adjacent(doc, cascade, &parent_of, idx, -1)
                && has_inline_adjacent(doc, cascade, &parent_of, idx, 1)
                && let Some(previous) = jobs.last_mut()
            {
                previous.text.push('\u{3000}');
                continue;
            }
            // Empty nodes neither consume nor (unless migrating themselves)
            // clear outstanding spaces: a boundary-dropped node must not
            // cancel earlier migrations.
            migrate_pending = pending_in.saturating_add(collapsed.migrate_count);
            if collapsed.migrate_count > 0 {
                migrate_pending_full_width = text_transform_has_full_width(cv.text_transform);
            }
            continue;
        }
        let pending_full_width = migrate_pending_full_width;
        migrate_pending = collapsed.migrate_count;
        migrate_pending_full_width =
            collapsed.migrate_count > 0 && text_transform_has_full_width(cv.text_transform);
        // Forward-migrated spaces (see `migrate_space`): prepend NBSPs iff
        // they still precede inline content from THIS node's edge — a
        // boundary element (e.g. `<br>`) in between correctly drops them —
        // and the node doesn't already start with one (a cross-node run
        // ending here collapses to the one already present).
        let mut migrated_prefix_count = 0;
        if pending_in > 0
            && !text.starts_with("\u{00A0}")
            && has_inline_adjacent(doc, cascade, &parent_of, idx, -1)
        {
            migrated_prefix_count = pending_in;
            text = "\u{00A0}".repeat(pending_in as usize) + &text;
        }
        // white-space phase 1 collapsing.
        // Preserve pre modes unchanged (expand tabs later); collapse others with edge trimming.
        let start = line_start_pos(doc, cascade, &parent_of, idx);
        let trim_end = trail_trim(doc, cascade, &parent_of, idx);
        let mut text = collapse_ws(&text, cv.white_space, start, trim_end).into_owned();
        // The explicit separator is inserted after phase-one white-space
        // collapse, so it does not merge with adjacent authored spaces. Keep
        // the DOM text unchanged; the inserted space is shaped by Parley.
        match cv.word_space_transform {
            WordSpaceTransform::Space => text = text.replace(ZERO_WIDTH_SPACE, " "),
            WordSpaceTransform::IdeographicSpace => {
                text = text.replace(ZERO_WIDTH_SPACE, "\u{3000}");
            }
            _ => {}
        }
        // CSS text transforms run on the post-collapse text. In particular,
        // `full-width` must see only the surviving U+0020 space; applying it
        // before whitespace collapsing would turn every source space into
        // U+3000 and incorrectly prevent the collapse.
        let mut transformed = apply_text_transform(&text, cv.text_transform, &language);
        if migrated_prefix_count > 0 && pending_full_width {
            let mut migrated = 0;
            let mut with_full_width_spaces = String::with_capacity(transformed.len());
            for c in transformed.chars() {
                if migrated < migrated_prefix_count && c == '\u{00A0}' {
                    with_full_width_spaces.push('\u{3000}');
                    migrated += 1;
                } else {
                    with_full_width_spaces.push(c);
                }
            }
            transformed = with_full_width_spaces;
        }
        // HTML tokenization may place U+0307 in its own text node. In
        // Turkic lowercase, an `I` followed by that combining dot maps to
        // `i`; fold the split pair back into the preceding shaping job.
        if transformed
            .chars()
            .eq(std::iter::once(char::from_u32(0x0307).unwrap()))
            && matches!(cv.text_transform, TextTransform::Lowercase)
            && uses_turkic_case_tailoring(&language)
            && let Some(previous) = jobs.last_mut()
            && parent_of[previous.idx] == parent_of[idx]
            && previous.text.ends_with('ı')
        {
            previous.text.pop();
            previous.text.push('i');
            continue;
        }
        text = transformed;
        // Each DOM text node gets its own Parley layout today. Preserve the
        // joining context that CSS Text requires across adjacent inline boxes
        // by mirroring the WPT reference's zero-width joiners at the edges of
        // joining-script runs. The joiner has no painted glyph or advance.
        text = add_boundary_shaping_joiners(
            text,
            boundary_shaping_adjacent(doc, cascade, &parent_of, idx, -1),
            boundary_shaping_adjacent(doc, cascade, &parent_of, idx, 1),
        );
        // A soft hyphen is only an opportunity when hyphenation is enabled.
        // Removing it for `hyphens: none` also prevents the shaping and line
        // breaker from treating it as a discretionary break.
        if cv.hyphens == Hyphens::None {
            text.retain(|c| c != '\u{00AD}');
        }
        // Phase-2 edge spacing for `word-space-transform` (issue um59.29):
        // a trailing U+200B at a text-node edge becomes a trailing ASCII
        // space (or U+3000) after phase-1 collapsing, but Parley `width()`
        // excludes trailing whitespace while `full_width()` includes it.
        // Taffy measures `width()`, so the inter-node advance would be lost.
        // Strip that trailing run here and re-add it as one wrapping
        // advance box after `ch`/tab/NBSP resolution. An `InFlow` box keeps
        // the advance in `width()` and still allows a break after it, unlike
        // an NBSP migration which forbids soft wrap. Only mid-line trailing
        // (facing inline content) is preserved; at a block end trimming
        // correctly drops it, so no box is kept there.
        let mut word_space_edge_spaces: u32 = 0;
        let mut word_space_edge_ideos: u32 = 0;
        if matches!(
            cv.word_space_transform,
            WordSpaceTransform::Space | WordSpaceTransform::IdeographicSpace
        ) && has_inline_adjacent(doc, cascade, &parent_of, idx, 1)
        {
            let mut spaces: u32 = 0;
            let mut ideos: u32 = 0;
            for c in text.chars().rev() {
                if c == ' ' {
                    spaces += 1;
                } else if c == '\u{3000}' {
                    ideos += 1;
                } else {
                    break;
                }
            }
            if spaces + ideos > 0 {
                let strip_bytes: usize = text
                    .chars()
                    .rev()
                    .take_while(|&c| c == ' ' || c == '\u{3000}')
                    .map(|c| c.len_utf8())
                    .sum();
                let new_len = text.len().saturating_sub(strip_bytes);
                text.truncate(new_len);
                word_space_edge_spaces = spaces;
                word_space_edge_ideos = ideos;
            }
        }
        let family_str: String = family_str_of(cv);
        // Tab-stop metrics use the block-container ancestor's font (CSS Text 3 §4.2).
        // If none exists, use this element's font (a fail-safe matching old behavior).
        let mcv = nearest_block_container(doc, cascade, &parent_of, idx)
            .map(|b| &cascade.computed[b])
            .unwrap_or(cv);
        // Page side margins define the inline containing block.  Keep the
        // shaped run on that same width so line breaks agree with the page
        // content box even when the remaining strip is very narrow.
        // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
        let multicol_advance =
            multicol_column_width_for_text(cascade, &parent_of, idx, max_advance);
        // cov:ignore: authored auto-width fallback is exercised by the ignored foundation WPT run.
        let authored_advance = if multicol_advance.is_none() {
            authored_containing_width_with_resolved_ch(doc, cascade, &parent_of, idx, max_advance)
        } else {
            None
        };
        // cov:ignore: multicol text shaping width selection is exercised by the ignored foundation WPT run.
        let shape_advance = if let Some(column_width) = multicol_advance {
            column_width
        } else if page_width > max_advance
            && (is_leading_body_text(doc, body_id, idx) || has_out_of_flow_ancestor(parent_of[idx]))
        {
            page_width
        } else if let Some(width) = authored_advance {
            width
        } else {
            max_advance
        };
        let autospace_value = if has_vertical_writing_mode(cascade, &parent_of, idx) {
            // This implementation normalizes vertical autospace to the
            // horizontal layout path until vertical inline metrics are implemented.
            TextAutospace::NoAutospace // cov:ignore: vertical-writing autospace is exercised by the ignored vertical WPT runs.
        } else {
            cv.text_autospace
        };
        // Own a cross-node boundary on the following nonempty text run.
        // Assigning it to both neighbors would double the advance.
        let autospace_before = autospace_adjacent_edge_char(doc, cascade, &parent_of, idx, -1);
        let autospace_width = if matches!(autospace_value, TextAutospace::NoAutospace) {
            0.0
        } else {
            let style = match cv.font_style {
                StyleFontStyle::Normal => 0,
                StyleFontStyle::Italic => 1,
                StyleFontStyle::Oblique => 2,
                // cov:ignore: StyleFontStyle currently has only three variants.
                _ => 0,
            };
            let key = (
                family_str.clone(),
                cv.font_size.px().to_bits(),
                cv.font_weight.to_bits(),
                style,
            );
            if let std::collections::hash_map::Entry::Vacant(entry) = ic_probes.entry(key.clone()) {
                entry.insert(probe_ic_text_advance(
                    fonts,
                    layout_cx,
                    &family_str,
                    cv.font_size.px(),
                    cv.font_weight,
                    cv.font_style,
                ));
            }
            ic_probes.get(&key).copied().unwrap_or(0.0) * 0.125
        };
        let autospace_boxes = text_autospace_boxes_with_width(
            &text,
            autospace_value,
            &language,
            autospace_width,
            autospace_before,
            None,
        );
        let simple_pre_block = matches!(cv.white_space, WhiteSpace::Pre)
            && parent_of[idx].is_some_and(|parent| {
                matches!(
                    cascade.computed[parent].display,
                    DisplayValue::Block | DisplayValue::InlineBlock | DisplayValue::ListItem
                ) && nearest_block_container(doc, cascade, &parent_of, idx) == Some(parent)
                    && doc.nodes[parent]
                        .children
                        .iter()
                        .filter(|&&child| doc.nodes[child].is_in_document())
                        .count()
                        == 1
            });
        // A single preserved text run wrapped in inline elements should use
        // the same metric quantization as a direct simple pre block.  Ignore
        // an otherwise-empty inline sibling containing only the terminal
        // preserved newline; it does not establish another line box.
        let simple_preserved_run = matches!(cv.white_space, WhiteSpace::PreWrap)
            && !text.contains('\n')
            && has_authored_non_whitespace_text(doc, idx)
            && parent_of[idx].is_some_and(|parent| {
                let parent_display = cascade.computed[parent].display;
                (parent_display == DisplayValue::Inline || parent_display == DisplayValue::Contents)
                    && doc.nodes[parent]
                        .children
                        .iter()
                        .filter(|&&child| has_authored_non_whitespace_text(doc, child))
                        .count()
                        == 1
                    && nearest_block_container(doc, cascade, &parent_of, idx).is_some_and(|block| {
                        doc.nodes[block].children.len() > 1
                            && matches!(cascade.computed[block].display, DisplayValue::InlineBlock)
                            && doc.nodes[block].children.iter().all(|&child| {
                                child == parent
                                    || (doc.nodes[child].kind() == NodeKind::Element
                                        && is_inline_element_box(cascade, child)
                                        && doc.nodes[child].tag_name() != Some("br")
                                        && !has_authored_non_whitespace_text(doc, child))
                                    || (doc.nodes[child].kind() == NodeKind::Text
                                        && text_of(doc, child) == Some("\n"))
                            })
                    })
            });
        // A terminal preserved newline establishes no following empty line.
        // Keeping it as a standalone Parley layout would nevertheless create
        // two line metrics, and letter-spacing makes that artifact visible.
        if text == "\n" && !has_inline_adjacent(doc, cascade, &parent_of, idx, 1) {
            continue;
        }
        jobs.push(Job {
            idx,
            had_tab: text.contains('\t'),
            text,
            family_str,
            font_size_raw: cv.font_size.px(),
            font_weight_raw: cv.font_weight,
            font_style: cv.font_style,
            line_height_raw: cv.line_height,
            letter_spacing_raw: cv.letter_spacing.px(),
            letter_spacing_ch_factor: cv.letter_spacing_ch_factor,
            letter_spacing_ch_offset: cv.letter_spacing_ch_offset,
            letter_spacing_ch_font: cv.letter_spacing_ch_font.clone(),
            word_spacing_raw: cv.word_spacing.px(),
            word_spacing_ch_factor: cv.word_spacing_ch_factor,
            word_spacing_ch_offset: cv.word_spacing_ch_offset,
            word_spacing_ch_font: cv.word_spacing_ch_font.clone(),
            tab_size: cv.tab_size,
            white_space: cv.white_space,
            word_break: cv.word_break,
            line_break: cv.line_break,
            line_break_override_enabled: false,
            has_inline_root_ancestor: has_inline_root_ancestor(doc, &parent_of, idx),
            overflow_wrap: cv.overflow_wrap,
            text_wrap_mode: cv.text_wrap,
            autospace_boxes,
            // A simple `white-space: pre` block is non-wrapping; this also
            // keeps its measured tab cursor independent of later line breaks.
            nowrap: simple_pre_block
                || cv.white_space == WhiteSpace::Nowrap
                || cv.text_wrap == TextWrapMode::Nowrap,
            max_advance: shape_advance,
            metrics_family: family_str_of(mcv),
            metrics_size: mcv.font_size.px(),
            metrics_weight: mcv.font_weight,
            metrics_style: mcv.font_style,
            metrics_letter_spacing_raw: mcv.letter_spacing.px(),
            metrics_letter_spacing_ch_factor: mcv.letter_spacing_ch_factor,
            metrics_letter_spacing_ch_offset: mcv.letter_spacing_ch_offset,
            metrics_letter_spacing_ch_font: mcv.letter_spacing_ch_font.clone(),
            metrics_word_spacing_raw: mcv.word_spacing.px(),
            metrics_word_spacing_ch_factor: mcv.word_spacing_ch_factor,
            metrics_word_spacing_ch_offset: mcv.word_spacing_ch_offset,
            metrics_word_spacing_ch_font: mcv.word_spacing_ch_font.clone(),
            simple_pre_block,
            simple_preserved_run,
            tab_spacing_ranges: Vec::new(),
            word_space_edge_spaces,
            word_space_edge_ideos,
        });
    }

    if jobs.is_empty() {
        return;
    }

    // Resolve `ch` before measuring block-container stops or text prefixes.
    // Spacing declared on the text's own element measures with its shaping
    // (or tab-metrics) font; spacing inherited in `ch` measures with the font
    // that declared it, since the computed value is an absolute length.
    let mut ch_probes: HashMap<(String, u32, u32, u8), f32> = HashMap::new();
    for job in &mut jobs {
        if let Some(factor) = job.letter_spacing_ch_factor {
            job.letter_spacing_raw = factor
                * spacing_ch_advance(
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                    job.letter_spacing_ch_font.as_ref(),
                    &job.family_str,
                    job.font_size_raw,
                    job.font_weight_raw,
                    job.font_style,
                )
                + job.letter_spacing_ch_offset;
        }
        if let Some(factor) = job.word_spacing_ch_factor {
            job.word_spacing_raw = factor
                * spacing_ch_advance(
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                    job.word_spacing_ch_font.as_ref(),
                    &job.family_str,
                    job.font_size_raw,
                    job.font_weight_raw,
                    job.font_style,
                )
                + job.word_spacing_ch_offset;
        }
        if let Some(factor) = job.metrics_letter_spacing_ch_factor {
            job.metrics_letter_spacing_raw = factor
                * spacing_ch_advance(
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                    job.metrics_letter_spacing_ch_font.as_ref(),
                    &job.metrics_family,
                    job.metrics_size,
                    job.metrics_weight,
                    job.metrics_style,
                )
                + job.metrics_letter_spacing_ch_offset;
        }
        if let Some(factor) = job.metrics_word_spacing_ch_factor {
            job.metrics_word_spacing_raw = factor
                * spacing_ch_advance(
                    fonts,
                    layout_cx,
                    &mut ch_probes,
                    job.metrics_word_spacing_ch_font.as_ref(),
                    &job.metrics_family,
                    job.metrics_size,
                    job.metrics_weight,
                    job.metrics_style,
                )
                + job.metrics_word_spacing_ch_offset;
        }
        if !matches!(
            job.white_space,
            WhiteSpace::Pre | WhiteSpace::PreWrap | WhiteSpace::BreakSpaces
        ) || !job.text.contains('\t')
        {
            continue;
        }
        if !job.simple_pre_block {
            // Inline siblings need one shared post-layout cursor. Keep their
            // established expansion until the inline bridge exposes that state.
            let space_advance = probe_text_advance(
                fonts,
                layout_cx,
                " ",
                &job.metrics_family,
                job.metrics_size,
                job.metrics_weight,
                job.metrics_style,
            );
            let (text, tab_lens) = expand_tabs_locally(&job.text, job.tab_size, space_advance);
            remap_boxes_through_tab_rewrite(&job.text, &tab_lens, &mut job.autospace_boxes);
            job.text = text;
            continue;
        }
        let block_space_advance = probe_text_full_width(
            fonts,
            layout_cx,
            " ",
            TextProbeStyle {
                family_str: &job.metrics_family,
                font_size_px: job.metrics_size,
                font_weight: job.metrics_weight,
                font_style: job.metrics_style,
                letter_spacing: job.metrics_letter_spacing_raw,
                word_spacing: job.metrics_word_spacing_raw,
            },
        );
        let interval = tab_stop_advance(job.tab_size, block_space_advance);
        let space_base_advance = probe_text_full_width(
            fonts,
            layout_cx,
            " ",
            TextProbeStyle {
                family_str: &job.family_str,
                font_size_px: job.font_size_raw,
                font_weight: job.font_weight_raw,
                font_style: job.font_style,
                letter_spacing: job.letter_spacing_raw,
                word_spacing: 0.0,
            },
        );
        let text_word_spacing = if job.word_spacing_ch_factor.is_some() {
            job.word_spacing_raw
        } else {
            0.0
        };
        let (text, spacing_ranges, tab_lens) =
            replace_tabs_with_styled_spaces(&job.text, interval, space_base_advance, |segment| {
                probe_text_full_width(
                    fonts,
                    layout_cx,
                    segment,
                    TextProbeStyle {
                        family_str: &job.family_str,
                        font_size_px: job.font_size_raw,
                        font_weight: job.font_weight_raw,
                        font_style: job.font_style,
                        letter_spacing: job.letter_spacing_raw,
                        word_spacing: text_word_spacing,
                    },
                )
            });
        remap_boxes_through_tab_rewrite(&job.text, &tab_lens, &mut job.autospace_boxes);
        job.text = text;
        job.tab_spacing_ranges = spacing_ranges;
    }

    for job in &mut jobs {
        let adapter_enabled = should_enable_nbsp_adapter(
            job.line_break,
            job.white_space,
            job.nowrap,
            &job.text,
            job.had_tab,
            job.has_inline_root_ancestor,
        );
        job.line_break_override_enabled = should_enable_anywhere_callback(
            job.line_break,
            job.white_space,
            job.nowrap,
            &job.text,
            adapter_enabled,
        );
        if job.white_space == WhiteSpace::PreWrap && !adapter_enabled {
            job.line_break_override_enabled = false;
        }
        if !adapter_enabled {
            continue;
        }

        let word_spacing = if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
            job.word_spacing_raw
        } else {
            0.0
        };
        let advance = probe_text_full_width(
            fonts,
            layout_cx,
            "\u{00A0}",
            TextProbeStyle {
                family_str: &job.family_str,
                font_size_px: job.font_size_raw,
                font_weight: job.font_weight_raw,
                font_style: job.font_style,
                letter_spacing: job.letter_spacing_raw,
                word_spacing,
            },
        );
        if !advance.is_finite() || advance <= 0.0 {
            job.line_break_override_enabled = false;
            continue;
        }
        match replace_nbsp_with_inline_boxes(&job.text, advance, &job.autospace_boxes) {
            Some((text, inline_boxes)) => {
                job.text = text;
                job.autospace_boxes = inline_boxes;
            }
            None => job.line_break_override_enabled = false,
        }
    }

    // Re-add stripped trailing `word-space-transform` separators as one
    // wrapping advance box (issue um59.29). The text run keeps its stripped
    // form so `width()` already includes the edge; the box supplies the exact
    // trailing advance measured with the resolved spacing. A plain trailing
    // space would be excluded from `width()` and lost by Taffy, while an NBSP
    // would forbid the soft-wrap opportunity the CSS Text 4 transform must
    // keep (013/014 keep-all cases).
    for job in &mut jobs {
        if job.word_space_edge_spaces == 0 && job.word_space_edge_ideos == 0 {
            continue;
        }
        // Edge id base (1<<61) is distinct from autospace (u64::MAX-...) and
        // line-break NBSP (1<<62..1<<63) ranges, so a collision cannot occur.
        // No guard is needed; the single edge box per layout keeps its fixed id.
        let word_spacing = if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
            job.word_spacing_raw
        } else {
            0.0
        };
        let mut total = 0.0f32;
        if job.word_space_edge_spaces > 0 {
            let advance = probe_text_full_width(
                fonts,
                layout_cx,
                " ",
                TextProbeStyle {
                    family_str: &job.family_str,
                    font_size_px: job.font_size_raw,
                    font_weight: job.font_weight_raw,
                    font_style: job.font_style,
                    letter_spacing: job.letter_spacing_raw,
                    word_spacing,
                },
            );
            if !advance.is_finite() || advance <= 0.0 {
                continue; // cov:ignore: space advance probing with a valid shaped font always succeeds.
            }
            total += advance * job.word_space_edge_spaces as f32;
        }
        if job.word_space_edge_ideos > 0 {
            let advance = probe_text_full_width(
                fonts,
                layout_cx,
                "\u{3000}",
                TextProbeStyle {
                    family_str: &job.family_str,
                    font_size_px: job.font_size_raw,
                    font_weight: job.font_weight_raw,
                    font_style: job.font_style,
                    letter_spacing: job.letter_spacing_raw,
                    word_spacing,
                },
            );
            if !advance.is_finite() || advance <= 0.0 {
                continue; // cov:ignore: ideographic advance probing with a valid shaped font always succeeds.
            }
            total += advance * job.word_space_edge_ideos as f32;
        }
        if !total.is_finite() || total <= 0.0 {
            continue; // cov:ignore: total sums finite positive advances, so it is always finite and positive here.
        }
        job.autospace_boxes.push(InlineBox {
            id: WORD_SPACE_EDGE_INLINE_BOX_ID_BASE,
            kind: InlineBoxKind::InFlow,
            index: job.text.len(),
            width: total,
            height: 0.0,
        });
    }

    // Copy original FontContext once; each rayon task clones from this base.
    // LayoutContext is cheap (clone returns new empty), so per-task new() is fine.
    // For very small job counts rayon overhead dominates; use sequential fallback.
    const PAR_THRESHOLD: usize = 32;
    if jobs.len() < PAR_THRESHOLD {
        for job in jobs {
            let mut warnings: Vec<LayoutWarn> = Vec::new();
            let font_size_px = sanitize_finite(
                job.font_size_raw,
                0.0,
                MAX_FONT_SIZE_PX,
                "font-size",
                &mut warnings,
            );
            let font_weight = sanitize_font_weight(job.font_weight_raw, &mut warnings);
            let line_height = sanitize_line_height(job.line_height_raw, &mut warnings);
            let letter_spacing = sanitize_finite(
                job.letter_spacing_raw,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "letter-spacing",
                &mut warnings,
            );
            let word_spacing = sanitize_finite(
                job.word_spacing_raw,
                -MAX_TAFFY_MAGNITUDE,
                MAX_TAFFY_MAGNITUDE,
                "word-spacing",
                &mut warnings,
            );
            let font_family = FontFamily::from(job.family_str.as_str());
            // In this simple block run, Taffy's fractional line flow must use
            // the same metrics as Parley's painted baselines.
            let quantize_metrics = !job.simple_pre_block && !job.simple_preserved_run;
            let mut builder = layout_cx.ranged_builder(fonts, &job.text, 1.0, quantize_metrics);
            builder.set_line_break_override(parley_line_break_override(
                job.line_break_override_enabled,
            ));
            builder.push_default(StyleProperty::FontFamily(font_family));
            builder.push_default(StyleProperty::FontSize(font_size_px));
            builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
            builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                job.font_style,
            )));
            builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                line_height,
            )));
            builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
            // Apply negative CSS word-spacing lengths in addition to the existing
            // font-metric-aware `ch` path. Other non-`ch` values remain deferred.
            if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
                builder.push_default(StyleProperty::WordSpacing(word_spacing));
            }
            for (range, tab_word_spacing) in &job.tab_spacing_ranges {
                builder.push(StyleProperty::WordSpacing(*tab_word_spacing), range.clone());
            }
            builder.push_default(StyleProperty::WordBreak(parley_word_break(
                job.word_break,
                job.line_break,
            )));
            builder.push_default(StyleProperty::OverflowWrap(parley_overflow_wrap(
                job.word_break,
                job.line_break,
                job.overflow_wrap,
            )));
            builder.push_default(StyleProperty::TextWrapMode(parley_text_wrap_mode(
                job.text_wrap_mode,
                job.nowrap,
            )));
            for inline_box in &job.autospace_boxes {
                builder.push_inline_box(inline_box.clone());
            }
            let mut layout: Layout<()> = builder.build(&job.text);
            layout.break_all_lines(if job.nowrap {
                None
            } else {
                Some(job.max_advance)
            });
            layout.align(Alignment::Start, AlignmentOptions::default());
            doc.layout_warnings.extend(warnings);
            if let Some(t) = doc.nodes[job.idx].data.as_text_mut() {
                t.text_layout = Some(layout);
                t.snap_glyph_x_to_1_64 = job.simple_pre_block || job.simple_preserved_run;
            }
        }
        return;
    }

    let base_fonts: FontContext = fonts.clone();
    // Parallel shaping: chunked to amortize FontContext/LayoutContext setup.
    // Per-job `LayoutContext::new()` is expensive (ICU AnalysisDataSources etc.)
    // so we reuse one FontContext+LayoutContext per rayon chunk.
    // Chunk size 128 reduces clones to ~4/16 for 500/2000 jobs.
    let results: Vec<(usize, Layout<()>, Vec<LayoutWarn>, bool)> = jobs
        .par_iter()
        .chunks(256)
        .flat_map(|chunk| {
            let mut fonts_thread = base_fonts.clone();
            let mut lcx = LayoutContext::<()>::new();
            let mut out = Vec::with_capacity(chunk.len());
            for job in chunk {
                let mut warnings: Vec<LayoutWarn> = Vec::new();
                let font_size_px = sanitize_finite(
                    job.font_size_raw,
                    0.0,
                    MAX_FONT_SIZE_PX,
                    "font-size",
                    &mut warnings,
                );
                let font_weight = sanitize_font_weight(job.font_weight_raw, &mut warnings);
                let line_height = sanitize_line_height(job.line_height_raw, &mut warnings);
                let letter_spacing = sanitize_finite(
                    job.letter_spacing_raw,
                    -MAX_TAFFY_MAGNITUDE,
                    MAX_TAFFY_MAGNITUDE,
                    "letter-spacing",
                    &mut warnings,
                );
                let word_spacing = sanitize_finite(
                    job.word_spacing_raw,
                    -MAX_TAFFY_MAGNITUDE,
                    MAX_TAFFY_MAGNITUDE,
                    "word-spacing",
                    &mut warnings,
                );
                let font_family = FontFamily::from(job.family_str.as_str());
                let quantize_metrics = !job.simple_pre_block && !job.simple_preserved_run;
                let mut builder =
                    lcx.ranged_builder(&mut fonts_thread, &job.text, 1.0, quantize_metrics);
                builder.set_line_break_override(parley_line_break_override(
                    job.line_break_override_enabled,
                ));
                builder.push_default(StyleProperty::FontFamily(font_family));
                builder.push_default(StyleProperty::FontSize(font_size_px));
                builder.push_default(StyleProperty::FontWeight(FontWeight::new(font_weight)));
                builder.push_default(StyleProperty::FontStyle(font_style_to_parley(
                    job.font_style,
                )));
                builder.push_default(StyleProperty::LineHeight(line_height_to_parley(
                    line_height,
                )));
                builder.push_default(StyleProperty::LetterSpacing(letter_spacing));
                if job.word_spacing_ch_factor.is_some() || job.word_spacing_raw < 0.0 {
                    builder.push_default(StyleProperty::WordSpacing(word_spacing));
                }
                for (range, tab_word_spacing) in &job.tab_spacing_ranges {
                    builder.push(StyleProperty::WordSpacing(*tab_word_spacing), range.clone());
                }
                builder.push_default(StyleProperty::WordBreak(parley_word_break(
                    job.word_break,
                    job.line_break,
                )));
                builder.push_default(StyleProperty::OverflowWrap(parley_overflow_wrap(
                    job.word_break,
                    job.line_break,
                    job.overflow_wrap,
                )));
                builder.push_default(StyleProperty::TextWrapMode(parley_text_wrap_mode(
                    job.text_wrap_mode,
                    job.nowrap,
                )));
                // cov:ignore: the parallel preshape path is exercised by resource-backed WPT runs with large inline job sets.
                for inline_box in &job.autospace_boxes {
                    builder.push_inline_box(inline_box.clone());
                }
                let mut layout: Layout<()> = builder.build(&job.text);
                layout.break_all_lines(if job.nowrap {
                    None
                } else {
                    Some(job.max_advance)
                });
                layout.align(Alignment::Start, AlignmentOptions::default());
                out.push((
                    job.idx,
                    layout,
                    warnings,
                    job.simple_pre_block || job.simple_preserved_run,
                ));
            }
            out
        })
        .collect();

    for (idx, layout, warnings, snap_glyph_x_to_1_64) in results {
        doc.layout_warnings.extend(warnings);
        if let Some(t) = doc.nodes[idx].data.as_text_mut() {
            t.text_layout = Some(layout);
            t.snap_glyph_x_to_1_64 = snap_glyph_x_to_1_64;
        }
    }
}

#[cfg(test)]
mod tests;
