//! The text of generated content (`::before`, `::after`, markers): the
//! resolution of `content` with counters, quotes and attributes, shared by
//! the inline engine, which lays the text of in-flow pseudo-elements out in
//! their paragraphs, and the painter, which draws the rest as overlays.

/// Marker resolution shared by inline layout and painting.
#[doc(hidden)]
pub mod markers;

use crate::Document;
use crate::target::{CompactCounterSnapshots, CounterSnapshot, CounterSnapshots};
use raikiri_style::property::{
    ContentComponent, CounterStyle, DisplayValue, FloatValue, PositionValue, QuoteKeyword,
};
use raikiri_style::{
    CascadeResult, ComputedValues, CounterStyleRegistry, PseudoElem, StyleNodeId,
    resolve_custom_counter,
};

/// Marks a shodo node id that stands for a pseudo-element: the inline engine
/// lays the text of `::before` and `::after` out under ids of their own,
/// outside the range of document node ids. The bit is the second highest of
/// `usize`, so the ids round-trip through `usize` on every target (a document
/// would need more than `usize::MAX / 16` nodes to reach it).
const GENERATED_ID_BIT: usize = 1 << (usize::BITS - 2);

/// Marks, together with [`GENERATED_ID_BIT`], the id of one image of a
/// `::before` or `::after` `content` list: the anonymous inline replaced
/// element the image stands for (CSS Content 3 §2.2) is an atomic inline of
/// its own in the paragraph.
const IMAGE_ID_BIT: usize = 1 << (usize::BITS - 3);

/// Low bits of an image id that hold the index of the image among the images
/// of its `content` list, above the bit that tells `::after` from `::before`.
const IMAGE_INDEX_BITS: u32 = 5;

/// The number of images of one `content` list that are laid out; later ones
/// represent nothing.
#[doc(hidden)]
pub const MAX_GENERATED_IMAGES: usize = 1 << IMAGE_INDEX_BITS;

/// Low bits of the id of an opacity group of a `::first-letter` other than
/// its first one, that number the group among the groups of the
/// pseudo-element in its paragraph.
const LETTER_GROUP_BITS: u32 = 8;

/// The number of opacity groups one `::first-letter` can form in its
/// paragraph.
///
/// A fragment past the limit shares an earlier group inside the same group
/// around it, or has none of its own, so its opacity is approximated and
/// [`crate::Document::paint_order_approximations`] reports the paragraph.
/// Every bit added to the limit halves the elements whose groups can be
/// named: with 8 bits, ids reach elements below 2^21 on 32-bit targets, and
/// with 16 bits only below 2^13.
#[doc(hidden)]
pub const MAX_LETTER_GROUPS: usize = 1 << LETTER_GROUP_BITS;

/// The node id the inline engine gives the `pseudo` of `element`.
#[doc(hidden)]
pub fn generated_node_id(element: usize, pseudo: PseudoElem) -> usize {
    GENERATED_ID_BIT
        | (element << 1)
        | match pseudo {
            PseudoElem::After => 1,
            PseudoElem::Marker => 1 << (usize::BITS - 1),
            PseudoElem::FirstLetter => (1 << (usize::BITS - 1)) | 1,
            _ => 0,
        }
}

/// The node id the inline engine gives image `index` (counted among the
/// images of the `content` list) of the `::before` or `::after` of
/// `element`; `None` for another pseudo-element, an index of
/// [`MAX_GENERATED_IMAGES`] or more, or an element too far into the arena to
/// encode.
#[doc(hidden)]
pub fn generated_image_node_id(element: usize, pseudo: PseudoElem, index: usize) -> Option<usize> {
    let after = match pseudo {
        PseudoElem::Before => 0,
        PseudoElem::After => 1,
        _ => return None,
    };
    if index >= MAX_GENERATED_IMAGES || element >= 1 << (usize::BITS - 4 - IMAGE_INDEX_BITS) {
        return None;
    }
    Some(
        GENERATED_ID_BIT
            | IMAGE_ID_BIT
            | (element << (IMAGE_INDEX_BITS + 1))
            | (index << 1)
            | after,
    )
}

/// The id of opacity group `index` of the `::first-letter` of `element`.
///
/// The fragments of one `::first-letter` keep the style they were given for
/// the text they wrap, so where its letters come from text with different
/// inherited values, the fragments can have different opacities, or lie in
/// different inline opacity groups, and so composite as separate groups.
/// Group 0 is named by the pseudo-element's own id of [`generated_node_id`];
/// the others by ids of their own, which [`generated_origin`] maps back to
/// the `::first-letter` of `element`. `None` for an index of
/// [`MAX_LETTER_GROUPS`] or more, or an element too far into the arena to
/// encode.
#[doc(hidden)]
pub fn first_letter_group_id(element: usize, index: usize) -> Option<usize> {
    if index == 0 {
        return Some(generated_node_id(element, PseudoElem::FirstLetter));
    }
    if index >= MAX_LETTER_GROUPS || element >= 1 << (usize::BITS - 3 - LETTER_GROUP_BITS) {
        return None;
    }
    let kind_bit = 1 << (usize::BITS - 1);
    Some(kind_bit | GENERATED_ID_BIT | IMAGE_ID_BIT | (element << LETTER_GROUP_BITS) | index)
}

