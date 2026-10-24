//! The one place that reads the style getters of a box.
//!
//! The text of a layout says where the boxes went. It cannot say that the
//! style a box was laid out with was read the same way: a getter that maps a
//! value of a rarely used keyword to the wrong one moves nothing in the
//! documents at hand and still moves boxes in the next. So the text also holds
//! what the getters of the style view return for every box, in the bits of
//! their numbers.
//!
//! The tool is built from one copy of its sources against the merge-base and
//! the head checkout, so everything it reads has to exist, with the same
//! meaning, in both. The view and the way it is reached are internal to the
//! DOM crate and may change under a change being tested, so every call of a
//! getter lives here and nowhere else: when the getters change shape, this file
//! is the only one to adapt, and it must then still compile against the
//! merge-base's. `build.rs` looks at the DOM crate's sources to say which of
//! the two this build has, as the `style_view_reader` cfg. A tree that has no
//! reader writes a marker line instead of the getters, so it differs from one
//! that has.
//!
//! # What is read
//!
//! A box is read as its last layout left it: the computed values, the facts
//! the DOM recorded for the box, and the overrides that were still in force
//! when layout returned. Layout reads a box through a role (`ReadRole` in the
//! DOM crate), each of which is one of the style traits below and a context
//! that tells how the box is projected:
//!
//! | role | traits | context that matters |
//! | --- | --- | --- |
//! | core container | core | containing-block claims prepared |
//! | block container, block item | core, block container, block item | geometry, percentage basis |
//! | flex container, flex item | core, flexbox container, flexbox item | geometry, align axis |
//! | grid container, grid item | core, grid container, grid item | geometry |
//! | out-of-flow item | core, out-of-flow item | geometry |
//! | physical, pagination | core | claims by position only |
//!
//! Every getter of every trait is written, and the ones whose result depends on
//! the context are written once per context that can change them: the writing
//! mode and projection the axes are read in, the basis percentage edges are
//! resolved against, the containing-block policy, the axis alignment is read
//! along. The scoped overrides and the frozen frame inputs of a vertical block
//! are state of a running layout; they are not part of the text.
//!
//! Lengths are written by what they are, with `f32`s through
//! [`crate::dump::number`]. A calc has no number of its own, only the address
//! of its payload, which differs from run to run; it is written as the values
//! it resolves to against a few bases.

use raikiri_dom::Document;
use raikiri_style::CascadeResult;

/// Writes the style getters of box `index` of `document`, indented under the
/// line of its node. A node with no style record writes nothing.
#[cfg(style_view_reader)]
pub(crate) fn write(out: &mut String, document: &Document, cascade: &CascadeResult, index: usize) {
    document.read_finished_style(cascade, index, |view| reader::write(out, &view));
}

/// Writes that this tree has no way to read the getters.
#[cfg(not(style_view_reader))]
pub(crate) fn write(out: &mut String, _: &Document, _: &CascadeResult, _: usize) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "  styles: unavailable");
}

#[cfg(style_view_reader)]
mod reader {
    use std::fmt::Write as _;

    use raikiri_layout_style::{
        BridgeProjection, ClaimsPolicy, LayoutStyleView, PercentBasis, ViewContext,
    };
    use raikiri_style::property::WritingMode;
    // Every Taffy name of this file is marked. The getters are the methods of
    // Taffy's style traits and what they return are Taffy's values, so this is
    // where they are named, and nowhere else in the tool.
    use taffy::OofItemStyle as OofItem; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{BlockContainerStyle as BlockContainer, BlockItemStyle as BlockItem, CoreStyle}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{Dimension, LengthPercentage, LengthPercentageAuto, TrackSizingFunction}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{ExpandedDimension, ExpandedLengthPercentage, ExpandedLengthPercentageAuto}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{ExpandedMaxTrackSizingFunction, ExpandedMinTrackSizingFunction}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{FlexboxContainerStyle as FlexContainer, FlexboxItemStyle as FlexItem}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{GenericGridTemplateComponent, GenericRepetition}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{GridContainerStyle as GridContainer, GridItemStyle as GridItem}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters
    use taffy::{MaxTrackSizingFunction, MinTrackSizingFunction}; // taffy-boundary-lint:ignore: the dump reads Taffy-typed getters

