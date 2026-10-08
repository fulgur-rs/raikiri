use super::*;
use std::cell::Cell;

// Flex traversal skips Contents boxes when globally ordering effective
// items; preserve their stored offsets relative to the visited parent.
fn pagination_child_parent_y(document: &Document, parent: usize, child: usize, raw_y: f32) -> f32 {
    let mut child_parent_y = raw_y;
    let mut skipped_parent = document.parent_of(child);
    while let Some(id) = skipped_parent.filter(|&id| id != parent) {
        child_parent_y += document.nodes[id].unrounded_layout.location.y;
        skipped_parent = document.parent_of(id);
    }
    child_parent_y
}

// Cache the nearest column context for each visited source node. Row flex
// suppresses local named-flow ownership, but its content still occupies the
// enclosing physical page. Each source edge is resolved once per layout pass.
fn enclosing_column_context(
    node_id: usize,
    parent_of: &[Option<usize>],
    cascade: &CascadeResult,
    cache: &mut [Option<Option<usize>>],
) -> Option<usize> {
    let mut current = node_id;
    let mut visited = Vec::new();
    let context = loop {
        if let Some(cached) = cache[current] {
            break cached;
        }
        visited.push(current);
        let Some(parent) = parent_of[current] else {
            break None;
        };
        let computed = &cascade.computed[parent];
        if matches!(
            computed.display,
            DisplayValue::Flex | DisplayValue::InlineFlex
        ) && matches!(
            computed.flex_direction,
            FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
        ) {
            break Some(parent);
        }
        current = parent;
    };
    for node in visited {
        cache[node] = Some(context);
    }
    context
}

fn column_context_occupied(
    context: usize,
    page: u32,
    occupied: &HashMap<usize, u32>,
    parent_of: &[Option<usize>],
    cascade: &CascadeResult,
    cache: &mut [Option<Option<usize>>],
) -> bool {
    let mut current = Some(context);
    while let Some(id) = current {
        if occupied.get(&id) == Some(&page) {
            return true;
        }
        current = enclosing_column_context(id, parent_of, cascade, cache);
    }
    false
}

fn occupy_column_contexts(
    mut context: Option<usize>,
    page: u32,
    occupied: &mut HashMap<usize, u32>,
    parent_of: &[Option<usize>],
    cascade: &CascadeResult,
    cache: &mut [Option<Option<usize>>],
) {
    while let Some(id) = context {
        if occupied.insert(id, page) == Some(page) {
            break;
        }
        context = enclosing_column_context(id, parent_of, cascade, cache);
    }
}

/// Break the lines of every paragraph again for a page-specific
/// containing-block width.
///
/// Pagination can change the page geometry after the first layout pass. This
/// helper refreshes only line breaking, leaving taffy's already computed box
/// geometry intact, so a page-aware painter can use the correct line breaks for
/// the page it is about to paint. It is intentionally separate from
/// [`layout_single_page`] because callers must opt into this narrow
/// post-pagination operation.
///
/// Each paragraph is broken at the content width of the nearest ancestor
/// with an authored width, otherwise at `max_advance`.
pub fn relayout_text_for_width(document: &mut Document, cascade: &CascadeResult, max_advance: f32) {
    document.page_projection.clear();
    crate::layout::ifc::flow::rebreak_roots(document, cascade, max_advance);
}

/// Correct the static position of grid abspos items whose placement is `auto`.
/// Taffy handles explicit grid-area placement, but its static-position fallback
/// uses the border edge instead of the grid content box.
fn realign_grid_abspos_static_positions(document: &mut Document, cascade: &CascadeResult) {
    fn align_offset(value: AlignSelfValue, parent: SelfAlignmentValue, free: f32) -> f32 {
        let value = match value {
            AlignSelfValue::Auto => parent,
            AlignSelfValue::Value(value) => value,
            _ => SelfAlignmentValue::Start,
        };
        match value {
            SelfAlignmentValue::Center => free / 2.0,
            SelfAlignmentValue::End | SelfAlignmentValue::FlexEnd => free,
            _ => 0.0,
        }
    }
    for child_id in 0..document.nodes.len() {
        let child = &cascade.computed[child_id];
        if !matches!(
            child.position,
            PositionValue::Absolute | PositionValue::Fixed
        ) || !matches!(child.top, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.right, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.bottom, ComputedLengthPercentageOrAuto::Auto)
            || !matches!(child.left, ComputedLengthPercentageOrAuto::Auto)
        {
            continue;
        }
        let Some(parent_id) = document
            .nodes
            .iter()
            .position(|node| node.children.contains(&child_id))
        else {
            continue;
        };
        let parent = &cascade.computed[parent_id];
        if !matches!(
            parent.display,
            DisplayValue::Grid | DisplayValue::InlineGrid
        ) || !matches!(child.grid_row_start, GridLineValue::Auto)
            || !matches!(child.grid_row_end, GridLineValue::Auto)
            || !matches!(child.grid_column_start, GridLineValue::Auto)
            || !matches!(child.grid_column_end, GridLineValue::Auto)
        {
            continue;
        }
        let parent_size = document.nodes[parent_id].unrounded_layout.size;
        let border_left = parent.border.left.width().px();
        let border_right = parent.border.right.width().px();
        let border_top = parent.border.top.width().px();
        let border_bottom = parent.border.bottom.width().px();
        let parent_style = &document.nodes[parent_id].style;
        let padding_left =
            used_style_length_percentage(parent_style.padding.left, parent_size.width)
                .unwrap_or(0.0);
        let padding_right =
            used_style_length_percentage(parent_style.padding.right, parent_size.width)
                .unwrap_or(0.0);
        let padding_top = used_style_length_percentage(parent_style.padding.top, parent_size.width)
            .unwrap_or(0.0);
        let padding_bottom =
            used_style_length_percentage(parent_style.padding.bottom, parent_size.width)
                .unwrap_or(0.0);
        let content_width =
            (parent_size.width - border_left - border_right - padding_left - padding_right)
                .max(0.0);
        let content_height =
            (parent_size.height - border_top - border_bottom - padding_top - padding_bottom)
                .max(0.0);
        let child_size = document.nodes[child_id].unrounded_layout.size;
        let x = border_left + padding_left;
        let y = border_top
            + padding_top
            + align_offset(
                child.align_self,
                parent.align_items,
                content_height - child_size.height,
            );
        let child_layout = &mut document.nodes[child_id].unrounded_layout;
        child_layout.location.x = x;
        child_layout.location.y = y;
        let _ = content_width; // horizontal normal alignment is start in this fallback.
    }
}

/// Lay out a Document on one A4 page (or the specified PageBox).
///
/// Text is laid out by the inline engine with the fonts the Document holds
/// ([`Document::set_font_collection`]). A Document that was given no fonts
/// takes the process-wide layer of the installed fonts
/// ([`crate::system_font_collection`]) on its first layout.
///
/// # In-place changes
/// - Bridge computed values to taffy::Style with `apply_computed_to_style`
///   and assign the paragraphs of the inline engine
/// - Set body.style.size to the page content box with `apply_page_content_box_to_body`
/// - Run taffy with the body as the root (`compute_body_root_layout`), store
///   results in Node.unrounded_layout and move the body's content below its
///   used block-start margin
///
/// # Errors
/// - `LayoutError::Internal` — no `<body>` element found (fragment parses
///   are not supported yet) or an internal taffy error
/// - `LayoutError::IfcUnsupported` / `LayoutError::IfcLimitExceeded` — a
///   paragraph the inline engine cannot lay out, or one over its limits
/// - `LayoutError::FragmentLimitExceeded` — aggregate fragment budget exhausted
///
/// # Current non-goals
/// - Calling this repeatedly on one Document is safe (per-pass state is
///   cleared each time), but incremental recomputation is planned for later.
/// - Consumer PageBox overrides will be handled by future per-page PageBox support.
/// - Fragment parses (without `<body>`) will be supported later.
pub fn layout_single_page(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<(), LayoutError> {
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    // At this observation-side entry point, synchronize membership.
    // `mark_in_document_flags` is an idempotent no-op when flags_dirty=false, so
    // this is effectively free after sink.finish(). It protects consumers that
    // skip cascade and call layout directly after post-parse mutation
    // (`Document::append_*`, etc.).
    //
    // Contract: cascade takes `&D: Dom` and cannot mutate, so its caller must
    // synchronize flags (automatic synchronization occurs only through parse.finish()).
    // Layout synchronizes here to ensure that stale flags never reach at least
    // the layout / paint stages.
    document.mark_in_document_flags();
    crate::image_resolve::resolve_inline_svg_intrinsic_sizes(document);
    crate::image_resolve::resolve_canvas_intrinsic_sizes(document);
    if document.layout_cascade_generation != Some(cascade.generation()) {
        // Computed Grid/Flex style can change without a DOM tree mutation. Do
        // not let Taffy's per-node cache or resolved Grid rows survive that
        // cascade transition.
        document.layout_dirty = true;
    }
    document.layout_cascade_generation = None;
    // Fonts are only read when a document is laid out: one that was given
    // none takes the installed fonts here, not when it is created.
    if document.ifc.is_none() {
        document.set_font_collection(crate::fonts::system_font_collection());
    }

    // Step 0: layout_warnings re-entrance clear: this Vec is populated
    // over the course of a pass (bridges below, then the taffy compute step
    // via `set_unrounded_layout`) and drained near the end of this function,
    // but an early `?` return (Step 3) would otherwise leave a previous call's
    // leftover entries for the next call to inherit.
    document.layout_warnings.clear();
    document.table_layout_error = None;
    document.fragment_tree.clear();
    document.fragmentation_stack.clear();

    // Step 1: ComputedValues → taffy::Style bridge (currently a no-op site).
    apply_computed_to_style(document, cascade)?;

    // Step 2: resolve the paper/content split before shaping.  Text wrapping
    // uses the content width, not the outer paper width.
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    // Page decorations affect the physical origin, not the inline size of the
    // initial containing block.  This also keeps text from wrapping merely
    // because an @page rule adds border/padding around the paper.
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);

    // Step 2b: resolve `ch` lengths of box properties with the inline
    // engine's fonts before taffy sizes the boxes.
    prepare_ch_box_values_before_taffy(document, cascade);
    crate::layout::ifc::assign::prepare_legacy_inside_markers_before_taffy(document, cascade)?;
    // Step 3: <body> lookup
    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;
    let body_insets = BodyInlineInsets::resolve(document, cascade, body_id, content_width);

    // Establish the foundational multicolumn fragmentainer projection after
    // text shaping, so direct text can be split by its actual line count.
    // Boxes with an `auto` width chain up to the body take the body's content
    // width, which its horizontal margins, padding and borders narrow.
    // cov:ignore: exercised by the ignored foundation WPT run; default coverage skips ignored reftests.
    prepare_multicol_layout(document, cascade, body_insets.content_width(content_width));

    // The body is the taffy root, which ignores its margins. Its block-start
    // margin is resolved here and placed after layout (or carried as top
    // padding, see below), where it can collapse with its first child's;
    // the bottom margin has no effect on a root sized to the page content
    // box. The horizontal margins, UA or authored, become inline padding of
    // the root after Step 4 (`move_body_inline_margins_into_padding`).
    let body_margin_top = body_margin_top(document, cascade, body_id, content_width);
    {
        let style_margin = &mut document.nodes[body_id].style.margin;
        style_margin.top = LengthPercentageAuto::length(0.0);
        style_margin.bottom = LengthPercentageAuto::length(0.0);
    }
    propagate_body_overflow_to_viewport(document, cascade, body_id);
    let body_top_may_collapse = body_top_margin_may_collapse(document, cascade, body_id);

    // Step 4: force body.style.size to the page content box, not the full paper size.
    // Page margins are painted/represented outside this taffy root.
    apply_page_content_box_to_body(document, body_id, page_box, margins, insets);
    move_body_inline_margins_into_padding(document, body_id, body_insets, content_width);
    // A body whose content is one inline-engine paragraph has line boxes that
    // keep its top margin from collapsing; carry it as top padding so the
    // lines and the boxes among them start below it.
    let body_is_paragraph = document.nodes[body_id].is_ifc_root();
    let body_top_padding = BodyTopPadding::capture(document, body_id, content_width);
    let mut body_padded_top = if body_is_paragraph {
        body_top_padding.apply(document, body_id, body_margin_top)
    } else {
        0.0
    };
    // CSS 2.1 §10.3.7 absolute width:auto shrink-to-fit needs no pre-pass:
    // taffy already shrink-wraps direct-body and nested absolute boxes alike.
    // See the §10.3.7 note on the layout helpers for the removed fill override.

    // Fixed boxes laid out by the inline engine shrink against the page area.
    if let Some(state) = document.ifc.as_mut() {
        state.page_width = Some(content_width);
    }
    // Step 5: taffy compute
    let available_space = taffy::Size {
        width: AvailableSpace::Definite(content_width),
        height: AvailableSpace::Definite(content_height),
    };
    let escaped_top_margin = compute_body_root_layout(
        document,
        TaffyNodeId::from(body_id),
        available_space,
        body_top_may_collapse,
    );
    // A block box that starts the body's paragraph was placed by its own
    // top margins inside the paragraph; those collapse with the body's. Lay
    // the body out again with only the part of the collapsed margin that
    // they leave over as top padding. Only the body's own box changes, so
    // the boxes inside keep their cached layouts.
    let mut paragraph_collapsed_top = None;
    if body_is_paragraph && let Some(leading) = body_paragraph_leading_margin(document, body_id) {
        let collapsed = leading.collapse_with_margin(body_margin_top).resolve();
        let padding = collapsed - leading.resolve();
        if (padding.max(0.0) - body_padded_top).abs() > 1e-4 {
            body_padded_top = body_top_padding.apply(document, body_id, padding);
            document.nodes[body_id].cache.clear();
            compute_body_root_layout(document, TaffyNodeId::from(body_id), available_space, false);
        }
        // A negative remainder is clamped, as padding cannot be negative.
        paragraph_collapsed_top = Some(body_padded_top + leading.resolve());
    }
    // CSS 2.1 §8.3.1: the body's top margin and the top margins that escaped
    // its first in-flow children collapse into one, the largest positive
    // margin plus the most negative one. When nothing escaped (top padding
    // or border, a new formatting context, no in-flow child) this is the
    // body's own margin. Move the content below it.
    document.body_block_start_margin = if body_is_paragraph {
        paragraph_collapsed_top.unwrap_or(body_padded_top)
    } else {
        let collapsed = escaped_top_margin
            .collapse_with_margin(body_margin_top)
            .resolve();
        shift_body_content(document, body_id, collapsed);
        collapsed
    };
    if let Some(message) = document.table_layout_error.take() {
        // A rejected pass must not cache its temporary zero-sized fallback.
        document.layout_dirty = true;
        return Err(LayoutError::Internal { message });
    }
    if document.fragment_tree.limit_exceeded {
        return Err(LayoutError::FragmentLimitExceeded {
            limit: document.fragment_tree.limit,
        });
    }
    // Step 5a: post-layout corrections taffy does not make: the static
    // position of auto-placed grid abspos items, and auto-height ancestors of
    // floats.
    realign_grid_abspos_static_positions(document, cascade);
    propagate_float_bottoms_to_auto_height_ancestors(document, cascade);
    refresh_projected_multicol_text_fragments(document);
    // Step 5b: check semantic parent-child geometry invariants and replace
    // any invalid subtree with the deterministic fallback (zero). Step 5
    // (`sanitize_taffy_layout` via `set_unrounded_layout`) guarantees only
    // finite values. This check therefore sits one layer above it; see the
    // `enforce_layout_invariants` documentation. Events enter the same
    // `document.layout_warnings` buffer as Steps 1 / 2; Step 6 drains it.
    enforce_layout_invariants(document, body_id);
    document.fragment_tree.finalize();

    // Step 6: replay buffered LayoutWarn events.
    //
    // `document.layout_warnings` accumulated events from this function's own
    // bridge calls (Step 1 / Step 2, via `&mut document.layout_warnings`
    // passed directly), from `<Document as
    // taffy::LayoutPartialTree>::set_unrounded_layout` (invoked internally
    // by `compute_root_layout` just above, via `self` — see
    // `Document::layout_warnings`'s doc for why that trait-fixed signature
    // can only reach an owned buffer, not a live observer), and from Step 5b
    // (`enforce_layout_invariants`) just above, which
    // pushes into the same buffer directly since it already holds `&mut
    // Document`.
    //
    // No external caller can supply an observer yet — `layout_single_page`'s
    // signature is a dom→paint boundary (`raikiri-paint` and `raikiri` both
    // call it directly) and adding a parameter, or a new `_with_observer`
    // sibling, is a decision for that wall rather than this task. `observer`
    // is therefore always `None` today, so this always falls back to the
    // same `eprintln!` shape `fonts.rs` uses when uncalled with an observer —
    // but the integration logic is real and ready for a future `_with_observer`
    // sibling to wire an observer through with no further refactor.
    let mut observer: LayoutWarnObserver<'_> = None;
    for event in document.layout_warnings.drain(..) {
        emit_layout_warn(&mut observer, event);
    }
    document.layout_cascade_generation = Some(cascade.generation());

    Ok(())
}