/// The element, pseudo-element and image index an id of
/// [`generated_image_node_id`] stands for; `None` for any other id.
#[doc(hidden)]
pub fn generated_image_origin(id: usize) -> Option<(usize, PseudoElem, usize)> {
    let kind_bit = 1 << (usize::BITS - 1);
    if id & (kind_bit | GENERATED_ID_BIT | IMAGE_ID_BIT) != GENERATED_ID_BIT | IMAGE_ID_BIT {
        return None;
    }
    let raw = id & !(GENERATED_ID_BIT | IMAGE_ID_BIT);
    let pseudo = if raw & 1 == 1 {
        PseudoElem::After
    } else {
        PseudoElem::Before
    };
    let index = (raw >> 1) & (MAX_GENERATED_IMAGES - 1);
    Some((raw >> (IMAGE_INDEX_BITS + 1), pseudo, index))
}

/// The element and pseudo-element an id of [`generated_node_id`],
/// [`generated_image_node_id`] or [`first_letter_group_id`] stands for;
/// `None` for a document node id.
#[doc(hidden)]
pub fn generated_origin(id: usize) -> Option<(usize, PseudoElem)> {
    if id & GENERATED_ID_BIT == 0 {
        return None;
    }
    if let Some((element, pseudo, _)) = generated_image_origin(id) {
        return Some((element, pseudo));
    }
    let kind_bit = 1 << (usize::BITS - 1);
    let letter_group = kind_bit | GENERATED_ID_BIT | IMAGE_ID_BIT;
    if id & letter_group == letter_group {
        let raw = id & !letter_group;
        return Some((raw >> LETTER_GROUP_BITS, PseudoElem::FirstLetter));
    }
    let raw = id & !(GENERATED_ID_BIT | kind_bit);
    let pseudo = if id & kind_bit != 0 {
        if raw & 1 == 1 {
            PseudoElem::FirstLetter
        } else {
            PseudoElem::Marker
        }
    } else if raw & 1 == 1 {
        PseudoElem::After
    } else {
        PseudoElem::Before
    };
    Some((raw >> 1, pseudo))
}

/// The computed values of a document node, or of the pseudo-element an id of
/// [`generated_node_id`] stands for.
#[doc(hidden)]
pub fn computed_for_id(cascade: &CascadeResult, id: usize) -> Option<&ComputedValues> {
    match generated_origin(id) {
        Some((element, pseudo)) => cascade
            .pseudo
            .get(&(StyleNodeId::new(element as u64), pseudo))
            .map(|style| style.as_ref())
            .or_else(|| {
                (pseudo == PseudoElem::Marker)
                    .then(|| cascade.computed.get(element))
                    .flatten()
            }),
        None => cascade.computed.get(id),
    }
}

/// Whether the `pseudo` of `element` is a rendered, in-flow box whose content
/// can produce text or images: its `display` is not `none`, it is neither
/// floated nor out of flow, and its `content` is neither `none` nor `normal`.
/// The inline engine lays such content out in the paragraph of its element;
/// the painter draws every other generated content as an overlay.
#[doc(hidden)]
pub fn is_in_flow_generated_text(
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
) -> bool {
    if pseudo == PseudoElem::Marker {
        if !inside_marker_in_flow(cascade, element) {
            return false;
        }
        let cv = &cascade.computed[element];
        if cascade
            .pseudo
            .get(&(StyleNodeId::new(element as u64), pseudo))
            .is_some_and(|marker| !marker.content.is_empty())
        {
            return true;
        }
        return matches!(
            cv.list_style_image,
            raikiri_style::property::BackgroundImage::Url(_)
        ) || match &cv.list_style_type {
            raikiri_style::ListStyleType::None => false,
            raikiri_style::ListStyleType::String(text) => !text.is_empty(),
            _ => true,
        };
    }
    let Some(cv) = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), pseudo))
    else {
        return false;
    };
    cv.display != DisplayValue::None
        && cv.float == FloatValue::None
        && matches!(
            cv.position,
            PositionValue::Static | PositionValue::Relative | PositionValue::Sticky
        )
        && !cv.content.is_empty()
        && !cv
            .content
            .iter()
            .any(|part| matches!(part, ContentComponent::None))
        && cv.content.iter().any(|part| {
            matches!(
                part,
                ContentComponent::Literal(_)
                    | ContentComponent::Attr { .. }
                    | ContentComponent::AttrFallback { .. }
                    | ContentComponent::Counter { .. }
                    | ContentComponent::Counters { .. }
                    | ContentComponent::Quote(_)
                    | ContentComponent::Image { .. }
            )
        })
}

