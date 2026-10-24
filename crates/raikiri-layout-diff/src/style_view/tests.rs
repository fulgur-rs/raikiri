use raikiri_html::{
    LayoutConfig, LayoutOptions, LayoutStatus, MediaContext, MediaType, PageDefaults,
    RenderResources, parse_html_with_resources,
};
use raikiri_layout_style::LayoutStyleView;
use raikiri_traits::PageBox;
use taffy::{CoreStyle, Dimension, LengthPercentage, LengthPercentageAuto}; // taffy-boundary-lint:ignore: the tests build the Taffy values the getters return
use taffy::{MaxTrackSizingFunction, MinMax, MinTrackSizingFunction}; // taffy-boundary-lint:ignore: the tests build the Taffy values the getters return

use super::reader::Text;
use crate::tests::{setup, text_of};
use crate::{PAGE_SIZE, PlaceholderImages};

/// A document with a box for each kind of getter: a column-reverse flex
/// container with gaps and an item that has calc sizes, a grid container with
/// named areas and every kind of track, and a float that claims containing
/// blocks.
const PROBE: &str = "<!doctype html><html><head><style>
body { margin: 0; font: 10px/12px Ahem }
#f { display: flex; flex-direction: column-reverse; gap: 3px 5%; align-items: center }
#c { flex: 2 1 calc(50% + 10px); width: calc(25% + 4px); padding: 10%; margin: 1px 2px 3px 4px;
     align-self: flex-end }
#g { display: grid; grid-template-columns: [a] 10px repeat(2, minmax(5px, 1fr)) fit-content(20px);
     grid-auto-rows: min-content; grid-template-areas: 'x y' 'z w' }
#t { transform: scale(1); float: left; clear: both }
</style></head><body>
<div id=f><div id=c>a</div></div><div id=g><span>b</span></div><div id=t>c</div>
</body></html>";