/// One page in the block-flow pagination result.
///
/// `content_origin_y` is measured in the single laid-out document's body
/// coordinate space.  A page-aware painter subtracts it before adding the
/// page's physical top margin.  Keeping the source coordinate here means a
/// later scene can select a page without cloning or relaying out the DOM.
#[derive(Debug, Clone, PartialEq)]
pub struct PageSlice {
    /// Zero-based page number.
    pub page_index: u32,
    /// Body-content y coordinate at which this page begins.
    pub content_origin_y: f32,
    /// Named page selected by the first class-A box on this page.
    pub page_name: Option<String>,
}

/// Controls page production and cancellation in the paginator.
///
/// The default matches [`raikiri_traits::RenderLimits::default`]. Pass `None`
/// to [`Self::new`] only when the caller intentionally wants unbounded pages.
pub struct PageLayoutControl<'a> {
    max_pages: Option<u32>,
    abort_check: Option<&'a dyn Fn() -> bool>,
    truncate_at_page_limit: bool,
    page_limit_reached: Cell<bool>,
}

impl<'a> PageLayoutControl<'a> {
    /// Create a paginator control with the supplied page limit.
    pub fn new(max_pages: Option<u32>) -> Self {
        Self {
            max_pages,
            abort_check: None,
            truncate_at_page_limit: false,
            page_limit_reached: Cell::new(false),
        }
    }

    /// Create a provisional paginator for page-geometry discovery.
    ///
    /// When a page limit is set and pagination reaches it, layout stops and
    /// returns the page prefix. Call [`Self::page_limit_reached`] to distinguish
    /// that partial result from a complete layout. A final scheduled pass
    /// should use [`Self::new`] so excess pages are rejected.
    pub fn for_geometry_discovery(max_pages: Option<u32>) -> Self {
        Self {
            max_pages,
            abort_check: None,
            truncate_at_page_limit: true,
            page_limit_reached: Cell::new(false),
        }
    }

    /// Check for cancellation while collecting and producing page candidates.
    pub fn with_abort_check(mut self, abort_check: &'a dyn Fn() -> bool) -> Self {
        self.abort_check = Some(abort_check);
        self
    }

    /// Return whether the most recent paginator call returned a partial page
    /// prefix because it reached this control's limit.
    pub fn page_limit_reached(&self) -> bool {
        self.page_limit_reached.get()
    }

    fn check_aborted(&self) -> Result<(), LayoutError> {
        if self.abort_check.is_some_and(|check| check()) {
            Err(LayoutError::Aborted)
        } else {
            Ok(())
        }
    }

    fn check_page_index(&self, page_index: u32) -> Result<(), LayoutError> {
        self.check_aborted()?;
        let actual = u64::from(page_index) + 1;
        if let Some(limit) = self.max_pages
            && actual > u64::from(limit)
        {
            return Err(LayoutError::PageLimitExceeded { limit, actual });
        }
        Ok(())
    }

    fn check_discovery_page_index(&self, page_index: u32) -> Result<(), LayoutError> {
        match self.check_page_index(page_index) {
            Err(LayoutError::PageLimitExceeded { .. }) if self.truncate_at_page_limit => {
                self.page_limit_reached.set(true);
                Ok(())
            }
            result => result,
        }
    }
}

impl Default for PageLayoutControl<'_> {
    fn default() -> Self {
        Self::new(raikiri_traits::RenderLimits::default().max_document_pages)
    }
}

struct PageOrigins {
    fixed_step: f32,
    scheduled_steps: Vec<f32>,
    scheduled_origins: Vec<f32>,
}

impl PageOrigins {
    fn new(page_steps: &[f32], fixed_step: f32) -> Self {
        let scheduled_steps: Vec<_> = page_steps
            .iter()
            .map(|step| {
                if step.is_finite() && *step > 0.0 {
                    *step
                } else {
                    fixed_step
                }
            })
            .collect();
        let mut scheduled_origins = Vec::with_capacity(scheduled_steps.len() + 1);
        scheduled_origins.push(0.0);
        for step in &scheduled_steps {
            let origin = scheduled_origins.last().copied().unwrap_or(0.0) + step;
            scheduled_origins.push(origin);
        }
        Self {
            fixed_step,
            scheduled_steps,
            scheduled_origins,
        }
    }

    fn step_at(&self, page_index: u32) -> f32 {
        self.scheduled_steps
            .get(page_index as usize)
            .copied()
            .unwrap_or(self.fixed_step)
    }

    fn origin(&self, page_index: u32) -> f32 {
        let index = page_index as usize;
        if self.scheduled_steps.is_empty() {
            return page_index as f32 * self.fixed_step;
        }
        if let Some(origin) = self.scheduled_origins.get(index) {
            return *origin;
        }
        let scheduled_pages = self.scheduled_steps.len();
        let scheduled_end = self.scheduled_origins[scheduled_pages];
        scheduled_end + (index - scheduled_pages) as f32 * self.fixed_step
    }

    fn page_index_for_y(&self, y: f32) -> u32 {
        if !y.is_finite() || y <= 0.0 {
            return 0;
        }
        if self.scheduled_steps.is_empty() {
            let tail_page = (f64::from(y) / f64::from(self.fixed_step)).floor();
            return self.correct_page_index_for_y(self.page_index_after_prefix(tail_page), y);
        }
        let scheduled_pages = self.scheduled_steps.len();
        let scheduled_end = self.scheduled_origins[scheduled_pages];
        if y < scheduled_end {
            let index = self.scheduled_origins[..=scheduled_pages]
                .partition_point(|origin| *origin <= y)
                .saturating_sub(1);
            return self.correct_page_index_for_y(index as u32, y);
        }
        let extra =
            ((f64::from(y) - f64::from(scheduled_end)) / f64::from(self.fixed_step)).floor();
        self.correct_page_index_for_y(self.page_index_after_prefix(extra), y)
    }

    fn page_index_for_end(&self, end: f32) -> u32 {
        if !end.is_finite() || end <= 0.0 {
            return 0;
        }
        if self.scheduled_steps.is_empty() {
            let tail_page = (f64::from(end) / f64::from(self.fixed_step)).ceil() - 1.0;
            return self
                .correct_page_index_for_end(self.page_index_after_prefix(tail_page.max(0.0)), end);
        }
        let scheduled_pages = self.scheduled_steps.len();
        let index = self.scheduled_origins[1..].partition_point(|origin| *origin < end);
        if index < scheduled_pages {
            return self.correct_page_index_for_end(index as u32, end);
        }
        let scheduled_end = self.scheduled_origins[scheduled_pages];
        let extra =
            ((f64::from(end) - f64::from(scheduled_end)) / f64::from(self.fixed_step)).ceil() - 1.0;
        self.correct_page_index_for_end(self.page_index_after_prefix(extra.max(0.0)), end)
    }

    fn correct_page_index_for_y(&self, page_index: u32, y: f32) -> u32 {
        let page_end = self.page_end(page_index);
        let previous_page_end = page_index
            .checked_sub(1)
            .map(|previous| self.page_end(previous))
            .unwrap_or(f32::NEG_INFINITY);
        if previous_page_end <= y && y < page_end {
            page_index
        } else {
            self.first_page_ending_after(y)
        }
    }

    fn correct_page_index_for_end(&self, page_index: u32, end: f32) -> u32 {
        let page_end = self.page_end_with_rounding_tolerance(page_index);
        let previous_page_end = page_index
            .checked_sub(1)
            .map(|previous| self.page_end_with_rounding_tolerance(previous))
            .unwrap_or(f32::NEG_INFINITY);
        if previous_page_end < end && end <= page_end {
            page_index
        } else {
            self.first_page_ending_at_or_after(end)
        }
    }

    fn page_end(&self, page_index: u32) -> f32 {
        self.origin(page_index.saturating_add(1))
    }

    fn page_end_with_rounding_tolerance(&self, page_index: u32) -> f32 {
        let page_end = self.page_end(page_index);
        let next = page_end.next_up();
        if next - page_end < self.step_at(page_index) {
            next
        } else {
            page_end
        }
    }

    fn first_page_ending_after(&self, y: f32) -> u32 {
        self.first_page_matching_end(y, |origins, page_index, y| origins.page_end(page_index) > y)
    }

    fn first_page_ending_at_or_after(&self, end: f32) -> u32 {
        self.first_page_matching_end(end, |origins, page_index, end| {
            origins.page_end_with_rounding_tolerance(page_index) >= end
        })
    }

    fn first_page_matching_end(
        &self,
        value: f32,
        matches: impl Fn(&Self, u32, f32) -> bool,
    ) -> u32 {
        let mut low = 0_u64;
        let mut high = u64::from(u32::MAX) + 1;
        while low < high {
            let middle = low + (high - low) / 2;
            if matches(self, middle as u32, value) {
                high = middle;
            } else {
                low = middle + 1;
            }
        }
        low.min(u64::from(u32::MAX)) as u32
    }

    fn page_index_after_prefix(&self, tail_page: f64) -> u32 {
        let scheduled_pages = u32::try_from(self.scheduled_steps.len()).unwrap_or(u32::MAX);
        let available = u32::MAX - scheduled_pages;
        if tail_page >= f64::from(available) {
            u32::MAX
        } else {
            scheduled_pages + tail_page as u32
        }
    }
}