    use crate::dump::number;

    /// The bases a calc is resolved against: none, a hundred, and a fraction
    /// that makes a clamp or a rounding show.
    const CALC_BASES: [f32; 3] = [0.0, 100.0, 250.5];

    /// The writing modes the axes of a box are read in.
    const WRITING_MODES: [WritingMode; 5] = [
        WritingMode::HorizontalTb,
        WritingMode::VerticalRl,
        WritingMode::VerticalLr,
        WritingMode::SidewaysRl,
        WritingMode::SidewaysLr,
    ];

    /// The projections applied to the axes after the writing mode.
    const PROJECTIONS: [BridgeProjection; 3] = [
        BridgeProjection::Identity,
        BridgeProjection::BlockTranspose {
            right_to_left: false,
        },
        BridgeProjection::BlockTranspose {
            right_to_left: true,
        },
    ];

    /// The bases percentage edges are resolved against instead of Taffy's own.
    const EDGE_BASES: [PercentBasis; 2] = [PercentBasis::Definite(137.5), PercentBasis::Indefinite];

    /// The basis a block item resolves its block edges against, apart from the
    /// others so that it shows where it is the one that counts.
    const BLOCK_ITEM_BASIS: f32 = 91.25;

    /// `(width, height)` of a size, without naming its type.
    macro_rules! size {
        ($size:expr) => {{
            let size = $size;
            (size.width, size.height)
        }};
    }

    /// `[left, right, top, bottom]` of a rectangle, without naming its type.
    macro_rules! rect {
        ($rect:expr) => {{
            let rect = $rect;
            [rect.left, rect.right, rect.top, rect.bottom]
        }};
    }

    /// Writes every getter of `view`, each group on a line of its own.
    pub(super) fn write(out: &mut String, view: &LayoutStyleView<'_>) {
        let text = Text { view: *view };
        facts(out, &text);
        block(out, &text);
        flex(out, &text);
        grid(out, &text);
        geometry(out, &text);
        edges(out, &text);
    }

    /// The getters that need no context: what the box is, how it positions and
    /// aligns itself and its children.
    fn facts(out: &mut String, text: &Text<'_>) {
        let view = &text.view;
        let _ = writeln!(
            out,
            "  core mode={:?} block={} compressible_replaced={} replaced={} box_sizing={:?} \
             position={:?} contain={:?} scrollbar_width={}",
            view.box_generation_mode(),
            view.is_block(),
            view.is_compressible_replaced(),
            view.is_replaced(),
            view.box_sizing(),
            view.position(),
            view.contain(),
            number(view.scrollbar_width()),
        );
        // The core container role asks for the prepared claims; every other
        // role, and a view made for flex and grid, for the position alone.
        let claims = |policy, flex_grid: bool| {
            let mut context = view.context();
            context.claims_policy = policy;
            let view = view.with_context(context);
            let view = if flex_grid {
                view.for_flex_grid()
            } else {
                view
            };
            view.is_containing_block()
        };
        let _ = writeln!(
            out,
            "  claims prepared={:?} positioning={:?} flex_grid={:?}",
            claims(ClaimsPolicy::Prepared, false),
            claims(ClaimsPolicy::Positioning, false),
            claims(ClaimsPolicy::Prepared, true),
        );
        let _ = writeln!(
            out,
            "  oof_item align_self={:?} justify_self={:?} table={} grid_row={:?} grid_column={:?}",
            OofItem::align_self(view),
            OofItem::justify_self(view),
            OofItem::is_table(view),
            OofItem::grid_row(view),
            OofItem::grid_column(view),
        );
        let _ = writeln!(
            out,
            "  bridge align_self={:?} justify_self={:?} align_content={:?}",
            view.bridge_align_self(),
            view.bridge_justify_self(),
            view.bridge_align_content(),
        );
    }

