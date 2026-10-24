//! The engine's own style values against the layout library's.
//!
//! Each conversion and each small rule that the library also implements is
//! compared with the library's on every variant, so a value cannot be mapped
//! to a different meaning without a test failing.

use crate::values::{
    AlignContent, AlignContentKeyword, AlignItems, AlignItemsKeyword, AlignmentSafety,
    BoxGenerationMode, BoxSizing, Clear, Contain, ContainingBlockClaims, Dimension, Direction,
    Display, Float, LengthPercentage, LengthPercentageAuto, Overflow, Position,
};
use crate::{BoxStyle, LeafStyle, ToTaffy};
use raikiri_layout_geometry::{Edges, Point, Size};
use raikiri_style::SmolStr;
use taffy::CoreStyle;
use taffy::prelude::TaffyZero;

/// Every value of each list is converted, and its name must be the library's.
macro_rules! assert_same_variants {
    ($($ty:ty: [$($variant:ident),+ $(,)?]),+ $(,)?) => {$({
        $(
            let owned = <$ty>::$variant;
            let library = owned.to_taffy();
            assert_eq!(
                format!("{owned:?}"),
                format!("{library:?}"),
                concat!(stringify!($ty), "::", stringify!($variant)),
            );
        )+
    })+};
}

#[test]
fn keyword_variants_convert_to_the_library_variant_of_the_same_name() {
    assert_same_variants! {
        Display: [Block, FlowRoot, Flex, Grid, None],
        BoxGenerationMode: [Normal, None],
        Position: [Static, Relative, Absolute, Fixed, Sticky],
        BoxSizing: [BorderBox, ContentBox],
        Overflow: [Visible, Clip, Hidden, Scroll],
        Direction: [Ltr, Rtl],
        Float: [Left, Right, None],
        Clear: [Left, Right, Both, None],
        AlignmentSafety: [Default, Unsafe, Safe],
        AlignContentKeyword: [
            Normal, Start, End, FlexStart, FlexEnd, Center, Stretch, SpaceBetween, SpaceEvenly,
            SpaceAround,
        ],
        AlignItemsKeyword: [
            Normal, Start, End, FlexStart, FlexEnd, SelfStart, SelfEnd, Center, Baseline, Stretch,
        ],
    }
}

const ALIGN_ITEMS_KEYWORDS: [AlignItemsKeyword; 10] = [
    AlignItemsKeyword::Normal,
    AlignItemsKeyword::Start,
    AlignItemsKeyword::End,
    AlignItemsKeyword::FlexStart,
    AlignItemsKeyword::FlexEnd,
    AlignItemsKeyword::SelfStart,
    AlignItemsKeyword::SelfEnd,
    AlignItemsKeyword::Center,
    AlignItemsKeyword::Baseline,
    AlignItemsKeyword::Stretch,
];

const ALIGN_CONTENT_KEYWORDS: [AlignContentKeyword; 10] = [
    AlignContentKeyword::Normal,
    AlignContentKeyword::Start,
    AlignContentKeyword::End,
    AlignContentKeyword::FlexStart,
    AlignContentKeyword::FlexEnd,
    AlignContentKeyword::Center,
    AlignContentKeyword::Stretch,
    AlignContentKeyword::SpaceBetween,
    AlignContentKeyword::SpaceEvenly,
    AlignContentKeyword::SpaceAround,
];

const SAFETIES: [AlignmentSafety; 3] = [
    AlignmentSafety::Default,
    AlignmentSafety::Unsafe,
    AlignmentSafety::Safe,
];

const POSITIONS: [Position; 5] = [
    Position::Static,
    Position::Relative,
    Position::Absolute,
    Position::Fixed,
    Position::Sticky,
];

const DIRECTIONS: [Direction; 2] = [Direction::Ltr, Direction::Rtl];