/// Whether the `pseudo` of `element` is in-flow generated text that makes
/// no line: an inline box whose `content` is only empty strings and that has
/// no margin, border or padding. It holds nothing a line could show, so it
/// neither is the first formatted line nor comes before content that is
/// (CSS 2.1 §9.4.2). A `content` that is resolved while the paragraph is laid
/// out, such as a counter, a quote or an attribute, is not judged here.
pub(crate) fn is_blank_inline_generated_text(
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
) -> bool {
    use raikiri_style::{ComputedLengthPercentageOrAuto, ComputedLengthPercentageOrCalc, Sides};
    is_in_flow_generated_text(cascade, element, pseudo)
        && cascade
            .pseudo
            .get(&(StyleNodeId::new(element as u64), pseudo))
            .is_some_and(|cv| {
                matches!(cv.display, DisplayValue::Inline | DisplayValue::Contents)
                    && cv.content.iter().all(
                        |part| matches!(part, ContentComponent::Literal(text) if text.is_empty()),
                    )
                    && cv.padding == Sides::all(ComputedLengthPercentageOrCalc::Px(0.0))
                    && cv.margin == Sides::all(ComputedLengthPercentageOrAuto::Px(0.0))
                    && [
                        &cv.border.top,
                        &cv.border.right,
                        &cv.border.bottom,
                        &cv.border.left,
                    ]
                    .iter()
                    .all(|side| side.width().0 == 0.0)
            })
}

/// Whether the `pseudo` of `element` is in-flow generated text that can make
/// a line: in-flow generated text (see [`is_in_flow_generated_text`]) other
/// than a blank inline box (see [`is_blank_inline_generated_text`]). Only
/// such text ends the search for the paragraph whose first line an ancestor's
/// `::first-line` and `::first-letter` reach.
pub(crate) fn is_line_making_generated_text(
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
) -> bool {
    is_in_flow_generated_text(cascade, element, pseudo)
        && !is_blank_inline_generated_text(cascade, element, pseudo)
}

/// Whether layout can place the images of the `::before` or `::after` of
/// `element`: the pseudo-element is in-flow generated content (see
/// [`is_in_flow_generated_text`]) and its element generates boxes, being in
/// the document with neither itself nor an ancestor `display: none`
/// (CSS Display 3 §2.5). Preparing or prefetching images that no box shows
/// would only spend resources that visible images need.
#[doc(hidden)]
pub fn content_images_are_placed(
    document: &Document,
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
) -> bool {
    if !is_in_flow_generated_text(cascade, element, pseudo)
        || !document
            .get_node(element)
            .is_some_and(|node| node.is_in_document())
    {
        return false;
    }
    !document.inclusive_ancestors(element).any(|id| {
        #[cfg(test)]
        tests::IMAGE_VISIBILITY_VISITS.with(|count| count.set(count.get() + 1));
        cascade
            .computed
            .get(id)
            .is_some_and(|cv| cv.display == DisplayValue::None)
    })
}

/// One piece of the content of a `::before` or `::after`, in order: a run
/// of text, or an image given by its index among the images of the `content`
/// list.
#[derive(Clone, Debug, PartialEq, Eq)]
#[doc(hidden)]
pub enum GeneratedPiece {
    /// Text of literals, attributes, counters and quotes.
    Text(String),
    /// An `<image>` (CSS Content 3 §2.2): an anonymous inline replaced
    /// element, counted among the images of the list.
    Image(usize),
}

/// The computed values and the content of the `::before` or `::after` of
/// `element` as text and images in order, with the counters resolved as in
/// [`generated_text`]. Adjacent text is merged; empty text is dropped.
#[doc(hidden)]
pub fn generated_pieces<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &CounterSnapshots,
) -> Option<(&'a ComputedValues, Vec<GeneratedPiece>)> {
    generated_pieces_impl(document, cascade, element, pseudo, snapshots)
}

/// Resolve generated content from compact snapshots without owned expansion.
#[doc(hidden)]
pub fn generated_pieces_compact<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &CompactCounterSnapshots,
) -> Option<(&'a ComputedValues, Vec<GeneratedPiece>)> {
    generated_pieces_impl(document, cascade, element, pseudo, snapshots)
}

fn generated_pieces_impl<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &impl SnapshotCollection,
) -> Option<(&'a ComputedValues, Vec<GeneratedPiece>)> {
    if !matches!(pseudo, PseudoElem::Before | PseudoElem::After) {
        return None;
    }
    let computed = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), pseudo))?;
    let counters = pseudo_counters(snapshots, element, pseudo, computed);
    let mut pieces = Vec::new();
    let tail = content_components_visit(
        document,
        element,
        &computed.content,
        &computed.quotes,
        computed.quotes_auto,
        &counters,
        &cascade.counter_styles,
        &mut |text, index| {
            if !text.is_empty() {
                pieces.push(GeneratedPiece::Text(std::mem::take(text)));
            }
            pieces.push(GeneratedPiece::Image(index));
        },
    )?;
    if !tail.is_empty() {
        pieces.push(GeneratedPiece::Text(tail));
    }
    Some((computed, pieces))
}