    fn block(out: &mut String, text: &Text<'_>) {
        let view = &text.view;
        let _ = writeln!(
            out,
            "  block_container text_align={:?} align_content={:?} justify_items={:?}",
            BlockContainer::text_align(view),
            BlockContainer::align_content(view),
            BlockContainer::justify_items(view),
        );
        let _ = writeln!(
            out,
            "  block_item table={} float={:?} clear={:?} own_clear={:?}",
            BlockItem::is_table(view),
            BlockItem::float(view),
            BlockItem::clear(view),
            view.clear(),
        );
    }

    fn flex(out: &mut String, text: &Text<'_>) {
        let view = &text.view;
        let _ = writeln!(
            out,
            "  flex_container direction={:?} wrap={:?} gap={} align_content={:?} \
             justify_content={:?} align_items={:?}",
            FlexContainer::flex_direction(view),
            FlexContainer::flex_wrap(view),
            text.pair(size!(FlexContainer::gap(view)), Text::length_percentage),
            FlexContainer::align_content(view),
            FlexContainer::justify_content(view),
            FlexContainer::align_items(view),
        );
        // Along the block axis the item aligns itself as the flex role asks;
        // along the inline axis, when the role has been told to.
        let along = |inline: bool| {
            let mut context = view.context();
            context.align_axis_is_inline = inline;
            FlexItem::align_self(&view.with_context(context))
        };
        let _ = writeln!(
            out,
            "  flex_item basis={} grow={} shrink={} align_self={:?} inline_align_self={:?}",
            text.dimension(FlexItem::flex_basis(view)),
            number(FlexItem::flex_grow(view)),
            number(FlexItem::flex_shrink(view)),
            along(false),
            along(true),
        );
    }

    fn grid(out: &mut String, text: &Text<'_>) {
        let view = &text.view;
        let areas: Vec<_> = GridContainer::grid_template_areas(view)
            .into_iter()
            .flatten()
            .collect();
        let _ = writeln!(
            out,
            "  grid_container auto_flow={:?} gap={} align_content={:?} justify_content={:?} \
             align_items={:?} justify_items={:?} area_rows={} area_columns={} areas={areas:?}",
            GridContainer::grid_auto_flow(view),
            text.pair(size!(GridContainer::gap(view)), Text::length_percentage),
            GridContainer::align_content(view),
            GridContainer::justify_content(view),
            GridContainer::align_items(view),
            GridContainer::justify_items(view),
            GridContainer::grid_template_area_row_count(view),
            GridContainer::grid_template_area_column_count(view),
        );
        for (axis, template, names, auto) in [
            (
                "rows",
                GridContainer::grid_template_rows(view),
                GridContainer::grid_template_row_names(view),
                text.tracks(GridContainer::grid_auto_rows(view)),
            ),
            (
                "columns",
                GridContainer::grid_template_columns(view),
                GridContainer::grid_template_column_names(view),
                text.tracks(GridContainer::grid_auto_columns(view)),
            ),
        ] {
            let components: Vec<String> = template
                .into_iter()
                .flatten()
                .map(|component| match component {
                    GenericGridTemplateComponent::Single(track) => text.track(track),
                    GenericGridTemplateComponent::Repeat(repetition) => format!(
                        "repeat({:?} {} names={:?})",
                        repetition.count(),
                        text.tracks(repetition.tracks()),
                        repetition
                            .lines_names()
                            .map(|set| set.collect::<Vec<_>>())
                            .collect::<Vec<_>>(),
                    ),
                })
                .collect();
            let names: Vec<Vec<_>> = names
                .into_iter()
                .flatten()
                .map(|set| set.collect())
                .collect();
            let _ = writeln!(
                out,
                "  grid_{axis} template=[{}] names={names:?} auto={auto}",
                components.join(" "),
            );
        }
        let _ = writeln!(
            out,
            "  grid_item row={:?} column={:?} align_self={:?} justify_self={:?}",
            GridItem::grid_row(view),
            GridItem::grid_column(view),
            GridItem::align_self(view),
            GridItem::justify_self(view),
        );
    }