#[test]
fn alignments_convert_every_keyword_and_safety_pair() {
    for keyword in ALIGN_ITEMS_KEYWORDS {
        for safety in SAFETIES {
            let owned = AlignItems { keyword, safety };
            let library = taffy::AlignItems {
                keyword: keyword.to_taffy(),
                safety: safety.to_taffy(),
            };
            assert_eq!(owned.to_taffy(), library);
        }
    }
    for keyword in ALIGN_CONTENT_KEYWORDS {
        for safety in SAFETIES {
            let owned = AlignContent { keyword, safety };
            let library = taffy::AlignContent {
                keyword: keyword.to_taffy(),
                safety: safety.to_taffy(),
            };
            assert_eq!(owned.to_taffy(), library);
        }
    }
}

#[test]
fn alignment_constants_are_the_library_constants() {
    let items = [
        (AlignItems::NORMAL, taffy::AlignItems::NORMAL),
        (AlignItems::START, taffy::AlignItems::START),
        (AlignItems::END, taffy::AlignItems::END),
        (AlignItems::FLEX_START, taffy::AlignItems::FLEX_START),
        (AlignItems::FLEX_END, taffy::AlignItems::FLEX_END),
        (AlignItems::SELF_START, taffy::AlignItems::SELF_START),
        (AlignItems::SELF_END, taffy::AlignItems::SELF_END),
        (AlignItems::CENTER, taffy::AlignItems::CENTER),
        (AlignItems::BASELINE, taffy::AlignItems::BASELINE),
        (AlignItems::STRETCH, taffy::AlignItems::STRETCH),
        (AlignItems::SAFE_SELF_END, taffy::AlignItems::SAFE_SELF_END),
    ];
    for (owned, library) in items {
        assert_eq!(owned.to_taffy(), library);
    }
    let content = [
        (AlignContent::NORMAL, taffy::AlignContent::NORMAL),
        (AlignContent::START, taffy::AlignContent::START),
        (AlignContent::END, taffy::AlignContent::END),
        (AlignContent::FLEX_START, taffy::AlignContent::FLEX_START),
        (AlignContent::FLEX_END, taffy::AlignContent::FLEX_END),
        (AlignContent::CENTER, taffy::AlignContent::CENTER),
        (AlignContent::STRETCH, taffy::AlignContent::STRETCH),
        (
            AlignContent::SPACE_BETWEEN,
            taffy::AlignContent::SPACE_BETWEEN,
        ),
        (
            AlignContent::SPACE_EVENLY,
            taffy::AlignContent::SPACE_EVENLY,
        ),
        (
            AlignContent::SPACE_AROUND,
            taffy::AlignContent::SPACE_AROUND,
        ),
    ];
    for (owned, library) in content {
        assert_eq!(owned.to_taffy(), library);
    }
}