/// Layout a document and return neutral per-page fragment snapshots.
///
/// This is the `raikiri-dom` producer-facing geometry API corresponding to
/// fulgur's `PaginationGeometryTable`.  The existing [`layout_pages`] API is
/// unchanged; this convenience wrapper runs it and projects its post-layout
/// DOM coordinates into page-local [`PageFragmentItem`] records.
///
/// The geometry pass clips a box at page boundaries. Shaped text placements
/// additionally carry a neutral line range based on each line's CSS-px center;
/// the pass does not expose the shaping engine or re-shape the text. Fixed-
/// positioned subtrees use their existing post-layout geometry as complete
/// per-page repeat records; table header/footer repetition is not synthesized
/// without corresponding pagination support. The projection is deterministic
/// and keeps source-node identity stable, so a consumer can select
/// continuation lines without re-running pagination.
/// Resolve `ch` lengths on box properties to px in the taffy style, with
/// the fonts the inline engine lays the text out with.
pub(crate) fn prepare_ch_box_values_before_taffy(doc: &mut Document, cascade: &CascadeResult) {
    // `ch` is measured with the inline engine's fonts, the ones the text is
    // laid out with.
    let Some(fonts) = doc.ifc.as_ref().map(|state| state.fonts.clone()) else {
        return;
    };
    for idx in 0..doc.nodes.len() {
        if doc.nodes[idx].kind() != NodeKind::Element {
            continue;
        }
        let cv = &cascade.computed[idx];
        let measure = |provenance: &Option<ChLengthProvenance>| {
            provenance.as_ref().map(|provenance| {
                let used = provenance.factor
                    * crate::layout::ifc::ch::ch_advance(&fonts, &provenance.font);
                if used.is_nan() {
                    0.0
                } else {
                    used.clamp(-MAX_TAFFY_MAGNITUDE, MAX_TAFFY_MAGNITUDE)
                }
            })
        };
        // A box sized by its vertical logical sizes takes their `ch` lengths
        // on the physical axes the layout bridge gave them.
        let (width_ch, height_ch) =
            match crate::layout::bridge::vertical_logical_size(doc, cascade, idx) {
                Some(size) => (&size.width_ch, &size.height_ch),
                None => (&cv.width_ch, &cv.height_ch),
            };
        let width = measure(width_ch).map(|value| value.max(0.0));
        let height = measure(height_ch).map(|value| value.max(0.0));
        let min_width = measure(&cv.min_width_ch).map(|value| value.max(0.0));
        // Use the same normalized logical block axis as the min/max bridge.
        let min_height_ch = if cv.writing_mode == WritingMode::HorizontalTb
            && cv.cssom_writing_mode != WritingMode::HorizontalTb
            && matches!(cv.min_height, ComputedLengthPercentageOrAuto::Auto)
        {
            &cv.min_block_size_ch
        } else {
            &cv.min_height_ch
        };
        let min_height = measure(min_height_ch).map(|value| value.max(0.0));
        let max_width = measure(&cv.max_width_ch).map(|value| value.max(0.0));
        let max_height = measure(&cv.max_height_ch).map(|value| value.max(0.0));
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
        if let Some(width) = min_width {
            style.min_size.width = LengthPercentageAuto::length(width);
        }
        if let Some(height) = min_height {
            style.min_size.height = LengthPercentageAuto::length(height);
        }
        if let Some(width) = max_width {
            style.max_size.width = LengthPercentageAuto::length(width);
        }
        if let Some(height) = max_height {
            style.max_size.height = LengthPercentageAuto::length(height);
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

/// Grow every auto-height ancestor of a left or right float to the float's
/// bottom edge, including the ancestor's bottom border.
///
/// Taffy does not include floated descendants in an auto-height containing
/// block's used height. This keeps following flow from moving upward after a
/// fragmented flex item with a float descendant.
pub(crate) fn propagate_float_bottoms_to_auto_height_ancestors(
    doc: &mut Document,
    cascade: &CascadeResult,
) {
    // Parent map: the arena has no parent pointers, so derive them from children.
    let mut parent_of: Vec<Option<usize>> = vec![None; doc.nodes.len()];
    for idx in 0..doc.nodes.len() {
        for &c in &doc.nodes[idx].children {
            if c < parent_of.len() {
                parent_of[c] = Some(idx);
            }
        }
    }
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

#[cfg(test)]
pub(crate) fn layout_page_fragments(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<Vec<PageFragment>, LayoutError> {
    let slices = layout_pages(document, cascade, page_box)?;
    Ok(page_fragments_from_slices(
        document, cascade, page_box, &slices,
    ))
}

/// Resolve one producer-owned page metadata record from a page cascade.
///
/// The returned `content_box.x/y` is the physical page-local offset for the
/// page's content-relative item rectangles. This helper keeps the conversion
/// in `raikiri-dom` so consumers do not recompute page margins or insets.
pub(crate) fn resolve_page_fragment_geometry(
    cascade: &CascadeResult,
    page_box: PageBox,
    page_index: u32,
) -> PageFragmentPageGeometry {
    let margins = page_margins(cascade, page_box);
    let content_insets = page_content_insets(cascade, page_box);
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height =
        (margins.content_height(page_box) - content_insets.top - content_insets.bottom).max(0.0);
    let content_box = PageFragmentRect::new(
        margins.left + content_insets.left,
        margins.top + content_insets.top,
        content_width,
        content_height,
    );
    let margins = PageFragmentInsets::new(margins.top, margins.right, margins.bottom, margins.left);
    let content_insets = PageFragmentInsets::new(
        content_insets.top,
        content_insets.right,
        content_insets.bottom,
        content_insets.left,
    );
    let orientation = if page_box.width > page_box.height {
        PageFragmentOrientation::Landscape
    } else {
        PageFragmentOrientation::Portrait
    };
    PageFragmentPageGeometry::new(
        page_index,
        page_box,
        margins,
        content_insets,
        content_box,
        orientation,
    )
}

/// Block-start and block-end edges of the lines of a text node, from its own
/// block-start. A text node of an ifc paragraph has no layout of its own, so
/// its lines come from the paragraph root, measured from its first line; any
/// other text node has none.
fn text_line_bounds(document: &Document, node_id: usize) -> Option<Vec<(f32, f32)>> {
    let lines = positioned_text_line_bounds(document, node_id)?;
    let first_top = lines.first().map_or(0.0, |line| line.0);
    Some(
        lines
            .into_iter()
            .map(|(top, bottom)| (top - first_top, bottom - first_top))
            .collect(),
    )
}

/// Block edges of a text node's lines after the paragraph's column ranges
/// have moved them from their unfragmented shaping offsets.
fn positioned_text_line_bounds(document: &Document, node_id: usize) -> Option<Vec<(f32, f32)>> {
    let owned = document.ifc_text_lines(node_id)?;
    let root = document.nodes.get(owned.root)?.ifc.as_ref()?;
    let root_lines = root.lines.as_ref()?;
    let fragments = root.multicol_fragments.as_deref();
    let mut fragment_cursor = 0;
    Some(
        owned
            .lines
            .iter()
            .map(|line| {
                // Both line owners and fragments are ordered by source line.
                let fragment = fragments.and_then(|fragments| {
                    while fragments
                        .get(fragment_cursor)
                        .is_some_and(|fragment| fragment.line_end <= line.line)
                    {
                        fragment_cursor += 1;
                    }
                    fragments.get(fragment_cursor).filter(|fragment| {
                        fragment.line_start <= line.line && line.line < fragment.line_end
                    })
                });
                let offset = fragment
                    .and_then(|fragment| {
                        root_lines
                            .lines
                            .get(fragment.line_start)
                            .map(|first| fragment.y - first.block_offset())
                    })
                    .unwrap_or(0.0);
                (line.top + offset, line.bottom + offset)
            })
            .collect(),
    )
}

/// Project an already-paginated document using one fixed geometry for all pages.
///
/// This compatibility entry point remains valid for fixed-page callers. New
/// page-aware callers should use [`page_fragments_from_slices_with_page_geometry`]
/// so each page carries its producer-resolved metadata.
#[cfg(test)]
pub(crate) fn page_fragments_from_slices(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
) -> Vec<PageFragment> {
    let geometries: Vec<_> = slices
        .iter()
        .map(|slice| resolve_page_fragment_geometry(cascade, page_box, slice.page_index))
        .collect();
    page_fragments_from_slices_with_page_geometry(document, cascade, page_box, slices, &geometries)
}

/// The page placements of [`project_slices`].
#[cfg(test)]
pub(crate) fn page_fragments_from_slices_with_page_geometry(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
) -> Vec<PageFragment> {
    project_slices(document, cascade, page_box, slices, page_geometries).0
}

/// Project slices using one resolved geometry record for each page, and
/// also return the paragraphs laid out by the inline engine, in document
/// order, at their content-box origins in the shared flow space.
///
/// `page_geometries` is producer-owned resolved metadata. A missing page index
/// falls back to `page_box` and the supplied cascade for compatibility, but a
/// page-aware caller should provide every emitted page explicitly. Item
/// rectangles remain relative to each page's `content_box` origin; consumers
/// add `content_box.x/y` exactly once when placing them on the physical page.
pub(crate) fn project_slices(
    document: &Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    slices: &[PageSlice],
    page_geometries: &[PageFragmentPageGeometry],
) -> (Vec<PageFragment>, Vec<ProjectedTextRoot>) {
    let fallback_geometry = resolve_page_fragment_geometry(cascade, page_box, 0);
    let mut ordered_slices: Vec<&PageSlice> = slices.iter().collect();
    ordered_slices.sort_by(|left, right| {
        left.page_index
            .cmp(&right.page_index)
            .then_with(|| left.content_origin_y.total_cmp(&right.content_origin_y))
    });
    let mut pages: Vec<PageFragment> = ordered_slices
        .iter()
        .map(|slice| {
            let geometry = page_geometries
                .iter()
                .find(|geometry| geometry.page_index == slice.page_index)
                .copied()
                .unwrap_or_else(|| fallback_geometry.with_page_index(slice.page_index));
            PageFragment::with_page_geometry(
                slice.page_index,
                geometry,
                slice.content_origin_y,
                slice.page_name.clone(),
            )
        })
        .collect();

    // Each page's slice of the shared flow space: from its origin to the next
    // page's origin, or its content height for the last page.
    for (page_slot, page) in pages.iter_mut().enumerate() {
        let page_start = page.content_origin_y;
        let page_end = ordered_slices
            .get(page_slot + 1)
            .map(|next| next.content_origin_y)
            .filter(|next| next.is_finite() && *next > page_start)
            .unwrap_or(page_start + page.content_box.height);
        page.flow_range = (page_start.is_finite() && page_end.is_finite() && page_end > page_start)
            .then_some((page_start, page_end));
    }

    let mut text_roots = Vec::new();
    let Some(body_id) = find_body(document) else {
        return (pages, text_roots);
    };

    // Collect absolute post-pagination coordinates.  The arena index is the
    // stable NodeId projection used by `raikiri_traits::Dom`; sorting by it
    // reproduces fulgur's deterministic BTreeMap iteration order regardless of
    // traversal implementation details.
    struct PageFragmentSource {
        node_id: NodeId,
        node_kind: NodeKind,
        tag_name: Option<String>,
        abs_x: f32,
        abs_y: f32,
        width: f32,
        height: f32,
        content_insets: Option<PageFragmentInsets>,
        line_metrics: Option<Vec<(f32, f32)>>,
        is_repeat: bool,
    }

    let mut nodes = Vec::new();
    let mut ifc_origin: HashMap<usize, (f32, f32)> = HashMap::new();
    // Each inline element's rectangle is the union of its pieces after
    // pagination moves their lines.
    let mut ifc_piece_bounds: HashMap<usize, HashMap<usize, BoxRect>> = HashMap::new();
    let mut fragmented_inline_nodes = HashSet::new();
    let mut ifc_text_lines = HashMap::new();
    let mut stack = vec![(body_id, 0.0_f32, 0.0_f32, false)];
    while let Some((node_id, parent_abs_x, parent_abs_y, inherited_repeat)) = stack.pop() {
        let Some(node) = document.get_node(node_id) else {
            continue; // cov:ignore: document-owned child links are valid by construction.
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let layout = node.unrounded_layout;
        let abs_x = parent_abs_x + layout.location.x;
        let abs_y = parent_abs_y + layout.location.y;
        let width = finite_nonnegative(layout.size.width);
        let height = finite_nonnegative(layout.size.height);
        // A fixed-position subtree is painted in every committed page. The
        // existing layout pass already computes one viewport-relative box;
        // preserve that geometry and mark the records as complete repeats
        // instead of clipping them as in-flow content.
        let is_repeat = inherited_repeat
            || matches!(
                cascade
                    .computed
                    .get(node_id)
                    .map(|computed| &computed.position),
                Some(PositionValue::Fixed)
            );
        let include = match node.kind() {
            NodeKind::Text => node
                .text_content()
                .is_some_and(|text| !text.trim().is_empty()),
            NodeKind::Element => true,
            _ => false, // cov:ignore: non-rendered node kinds are filtered by the document invariant.
        };
        if node.is_ifc_root() {
            // The content-box origin of a paragraph laid out by the inline
            // engine: its text nodes have no layout of their own, and their
            // lines are measured from here. A root is visited before its text.
            let origin = (
                abs_x + layout.border.left + layout.padding.left,
                abs_y + layout.border.top + layout.padding.top,
            );
            ifc_origin.insert(node_id, origin);
            if origin.0.is_finite() && origin.1.is_finite() {
                text_roots.push(ProjectedTextRoot {
                    node: node_id,
                    x: origin.0,
                    y: origin.1,
                    is_repeat,
                });
            }
            let mut bounds_by_node: HashMap<usize, BoxRect> = HashMap::new();
            for piece in node.ifc_inline_boxes().unwrap_or_default() {
                bounds_by_node
                    .entry(piece.node)
                    .and_modify(|bounds| {
                        fragmented_inline_nodes.insert(piece.node);
                        let rect = piece.border_box;
                        let x = bounds.x.min(rect.x);
                        let y = bounds.y.min(rect.y);
                        *bounds = BoxRect {
                            x,
                            y,
                            width: (bounds.x + bounds.width).max(rect.x + rect.width) - x,
                            height: (bounds.y + bounds.height).max(rect.y + rect.height) - y,
                        };
                    })
                    .or_insert(piece.border_box);
            }
            ifc_piece_bounds.insert(node_id, bounds_by_node);
            ifc_text_lines.extend(document.ifc_text_lines_by_node(node_id));
        }
        // An inline element of an inline engine paragraph is where its pieces
        // are on the lines; its recorded location is relative to its nearest
        // inline ancestor and does not follow lines that pagination moved.
        let (abs_x, abs_y, width, height) = if node.kind() == NodeKind::Element
            && node.in_ifc_subtree()
            && let Some(root) = document.ifc_root_of(node_id)
            && let (Some(&(root_x, root_y)), Some(bounds_by_node)) =
                (ifc_origin.get(&root), ifc_piece_bounds.get(&root))
            && let Some(rect) = bounds_by_node.get(&node_id)
        {
            (
                root_x + rect.x,
                root_y + rect.y,
                finite_nonnegative(rect.width),
                finite_nonnegative(rect.height),
            )
        } else {
            (abs_x, abs_y, width, height)
        };
        if include && abs_x.is_finite() && abs_y.is_finite() {
            let ifc_lines = (node.kind() == NodeKind::Text)
                .then(|| ifc_text_lines.remove(&node_id))
                .flatten();
            let (abs_x, abs_y, width, height, line_metrics) = match ifc_lines {
                // A text node of an ifc paragraph starts at the first line it
                // owns, in the root's content box; its line metrics are
                // measured from that line.
                Some(owned) => {
                    let (root_x, root_y) = ifc_origin
                        .get(&owned.root)
                        .copied()
                        .unwrap_or((abs_x, abs_y));
                    let first_top = owned.lines.first().map_or(0.0, |l| l.top);
                    let last_bottom = owned.lines.last().map_or(0.0, |l| l.bottom);
                    let metrics: Vec<(f32, f32)> = owned
                        .lines
                        .iter()
                        .map(|l| (l.top - first_top, l.bottom - first_top))
                        .collect();
                    (
                        root_x,
                        root_y + first_top,
                        finite_nonnegative(owned.width),
                        finite_nonnegative(last_bottom - first_top),
                        Some(metrics),
                    )
                }
                None => (
                    abs_x,
                    abs_y,
                    width,
                    height,
                    // A text node outside every paragraph has no lines.
                    (node.kind() == NodeKind::Text).then(Vec::new),
                ),
            };
            nodes.push(PageFragmentSource {
                node_id: NodeId::new(node_id as u64),
                node_kind: node.kind(),
                tag_name: node.tag_name().map(str::to_owned),
                abs_x: abs_x + node.table_grid_box.map_or(0.0, |rect| rect.x),
                abs_y: abs_y + node.table_grid_box.map_or(0.0, |rect| rect.y),
                width: node.table_grid_box.map_or(width, |rect| rect.width),
                height: node.table_grid_box.map_or(height, |rect| rect.height),
                content_insets: (node.kind() == NodeKind::Element
                    && !fragmented_inline_nodes.contains(&node_id))
                .then(|| {
                    PageFragmentInsets::new(
                        layout.border.top + layout.padding.top,
                        layout.border.right + layout.padding.right,
                        layout.border.bottom + layout.padding.bottom,
                        layout.border.left + layout.padding.left,
                    )
                }),
                line_metrics,
                is_repeat,
            });
        } // cov:ignore: layout sanitization normally keeps source coordinates finite.

        if node.kind() == NodeKind::Element {
            // The children of an inline element of an inline engine paragraph
            // are located from the paragraph's root, not from the element.
            let (base_x, base_y) = if document.contributes_layout_offset(node_id) {
                (abs_x, abs_y)
            } else {
                (parent_abs_x, parent_abs_y)
            };
            for &child_id in node.children.iter().rev() {
                stack.push((child_id, base_x, base_y, is_repeat));
            }
        }
    }
    nodes.sort_by_key(|node| node.node_id);

    for source in nodes {
        let kind = match source.node_kind {
            NodeKind::Text => PageFragmentKind::Text,
            NodeKind::Element if source.tag_name.as_deref() == Some("img") => {
                PageFragmentKind::Replaced
            }
            _ => PageFragmentKind::Box,
        };
        let mut placements = Vec::new();
        let repeat_line_range = source.line_metrics.as_deref().and_then(|metrics| {
            (!metrics.is_empty()).then(|| {
                PageFragmentLineRange::new(0, u32::try_from(metrics.len()).unwrap_or(u32::MAX))
            })
        });
        for (page_slot, page) in pages.iter().enumerate() {
            let Some((page_start, page_end)) = page.flow_range else {
                continue;
            };
            if source.is_repeat {
                placements.push((
                    page_slot,
                    source.abs_y,
                    source.height,
                    repeat_line_range,
                    source.abs_y,
                ));
                continue;
            }
            let (intersects, fragment_y, fragment_height) = if source.height > 0.0 {
                let bottom = source.abs_y + source.height;
                let intersects = source.abs_y < page_end && bottom > page_start;
                let top = source.abs_y.max(page_start);
                let bottom = bottom.min(page_end);
                (
                    intersects,
                    (top - page_start).max(0.0),
                    (bottom - top).max(0.0),
                )
            } else {
                (
                    source.abs_y >= page_start && source.abs_y <= page_end,
                    (source.abs_y - page_start).max(0.0),
                    0.0,
                )
            };
            if intersects {
                let line_range = source.line_metrics.as_deref().and_then(|metrics| {
                    line_range_for_page(metrics, source.abs_y, page_start, page_end)
                });
                placements.push((
                    page_slot,
                    fragment_y,
                    fragment_height,
                    line_range,
                    source.abs_y - page_start,
                ));
            }
        }
        let fragment_count = placements.len() as u32;
        for (fragment_index, (page_slot, y, fragment_height, line_range, box_y)) in
            placements.into_iter().enumerate()
        {
            let Some(page) = pages.get_mut(page_slot) else {
                continue; // cov:ignore: placements are indexed from the same slices used to build pages.
            };
            let item = PageFragmentItem::new(
                source.node_id,
                PageFragmentRect::new(source.abs_x, y, source.width, fragment_height),
                kind,
                fragment_index as u32,
                fragment_count,
                source.is_repeat,
            )
            .with_page_index(page.page_index)
            .with_box_extent(box_y, source.height)
            .with_content_rect(source.content_insets.map(|insets| {
                PageFragmentRect::new(
                    source.abs_x + insets.left,
                    box_y + insets.top,
                    (source.width - insets.left - insets.right).max(0.0),
                    (source.height - insets.top - insets.bottom).max(0.0),
                )
            }));
            page.items.push(match line_range {
                Some(range) => item.with_line_range(range),
                None => item,
            });
        }
    }

    (pages, text_roots)
}

/// Collect deterministic page-local link events from page snapshots.
///
/// The event geometry is copied from the correlated `PageFragmentItem`, so a
/// consumer can join on `(placement_node_id, page_index)` without seeing the
/// layout engine's internal tree. `anchor_node_id` preserves the owning `<a>`
/// identity when a link wraps text or replaced descendants. Raw trimmed
/// `href` values, including empty and relative values, are not URL-parsed.
/// A box ancestor is omitted when a more specific text or replaced placement
/// already represents the same link, retaining one useful hit rectangle per
/// visible leaf and a fallback for empty/non-text links.
pub(crate) fn page_fragment_events_from_pages(
    document: &Document,
    pages: &[PageFragment],
) -> Vec<PageFragmentEvent> {
    let Some(body_id) = find_body(document) else {
        return Vec::new();
    };

    let mut parent_by_node = HashMap::new();
    for (parent_id, node) in document.nodes.iter().enumerate() {
        for &child_id in &node.children {
            parent_by_node.insert(child_id, parent_id);
        }
    }

    let mut owners = HashMap::<usize, usize>::new();
    let mut hrefs = HashMap::<usize, String>::new();
    let mut stack = vec![(body_id, None::<usize>)];
    while let Some((node_id, inherited_owner)) = stack.pop() {
        let node = &document.nodes[node_id];
        if !node.is_in_document() || node.is_non_rendered_html_element() || node.is_display_none() {
            continue;
        }
        let owner = if node.kind() == NodeKind::Element && node.tag_name() == Some("a") {
            node.attribute("href")
                .map(str::trim)
                .map(|href| {
                    hrefs.entry(node_id).or_insert_with(|| href.to_owned());
                    node_id
                })
                .or(inherited_owner)
        } else {
            inherited_owner
        };
        if let Some(owner) = owner {
            owners.insert(node_id, owner);
        }
        for &child_id in node.children.iter().rev() {
            stack.push((child_id, owner));
        }
    }

    let mut events = Vec::new();
    for page in pages {
        let linked_items: Vec<&PageFragmentItem> = page
            .items
            .iter()
            .filter(|item| {
                usize::try_from(item.node_id.0)
                    .ok()
                    .and_then(|node_id| owners.get(&node_id))
                    .and_then(|owner| hrefs.get(owner))
                    .is_some()
                    && item.rect.width > 0.0
                    && item.rect.height > 0.0
            })
            .collect();
        for item in linked_items.iter().copied() {
            // `linked_items` has already validated all three lookups above.
            // Keeping the invariant explicit here avoids a second set of
            // impossible branches in the hot event projection loop.
            let placement_node_id = usize::try_from(item.node_id.0)
                .expect("linked item NodeId must fit the local arena index");
            let anchor_id = *owners
                .get(&placement_node_id)
                .expect("linked item must have an anchor owner");
            let href = hrefs
                .get(&anchor_id)
                .expect("anchor owner must retain its href");
            if item.kind == PageFragmentKind::Box
                && linked_items.iter().any(|other| {
                    let other_node_id = usize::try_from(other.node_id.0)
                        .expect("linked item NodeId must fit the local arena index");
                    other.node_id != item.node_id
                        && owners.get(&other_node_id) == Some(&anchor_id)
                        && is_descendant_of(other_node_id, placement_node_id, &parent_by_node)
                })
            {
                continue;
            }
            events.push(PageFragmentEvent::Link(PageFragmentLinkEvent::new(
                NodeId::new(anchor_id as u64),
                item.node_id,
                page.page_index,
                item.rect,
                item.fragment_index,
                item.fragment_count,
                item.is_repeat,
                item.line_range,
                PageFragmentLink::new(href.clone()),
            )));
        }
    }
    events.sort_by(|left, right| match (left, right) {
        (PageFragmentEvent::Link(left), PageFragmentEvent::Link(right)) => left
            .page_index
            .cmp(&right.page_index)
            .then_with(|| left.anchor_node_id.cmp(&right.anchor_node_id))
            .then_with(|| left.placement_node_id.cmp(&right.placement_node_id))
            .then_with(|| left.rect.y.total_cmp(&right.rect.y))
            .then_with(|| left.rect.x.total_cmp(&right.rect.x))
            .then_with(|| left.fragment_index.cmp(&right.fragment_index)),
    });
    events
}

fn is_descendant_of(
    mut candidate: usize,
    ancestor: usize,
    parent_by_node: &HashMap<usize, usize>,
) -> bool {
    while let Some(parent) = parent_by_node.get(&candidate).copied() {
        if parent == ancestor {
            return true;
        }
        candidate = parent;
    }
    false
}

/// Group page snapshots into a deterministic NodeId-ordered geometry table.
///
/// `pages` is normally the ordered snapshot returned by
/// [`layout_page_fragments`] or [`page_fragments_from_slices`]. The result
/// normalizes each item to its containing page and sorts node fragments by
/// page/fragment index, while preserving the producer's `is_repeat` flag.
#[cfg(test)]
pub(crate) fn page_fragment_geometry_table(pages: &[PageFragment]) -> PageFragmentGeometryTable {
    let mut table = PageFragmentGeometryTable::new();
    for page in pages {
        for item in &page.items {
            // The page snapshot is authoritative for this placement. Normalize
            // manually assembled snapshots as well as producer output so the
            // node-centric table always retains the page index required by a
            // fulgur-compatible consumer.
            let item = item.clone().with_page_index(page.page_index);
            let geometry = table
                .entry(item.node_id)
                .or_insert_with(|| PageFragmentGeometry::new(item.node_id, item.is_repeat));
            debug_assert_eq!(geometry.is_repeat, item.is_repeat); // cov:ignore: one producer cannot mix split and repeat records.
            geometry.fragments.push(item);
        }
    }
    for geometry in table.values_mut() {
        geometry
            .fragments
            .sort_by_key(|item| (item.page_index, item.fragment_index));
    }
    table
}

fn line_range_for_page(
    line_metrics: &[(f32, f32)],
    text_abs_y: f32,
    page_start: f32,
    page_end: f32,
) -> Option<PageFragmentLineRange> {
    let mut first = None;
    let mut end = 0_u32;
    for (index, (line_top, line_bottom)) in line_metrics.iter().copied().enumerate() {
        let top = text_abs_y + line_top;
        let bottom = text_abs_y + line_bottom;
        if !line_center_on_page(top, bottom, page_start, page_end) {
            continue;
        }
        let index = index as u32;
        first.get_or_insert(index);
        end = index.saturating_add(1);
    }
    first.map(|start| PageFragmentLineRange::new(start, end))
}

/// Whether a line from `top` to `bottom` in the shared flow space belongs to
/// the page whose slice runs from `page_start` to `page_end`: the page that
/// holds the line's center. A line therefore belongs to exactly one page.
pub(crate) fn line_center_on_page(top: f32, bottom: f32, page_start: f32, page_end: f32) -> bool {
    let center = (top + bottom) * 0.5;
    center.is_finite() && center >= page_start - 0.001 && center < page_end - 0.001
}

fn finite_nonnegative(value: f32) -> f32 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

pub(super) fn page_break_is_forced(value: BreakBetween) -> bool {
    matches!(value, BreakBetween::Page)
}

pub(crate) fn selected_page_name(cascade: &CascadeResult, node_id: usize) -> Option<String> {
    match cascade.page_values.get(node_id) {
        Some(raikiri_style::property::PageValue::Named(name)) => Some(name.to_string()),
        _ => None,
    }
}

/// Layout a document once and produce ordered page slices.
///
/// The current fragmentainer handles ordinary block-flow children of
/// `<body>`.  It honors forced `page` breaks (including the legacy aliases
/// already normalized by the style cascade), coalesces adjacent forced
/// breaks, and keeps an empty explicitly-created page.  A block that is taller
/// than a page is exposed on each intersecting slice; splitting its internal
/// line/child fragments is deliberately left to the next fragmentation pass.
/// Direct tables additionally keep a row whose cell boxes would cross a page
/// together by moving that row to the next fragmentainer. Rowspans, repeated
/// header/footer groups, and cell-internal breaks remain outside this pass.
/// Column-direction flex containers likewise keep a direct flex item together;
/// row-direction flex, wrapping-line, and intrinsic-item fragmentation remain
/// outside this pass. A single explicit-column grid likewise keeps direct
/// non-spanning items together; multi-column placement and spanning items are
/// outside this pass.
/// For an ordinary direct-body block whose complete text layout fits within one
/// fragmentainer, `orphans` / `widows` can move the block intact when the
/// natural split would leave too few line boxes on either side. Oversized or
/// nested inline formatting contexts remain outside this pass.
///
/// This is intentionally separate from [`layout_single_page`]: existing
/// callers retain the single-page contract while paged callers get a real
/// per-page result and the same post-layout DOM as the scene builder.
pub fn layout_pages(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_steps(document, cascade, page_box, &[])
}

/// Like [`layout_pages`], but first use `resolver` to resolve the intrinsic
/// size of `<img>` elements. The result is stored in the same `Document`,
/// so normal layout after pagination and the paint pixel source can refer
/// to the same image.
pub fn layout_pages_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_resolver_and_base_url(document, cascade, page_box, resolver, None)
}

/// [`layout_pages_with_resolver`] with relative image URLs resolved against a
/// document base URL.
pub fn layout_pages_with_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_resolver_and_base_url_and_control(
        document,
        cascade,
        page_box,
        resolver,
        base_url,
        &PageLayoutControl::default(),
    )
}

/// [`layout_pages_with_resolver_and_base_url`] with an explicit page limit
/// and cancellation check.
pub fn layout_pages_with_resolver_and_base_url_and_control(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    control.check_page_index(0)?;
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    document.mark_in_document_flags();
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)?;
    layout_pages_with_page_geometry_and_control(document, cascade, page_box, &[], &[], control)
}

/// Like [`layout_pages_with_page_geometry`], but first use `resolver` to
/// resolve the intrinsic sizes of `<img>` elements.
pub fn layout_pages_with_page_geometry_and_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_resolver_and_base_url(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        resolver,
        None,
    )
}

/// [`layout_pages_with_page_geometry_and_resolver`] with document-relative image URLs.
#[allow(clippy::too_many_arguments)]
pub fn layout_pages_with_page_geometry_and_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        resolver,
        base_url,
        &PageLayoutControl::default(),
    )
}