    /// The getters whose axes depend on how the box is read: its size and
    /// limits, its edges, its overflow and its direction, once for every
    /// writing mode and projection.
    fn geometry(out: &mut String, text: &Text<'_>) {
        for writing_mode in WRITING_MODES {
            for projection in PROJECTIONS {
                let mut context = text.view.context();
                context.layout_wm = writing_mode;
                context.bridge_projection = projection;
                let view = text.view.with_context(context);
                let text = Text { view };
                let view = &text.view;
                let overflow = CoreStyle::overflow(view);
                let _ = writeln!(
                    out,
                    "  geometry {writing_mode:?} {projection:?} direction={:?} size={} min={} \
                     max={} aspect_ratio={} margin={} inset={} padding={} border={} \
                     overflow=[{:?} {:?}]",
                    CoreStyle::direction(view),
                    text.pair(size!(CoreStyle::size(view)), Text::dimension),
                    text.pair(
                        size!(CoreStyle::min_size(view)),
                        Text::length_percentage_auto
                    ),
                    text.pair(
                        size!(CoreStyle::max_size(view)),
                        Text::length_percentage_auto
                    ),
                    CoreStyle::aspect_ratio(view).map_or_else(|| "-".to_owned(), number),
                    text.edges(rect!(CoreStyle::margin(view)), Text::length_percentage_auto),
                    text.edges(rect!(CoreStyle::inset(view)), Text::length_percentage_auto),
                    text.edges(rect!(CoreStyle::padding(view)), Text::length_percentage),
                    text.edges(rect!(CoreStyle::border(view)), Text::length_percentage),
                    overflow.x,
                    overflow.y,
                );
            }
        }
    }

    /// The padding and border a role resolves itself instead of leaving to
    /// Taffy: against a basis it was given, against the basis of a block item,
    /// or from padding that was already resolved.
    fn edges(out: &mut String, text: &Text<'_>) {
        let mut variants: Vec<(String, ViewContext)> = Vec::new();
        for writing_mode in [WritingMode::HorizontalTb, WritingMode::VerticalRl] {
            for basis in EDGE_BASES {
                let mut context = text.view.context();
                context.layout_wm = writing_mode;
                context.percent_basis = basis;
                variants.push((format!("{writing_mode:?} {basis:?}"), context));
            }
            let mut context = text.view.context();
            context.layout_wm = writing_mode;
            context.block_item_basis = Some(BLOCK_ITEM_BASIS);
            variants.push((
                format!("{writing_mode:?} block_item_basis={BLOCK_ITEM_BASIS}"),
                context,
            ));
        }
        // Padding that a frame has already resolved: a different length on
        // every side.
        let mut resolved = CoreStyle::padding(&text.view).map(|_| 0.0);
        (resolved.left, resolved.right, resolved.top, resolved.bottom) = (1.5, 2.5, 3.5, 4.5);
        let mut context = text.view.context();
        context.resolved_padding = Some(resolved);
        variants.push(("resolved_padding".to_owned(), context));
        for (label, context) in variants {
            let view = text.view.with_context(context);
            let text = Text { view };
            let _ = writeln!(
                out,
                "  edges {label} padding={} border={}",
                text.edges(rect!(CoreStyle::padding(&view)), Text::length_percentage),
                text.edges(rect!(CoreStyle::border(&view)), Text::length_percentage),
            );
        }
    }