#[test]
fn self_relative_alignment_matches_the_library_for_every_input() {
    for keyword in ALIGN_ITEMS_KEYWORDS {
        for safety in SAFETIES {
            for item in DIRECTIONS {
                for container in DIRECTIONS {
                    for axis_is_inline in [false, true] {
                        let owned = AlignItems { keyword, safety }.resolve_self_relative(
                            item,
                            container,
                            axis_is_inline,
                        );
                        let library = taffy::AlignItems {
                            keyword: keyword.to_taffy(),
                            safety: safety.to_taffy(),
                        }
                        .resolve_self_relative(
                            item.to_taffy(),
                            container.to_taffy(),
                            axis_is_inline,
                        );
                        assert_eq!(
                            owned.to_taffy(),
                            library,
                            "{keyword:?} {safety:?} {item:?} {container:?} inline={axis_is_inline}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn containing_block_claims_match_the_library_for_every_position() {
    for absolute in [false, true] {
        for fixed in [false, true] {
            let owned = ContainingBlockClaims { absolute, fixed };
            let library = taffy::ContainingBlockClaims { absolute, fixed };
            assert_eq!(owned.to_taffy(), library);
            for position in POSITIONS {
                assert_eq!(
                    owned.for_position(position),
                    library.for_position(position.to_taffy()),
                    "{owned:?} {position:?}"
                );
            }
        }
    }
    assert_eq!(
        ContainingBlockClaims::NONE.to_taffy(),
        taffy::ContainingBlockClaims::NONE
    );
}

#[test]
fn position_overflow_direction_and_float_rules_match_the_library() {
    for position in POSITIONS {
        let library = position.to_taffy();
        assert_eq!(
            position.is_out_of_flow(),
            library.is_out_of_flow(),
            "{position:?}"
        );
        assert_eq!(
            position.is_positioned(),
            library.is_positioned(),
            "{position:?}"
        );
    }
    for overflow in [
        Overflow::Visible,
        Overflow::Clip,
        Overflow::Hidden,
        Overflow::Scroll,
    ] {
        assert_eq!(
            overflow.is_scroll_container(),
            overflow.to_taffy().is_scroll_container(),
            "{overflow:?}"
        );
    }
    for direction in DIRECTIONS {
        assert_eq!(
            direction.is_rtl(),
            direction.to_taffy().is_rtl(),
            "{direction:?}"
        );
    }
    for float in [Float::Left, Float::Right, Float::None] {
        assert_eq!(
            float.is_floated(),
            float.to_taffy().is_floated(),
            "{float:?}"
        );
    }
    assert_eq!(Position::default().to_taffy(), taffy::Position::Static);
    assert_eq!(Direction::default().to_taffy(), taffy::Direction::Ltr);
    assert_eq!(Float::default().to_taffy(), taffy::Float::None);
    assert_eq!(Clear::default().to_taffy(), taffy::Clear::None);
    assert_eq!(Overflow::default().to_taffy(), taffy::Overflow::Visible);
    assert_eq!(BoxSizing::default().to_taffy(), taffy::BoxSizing::BorderBox);
}

#[test]
fn contain_converts_each_kind_and_answers_like_the_library() {
    let cases = [
        (Contain::NONE, taffy::Contain::NONE),
        (Contain::LAYOUT, taffy::Contain::LAYOUT),
        (Contain::PAINT, taffy::Contain::PAINT),
        (
            Contain::CONTENT,
            taffy::Contain::LAYOUT | taffy::Contain::PAINT,
        ),
    ];
    for (owned, library) in cases {
        assert_eq!(owned.to_taffy(), library);
        assert_eq!(
            owned.establishes_independent_formatting_context(),
            library.establishes_independent_formatting_context()
        );
        assert_eq!(
            owned.contains_scrollable_overflow(),
            library.contains_scrollable_overflow()
        );
        for (kind, library_kind) in [
            (Contain::LAYOUT, taffy::Contain::LAYOUT),
            (Contain::PAINT, taffy::Contain::PAINT),
        ] {
            assert_eq!(owned.contains(kind), library.contains(library_kind));
        }
    }
    assert_eq!(Contain::default(), Contain::NONE);
}

#[test]
fn lengths_convert_to_the_same_library_length() {
    assert_eq!(
        Dimension::length(3.5).to_taffy(),
        taffy::Dimension::length(3.5)
    );
    assert_eq!(
        Dimension::percent(0.25).to_taffy(),
        taffy::Dimension::percent(0.25)
    );
    assert_eq!(Dimension::auto().to_taffy(), taffy::Dimension::auto());
    assert_eq!(
        Dimension::min_content().to_taffy(),
        taffy::Dimension::min_content()
    );
    assert_eq!(
        Dimension::max_content().to_taffy(),
        taffy::Dimension::max_content()
    );
    assert_eq!(
        Dimension::fit_content().to_taffy(),
        taffy::Dimension::fit_content()
    );
    assert_eq!(Dimension::stretch().to_taffy(), taffy::Dimension::stretch());
    assert_eq!(
        LengthPercentage::length(2.0).to_taffy(),
        taffy::LengthPercentage::length(2.0)
    );
    assert_eq!(
        LengthPercentage::percent(0.5).to_taffy(),
        taffy::LengthPercentage::percent(0.5)
    );
    assert_eq!(
        LengthPercentage::ZERO.to_taffy(),
        taffy::LengthPercentage::ZERO
    );
    assert_eq!(
        LengthPercentageAuto::length(2.0).to_taffy(),
        taffy::LengthPercentageAuto::length(2.0)
    );
    assert_eq!(
        LengthPercentageAuto::percent(0.5).to_taffy(),
        taffy::LengthPercentageAuto::percent(0.5)
    );
    assert_eq!(
        LengthPercentageAuto::auto().to_taffy(),
        taffy::LengthPercentageAuto::auto()
    );
    assert_eq!(
        LengthPercentageAuto::ZERO.to_taffy(),
        taffy::LengthPercentageAuto::ZERO
    );
}

#[test]
fn wrappers_convert_each_field_in_place() {
    assert_eq!(Some(1.5_f32).to_taffy(), Some(1.5_f32));
    assert_eq!(None::<f32>.to_taffy(), None);
    assert!(true.to_taffy());
    assert!(!false.to_taffy());
    assert_eq!(
        Some(Position::Fixed).to_taffy(),
        Some(taffy::Position::Fixed)
    );
    let size = Size {
        width: Dimension::length(1.0),
        height: Dimension::percent(0.5),
    };
    assert_eq!(
        size.to_taffy(),
        taffy::Size {
            width: taffy::Dimension::length(1.0),
            height: taffy::Dimension::percent(0.5),
        }
    );
    let edges = Edges {
        left: LengthPercentage::length(1.0),
        right: LengthPercentage::length(2.0),
        top: LengthPercentage::length(3.0),
        bottom: LengthPercentage::length(4.0),
    };
    assert_eq!(
        edges.to_taffy(),
        taffy::Rect {
            left: taffy::LengthPercentage::length(1.0),
            right: taffy::LengthPercentage::length(2.0),
            top: taffy::LengthPercentage::length(3.0),
            bottom: taffy::LengthPercentage::length(4.0),
        }
    );
    let point = Point {
        x: Overflow::Hidden,
        y: Overflow::Scroll,
    };
    assert_eq!(
        point.to_taffy(),
        taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Scroll,
        }
    );
}

/// A box whose every field differs from the default, and the library's style
/// of the same box.
fn busy_box() -> (BoxStyle, taffy::Style<SmolStr>) {
    let owned = BoxStyle {
        display: Display::Block,
        item_is_table: true,
        item_is_replaced: true,
        item_is_compressible_replaced: false,
        box_sizing: BoxSizing::ContentBox,
        direction: Direction::Rtl,
        overflow: Point {
            x: Overflow::Hidden,
            y: Overflow::Clip,
        },
        contain: Contain::CONTENT,
        position: Position::Sticky,
        inset: Edges {
            left: LengthPercentageAuto::length(1.0),
            right: LengthPercentageAuto::percent(0.1),
            top: LengthPercentageAuto::auto(),
            bottom: LengthPercentageAuto::length(4.0),
        },
        size: Size {
            width: Dimension::length(100.0),
            height: Dimension::fit_content(),
        },
        min_size: Size {
            width: LengthPercentageAuto::length(10.0),
            height: LengthPercentageAuto::percent(0.2),
        },
        max_size: Size {
            width: LengthPercentageAuto::percent(0.9),
            height: LengthPercentageAuto::length(500.0),
        },
        aspect_ratio: Some(1.5),
        margin: Edges {
            left: LengthPercentageAuto::auto(),
            right: LengthPercentageAuto::length(2.0),
            top: LengthPercentageAuto::percent(0.05),
            bottom: LengthPercentageAuto::ZERO,
        },
        padding: Edges {
            left: LengthPercentage::length(3.0),
            right: LengthPercentage::percent(0.03),
            top: LengthPercentage::ZERO,
            bottom: LengthPercentage::length(6.0),
        },
        border: Edges {
            left: LengthPercentage::length(1.0),
            right: LengthPercentage::length(1.0),
            top: LengthPercentage::length(2.0),
            bottom: LengthPercentage::ZERO,
        },
        float: Float::Right,
        clear: Clear::Both,
        justify_items: AlignItems::CENTER,
        align_content: AlignContent::SPACE_BETWEEN,
        align_self: Some(AlignItems::BASELINE),
        justify_self: Some(AlignItems::SAFE_SELF_END),
    };
    let library = taffy::Style {
        display: taffy::Display::Block,
        item_is_table: true,
        item_is_replaced: true,
        item_is_compressible_replaced: false,
        box_sizing: taffy::BoxSizing::ContentBox,
        direction: taffy::Direction::Rtl,
        overflow: taffy::Point {
            x: taffy::Overflow::Hidden,
            y: taffy::Overflow::Clip,
        },
        contain: taffy::Contain::LAYOUT | taffy::Contain::PAINT,
        position: taffy::Position::Sticky,
        inset: owned.inset.to_taffy(),
        size: taffy::Size {
            width: taffy::Dimension::length(100.0),
            height: taffy::Dimension::fit_content(),
        },
        min_size: owned.min_size.to_taffy(),
        max_size: owned.max_size.to_taffy(),
        aspect_ratio: Some(1.5),
        margin: owned.margin.to_taffy(),
        padding: owned.padding.to_taffy(),
        border: owned.border.to_taffy(),
        ..taffy::Style::default()
    };
    (owned, library)
}

/// Every getter of the core style trait, as a comparable record.
fn core_record(style: &impl CoreStyle) -> String {
    format!(
        "{:?}",
        (
            (
                style.box_generation_mode(),
                style.is_block(),
                style.is_compressible_replaced(),
                style.is_replaced(),
                style.box_sizing(),
                style.direction(),
                style.overflow(),
                style.scrollbar_width(),
                style.position(),
                style.inset(),
            ),
            (
                style.is_containing_block(),
                style.size(),
                style.min_size(),
                style.max_size(),
                style.aspect_ratio(),
                style.margin(),
                style.padding(),
                style.border(),
                style.contain(),
            ),
        )
    )
}

#[test]
fn the_default_box_style_reads_like_the_library_default_style() {
    let owned = BoxStyle::default();
    let library = taffy::Style::<SmolStr>::default();
    assert_eq!(core_record(&LeafStyle::new(&owned)), core_record(&library));
}

#[test]
fn a_leaf_style_reads_like_the_library_style_of_the_same_box() {
    let (owned, library) = busy_box();
    assert_eq!(core_record(&LeafStyle::new(&owned)), core_record(&library));
}

#[test]
fn a_leaf_style_tells_blocks_and_replaced_boxes_apart_like_the_library() {
    for display in [
        Display::Block,
        Display::FlowRoot,
        Display::Flex,
        Display::Grid,
        Display::None,
    ] {
        for (replaced, compressible) in [(false, false), (true, false), (false, true), (true, true)]
        {
            let owned = BoxStyle {
                display,
                item_is_replaced: replaced,
                item_is_compressible_replaced: compressible,
                ..BoxStyle::default()
            };
            let library = taffy::Style::<SmolStr> {
                display: display.to_taffy(),
                item_is_replaced: replaced,
                item_is_compressible_replaced: compressible,
                ..taffy::Style::default()
            };
            assert_eq!(
                core_record(&LeafStyle::new(&owned)),
                core_record(&library),
                "{display:?} replaced={replaced} compressible={compressible}"
            );
        }
    }
}

#[test]
fn a_positioned_leaf_claims_the_containing_blocks_the_library_does() {
    for position in POSITIONS {
        let owned = BoxStyle {
            position,
            ..BoxStyle::default()
        };
        let library = taffy::Style::<SmolStr> {
            position: position.to_taffy(),
            ..taffy::Style::default()
        };
        assert_eq!(
            LeafStyle::new(&owned).is_containing_block(),
            library.is_containing_block(),
            "{position:?}"
        );
    }
}

#[test]
fn box_style_rules_follow_the_display() {
    for display in [
        Display::Block,
        Display::FlowRoot,
        Display::Flex,
        Display::Grid,
        Display::None,
    ] {
        let style = BoxStyle {
            display,
            ..BoxStyle::default()
        };
        assert_eq!(style.is_block(), display == Display::Block, "{display:?}");
        assert_eq!(
            style.box_generation_mode(),
            if display == Display::None {
                BoxGenerationMode::None
            } else {
                BoxGenerationMode::Normal
            },
            "{display:?}"
        );
    }
    let compressible = BoxStyle {
        item_is_compressible_replaced: true,
        ..BoxStyle::default()
    };
    assert!(compressible.is_compressible_replaced());
    assert!(!compressible.item_is_replaced);
}