/// [`layout_pages_with_page_geometry_and_resolver_and_base_url`] with an
/// explicit page limit and cancellation check.
#[allow(clippy::too_many_arguments)]
pub fn layout_pages_with_page_geometry_and_resolver_and_base_url_and_control(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    control.check_page_index(0)?;
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    document.mark_in_document_flags();
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)?;
    layout_pages_with_page_geometry_and_control(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        control,
    )
}

/// Layout ordinary block flow with an optional per-page content-height schedule.
///
/// An empty schedule preserves the historical fixed fragmentainer height.  The
/// paged WPT adapter supplies a schedule when `@page` changes the page size or
/// margins after the first page; the normal API remains fixed-size by default.
pub fn layout_pages_with_page_steps(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry(document, cascade, page_box, page_steps, &[])
}

/// Layout ordinary block flow with per-page content heights and widths.
///
/// `page_widths` contains content-box widths corresponding to `page_steps`.
/// When present, percentage-sized boxes are rescaled after pagination so a
/// later page with a different containing-block width does not retain the
/// first page's used percentage width.  An empty width schedule keeps the
/// existing fixed-page behavior.
pub fn layout_pages_with_page_geometry(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
) -> Result<Vec<PageSlice>, LayoutError> {
    layout_pages_with_page_geometry_and_control(
        document,
        cascade,
        page_box,
        page_steps,
        page_widths,
        &PageLayoutControl::default(),
    )
}