    /// Writes the values of one view as text.
    pub(super) struct Text<'a> {
        pub(super) view: LayoutStyleView<'a>,
    }

    impl Text<'_> {
        /// `[first second]`, each written by `side`.
        fn pair<T>(&self, (first, second): (T, T), side: fn(&Self, T) -> String) -> String {
            format!("[{} {}]", side(self, first), side(self, second))
        }

        /// `[left right top bottom]`, each written by `side`.
        fn edges<T>(&self, sides: [T; 4], side: fn(&Self, T) -> String) -> String {
            let sides: Vec<String> = sides.into_iter().map(|value| side(self, value)).collect();
            format!("[{}]", sides.join(" "))
        }

        /// What a calc handle resolves to against each of [`CALC_BASES`].
        fn calc(&self, kind: &str, handle: *const ()) -> String {
            let resolved: Vec<String> = CALC_BASES
                .iter()
                .map(|&basis| {
                    self.view
                        .resolve_calc_handle(handle, basis)
                        .map_or_else(|| "?".to_owned(), number)
                })
                .collect();
            format!("{kind}[{}]", resolved.join(" "))
        }

        pub(super) fn dimension(&self, value: Dimension) -> String {
            use ExpandedDimension as D;
            match value.expand() {
                D::Length(v) => format!("px({})", number(v)),
                D::Percent(v) => format!("percent({})", number(v)),
                D::Auto => "auto".to_owned(),
                D::MinContent => "min-content".to_owned(),
                D::MaxContent => "max-content".to_owned(),
                D::FitContentPx(v) => format!("fit-content-px({})", number(v)),
                D::FitContentPercent(v) => format!("fit-content-percent({})", number(v)),
                D::FitContentCalc(handle) => self.calc("fit-content-calc", handle),
                D::FitContent => "fit-content".to_owned(),
                D::Stretch => "stretch".to_owned(),
                D::Content => "content".to_owned(),
                D::Calc(handle) => self.calc("calc", handle),
            }
        }

        pub(super) fn length_percentage_auto(&self, value: LengthPercentageAuto) -> String {
            use ExpandedLengthPercentageAuto as L;
            match value.expand() {
                L::Length(v) => format!("px({})", number(v)),
                L::Percent(v) => format!("percent({})", number(v)),
                L::Auto => "auto".to_owned(),
                L::Calc(handle) => self.calc("calc", handle),
            }
        }

        pub(super) fn length_percentage(&self, value: LengthPercentage) -> String {
            use ExpandedLengthPercentage as L;
            match value.expand() {
                L::Length(v) => format!("px({})", number(v)),
                L::Percent(v) => format!("percent({})", number(v)),
                L::Calc(handle) => self.calc("calc", handle),
            }
        }

        pub(super) fn min_track(&self, value: MinTrackSizingFunction) -> String {
            use ExpandedMinTrackSizingFunction as M;
            match value.expand() {
                M::Length(v) => format!("px({})", number(v)),
                M::Percent(v) => format!("percent({})", number(v)),
                M::Auto => "auto".to_owned(),
                M::MinContent => "min-content".to_owned(),
                M::MaxContent => "max-content".to_owned(),
                M::Calc(handle) => self.calc("calc", handle),
            }
        }

        pub(super) fn max_track(&self, value: MaxTrackSizingFunction) -> String {
            use ExpandedMaxTrackSizingFunction as M;
            match value.expand() {
                M::Length(v) => format!("px({})", number(v)),
                M::Percent(v) => format!("percent({})", number(v)),
                M::Auto => "auto".to_owned(),
                M::MinContent => "min-content".to_owned(),
                M::MaxContent => "max-content".to_owned(),
                M::FitContentPx(v) => format!("fit-content-px({})", number(v)),
                M::FitContentPercent(v) => format!("fit-content-percent({})", number(v)),
                M::FitContentCalc(handle) => self.calc("fit-content-calc", handle),
                M::Fr(v) => format!("fr({})", number(v)),
                M::Calc(handle) => self.calc("calc", handle),
            }
        }

        /// `minmax(min max)` of one track.
        pub(super) fn track(&self, track: TrackSizingFunction) -> String {
            format!(
                "minmax({} {})",
                self.min_track(track.min),
                self.max_track(track.max)
            )
        }

        /// The tracks of an implicit list or of a repeat, in brackets.
        pub(super) fn tracks(&self, tracks: impl Iterator<Item = TrackSizingFunction>) -> String {
            let tracks: Vec<String> = tracks.map(|track| self.track(track)).collect();
            format!("[{}]", tracks.join(" "))
        }
    }
}

#[cfg(all(test, style_view_reader))]
mod tests;