/// The computed values and the text of the `pseudo` of `element`, with the
/// counters in effect at the element (`snapshots`, from
/// [`crate::counter_snapshots`]) updated by the pseudo-element's own
/// directives. Components that are not text (images) contribute nothing.
#[doc(hidden)]
pub fn generated_text<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &CounterSnapshots,
) -> Option<(&'a ComputedValues, String)> {
    generated_text_impl(document, cascade, element, pseudo, snapshots)
}

/// Resolve generated content from compact snapshots without owned expansion.
#[doc(hidden)]
pub fn generated_text_compact<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &CompactCounterSnapshots,
) -> Option<(&'a ComputedValues, String)> {
    generated_text_impl(document, cascade, element, pseudo, snapshots)
}

fn generated_text_impl<'a>(
    document: &Document,
    cascade: &'a CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &impl SnapshotCollection,
) -> Option<(&'a ComputedValues, String)> {
    if pseudo == PseudoElem::Marker {
        return markers::marker_render_info_impl(document, cascade, element, snapshots);
    }
    let computed = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), pseudo))?;
    let counters = pseudo_counters(snapshots, element, pseudo, computed);
    let content = content_components_to_text_with_quotes(
        document,
        element,
        &computed.content,
        &computed.quotes,
        computed.quotes_auto,
        &counters,
        &cascade.counter_styles,
    )?;
    Some((computed, content))
}

/// The alternative text of the `content` of the `::before`, `::after` or
/// `::marker` of `element`, the components after `/` (CSS Content 3
/// <https://www.w3.org/TR/css-content-3/#alt>), with `attr()` and the
/// counters resolved as in [`generated_text`]. `None` when the value gives
/// no alternative text; an empty string marks decorative content.
#[doc(hidden)]
pub fn generated_alt_text(
    document: &Document,
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &CounterSnapshots,
) -> Option<String> {
    generated_alt_text_impl(document, cascade, element, pseudo, snapshots)
}

/// Resolve generated content from compact snapshots without owned expansion.
#[doc(hidden)]
pub fn generated_alt_text_compact(
    document: &Document,
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &CompactCounterSnapshots,
) -> Option<String> {
    generated_alt_text_impl(document, cascade, element, pseudo, snapshots)
}

fn generated_alt_text_impl(
    document: &Document,
    cascade: &CascadeResult,
    element: usize,
    pseudo: PseudoElem,
    snapshots: &impl SnapshotCollection,
) -> Option<String> {
    let computed = cascade
        .pseudo
        .get(&(StyleNodeId::new(element as u64), pseudo))?;
    let alt = computed.content.alt()?;
    let counters = pseudo_counters(snapshots, element, pseudo, computed);
    content_components_to_text_with_quotes(
        document,
        element,
        alt,
        &computed.quotes,
        computed.quotes_auto,
        &counters,
        &cascade.counter_styles,
    )
}

/// The counters that the content of the `pseudo` of `element` reads: the
/// walk's snapshot after the pseudo-element's own directives, or, when the
/// walk took none for it, the element's snapshot with the pseudo-element's
/// directives applied locally. The view also carries the quote depth at which
/// the content starts.
fn pseudo_counters<'a>(
    snapshots: &'a impl SnapshotCollection,
    element: usize,
    pseudo: PseudoElem,
    computed: &'a ComputedValues,
) -> CounterSnapshotView<'a> {
    let mut view = match snapshots.pseudo_view(element, pseudo) {
        Some(snapshot) => snapshot,
        None => {
            let mut view = snapshots
                .node_view(element)
                .unwrap_or_else(|| CounterSnapshotView::new(&EMPTY_COUNTER_SNAPSHOT, None));
            view.computed = Some(computed);
            view
        }
    };
    view.open_quotes = snapshots.quote_start(element, pseudo);
    view
}

static EMPTY_COUNTER_SNAPSHOT: std::sync::LazyLock<CounterSnapshot> =
    std::sync::LazyLock::new(std::collections::HashMap::new);

pub(crate) trait SnapshotCollection {
    fn node_view(&self, element: usize) -> Option<CounterSnapshotView<'_>>;
    fn pseudo_view(&self, element: usize, pseudo: PseudoElem) -> Option<CounterSnapshotView<'_>>;
    fn quote_start(&self, element: usize, pseudo: PseudoElem) -> usize;
}