/// Runs `read` on the style view of the first `<main>` element of `html`, as
/// the layout of the document left it.
fn with_main<R>(html: &str, read: impl FnOnce(LayoutStyleView<'_>) -> R) -> R {
    let fonts = setup().fonts.expect("the test font");
    let images = PlaceholderImages::new();
    let resources = RenderResources::new()
        .image_pixel_source(&images)
        .fonts(fonts);
    let document = parse_html_with_resources(html.as_bytes(), &resources).expect("a document");
    let mut page_box = PageBox::new();
    (page_box.width, page_box.height) = (PAGE_SIZE.0 as f32, PAGE_SIZE.1 as f32);
    let status = raikiri_html::layout(
        &document,
        PageDefaults::builder().page_box(page_box).build(),
        LayoutConfig::builder()
            .media_context(MediaContext::with_viewport(
                MediaType::Print,
                PAGE_SIZE.0,
                PAGE_SIZE.1,
            ))
            .build(),
        LayoutOptions::new().resources(&resources),
    );
    let Ok(LayoutStatus::Completed(layout)) = status else {
        panic!("the document lays out");
    };
    let page = layout.pages().next().expect("a page");
    let (document, cascade, ..) = page.paint_inputs();
    let index = (0..document.node_count())
        .find(|&index| {
            document
                .get_node(index)
                .is_some_and(|node| node.tag_name() == Some("main"))
        })
        .expect("a <main> element");
    document
        .read_finished_style(cascade, index, read)
        .expect("the box has a style record")
}

/// The text of a `<main>` element that has the calc `width: calc(5px + 10%)`,
/// which resolves to 5, 15 and 30.05 against the bases the text uses.
const CALC: &str = "calc[5.0 15.0 30.05]";

/// Runs `check` on the writer of a `<main>` with a calc width, and on the
/// handle of that calc.
fn with_calc(check: impl FnOnce(&Text<'_>, *const ())) {
    with_main("<main style='width: calc(5px + 10%)'>x</main>", |view| {
        let handle = CoreStyle::size(&view).width.into_raw().calc_value();
        check(&Text { view }, handle);
    });
}

#[test]
fn every_box_has_a_line_for_each_group_of_getters() {
    let text = text_of(PROBE);
    let lines = |prefix: &str| text.lines().filter(|line| line.starts_with(prefix)).count();
    let boxes = lines("  core ");
    // The html, head, style, body and the four divs and span have a style
    // record; text nodes have none.
    assert!(boxes >= 8, "{text}");
    for group in [
        "  claims ",
        "  oof_item ",
        "  bridge ",
        "  block_container ",
        "  block_item ",
        "  flex_container ",
        "  flex_item ",
        "  grid_container ",
        "  grid_rows ",
        "  grid_columns ",
        "  grid_item ",
    ] {
        assert_eq!(lines(group), boxes, "{group}");
    }
    // Five writing modes in each of three projections, and the seven ways
    // the edges a role resolves for itself are read.
    assert_eq!(lines("  geometry "), 15 * boxes);
    assert_eq!(lines("  edges "), 7 * boxes);
}

#[test]
fn the_getters_of_flex_and_grid_boxes_are_written_in_the_order_css_names_them() {
    let text = text_of(PROBE);
    for expected in [
        // `gap: 3px 5%` is the row gap, then the column gap; Taffy's size
        // holds the column gap as its width.
        "flex_container direction=ColumnReverse wrap=NoWrap gap=[percent(0.05) px(3.0)]",
        "flex_item basis=calc[10.0 60.0 135.25] grow=2.0 shrink=1.0 \
         align_self=Some(AlignItems { keyword: FlexEnd, safety: Default }) inline_align_self=None",
        "grid_rows template=[] names=[] auto=[minmax(min-content min-content)]",
        "grid_columns template=[minmax(px(10.0) px(10.0)) \
         repeat(Count(2) [minmax(px(5.0) fr(1.0))] names=[[], []]) \
         minmax(auto fit-content-px(20.0))] names=[[\"a\"], [], [], []] auto=[minmax(auto auto)]",
        "areas=[GridTemplateArea { name: \"x\", row_start: 1, row_end: 2, column_start: 1, \
         column_end: 2 }",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

#[test]
fn sizes_edges_and_axes_follow_the_writing_mode_and_projection_they_are_read_in() {
    let text = text_of(PROBE);
    let geometry = |writing_mode: &str, projection: &str| {
        let prefix = format!("  geometry {writing_mode} {projection} ");
        text.lines()
            .find(|line| line.starts_with(&prefix) && line.contains("padding=[percent(0.1)"))
            .unwrap_or_else(|| panic!("{prefix}\n{text}"))
    };
    // The item's `width: calc(25% + 4px)` and its margins `1px 2px 3px 4px`,
    // as `left right top bottom`.
    let item = geometry("HorizontalTb", "Identity");
    assert!(item.contains("size=[calc[4.0 29.0 66.625] auto]"), "{item}");
    assert!(
        item.contains("margin=[px(4.0) px(2.0) px(1.0) px(3.0)]"),
        "{item}"
    );
    // A vertical mode reads the block axis along the width.
    let vertical = geometry("VerticalRl", "Identity");
    assert!(
        vertical.contains("size=[auto calc[4.0 29.0 66.625]]"),
        "{vertical}"
    );
    assert!(
        vertical.contains("margin=[px(1.0) px(3.0) px(2.0) px(4.0)]"),
        "{vertical}"
    );
    // Transposing the projection undoes it again.
    let transposed = geometry("VerticalRl", "BlockTranspose { right_to_left: false }");
    assert!(
        transposed.contains("size=[calc[4.0 29.0 66.625] auto]"),
        "{transposed}"
    );
}

#[test]
fn edges_are_resolved_against_the_basis_a_role_gives() {
    let text = text_of(PROBE);
    for expected in [
        "edges HorizontalTb Definite(137.5) padding=[px(13.75) px(13.75) px(13.75) px(13.75)]",
        "edges HorizontalTb Indefinite padding=[px(0.0) px(0.0) px(0.0) px(0.0)]",
        // A block item resolves its block edges against its own basis and
        // leaves the inline ones to Taffy.
        "edges HorizontalTb block_item_basis=91.25 \
         padding=[percent(0.1) percent(0.1) px(9.125) px(9.125)]",
        "edges resolved_padding padding=[px(1.5) px(2.5) px(3.5) px(4.5)]",
    ] {
        assert!(text.contains(expected), "{expected}\n{text}");
    }
}

#[test]
fn the_claims_of_a_box_depend_on_the_policy_that_reads_them() {
    let text = text_of(PROBE);
    // A transformed box is a containing block for absolute and fixed boxes.
    // Only the core container role asks for that.
    assert!(
        text.contains(
            "claims prepared=ContainingBlockClaims { absolute: true, fixed: true } \
             positioning=ContainingBlockClaims { absolute: false, fixed: false } \
             flex_grid=ContainingBlockClaims { absolute: false, fixed: false }"
        ),
        "{text}"
    );
    assert!(
        text.contains("block_item table=false float=Left clear=Both own_clear=Both"),
        "{text}"
    );
}

#[test]
fn a_calc_is_never_written_as_an_address() {
    // A calc handle is an address and differs between runs and between
    // documents; the text holds only what it resolves to.
    let text = text_of(PROBE);
    assert!(!text.contains("0x"), "{text}");
    let mut calcs = 0;
    for (at, _) in text.match_indices("calc[") {
        let rest = &text[at + "calc[".len()..];
        let values = &rest[..rest.find(']').expect("a calc is closed")];
        assert_eq!(values.split(' ').count(), 3, "{values}");
        for value in values.split(' ') {
            assert!(value == "?" || value.parse::<f32>().is_ok(), "{values}");
        }
        calcs += 1;
    }
    assert!(calcs >= 3, "{text}");
    assert_eq!(text, text_of(PROBE));
}

#[test]
fn a_getter_that_reads_differently_changes_the_text_alone() {
    let page = |style: &str| text_of(&format!("<div style='{style}'>x</div>"));
    let base = page("");
    let changed = page("flex-grow: 3");
    // Nothing moved: the divs are not flex items.
    let differing: Vec<_> = base
        .lines()
        .zip(changed.lines())
        .filter(|(base, changed)| base != changed)
        .collect();
    assert_eq!(base.lines().count(), changed.lines().count());
    let [(base_line, changed_line)] = differing[..] else {
        panic!("one line differs: {differing:?}");
    };
    assert!(base_line.starts_with("  flex_item "), "{base_line}");
    assert!(changed_line.contains("grow=3.0"), "{changed_line}");
}

#[test]
fn a_calc_is_written_as_the_values_it_resolves_to() {
    with_calc(|text, handle| {
        assert_eq!(text.dimension(Dimension::calc(handle)), CALC);
        assert_eq!(
            text.dimension(Dimension::fit_content_calc(handle)),
            format!("fit-content-{CALC}")
        );
        assert_eq!(
            text.length_percentage_auto(LengthPercentageAuto::calc(handle)),
            CALC
        );
        assert_eq!(text.length_percentage(LengthPercentage::calc(handle)), CALC);
        assert_eq!(text.min_track(MinTrackSizingFunction::calc(handle)), CALC);
        assert_eq!(text.max_track(MaxTrackSizingFunction::calc(handle)), CALC);
        assert_eq!(
            text.max_track(MaxTrackSizingFunction::fit_content_calc(handle)),
            format!("fit-content-{CALC}")
        );
    });
}

#[test]
fn an_address_the_view_never_handed_out_is_not_resolved() {
    with_calc(|text, _| {
        // Aligned like a payload, and not one.
        let foreign = &0u64 as *const u64 as *const ();
        assert_eq!(text.dimension(Dimension::calc(foreign)), "calc[? ? ?]");
    });
}

#[test]
fn every_kind_of_length_is_written_by_what_it_is() {
    with_calc(|text, _| {
        for (value, expected) in [
            (Dimension::length(3.0), "px(3.0)"),
            (Dimension::percent(0.25), "percent(0.25)"),
            (Dimension::auto(), "auto"),
            (Dimension::min_content(), "min-content"),
            (Dimension::max_content(), "max-content"),
            (Dimension::fit_content(), "fit-content"),
            (Dimension::fit_content_px(7.5), "fit-content-px(7.5)"),
            (
                Dimension::fit_content_percent(0.5),
                "fit-content-percent(0.5)",
            ),
            (Dimension::stretch(), "stretch"),
            (Dimension::content(), "content"),
        ] {
            assert_eq!(text.dimension(value), expected);
        }
        for (value, expected) in [
            (LengthPercentageAuto::length(3.0), "px(3.0)"),
            (LengthPercentageAuto::percent(0.25), "percent(0.25)"),
            (LengthPercentageAuto::auto(), "auto"),
        ] {
            assert_eq!(text.length_percentage_auto(value), expected);
        }
        assert_eq!(
            text.length_percentage(LengthPercentage::length(3.0)),
            "px(3.0)"
        );
        assert_eq!(
            text.length_percentage(LengthPercentage::percent(0.25)),
            "percent(0.25)"
        );
        for (value, expected) in [
            (MinTrackSizingFunction::length(3.0), "px(3.0)"),
            (MinTrackSizingFunction::percent(0.25), "percent(0.25)"),
            (MinTrackSizingFunction::auto(), "auto"),
            (MinTrackSizingFunction::min_content(), "min-content"),
            (MinTrackSizingFunction::max_content(), "max-content"),
        ] {
            assert_eq!(text.min_track(value), expected);
        }
        for (value, expected) in [
            (MaxTrackSizingFunction::length(3.0), "px(3.0)"),
            (MaxTrackSizingFunction::percent(0.25), "percent(0.25)"),
            (MaxTrackSizingFunction::auto(), "auto"),
            (MaxTrackSizingFunction::min_content(), "min-content"),
            (MaxTrackSizingFunction::max_content(), "max-content"),
            (
                MaxTrackSizingFunction::fit_content_px(7.5),
                "fit-content-px(7.5)",
            ),
            (
                MaxTrackSizingFunction::fit_content_percent(0.5),
                "fit-content-percent(0.5)",
            ),
            (MaxTrackSizingFunction::fr(2.0), "fr(2.0)"),
        ] {
            assert_eq!(text.max_track(value), expected);
        }
    });
}

#[test]
fn a_track_is_written_as_the_pair_of_its_limits() {
    with_calc(|text, _| {
        let track = MinMax {
            min: MinTrackSizingFunction::length(5.0),
            max: MaxTrackSizingFunction::fr(1.0),
        };
        assert_eq!(text.track(track), "minmax(px(5.0) fr(1.0))");
        let tracks = [
            track,
            MinMax {
                min: MinTrackSizingFunction::auto(),
                max: MaxTrackSizingFunction::auto(),
            },
        ];
        assert_eq!(
            text.tracks(tracks.into_iter()),
            "[minmax(px(5.0) fr(1.0)) minmax(auto auto)]"
        );
    });
}