/// [`layout_pages_with_page_geometry`] with an explicit page limit and
/// cancellation check.
pub fn layout_pages_with_page_geometry_and_control(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    page_steps: &[f32],
    page_widths: &[f32],
    control: &PageLayoutControl<'_>,
) -> Result<Vec<PageSlice>, LayoutError> {
    control.page_limit_reached.set(false);
    control.check_page_index(0)?;
    layout_single_page(document, cascade, page_box)?;
    control.check_page_index(0)?;

    let body_id = find_body(document).ok_or_else(|| LayoutError::Internal {
        message: "no <body> element found (fragment parse not supported yet)".to_string(),
    })?;
    let margins = page_margins(cascade, page_box);
    let insets = page_content_insets(cascade, page_box);
    // The body root sits at the top of the page content box, so the html
    // element's block-start margin is not represented in descendant
    // coordinates; carry it into the initial fragmentainer cursor. Margins of
    // the root element's box do not collapse (CSS 2.1 §8.3.1), so it simply
    // adds to the body's used block-start margin, which layout already
    // placed inside the body root.
    let html_margin_top = document.nodes[document.root]
        .children
        .iter()
        .copied()
        .find(|&node_id| document.nodes[node_id].tag_name() == Some("html"))
        .and_then(|html_id| {
            used_style_length_percentage_auto(
                document.nodes[html_id].style.margin.top,
                margins.content_width(page_box),
            )
            .or_else(|| {
                used_computed_length_percentage_or_auto(
                    cascade.computed[html_id].margin.top,
                    margins.content_width(page_box),
                )
            })
            .map(|value| value.max(0.0))
        })
        .unwrap_or(0.0);
    let body_margin_top = document.body_block_start_margin;
    let root_margin_top = html_margin_top + body_margin_top;
    // Keep the scheduled inline size identical to the first layout pass;
    // page border/padding are applied as a paint offset, not as a narrower
    // containing block.
    let content_width = margins.content_width(page_box).max(0.0);
    let content_height = (margins.content_height(page_box) - insets.top - insets.bottom).max(0.0);
    // A page with margins consuming the entire paper still needs a finite
    // cursor for forced breaks.  No valid page box reaches this path in normal
    // CSS, but the fallback keeps the API panic-free for direct callers.
    let fixed_page_step = if content_height.is_finite() && content_height > 0.0 {
        content_height
    } else {
        page_box.height.max(1.0)
    };
    // A page-size/margin change can make the fragmentainer height vary by
    // page.  The default API supplies no schedule and therefore follows the
    // fixed first-page height used historically.
    let page_step = fixed_page_step;
    let page_origins = PageOrigins::new(page_steps, fixed_page_step);
    let page_step_at = |page_index: u32| page_origins.step_at(page_index);
    let page_origin = |page_index: u32| page_origins.origin(page_index);
    let page_index_for_y = |y: f32| page_origins.page_index_for_y(y);
    let page_index_for_end = |end: f32| page_origins.page_index_for_end(end);
    // A root margin that reaches into a later fragmentainer consumes whole
    // leading pages.  Do not leave the first content run stranded halfway
    // down the first nonblank page.
    // The body's part of that margin is already in the layout coordinates,
    // so only the rest is added to the content.
    let root_flow_offset = if root_margin_top >= fixed_page_step {
        let page_index = page_index_for_y(root_margin_top);
        control.check_discovery_page_index(page_index)?;
        page_origin(page_index) - body_margin_top
    } else {
        html_margin_top
    };

    // Pagination adjusts selected boxes after taffy has produced one normal
    // flow layout.  Keep the arena's parent relation so a descendant can be
    // materialized relative to an already-shifted ancestor instead of being
    // shifted twice.
    let mut parent_of = vec![None; document.nodes.len()];
    for (parent_id, node) in document.nodes.iter().enumerate() {
        for &child_id in &node.children {
            if child_id < parent_of.len() {
                parent_of[child_id] = Some(parent_id);
            }
        }
    }

    fn current_abs_y(document: &Document, node_id: usize, parent_of: &[Option<usize>]) -> f32 {
        let mut id = node_id;
        let mut y = 0.0_f32;
        let mut guard = 0_usize;
        while guard <= parent_of.len() {
            y += document.nodes[id].unrounded_layout.location.y;
            // A box of an inline engine paragraph is located from the root,
            // not from the inline elements around it.
            let Some(parent_id) = parent_of[id].and_then(|_| document.layout_parent_of(id)) else {
                break;
            };
            id = parent_id;
            guard += 1;
            if id >= document.nodes.len() {
                break;
            }
        }
        y
    }

    fn materialize_y(
        document: &mut Document,
        node_id: usize,
        desired_y: f32,
        parent_of: &[Option<usize>],
    ) {
        if !desired_y.is_finite() {
            return;
        }
        let actual_y = current_abs_y(document, node_id, parent_of);
        let delta = desired_y - actual_y;
        if delta.is_finite() {
            document.nodes[node_id].unrounded_layout.location.y += delta;
            follow_moved_ifc_block(document, node_id, delta);
        }
    }

    /// The lines of the text `text` of the paragraph `root` start a later
    /// page: they, the lines after them and the paragraph's boxes below their
    /// top move down by `delta`.
    fn follow_moved_ifc_text(document: &mut Document, root: usize, text: usize, delta: f32) {
        if delta == 0.0 || !delta.is_finite() {
            return;
        }
        let Some(first) = document
            .ifc_text_lines(text)
            .and_then(|owned| owned.lines.first().copied())
        else {
            return;
        };
        let first_top = positioned_text_line_bounds(document, text)
            .and_then(|lines| lines.first().map(|line| line.0))
            .unwrap_or(first.top);
        let layout = document.nodes[root].unrounded_layout;
        let old_top = layout.border.top + layout.padding.top + first_top;
        let Some(ifc) = document.nodes[root].ifc.as_mut() else {
            return;
        };
        let Some(lines) = ifc.lines.as_mut() else {
            return;
        };
        lines.shifts.push((first.line, delta));
        let boxes: Vec<usize> = ifc.boxes.iter().map(|b| b.node).collect();
        for b in boxes {
            let location = &mut document.nodes[b].unrounded_layout.location;
            if location.y >= old_top - 0.01 {
                location.y += delta;
            }
        }
    }

    /// A block child of a paragraph laid out by the inline engine was moved
    /// by `delta` (to a later page): the lines after it and the paragraph's
    /// boxes below it move with it, as the content that follows a block
    /// follows it in the block's formatting context. Lines and boxes above
    /// it stay where they are.
    fn follow_moved_ifc_block(document: &mut Document, node_id: usize, delta: f32) {
        if delta == 0.0 || !delta.is_finite() || document.nodes[node_id].in_ifc_subtree() {
            return;
        }
        let Some(root) = document.layout_parent_of(node_id) else {
            return;
        };
        let old_bottom = document.nodes[node_id].unrounded_layout.location.y - delta
            + document.nodes[node_id].unrounded_layout.size.height;
        let Some(ifc) = document.nodes[root].ifc.as_mut() else {
            return;
        };
        let Some(lines) = ifc.lines.as_mut() else {
            return;
        };
        let Some(&(_, first_after)) = lines
            .block_line_starts
            .iter()
            .find(|(block, _)| *block == node_id)
        else {
            return;
        };
        lines.shifts.push((first_after, delta));
        let boxes: Vec<usize> = ifc
            .boxes
            .iter()
            .map(|b| b.node)
            .filter(|&b| b != node_id)
            .collect();
        for b in boxes {
            let location = &mut document.nodes[b].unrounded_layout.location;
            if location.y >= old_bottom - 0.01 {
                location.y += delta;
            }
        }
    }

    fn is_descendant_or_self(
        document: &Document,
        node_id: usize,
        ancestor_id: usize,
        parent_of: &[Option<usize>],
    ) -> bool {
        let mut current = Some(node_id);
        let mut guard = 0_usize;
        while let Some(id) = current {
            if id == ancestor_id {
                return true;
            }
            if id >= document.nodes.len() || guard > parent_of.len() {
                break;
            }
            current = parent_of[id];
            guard += 1;
        }
        false
    }

    fn has_footnote_ancestor(
        cascade: &CascadeResult,
        node_id: usize,
        parent_of: &[Option<usize>],
    ) -> bool {
        let mut current = parent_of.get(node_id).copied().flatten();
        let mut guard = 0_usize;
        while let Some(id) = current {
            if cascade
                .computed
                .get(id)
                .is_some_and(|computed| matches!(computed.float, FloatValue::Footnote))
            {
                return true;
            }
            current = parent_of.get(id).copied().flatten();
            guard += 1;
            if guard > parent_of.len() {
                break;
            }
        }
        false
    }

    #[derive(Clone)]
    struct PageCandidate {
        node_id: usize,
        raw_y: f32,
        height: f32,
        is_text: bool,
        is_rendered_leaf: bool,
        is_direct_body_element: bool,
        is_table_row: bool,
        is_flex_item: bool,
        is_grid_item: bool,
        is_named: bool,
        is_float_descendant: bool,
        is_out_of_flow_descendant: bool,
        /// Page type inherited from the nearest containing class-A box.
        /// `None` is the anonymous page type, not an unknown value.
        page_name: Option<String>,
        /// Column-flex descendants compare names within their containing flow.
        named_flex_context: Option<usize>,
        /// An inline canvas with a named page is a boundary marker, but its
        /// inline-level box does not itself establish the named page type.
        inline_named_page: bool,
        deferred_named_break_after: bool,
        /// For a text node of an ifc paragraph: the paragraph root and the
        /// text's offset below the root's border-box top. The root's lines
        /// are painted from the root, so the text moves by moving the root.
        ifc_root: Option<(usize, f32)>,
    }

    fn has_nested_named_page_descendant(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        depth: u32,
    ) -> bool {
        let mut pending: Vec<_> = document
            .get_node(node_id)
            .into_iter()
            .flat_map(|node| node.children.iter().rev().copied())
            .map(|child_id| (child_id, depth.saturating_add(1)))
            .collect();
        while let Some((current_id, current_depth)) = pending.pop() {
            let node = &document.nodes[current_id];
            let computed = &cascade.computed[current_id];
            if !node.is_in_document()
                || node.is_non_rendered_html_element()
                || computed.display == DisplayValue::None
                || matches!(
                    computed.position,
                    PositionValue::Absolute | PositionValue::Fixed
                )
                || is_floating_box_for_pagination(document, cascade, current_id)
            {
                continue;
            }
            if current_depth >= 2 && selected_page_name(cascade, current_id).is_some() {
                return true;
            }
            if let Some(node) = document.get_node(current_id) {
                pending.extend(
                    node.children
                        .iter()
                        .rev()
                        .copied()
                        .map(|child_id| (child_id, current_depth.saturating_add(1))),
                );
            }
        }
        false
    }

    /// A named box containing a hidden subtree still establishes an explicit
    /// page boundary. Keep that boundary distinct from the zero-height named
    /// runs that are otherwise coalesced at one source coordinate.
    fn has_display_none_descendant(document: &Document, node_id: usize) -> bool {
        let mut pending: Vec<_> = document
            .get_node(node_id)
            .into_iter()
            .flat_map(|node| node.children.iter().rev().copied())
            .collect();
        while let Some(current_id) = pending.pop() {
            let Some(node) = document.get_node(current_id) else {
                continue;
            };
            if node.is_display_none() {
                return true;
            }
            pending.extend(node.children.iter().rev().copied());
        }
        false
    }

    // Candidate collection carries the recursive layout state explicitly so page
    // membership is decided from source coordinates before local page offsets.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn collect_candidates(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        parent_abs_y: f32,
        direct_body_child: bool,
        body_id: usize,
        parent_height: f32,
        page_step: f32,
        inherited_page_name: Option<String>,
        inside_table: bool,
        flex_column_parent: bool,
        grid_single_column_parent: bool,
        inside_flex: bool,
        named_flex_context: Option<usize>,
        inside_float: bool,
        inside_out_of_flow: bool,
        out: &mut Vec<PageCandidate>,
        control: &PageLayoutControl<'_>,
    ) -> Result<(), LayoutError> {
        stacker::maybe_grow(128 * 1024, 1024 * 1024, || {
            collect_candidates_inner(
                document,
                cascade,
                node_id,
                parent_abs_y,
                direct_body_child,
                body_id,
                parent_height,
                page_step,
                inherited_page_name,
                inside_table,
                flex_column_parent,
                grid_single_column_parent,
                inside_flex,
                named_flex_context,
                inside_float,
                inside_out_of_flow,
                out,
                control,
            )
        })
    }

    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn collect_candidates_inner(
        document: &Document,
        cascade: &CascadeResult,
        node_id: usize,
        parent_abs_y: f32,
        direct_body_child: bool,
        body_id: usize,
        parent_height: f32,
        page_step: f32,
        inherited_page_name: Option<String>,
        inside_table: bool,
        flex_column_parent: bool,
        grid_single_column_parent: bool,
        inside_flex: bool,
        named_flex_context: Option<usize>,
        inside_float: bool,
        inside_out_of_flow: bool,
        out: &mut Vec<PageCandidate>,
        control: &PageLayoutControl<'_>,
    ) -> Result<(), LayoutError> {
        control.check_aborted()?;
        let Some(node) = document.get_node(node_id) else {
            return Ok(()); // cov:ignore: traversal IDs come from this document's valid child graph.
        };
        if !node.is_in_document() || node.is_non_rendered_html_element() {
            return Ok(());
        }
        match node.kind() {
            NodeKind::Text => {
                if (direct_body_child || parent_height > 0.0)
                    && matches!(
                        &node.data,
                        crate::node::NodeData::Text(text) if !text.text_content.trim().is_empty()
                    )
                {
                    // A text node of an ifc paragraph has no layout of its own:
                    // it stands for the lines it owns, measured from the root's
                    // content box. The inline elements between the root and
                    // the text pass the root's border-box y down unchanged, so
                    // `parent_abs_y` is the root's. A text node that is a root
                    // itself (an anonymous flex or grid item) has a box of its
                    // own, located like any other.
                    let (raw_y, height, ifc_root) = match document.ifc_text_lines(node_id) {
                        Some(owned) if owned.root != node_id => {
                            let root_layout = document.nodes[owned.root].unrounded_layout;
                            let positioned = positioned_text_line_bounds(document, node_id)
                                // cov:ignore: ifc_text_lines returns Some only when these root line records exist.
                                .unwrap_or_else(|| {
                                    owned
                                        .lines
                                        .iter()
                                        .map(|line| (line.top, line.bottom))
                                        .collect()
                                });
                            let first_top = positioned.first().map_or(0.0, |line| line.0);
                            let last_bottom = positioned
                                .iter()
                                .map(|line| line.1)
                                .fold(f32::NEG_INFINITY, f32::max);
                            let offset =
                                root_layout.border.top + root_layout.padding.top + first_top;
                            (
                                parent_abs_y + offset,
                                (last_bottom - first_top).max(0.0),
                                Some((owned.root, offset)),
                            )
                        }
                        _ => (
                            parent_abs_y + node.unrounded_layout.location.y,
                            node.unrounded_layout.size.height.max(0.0),
                            None,
                        ),
                    };
                    out.push(PageCandidate {
                        node_id,
                        raw_y,
                        height,
                        is_text: true,
                        is_rendered_leaf: false,
                        is_direct_body_element: false,
                        is_table_row: false,
                        is_flex_item: false,
                        is_grid_item: false,
                        is_named: false,
                        is_float_descendant: inside_float,
                        is_out_of_flow_descendant: inside_out_of_flow,
                        page_name: inherited_page_name,
                        named_flex_context,
                        inline_named_page: false,
                        deferred_named_break_after: false,
                        ifc_root,
                    });
                }
            }
            NodeKind::Element => {
                if node.is_display_none() {
                    return Ok(());
                }
                // An element inside a paragraph laid out by the inline engine
                // moves with the paragraph's lines; breaking at it on its own
                // would leave the lines where they are. Its text stands for
                // lines of the root and the boxes inside it are located from
                // the root, like the root's own children, so both are
                // collected as if they were. The flag is set only when the
                // inline engine is switched on.
                if node.flags.contains(NodeFlags::IN_IFC_SUBTREE) {
                    for &child_id in &node.children {
                        if matches!(
                            document.nodes[child_id].kind(),
                            NodeKind::Element | NodeKind::Text
                        ) {
                            collect_candidates(
                                document,
                                cascade,
                                child_id,
                                parent_abs_y,
                                direct_body_child,
                                body_id,
                                parent_height,
                                page_step,
                                inherited_page_name.clone(),
                                inside_table,
                                flex_column_parent,
                                grid_single_column_parent,
                                inside_flex,
                                named_flex_context,
                                inside_float,
                                inside_out_of_flow,
                                out,
                                control,
                            )?; // cov:ignore: LLVM coverage maps this multiline call to its first expression line.
                        }
                    }
                    return Ok(());
                }
                let raw_y = parent_abs_y + node.unrounded_layout.location.y;
                let computed = &cascade.computed[node_id];
                // A floated box does not establish a page transition merely
                // because it carries an inherited/explicit `page` value.  In
                // particular, CSS Page 3 keeps a float in the preceding
                // fragmentainer in the page-name-float cases.
                let is_float = is_floating_box_for_pagination(document, cascade, node_id);
                let float_subtree = inside_float || is_float;
                let out_of_flow_subtree = inside_out_of_flow
                    || matches!(
                        computed.position,
                        PositionValue::Absolute | PositionValue::Fixed
                    );
                let explicit_page_name = selected_page_name(cascade, node_id);
                // Inline canvas boxes do not establish the named page type,
                // but their `page` value still marks the boundary between the
                // surrounding class-A boxes. This mirrors the inline canvas
                // CSS Page cases without treating a replaced inline box as a
                // block-level named-page candidate.
                let inline_named_page = direct_body_child
                    && explicit_page_name.is_some()
                    && matches!(computed.display, DisplayValue::Inline)
                    && node
                        .tag_name()
                        .is_some_and(|tag| tag.eq_ignore_ascii_case("canvas"));
                // The `page` property on an out-of-flow box does not open a
                // normal-flow page boundary. Keep its inherited page context,
                // but do not use its explicit name to split pagination.
                let box_parent = pagination_box_parent(document, cascade, node_id);
                let is_direct_flex_item = box_parent.is_some_and(|parent| {
                    matches!(
                        cascade.computed[parent].display,
                        DisplayValue::Flex | DisplayValue::InlineFlex
                    )
                });
                let own_page_name = if !inline_named_page
                    && !float_subtree
                    && (!inside_flex || named_flex_context.is_some())
                    && !is_direct_flex_item
                    && !out_of_flow_subtree
                {
                    explicit_page_name.clone()
                } else {
                    None
                };
                let (has_propagated_page_name, propagated_page_name) = propagated_start_page_name(
                    document,
                    cascade,
                    node_id,
                    inherited_page_name.as_deref(),
                );
                let column_flex = matches!(
                    computed.display,
                    DisplayValue::Flex | DisplayValue::InlineFlex
                ) && matches!(
                    computed.flex_direction,
                    FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                );
                let deferred_named_break_after = matches!(computed.display, DisplayValue::Flex)
                    && !column_flex
                    && has_nested_named_page_descendant(document, cascade, node_id, 0);
                let page_name = if column_flex
                    || (named_flex_context.is_some() && !deferred_named_break_after)
                {
                    own_page_name.clone().or(inherited_page_name.clone())
                } else if has_propagated_page_name {
                    propagated_page_name
                } else {
                    own_page_name.clone().or(inherited_page_name.clone())
                };
                let child_page_name = own_page_name.clone().or(inherited_page_name);
                let is_body = node_id == body_id;
                let participates_in_flow = matches!(
                    computed.position,
                    PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
                );
                let is_tall_direct_absolute = direct_body_child
                    && matches!(computed.position, PositionValue::Absolute)
                    && node.unrounded_layout.size.height > page_step;
                let table_row_candidate = inside_table
                    && matches!(computed.display, DisplayValue::TableRow)
                    && !inside_float
                    && !inside_flex;
                let flex_item_candidate =
                    flex_column_parent && !inside_float && !out_of_flow_subtree;
                let grid_item_candidate =
                    grid_single_column_parent && !inside_float && !out_of_flow_subtree;
                // The table engine stores row geometry on its cells rather
                // than on the anonymous row box. Derive the row border box
                // from those direct cells so pagination can keep the row
                // together without changing table sizing.
                let (candidate_raw_y, candidate_height) = if table_row_candidate {
                    let mut row_top = f32::INFINITY;
                    let mut row_bottom = f32::NEG_INFINITY;
                    for &child_id in &node.children {
                        let Some(child) = document.get_node(child_id) else {
                            continue;
                        };
                        let child_top = child.unrounded_layout.location.y;
                        row_top = row_top.min(child_top);
                        row_bottom = row_bottom.max(child_top + child.unrounded_layout.size.height);
                    }
                    if row_top.is_finite() && row_bottom.is_finite() && row_bottom > row_top {
                        (raw_y + row_top, row_bottom - row_top)
                    } else {
                        (raw_y, node.unrounded_layout.size.height.max(0.0))
                    }
                } else {
                    (raw_y, node.unrounded_layout.size.height.max(0.0))
                };
                // Non-text content can establish the anonymous page before
                // a later named descendant, even within the first flex item.
                // Container geometry alone does not establish that context.
                let is_rendered_leaf = named_flex_context.is_some()
                    && computed.display != DisplayValue::Contents
                    && !float_subtree
                    && !out_of_flow_subtree
                    && candidate_height > 0.0
                    && !node.children.iter().any(|&child| {
                        let child_node = &document.nodes[child];
                        if child_node.is_display_none()
                            || child_node.is_non_rendered_html_element()
                            || matches!(
                                cascade.computed[child].position,
                                PositionValue::Absolute | PositionValue::Fixed
                            )
                            || is_floating_box_for_pagination(document, cascade, child)
                        {
                            return false;
                        }
                        match &child_node.data {
                            crate::node::NodeData::Text(text) => {
                                !text.text_content.trim().is_empty()
                            }
                            crate::node::NodeData::Element(_) => {
                                child_node.unrounded_layout.size.height > 0.0
                            }
                            _ => false,
                        }
                    });
                let is_break_candidate = !is_body
                    && computed.display != DisplayValue::Contents
                    && !inside_float
                    && ((participates_in_flow
                        && direct_body_child
                        && explicit_page_name.is_none())
                        || is_tall_direct_absolute
                        || table_row_candidate
                        || flex_item_candidate
                        || is_rendered_leaf
                        || grid_item_candidate
                        || deferred_named_break_after
                        || own_page_name.is_some()
                        || inline_named_page
                        || page_break_is_forced(computed.break_before)
                        || page_break_is_forced(computed.break_after));
                if is_break_candidate {
                    out.push(PageCandidate {
                        node_id,
                        raw_y: candidate_raw_y,
                        height: candidate_height,
                        is_text: false,
                        is_rendered_leaf,
                        is_direct_body_element: direct_body_child,
                        is_table_row: table_row_candidate,
                        is_flex_item: flex_item_candidate,
                        is_grid_item: grid_item_candidate,
                        is_named: own_page_name.is_some(),
                        is_float_descendant: inside_float,
                        is_out_of_flow_descendant: out_of_flow_subtree,
                        page_name: page_name.clone(),
                        named_flex_context,
                        inline_named_page,
                        deferred_named_break_after,
                        ifc_root: None,
                    });
                }
                let child_order = pagination_child_order(document, cascade, node_id);
                for child_id in child_order {
                    let child_parent_y =
                        pagination_child_parent_y(document, node_id, child_id, raw_y);
                    collect_candidates(
                        document,
                        cascade,
                        child_id,
                        child_parent_y,
                        document.parent_of(child_id) == Some(body_id),
                        body_id,
                        node.unrounded_layout.size.height.max(0.0),
                        page_step,
                        child_page_name.clone(),
                        inside_table
                            || matches!(
                                computed.display,
                                DisplayValue::Table
                                    | DisplayValue::InlineTable
                                    | DisplayValue::TableRowGroup
                                    | DisplayValue::TableHeaderGroup
                                    | DisplayValue::TableFooterGroup
                                    | DisplayValue::TableRow
                            ),
                        matches!(
                            computed.display,
                            DisplayValue::Flex | DisplayValue::InlineFlex
                        ) && matches!(
                            computed.flex_direction,
                            FlexDirectionValue::Column | FlexDirectionValue::ColumnReverse
                        ) || (computed.display == DisplayValue::Contents && flex_column_parent),
                        matches!(
                            computed.display,
                            DisplayValue::Grid | DisplayValue::InlineGrid
                        ) && document.nodes[node_id].grid_column_count == 1,
                        inside_flex || matches!(computed.display, DisplayValue::Flex),
                        if column_flex {
                            Some(node_id)
                        } else if matches!(
                            computed.display,
                            DisplayValue::Flex | DisplayValue::InlineFlex
                        ) {
                            None
                        } else {
                            named_flex_context
                        },
                        float_subtree,
                        out_of_flow_subtree,
                        out,
                        control,
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut candidates = Vec::new();
    collect_candidates(
        document,
        cascade,
        body_id,
        root_flow_offset,
        false,
        body_id,
        0.0,
        page_step,
        None,
        false,
        false,
        false,
        false,
        None,
        false,
        false,
        &mut candidates,
        control,
    )?;
    let mut flow_shift = 0.0_f32;
    let mut current_page = 0_u32;
    let mut max_page = 0_u32;
    // Keep a break-after attached to its source box until its descendants
    // have been visited.  Applying it to the first flex child would turn a
    // break-after on the flex container into an extra blank page.
    let mut pending_break_source: Option<usize> = None;
    // When a forced-break box underflows into the preceding page, descendants
    // follow the box while later siblings still begin after the break boundary.
    let mut pending_underflow: Option<(usize, f32)> = None;
    // A positive margin on a box after a forced break moves its descendants
    // with the box, but does not move later siblings a second time.
    let mut pending_descendant_margin: Option<(usize, f32)> = None;
    // Named class-A boxes at one source coordinate form one page transition.
    // This coalesces zero-height named runs such as a/b/c/d/e without creating
    // one blank page per zero-height box.
    let mut last_named_raw_y: Option<f32> = None;
    let mut last_named_context: Option<usize> = None;
    let mut last_named_was_zero_height = false;
    let mut last_named_had_display_none_descendant = false;
    let mut saw_child = false;
    // Direct fixed-height blocks are the only class-A boxes for which this
    // minimal paginator can safely move a leading line as a unit. Track the
    // first text child so a block is not reflowed repeatedly for later lines.
    let mut checked_block_text = HashSet::new();
    let mut checked_orphans_widows = HashSet::new();
    // Footnotes are collected in source order and stacked upward from the
    // bottom edge of their anchor page. The normal-flow height is removed
    // from `flow_shift`, leaving the box only in this page-local area.
    let mut footnote_heights: HashMap<u32, f32> = HashMap::new();
    let mut current_page_name =
        selected_page_name(cascade, body_id).or_else(|| first_page_name(document, cascade));
    let mut page_names = vec![current_page_name.clone()];
    let mut outer_page_name = current_page_name.clone();
    let mut seen_named_flex_contexts = HashMap::new();
    let mut column_context_cache = vec![None; parent_of.len()];
    let mut preceding_flex_items = HashMap::<usize, (usize, f32, u32)>::new();

    let mut trailing_flex_child_by_parent = HashMap::<usize, Option<usize>>::new();
    // Paragraphs laid out by the inline engine that already had a candidate
    // inside them. Only the first candidate of a paragraph may move its root:
    // a later one would move again what the earlier ones placed.
    let mut entered_ifc_roots = HashSet::new();
    macro_rules! check_candidate_page {
        ($label:lifetime, $page_index:expr) => {
            match control.check_page_index($page_index) {
                Ok(()) => {}
                Err(LayoutError::PageLimitExceeded { .. }) if control.truncate_at_page_limit => {
                    control.page_limit_reached.set(true);
                    break $label;
                }
                Err(error) => return Err(error),
            }
        };
    }

    'candidate_loop: for candidate in candidates {
        check_candidate_page!('candidate_loop, current_page);
        let node_id = candidate.node_id;
        let name_participates_in_flow =
            !candidate.is_float_descendant && !candidate.is_out_of_flow_descendant;
        if name_participates_in_flow
            && candidate.is_flex_item
            && let Some(context) = candidate.named_flex_context
            && let Some((_, height, page)) =
                preceding_flex_items.insert(context, (node_id, candidate.height, current_page))
            && height > 0.0
            && page == current_page
        {
            occupy_column_contexts(
                Some(context),
                current_page,
                &mut seen_named_flex_contexts,
                &parent_of,
                cascade,
                &mut column_context_cache,
            );
        }
        let (name_context_seen, comparison_page_name) = if !name_participates_in_flow {
            (false, current_page_name.clone())
        } else {
            match candidate.named_flex_context {
                Some(context) if candidate.deferred_named_break_after => {
                    // Empty enclosing column boxes do not open an anonymous
                    // page before the row's established deferred boundary.
                    let seen = column_context_occupied(
                        context,
                        current_page,
                        &seen_named_flex_contexts,
                        &parent_of,
                        cascade,
                        &mut column_context_cache,
                    ) || current_page_name.is_some();
                    outer_page_name = candidate.page_name.clone();
                    (saw_child && seen, current_page_name.clone())
                }
                Some(context)
                    if candidate.is_named || candidate.is_text || candidate.is_rendered_leaf =>
                {
                    // Nested column flows share the physical page opened by
                    // their preceding sibling, including an inner flow's end.
                    // A previously seen anonymous type is still a page type.
                    let seen = column_context_occupied(
                        context,
                        current_page,
                        &seen_named_flex_contexts,
                        &parent_of,
                        cascade,
                        &mut column_context_cache,
                    ) || current_page_name.is_some();
                    (saw_child && seen, current_page_name.clone())
                }
                Some(_) => (false, candidate.page_name.clone()),
                None => {
                    let previous = if candidate.is_named {
                        current_page_name.clone()
                    } else {
                        outer_page_name.clone()
                    };
                    if !candidate.is_float_descendant {
                        outer_page_name = candidate.page_name.clone();
                    }
                    (saw_child, previous)
                }
            }
        };
        // Leaving a flex item's internal named flow keeps the last opened
        // physical page until an outer boundary or natural overflow opens one.
        let keep_local_page_type =
            candidate.named_flex_context.is_none() && current_page_name != comparison_page_name;
        let page_before_candidate = current_page;
        let moves_ifc_root = {
            let mut first = true;
            let mut current = parent_of.get(node_id).copied().flatten();
            while let Some(id) = current {
                if document.nodes[id].is_ifc_root() && !entered_ifc_roots.insert(id) {
                    first = false;
                }
                current = parent_of.get(id).copied().flatten();
            }
            first
        };
        let ifc_root = candidate.ifc_root.filter(|_| moves_ifc_root);
        if let Some((ancestor_id, correction)) = pending_underflow
            && !is_descendant_or_self(document, node_id, ancestor_id, &parent_of)
        {
            flow_shift += correction;
            pending_underflow = None;
        }
        if document.get_node(node_id).is_none() {
            continue;
        }
        let descendant_margin = if let Some((ancestor_id, margin)) = pending_descendant_margin {
            if is_descendant_or_self(document, node_id, ancestor_id, &parent_of) {
                margin
            } else {
                pending_descendant_margin = None;
                0.0
            }
        } else {
            0.0
        };
        if candidate.is_text {
            // A footnote's descendants follow the moved footnote box. They are
            // deliberately not paginated independently, otherwise the stale
            // pre-pagination coordinates would move them back into normal flow.
            if candidate.is_float_descendant && has_footnote_ancestor(cascade, node_id, &parent_of)
            {
                continue;
            }
            // Text is not itself a class-A box, but non-empty text can be the
            // content that follows a nested named box.  Carry the containing
            // box's page type so that content after `page:b` inside a
            // `page:a` box resumes on an `a` page rather than becoming an
            // anonymous page.  Only direct body text consumes a pending
            // break-after; descendants must wait until their containing box
            // has finished.
            let raw_y = candidate.raw_y;
            let height = candidate.height;
            let candidate_page_name = candidate.page_name.clone();
            let mut effective_y = raw_y + flow_shift + descendant_margin;
            // A fixed-height direct block whose first line would straddle the
            // current fragmentainer is pushed as a unit. This matches normal
            // block fragmentation for monolithic content while leaving tall
            // blocks and already-forced page transitions to the existing
            // fragment logic.
            // A block inside an inline element of an inline engine paragraph
            // is laid out from the paragraph's root, as a direct child is.
            let direct_block_parent = parent_of[node_id].and_then(|parent_id| {
                (document.layout_parent_of(parent_id) == Some(body_id)).then_some(parent_id)
            });
            if let Some(block_id) = direct_block_parent
                && checked_block_text.insert(block_id)
                && style_dimension_length(document.nodes[block_id].style.size.height)
                    .is_some_and(|value| value >= 0.0)
            {
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let block_raw_y =
                    root_flow_offset + current_abs_y(document, block_id, &parent_of) - flow_shift;
                let relative_text_y = raw_y - block_raw_y;
                let block_effective_y = effective_y - relative_text_y;
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let first_line_overflows = effective_y.is_finite()
                    && height.is_finite()
                    && effective_y >= page_start
                    && block_effective_y >= page_start
                    && block_effective_y < page_end
                    && block_height.is_finite()
                    && block_height <= page_step_at(current_page) + 0.001
                    && effective_y + height > page_end;
                if first_line_overflows {
                    let natural_page = page_index_for_y(effective_y + height);
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    check_candidate_page!('candidate_loop, target_page);
                    let target_y = page_origin(target_page);
                    let delta = target_y - block_effective_y;
                    if delta.is_finite() && delta > 0.0 {
                        materialize_y(document, block_id, block_effective_y + delta, &parent_of);
                        flow_shift += delta;
                        effective_y += delta;
                    }
                }
            }
            // `orphans` and `widows` constrain breaks between line boxes, not
            // breaks between block-level children.  The current paginator keeps
            // a paragraph in one piece, so the safe first step is to
            // move a fitting direct block as a unit when its natural split would
            // violate either constraint.  Oversized blocks stay on the
            // existing whole-box path; their line-level fragment map is a
            // separate concern.
            if let Some(block_id) = direct_block_parent
                && checked_orphans_widows.insert(block_id)
                && let Some(line_metrics) = text_line_bounds(document, node_id)
            {
                let page_start = page_origin(current_page);
                let page_end = page_start + page_step_at(current_page);
                let total_lines = line_metrics.len();
                let starts_on_page = line_metrics.first().is_some_and(|(line_top, _)| {
                    let top = effective_y + line_top;
                    top >= page_start - 0.001 && top < page_end
                });
                let block_height = document.nodes[block_id].unrounded_layout.size.height;
                let fits_on_page =
                    block_height.is_finite() && block_height <= page_step_at(current_page) + 0.001;
                let lines_before_break = line_metrics
                    .iter()
                    .take_while(|(_, line_bottom)| effective_y + line_bottom <= page_end + 0.001)
                    .count();
                let needs_break = lines_before_break < total_lines;
                let orphans = cascade.computed[node_id].orphans.max(1) as usize;
                let widows = cascade.computed[node_id].widows.max(1) as usize;
                let violates_line_constraints = needs_break
                    && (lines_before_break < orphans
                        || total_lines.saturating_sub(lines_before_break) < widows);
                if starts_on_page && fits_on_page && violates_line_constraints {
                    let block_raw_y = root_flow_offset
                        + current_abs_y(document, block_id, &parent_of)
                        - flow_shift;
                    let target_page = current_page.saturating_add(1);
                    check_candidate_page!('candidate_loop, target_page);
                    let delta = page_origin(target_page) - block_raw_y;
                    if delta.is_finite() && delta > 0.0 {
                        materialize_y(document, block_id, block_raw_y + delta, &parent_of);
                        flow_shift += delta;
                        effective_y += delta;
                    }
                }
            }
            if let Some((ifc_root, offset)) = ifc_root {
                materialize_y(document, ifc_root, effective_y - offset, &parent_of);
            } else if let Some((root, _)) = candidate.ifc_root
                && let Some(first) = document
                    .ifc_text_lines(node_id)
                    .and_then(|owned| owned.lines.first().copied())
            {
                // A later text of a paragraph whose root already moved: its
                // lines go where the flow puts them, and the ones after
                // follow (an earlier box of the paragraph may have grown).
                let layout = document.nodes[root].unrounded_layout;
                let first_top = positioned_text_line_bounds(document, node_id)
                    .and_then(|lines| lines.first().map(|line| line.0))
                    .unwrap_or(first.top);
                let current = current_abs_y(document, root, &parent_of)
                    + layout.border.top
                    + layout.padding.top
                    + first_top;
                follow_moved_ifc_text(document, root, node_id, effective_y - current);
            }
            materialize_y(document, node_id, effective_y, &parent_of);
            let named_page_change = name_context_seen
                && height > 0.0
                && name_participates_in_flow
                && candidate_page_name != comparison_page_name;
            let pending_break_applies = pending_break_source.is_some_and(|source| {
                !is_descendant_or_self(document, node_id, source, &parent_of)
            });
            let consumes_pending_break = pending_break_applies && name_participates_in_flow;
            if saw_child {
                if consumes_pending_break || named_page_change {
                    let natural_page = if effective_y.is_finite() && effective_y >= 0.0 {
                        page_index_for_y(effective_y)
                    } else {
                        current_page
                    };
                    let target_page = current_page.saturating_add(1).max(natural_page);
                    check_candidate_page!('candidate_loop, target_page);
                    let target_y = page_origin(target_page);
                    // The candidate was materialized to `effective_y` above,
                    // so this is the additional movement for this boundary.
                    // Keep it separate from `flow_shift`: the latter also
                    // affects candidates whose ancestor is not moved here.
                    let node_delta = target_y - effective_y;
                    let shift_delta = target_y - effective_y;
                    if node_delta.is_finite() && shift_delta.is_finite() {
                        // The text of an ifc paragraph is painted from its
                        // root, so the root carries the movement.
                        let moved = ifc_root.map_or(node_id, |(root, _)| root);
                        document.nodes[moved].unrounded_layout.location.y += node_delta;
                        // A later text of a paragraph whose root already
                        // moved: its lines, and the ones after, move alone.
                        if ifc_root.is_none()
                            && let Some((root, _)) = candidate.ifc_root
                        {
                            follow_moved_ifc_text(document, root, node_id, node_delta);
                        }
                        flow_shift += shift_delta;
                        effective_y += shift_delta;
                    }
                    current_page = target_page;
                    current_page_name = candidate_page_name.clone();
                } else if effective_y.is_finite() && effective_y >= 0.0 {
                    let natural_page = page_index_for_y(effective_y);
                    check_candidate_page!('candidate_loop, natural_page);
                    current_page = current_page.max(natural_page);
                }
            } else {
                saw_child = true;
                current_page = 0;
                if name_participates_in_flow {
                    current_page_name = candidate_page_name.clone();
                }
            }
            if height > 0.0
                && name_participates_in_flow
                && !named_page_change
                && !consumes_pending_break
                && (!keep_local_page_type || current_page > page_before_candidate)
            {
                current_page_name = candidate_page_name;
            }
            check_candidate_page!('candidate_loop, current_page);
            if page_names.len() <= current_page as usize {
                page_names.resize(current_page as usize + 1, None);
            }
            page_names[current_page as usize] = current_page_name.clone();
            max_page = max_page.max(current_page);
            if name_participates_in_flow && height > 0.0 {
                let context = enclosing_column_context(
                    node_id,
                    &parent_of,
                    cascade,
                    &mut column_context_cache,
                );
                occupy_column_contexts(
                    context,
                    current_page,
                    &mut seen_named_flex_contexts,
                    &parent_of,
                    cascade,
                    &mut column_context_cache,
                );
            }
            if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
                let end = (effective_y + height).max(effective_y);
                if end.is_finite() && end > 0.0 {
                    let end_page = page_index_for_end(end);
                    control.check_discovery_page_index(end_page)?;
                    max_page = max_page.max(end_page);
                }
            }
            if consumes_pending_break {
                pending_break_source = None;
            }
            let candidate_break_after = page_break_is_forced(cascade.computed[node_id].break_after)
                || candidate.deferred_named_break_after;
            let pending_source_is_ancestor = pending_break_source
                .is_some_and(|source| is_descendant_or_self(document, node_id, source, &parent_of));
            if candidate_break_after && !pending_source_is_ancestor {
                pending_break_source = Some(node_id);
            }
            continue;
        }

        let computed = &cascade.computed[node_id];
        let candidate_page_name = candidate.page_name.clone();
        // `float: footnote` is a page-local out-of-flow placement. Keep the
        // anchor's page, stack multiple notes from the bottom, and remove the
        // note's ordinary-flow footprint so following content can continue.
        if matches!(computed.float, FloatValue::Footnote) {
            let raw_y = candidate.raw_y;
            let height = candidate.height;
            let effective_y = raw_y + flow_shift + descendant_margin;
            let anchor_page = if effective_y.is_finite() && effective_y >= 0.0 {
                page_index_for_y(effective_y)
            } else {
                current_page
            };
            let note_page = current_page.max(anchor_page);
            check_candidate_page!('candidate_loop, note_page);
            let used = footnote_heights.entry(note_page).or_insert(0.0);
            let target_y = (page_origin(note_page) + page_step_at(note_page) - *used - height)
                .max(page_origin(note_page));
            materialize_y(document, node_id, target_y, &parent_of);
            *used += height;
            flow_shift -= height;
            max_page = max_page.max(note_page);
            saw_child = true;
            continue;
        }
        // Copy layout data before any break adjustment so the immutable node
        // borrow does not overlap the in-place location update below.
        let raw_y = candidate.raw_y;
        let height = candidate.height;
        let mut effective_y = raw_y + flow_shift + descendant_margin;
        materialize_y(document, node_id, effective_y, &parent_of);
        let forced_before =
            page_break_is_forced(computed.break_before) || candidate.inline_named_page;
        let style = &document.nodes[node_id].style;
        let margin_top = used_style_length_percentage_auto(style.margin.top, content_width)
            .or_else(|| used_computed_length_percentage_or_auto(computed.margin.top, content_width))
            .unwrap_or(0.0);
        let margin_bottom = used_style_length_percentage_auto(style.margin.bottom, content_width)
            .or_else(|| {
                used_computed_length_percentage_or_auto(computed.margin.bottom, content_width)
            })
            .unwrap_or(0.0)
            .max(0.0);
        // A block with `break-inside: avoid-page` stays intact when its used
        // block size plus its trailing margin would cross the current page.
        // This is the direct block-flow case; nested formatting contexts still
        // require fragment-level break opportunities.
        let page_overflow = matches!(
            computed.break_inside,
            raikiri_style::property::BreakInside::Avoid
                | raikiri_style::property::BreakInside::AvoidPage
        ) && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height + margin_bottom
                > page_origin(current_page) + page_step_at(current_page);
        // A named page requested by a class-A box starts a new page when it
        // differs from the current named page. `auto` leaves the current page
        // type in place; it does not manufacture a second break boundary.
        // Any non-empty class-A box with a different page name starts a new
        // page.  `None` is the anonymous/unnamed page type, so an anonymous
        // box after a named one is a transition too.  Zero-height named boxes
        // at the same source coordinate are coalesced into one transition;
        // this matters for chains of empty named boxes with overflowing text.
        let same_named_coordinate = candidate.is_named
            && (candidate.is_direct_body_element || candidate.named_flex_context.is_some())
            && candidate.named_flex_context == last_named_context
            && height <= 0.001
            && last_named_was_zero_height
            && !last_named_had_display_none_descendant
            && last_named_raw_y.is_some_and(|previous| (raw_y - previous).abs() <= 0.001);
        let named_page_change = name_context_seen
            && candidate_page_name != comparison_page_name
            && !same_named_coordinate;
        let at_page_start =
            effective_y.is_finite() && (effective_y - page_origin(current_page)).abs() <= 0.001;
        let natural_page_at_position = if effective_y.is_finite() && effective_y >= 0.0 {
            page_index_for_y(effective_y)
        } else {
            current_page
        };
        // A forced break at the start of a page already opened by natural
        // overflow describes the same boundary and must not create a blank
        // page.  This matters when the page height changes after page zero:
        // the fixed-height pass may have considered the box to be on the
        // current page, while the scheduled pass places it exactly at the
        // next page origin.
        let forced_break_at_page_start =
            forced_before && (at_page_start || natural_page_at_position > current_page);
        // Keep an ordinary table row intact when it would cross the current
        // page. The row is a class-A boundary owned by the table engine; its
        // cells and descendants follow the same flow shift below. Rowspans,
        // repeated header groups, and cell-internal breaks remain out of this
        // narrow row-boundary pass.
        let table_row_overflow = candidate.is_table_row
            && height <= page_step_at(current_page) + 0.001
            && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);
        // A trailing flex item may extend into the containing flex box's
        // continuation at the page edge. Earlier items still move intact when
        // they would cross a fragmentainer, preserving the existing column
        // flex pagination behavior.
        let flex_item_is_last = candidate.is_flex_item
            && pagination_box_parent(document, cascade, node_id).is_some_and(|parent_id| {
                let trailing_child = trailing_flex_child_by_parent
                    .entry(parent_id)
                    .or_insert_with(|| {
                        trailing_flex_child_for_pagination(document, cascade, parent_id)
                    });
                *trailing_child == Some(node_id)
            });
        let flex_item_overflow = candidate.is_flex_item
            && !flex_item_is_last
            && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);
        let grid_item_overflow = candidate.is_grid_item
            && effective_y.is_finite()
            && effective_y >= page_origin(current_page)
            && effective_y < page_origin(current_page) + page_step_at(current_page)
            && effective_y + height > page_origin(current_page) + page_step_at(current_page);

        let pending_break_applies = pending_break_source
            .is_some_and(|source| !is_descendant_or_self(document, node_id, source, &parent_of));
        let page_transition = saw_child
            && (pending_break_applies
                || (forced_before && !forced_break_at_page_start)
                || named_page_change
                || page_overflow
                || table_row_overflow
                || flex_item_overflow
                || grid_item_overflow);
        if saw_child {
            // A break-after on the preceding box and a break-before (or named
            // page transition) on this box describe the same boundary, not two
            // blank pages.
            if page_transition {
                let natural_page = if effective_y.is_finite() && effective_y >= 0.0 {
                    page_index_for_y(effective_y)
                } else {
                    current_page
                };
                let target_page = current_page.saturating_add(1).max(natural_page);
                check_candidate_page!('candidate_loop, target_page);
                let target_y = page_origin(target_page);
                // A negative block-start margin can pull a forced-break box
                // back into the preceding page.  Keep the break boundary for
                // following siblings (and for page count), but materialize
                // this box at the underflowed coordinate.  This is the
                // `underflow-from-next-page` case from CSS Break.
                let underflow_y = target_y + margin_top;
                let node_target_y = if margin_top < 0.0 && underflow_y < target_y {
                    underflow_y
                } else {
                    // The forced break is before the box's margin box, so a
                    // positive block-start margin remains on the new page.
                    target_y + margin_top.max(0.0)
                };
                // The candidate was materialized to `effective_y` above,
                // so this is the additional movement for this boundary.
                // Keep `flow_shift` at the page boundary even when the
                // candidate itself underflows into the preceding page.
                let node_delta = node_target_y - effective_y;
                let shift_delta = target_y - effective_y;
                let row_flex_break_before = forced_before
                    && parent_of[node_id].is_some_and(|parent_id| {
                        matches!(
                            cascade.computed[parent_id].display,
                            DisplayValue::Flex | DisplayValue::InlineFlex
                        ) && matches!(
                            cascade.computed[parent_id].flex_direction,
                            FlexDirectionValue::Row | FlexDirectionValue::RowReverse
                        )
                    });
                if node_delta.is_finite() && shift_delta.is_finite() {
                    document.nodes[node_id].unrounded_layout.location.y += node_delta;
                    follow_moved_ifc_block(document, node_id, node_delta);
                    // A forced break on one wrapped row-flex item belongs to
                    // its whole flex line. Move same-line siblings together;
                    // other lines keep their existing flow coordinates.
                    if row_flex_break_before && let Some(parent_id) = parent_of[node_id] {
                        let siblings = document.nodes[parent_id].children.clone();
                        for sibling_id in siblings {
                            if sibling_id == node_id {
                                continue;
                            }
                            let sibling_raw_y = current_abs_y(document, sibling_id, &parent_of);
                            if (sibling_raw_y - raw_y).abs() <= 0.001 {
                                materialize_y(
                                    document,
                                    sibling_id,
                                    sibling_raw_y + shift_delta,
                                    &parent_of,
                                );
                            }
                        }
                    }
                    if node_target_y < target_y {
                        flow_shift += node_delta;
                        pending_underflow = Some((node_id, shift_delta - node_delta));
                    } else {
                        flow_shift += shift_delta;
                    }
                    effective_y += node_delta;
                }
                current_page = target_page;
                if pending_break_applies {
                    pending_break_source = None;
                }
            } else if effective_y.is_finite() && effective_y >= 0.0 {
                let natural_page = page_index_for_y(effective_y);
                check_candidate_page!('candidate_loop, natural_page);
                current_page = current_page.max(natural_page);
            }
            if page_transition && margin_top > 0.0 {
                pending_descendant_margin = Some((node_id, margin_top));
            }
            // An inline direct child keeps its source-order inline x position
            // in taffy's single pre-pagination layout. A forced page boundary
            // starts a fresh page formatting context, so reset that box's
            // inline origin before painting the next slice.
            if page_transition
                && candidate.is_direct_body_element
                && matches!(computed.display, DisplayValue::Inline)
            {
                document.nodes[node_id].unrounded_layout.location.x = 0.0;
            }
        } else {
            // A forced break before the first class-A box does not manufacture
            // a leading blank page.
            saw_child = true;
            current_page = 0;
        }

        if name_participates_in_flow
            && (candidate.is_direct_body_element
                || candidate.is_named
                || candidate.is_rendered_leaf
                || candidate.deferred_named_break_after)
            && (!keep_local_page_type || page_transition || current_page > page_before_candidate)
        {
            current_page_name = candidate_page_name;
            if candidate.is_direct_body_element || candidate.named_flex_context.is_some() {
                last_named_context = candidate.named_flex_context;
                last_named_raw_y = candidate.is_named.then_some(raw_y);
                last_named_was_zero_height = candidate.is_named && height <= 0.001;
                last_named_had_display_none_descendant =
                    candidate.is_named && has_display_none_descendant(document, node_id);
            }
        }
        check_candidate_page!('candidate_loop, current_page);
        if page_names.len() <= current_page as usize {
            page_names.resize(current_page as usize + 1, None);
        }
        page_names[current_page as usize] = current_page_name.clone();

        max_page = max_page.max(current_page);
        if height > 0.0 && effective_y.is_finite() && effective_y >= 0.0 {
            // A box ending exactly at a page edge belongs to the preceding
            // page.  `f32::EPSILON` is too small at ordinary CSS coordinates
            // and rounds away, so use the half-open interval directly:
            // ceil(end / step) - 1.  The box remains visible on every slice it
            // intersects; line-level splitting is a later pass.
            let end = (effective_y + height).max(effective_y);
            if end.is_finite() && end > 0.0 {
                // A fixed-height direct block that fits on the physical paper
                // is monolithic in this minimal paginator.  Page border/padding
                // reduce the fragmentainer's content height, but must not make
                // the block manufacture a second page merely to expose its
                // overflow into the page decoration area.
                let monolithic_paper_fit = candidate.is_direct_body_element
                    && (insets.top > 0.0
                        || insets.right > 0.0
                        || insets.bottom > 0.0
                        || insets.left > 0.0)
                    && effective_y >= page_origin(current_page)
                    && effective_y + height <= page_origin(current_page) + page_box.height + 0.001
                    && style_dimension_length(document.nodes[node_id].style.size.height)
                        .is_some_and(|value| value >= 0.0);
                if !monolithic_paper_fit {
                    let end_page = page_index_for_end(end);
                    control.check_discovery_page_index(end_page)?;
                    max_page = max_page.max(end_page);
                }
            }
        }
        if name_participates_in_flow
            && (candidate.is_named || (candidate.is_rendered_leaf && height > 0.0))
        {
            let context =
                enclosing_column_context(node_id, &parent_of, cascade, &mut column_context_cache);
            occupy_column_contexts(
                context,
                current_page,
                &mut seen_named_flex_contexts,
                &parent_of,
                cascade,
                &mut column_context_cache,
            );
        }
        if candidate.is_flex_item
            && let Some(context) = candidate.named_flex_context
        {
            preceding_flex_items.insert(context, (node_id, height, current_page));
        }
        let candidate_break_after = page_break_is_forced(computed.break_after)
            || candidate.inline_named_page
            || candidate.deferred_named_break_after;
        let pending_source_is_ancestor = pending_break_source
            .is_some_and(|source| is_descendant_or_self(document, node_id, source, &parent_of));
        if candidate_break_after && !pending_source_is_ancestor {
            pending_break_source = Some(node_id);
        }
    }

    if !page_widths.is_empty() {
        let base_width = page_widths
            .first()
            .copied()
            .filter(|width| width.is_finite() && *width > 0.0)
            .unwrap_or(content_width);
        if base_width.is_finite() && base_width > 0.0 {
            for node_id in 0..document.nodes.len() {
                control.check_aborted()?;
                if !matches!(
                    cascade.computed[node_id].width,
                    ComputedLengthPercentageOrAuto::Percent(_)
                ) {
                    continue;
                }
                let page_index = page_index_for_y(current_abs_y(document, node_id, &parent_of));
                let Some(target_width) = page_widths
                    .get(page_index as usize)
                    .copied()
                    .filter(|width| width.is_finite() && *width > 0.0)
                else {
                    continue;
                };
                let scale = target_width / base_width;
                if scale.is_finite() && (scale - 1.0).abs() > 0.0001 {
                    document.nodes[node_id].unrounded_layout.size.width *= scale;
                }
            }
        }
    }

    control.check_discovery_page_index(max_page)?;
    let last_page = if control.page_limit_reached.get() {
        control.max_pages.unwrap_or(0).saturating_sub(1)
    } else {
        max_page
    };
    Ok((0..=last_page)
        .map(|page_index| PageSlice {
            page_index,
            content_origin_y: page_origin(page_index),
            page_name: page_names.get(page_index as usize).cloned().flatten(),
        })
        .collect())
}

/// Like [`layout_single_page`], but first resolve intrinsic sizes of
/// replaced elements such as `<img>` through `resolver`.
///
/// # Errors
/// As for [`layout_single_page`], plus this function’s own
/// `LayoutError::Resolver`: abort the pre-pass as soon as `resolver.resolve()`
/// returns `Err`, wrap that error in `LayoutError::Resolver`, and return
/// without running taffy layout. The `ReplacedResolver` contract makes `Err`
/// terminal; consumers request placeholder degradation by returning
/// `Ok(ResolvedIntrinsic { disposition: Fallback { .. } })` instead.
/// See the documentation for `crate::image_resolve::resolve_images`.
///
/// # Execution order (essential)
/// Call [`Document::mark_in_document_flags`] before `resolve_images`.
/// `resolve_images` skips `<img>` in inert subtrees (such as descendants of
/// `<template>`) using membership flags. If flags are stale, it may fetch a URL
/// that should not be fetched. [`layout_single_page`] synchronizes flags too,
/// but only **after** this pre-pass, which would be too late.
/// `mark_in_document_flags` is O(1) when `flags_dirty == false`,
/// so calling it twice has no meaningful cost.
pub fn layout_single_page_with_resolver(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
) -> Result<(), LayoutError> {
    layout_single_page_with_resolver_and_base_url(document, cascade, page_box, resolver, None)
}

/// [`layout_single_page_with_resolver`] with document-relative image URLs.
pub fn layout_single_page_with_resolver_and_base_url(
    document: &mut Document,
    cascade: &CascadeResult,
    page_box: PageBox,
    resolver: &dyn ReplacedResolver,
    base_url: Option<&url::Url>,
) -> Result<(), LayoutError> {
    super::validate_layout_depth(document)?;
    document.page_projection.clear();
    // See “Execution order” above — this must precede `resolve_images`, whose
    // membership gate reads the flags this refreshes.
    document.mark_in_document_flags();
    match base_url {
        Some(base_url) => {
            crate::image_resolve::resolve_images_with_base(document, resolver, Some(base_url))
        }
        None => crate::image_resolve::resolve_images(document, resolver),
    }
    .map_err(LayoutError::Resolver)?;
    layout_single_page(document, cascade, page_box)
}

#[cfg(test)]
mod tests;