impl SnapshotCollection for CounterSnapshots {
    fn node_view(&self, element: usize) -> Option<CounterSnapshotView<'_>> {
        self.get(element)
            .map(|snapshot| CounterSnapshotView::new(snapshot, None))
    }
    fn pseudo_view(&self, element: usize, pseudo: PseudoElem) -> Option<CounterSnapshotView<'_>> {
        self.pseudo(element, pseudo)
            .map(|snapshot| CounterSnapshotView::new(snapshot, None))
    }
    fn quote_start(&self, element: usize, pseudo: PseudoElem) -> usize {
        self.quote_depth(element, pseudo)
    }
}

impl SnapshotCollection for CompactCounterSnapshots {
    fn node_view(&self, element: usize) -> Option<CounterSnapshotView<'_>> {
        self.get(element)
    }
    fn pseudo_view(&self, element: usize, pseudo: PseudoElem) -> Option<CounterSnapshotView<'_>> {
        self.pseudo(element, pseudo)
    }
    fn quote_start(&self, element: usize, pseudo: PseudoElem) -> usize {
        self.quote_depth(element, pseudo)
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CounterStack<'a> {
    Slice(&'a [i32]),
    Prefix(&'a CompactCounterSnapshots, usize),
}

impl<'a> CounterStack<'a> {
    fn last(self) -> Option<i32> {
        match self {
            Self::Slice(values) => values.last().copied(),
            Self::Prefix(arena, head) => arena.last(head),
        }
    }
    fn is_empty(self) -> bool {
        self.last().is_none()
    }
    fn iter(self) -> CounterStackIter<'a> {
        match self {
            Self::Slice(values) => CounterStackIter::Slice(values.iter().copied()),
            Self::Prefix(arena, head) => CounterStackIter::Prefix(arena.values(head).into_iter()),
        }
    }
}

enum CounterStackIter<'a> {
    Slice(std::iter::Copied<std::slice::Iter<'a, i32>>),
    Prefix(std::vec::IntoIter<i32>),
}

impl Iterator for CounterStackIter<'_> {
    type Item = i32;
    fn next(&mut self) -> Option<i32> {
        match self {
            Self::Slice(values) => values.next(),
            Self::Prefix(values) => values.next(),
        }
    }
}

/// Read-only access to the effective stack for a counter name.
#[doc(hidden)]
pub trait CounterSnapshotLookup {
    /// Return the effective values for `name`, including pseudo-element
    /// directives when this lookup is a `CounterSnapshotView`.
    fn values_for(&self, name: &str) -> CounterValues<'_>;

    /// How many quotes are open where the content read through this lookup
    /// starts (CSS Content 3 §2.4.2). A lookup that knows no document
    /// position starts outside every quote.
    fn open_quotes(&self) -> usize {
        0
    }
}

/// A counter stack view that can replace or append its innermost value
/// without copying the underlying snapshot map or stack.
#[derive(Clone, Copy, Debug)]
#[doc(hidden)]
pub struct CounterValues<'a> {
    base: CounterStack<'a>,
    effective_last: Option<i32>,
    appended: bool,
}

impl<'a> CounterValues<'a> {
    fn unchanged(base: &'a [i32]) -> Self {
        Self::from_stack(CounterStack::Slice(base))
    }

    fn from_stack(base: CounterStack<'a>) -> Self {
        Self {
            base,
            effective_last: None,
            appended: false,
        }
    }

    fn with_directives(
        base: CounterStack<'a>,
        computed: Option<&ComputedValues>,
        name: &str,
    ) -> Self {
        let Some(computed) = computed else {
            return Self::from_stack(base);
        };
        let list_item = computed.display == DisplayValue::ListItem;
        let mut effective_last = None;
        let mut appended = false;
        if let Some((_, value)) = computed
            .counter_reset
            .iter()
            .rev()
            .find(|(counter_name, _)| counter_name.as_str() == name)
        {
            effective_last = Some(crate::target::local_reset_value(
                computed, list_item, name, *value,
            ));
            appended = true;
        }

        // A `display: list-item` pseudo increments list-item for its own
        // content as well as for the element's later children.
        let implicit = crate::target::local_implicit_list_item_increment(computed, list_item);
        for (counter_name, delta) in computed.counter_increment.iter().chain(implicit.iter()) {
            if counter_name.as_str() != name {
                continue;
            }
            effective_last = Some(
                effective_last
                    .or_else(|| base.last())
                    .map_or(*delta, |value| value.saturating_add(*delta)),
            );
            appended |= base.is_empty() && !appended;
        }
        for (counter_name, value) in computed.counter_set.iter() {
            if counter_name.as_str() != name {
                continue;
            }
            effective_last = Some(*value);
            appended |= base.is_empty() && !appended;
        }

        Self {
            base,
            effective_last,
            appended,
        }
    }

    /// The innermost effective value, if the counter exists.
    pub fn last(self) -> Option<i32> {
        self.effective_last.or_else(|| self.base.last())
    }

    /// Whether the effective stack has no values.
    pub fn is_empty(self) -> bool {
        self.base.is_empty() && self.effective_last.is_none()
    }

    /// Iterate the effective stack from outermost to innermost.
    pub fn iter(self) -> impl Iterator<Item = i32> + 'a {
        let replaced = self.effective_last.is_some() && !self.appended;
        let mut values = self.base.iter().peekable();
        std::iter::from_fn(move || {
            let value = values.next()?;
            if replaced && values.peek().is_none() {
                None
            } else {
                Some(value)
            }
        })
        .chain(self.effective_last)
    }
}

/// A borrowed counter snapshot with one pseudo-element's directives applied
/// lazily to each requested counter. Looking up generated content therefore
/// copies neither the whole map nor unaffected counter stacks.
#[doc(hidden)]
pub struct CounterSnapshotView<'a> {
    snapshot: SnapshotSource<'a>,
    computed: Option<&'a ComputedValues>,
    list_item_fallback: Option<CounterStack<'a>>,
    open_quotes: usize,
}

enum SnapshotSource<'a> {
    Owned(&'a CounterSnapshot),
    Compact(&'a CompactCounterSnapshots, usize),
}

impl<'a> CounterSnapshotView<'a> {
    /// Borrow `snapshot` and optionally apply pseudo-element counter
    /// directives on lookup.
    pub fn new(snapshot: &'a CounterSnapshot, computed: Option<&'a ComputedValues>) -> Self {
        Self {
            snapshot: SnapshotSource::Owned(snapshot),
            computed,
            list_item_fallback: None,
            open_quotes: 0,
        }
    }

    pub(crate) fn compact(snapshot: &'a CompactCounterSnapshots, root: usize) -> Self {
        Self {
            snapshot: SnapshotSource::Compact(snapshot, root),
            computed: None,
            list_item_fallback: None,
            open_quotes: 0,
        }
    }

    pub(crate) fn stack(&self, name: &str) -> CounterStack<'a> {
        match self.snapshot {
            SnapshotSource::Owned(snapshot) => CounterStack::Slice(
                snapshot
                    .get(&raikiri_traits::Symbol::new(name))
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            ),
            SnapshotSource::Compact(snapshot, root) => snapshot.stack(root, name),
        }
    }

    /// Start the content read through this view with `open_quotes` quotes
    /// open.
    pub(crate) fn starting_quotes_at(mut self, open_quotes: usize) -> Self {
        self.open_quotes = open_quotes;
        self
    }
}

impl CounterSnapshotLookup for CounterSnapshotView<'_> {
    fn values_for(&self, name: &str) -> CounterValues<'_> {
        let snapshot_values = self.stack(name);
        let base = if name == "list-item" && snapshot_values.is_empty() {
            self.list_item_fallback.unwrap_or(snapshot_values)
        } else {
            snapshot_values
        };
        CounterValues::with_directives(base, self.computed, name)
    }

    fn open_quotes(&self) -> usize {
        self.open_quotes
    }
}

impl CounterSnapshotLookup for CounterSnapshot {
    fn values_for(&self, name: &str) -> CounterValues<'_> {
        CounterValues::unchanged(
            self.get(&raikiri_traits::Symbol::new(name))
                .map(Vec::as_slice)
                .unwrap_or_default(),
        )
    }
}

/// Apply the `counter-reset`, `counter-increment` and `counter-set` of
/// `computed` (a pseudo-element) to a copy of the counters in effect at its
/// element, with the implicit list-item increment of a `display: list-item`
/// pseudo, as its [`CounterSnapshotView`] does.
#[doc(hidden)]
pub fn apply_counter_directives_to_snapshot(
    snapshot: &mut CounterSnapshot,
    computed: &raikiri_style::ComputedValues,
) {
    // Apply pseudo directives to the local content snapshot. `::before`
    // scope propagation to descendants is handled by
    // `raikiri_dom::counter_snapshots`; this clone resolves the pseudo's own
    // generated content before the scope is used by later real children.
    let mut reset_values = std::collections::HashMap::new();
    for (name, value) in computed.counter_reset.iter() {
        reset_values.insert(
            name.as_str(),
            crate::target::local_reset_value(
                computed,
                computed.display == DisplayValue::ListItem,
                name,
                *value,
            ),
        );
    }
    for (name, value) in reset_values {
        snapshot
            .entry(raikiri_traits::Symbol::new(name))
            .or_default()
            .push(value);
    }
    let implicit = crate::target::local_implicit_list_item_increment(
        computed,
        computed.display == DisplayValue::ListItem,
    );
    for (name, delta) in computed.counter_increment.iter().chain(implicit.iter()) {
        let stack = snapshot
            .entry(raikiri_traits::Symbol::new(name.as_str()))
            .or_default();
        if let Some(top) = stack.last_mut() {
            *top = top.saturating_add(*delta);
        } else {
            stack.push(*delta);
        }
    }
    for (name, value) in computed.counter_set.iter() {
        let stack = snapshot
            .entry(raikiri_traits::Symbol::new(name.as_str()))
            .or_default();
        if let Some(top) = stack.last_mut() {
            *top = *value;
        } else {
            stack.push(*value);
        }
    }
}

/// The innermost value of the counter `name` in `style`, `0` when the
/// counter does not exist (`counter()`).
#[doc(hidden)]
pub fn format_counter_component<C: CounterSnapshotLookup + ?Sized>(
    snapshot: &C,
    name: &str,
    style: &CounterStyle,
    registry: &CounterStyleRegistry,
) -> String {
    let value = snapshot.values_for(name).last().unwrap_or(0);
    format_counter(value, style, registry)
}

/// Every value of the counter `name`, outermost first, in `style` and joined
/// by `separator` (`counters()`).
#[doc(hidden)]
pub fn format_counters_component<C: CounterSnapshotLookup + ?Sized>(
    snapshot: &C,
    name: &str,
    separator: &str,
    style: &CounterStyle,
    registry: &CounterStyleRegistry,
) -> String {
    let values = snapshot.values_for(name);
    if values.is_empty() {
        String::new()
    } else {
        values
            .iter()
            .map(|value| format_counter(value, style, registry))
            .collect::<Vec<_>>()
            .join(separator)
    }
}

/// The `quotes: auto` fallback glyph for one nesting depth: open quotes
/// use outer marks at depth zero and inner marks below, and likewise for
/// close quotes.
fn fallback_quote(depth: usize, open: bool) -> &'static str {
    if open {
        if depth == 0 { "“" } else { "‘" }
    } else if depth == 0 {
        "”"
    } else {
        "’"
    }
}

/// The quote nesting depth that generated text has reached, shared by normal
/// content, list markers and page-margin boxes so the three resolve quotes
/// alike.
///
/// The depth counts the `open-quote`s of the generated text before a quote,
/// less its `close-quote`s (CSS Content 3 §2.4.2). The document walk of
/// [`crate::counter_snapshots`] carries it across pseudo-elements; a margin
/// box is outside the document's text and starts at zero.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct QuoteDepth(usize);

impl QuoteDepth {
    /// A depth that starts at `depth` open quotes.
    pub(crate) fn starting_at(depth: usize) -> Self {
        Self(depth)
    }

    /// How many quotes are open.
    pub(crate) fn open_quotes(self) -> usize {
        self.0
    }

    /// Move the depth as `keyword` says and return the nesting level whose
    /// pair the keyword inserts, or `None` when it inserts nothing.
    ///
    /// `open-quote` and `no-open-quote` deepen the nesting. `close-quote` and
    /// `no-close-quote` make it shallower, and where no quote is open they do
    /// nothing at all: a stray close inserts no mark and leaves the depth at
    /// zero (CSS 2 §12.3.2). The `no-` forms insert nothing.
    pub(crate) fn advance(&mut self, keyword: QuoteKeyword) -> Option<usize> {
        let opens = matches!(keyword, QuoteKeyword::OpenQuote | QuoteKeyword::NoOpenQuote);
        let closes = matches!(
            keyword,
            QuoteKeyword::CloseQuote | QuoteKeyword::NoCloseQuote
        );
        let inserts = matches!(keyword, QuoteKeyword::OpenQuote | QuoteKeyword::CloseQuote);
        if closes {
            self.0 = self.0.checked_sub(1)?;
        }
        let level = self.0;
        if opens {
            self.0 = level.saturating_add(1);
        }
        inserts.then_some(level)
    }

    /// The text `keyword` inserts at the current depth, moving the depth as
    /// [`Self::advance`] says (CSS Content 3 §2.4.2).
    ///
    /// A depth past the last pair of an explicit `quotes` list repeats that
    /// pair. An empty list is `auto` when `quotes_auto` is set and `none`
    /// otherwise.
    pub(crate) fn apply<'q, T: AsRef<str>>(
        &mut self,
        keyword: QuoteKeyword,
        quotes: &'q [(T, T)],
        quotes_auto: bool,
    ) -> &'q str {
        let Some(level) = self.advance(keyword) else {
            return "";
        };
        let opens = keyword == QuoteKeyword::OpenQuote;
        match quotes.get(level).or_else(|| quotes.last()) {
            Some((open, _)) if opens => open.as_ref(),
            Some((_, close)) => close.as_ref(),
            None if quotes_auto => fallback_quote(level, opens),
            None => "",
        }
    }
}

/// The text of a `content` value: literals, attributes of `node_id`,
/// counters and quotes; other components (images) contribute nothing.
/// `None` for an empty value.
#[doc(hidden)]
pub fn content_components_to_text_with_quotes<T: AsRef<str>, C: CounterSnapshotLookup + ?Sized>(
    document: &Document,
    node_id: usize,
    components: &[ContentComponent],
    quotes: &[(T, T)],
    quotes_auto: bool,
    counters: &C,
    registry: &CounterStyleRegistry,
) -> Option<String> {
    content_components_visit(
        document,
        node_id,
        components,
        quotes,
        quotes_auto,
        counters,
        registry,
        &mut |_, _| {},
    )
}

/// The text of a `content` value as [`content_components_to_text_with_quotes`]
/// builds it, calling `image` at each image with the text built so far and
/// the index of the image among the images of the list. The text left after
/// the last call is returned.
#[allow(clippy::too_many_arguments)]
fn content_components_visit<T: AsRef<str>, C: CounterSnapshotLookup + ?Sized>(
    document: &Document,
    node_id: usize,
    components: &[ContentComponent],
    quotes: &[(T, T)],
    quotes_auto: bool,
    counters: &C,
    registry: &CounterStyleRegistry,
    image: &mut dyn FnMut(&mut String, usize),
) -> Option<String> {
    if components.is_empty() {
        return None;
    }
    let mut text = String::new();
    let mut depth = QuoteDepth::starting_at(counters.open_quotes());
    let mut images = 0_usize;
    for component in components {
        match component {
            ContentComponent::Image { .. } => {
                image(&mut text, images);
                images += 1;
            }
            ContentComponent::Literal(value) => text.push_str(value.as_str()),
            ContentComponent::Attr { name } => {
                if let Some(node) = document.get_node(node_id) {
                    text.push_str(node.attribute(name.as_str()).unwrap_or_default());
                }
            }
            ContentComponent::AttrFallback { name, fallback } => {
                let value = document
                    .get_node(node_id)
                    .and_then(|node| node.attribute(name.as_str()))
                    .map(str::to_owned)
                    .or_else(|| fallback.as_ref().map(|value| value.to_string()))
                    .unwrap_or_default();
                text.push_str(&value);
            }
            ContentComponent::Counter { name, style } => {
                text.push_str(&format_counter_component(
                    counters,
                    name.as_str(),
                    style,
                    registry,
                ));
            }
            ContentComponent::Counters {
                name,
                separator,
                style,
            } => {
                text.push_str(&format_counters_component(
                    counters,
                    name.as_str(),
                    separator.as_str(),
                    style,
                    registry,
                ));
            }
            ContentComponent::Quote(keyword) => {
                text.push_str(depth.apply(*keyword, quotes, quotes_auto));
            }
            _ => {}
        }
    }
    Some(text)
}

/// `value` in the counter style `style`: the roman styles, a custom
/// `@counter-style` of `registry`, else decimal.
#[doc(hidden)]
pub fn format_counter(value: i32, style: &CounterStyle, registry: &CounterStyleRegistry) -> String {
    match style {
        CounterStyle::Named(name) if name.as_str().eq_ignore_ascii_case("lower-roman") => {
            if value <= 0 {
                return value.to_string();
            }
            let mut n = value;
            let mut result = String::new();
            for (unit, glyph) in [
                (1000, "m"),
                (900, "cm"),
                (500, "d"),
                (400, "cd"),
                (100, "c"),
                (90, "xc"),
                (50, "l"),
                (40, "xl"),
                (10, "x"),
                (9, "ix"),
                (5, "v"),
                (4, "iv"),
                (1, "i"),
            ] {
                while n >= unit {
                    result.push_str(glyph);
                    n -= unit;
                }
            }
            result
        }
        CounterStyle::Named(name) if name.as_str().eq_ignore_ascii_case("upper-roman") => {
            format_counter(value, &CounterStyle::Named("lower-roman".into()), registry)
                .to_uppercase()
        }
        CounterStyle::Named(name) => resolve_custom_counter(registry, name.as_str(), value)
            .unwrap_or_else(|| value.to_string()),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod tests;

/// Whether this list item's marker belongs to its inline formatting context.
#[doc(hidden)]
pub fn inside_marker_in_flow(cascade: &CascadeResult, element: usize) -> bool {
    marker_is_enabled(cascade, element)
        && cascade.computed[element].list_style_position == raikiri_style::ListStylePosition::Inside
}

/// Whether a list item generates a marker, independently of its position.
#[doc(hidden)]
pub fn marker_is_enabled(cascade: &CascadeResult, element: usize) -> bool {
    cascade.computed.get(element).is_some_and(|cv| {
        cv.display == DisplayValue::ListItem
            && cascade
                .pseudo
                .get(&(StyleNodeId::new(element as u64), PseudoElem::Marker))
                .is_none_or(|marker| {
                    marker.display != DisplayValue::None
                        && !marker
                            .content
                            .iter()
                            .any(|part| matches!(part, ContentComponent::None))
                })
    })
}
