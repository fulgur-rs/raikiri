//! Allocation measurements exclude preparation, assertions and payload destruction.
#![allow(unsafe_code)]
use raikiri_layout_style::*;
use raikiri_style::{
    ComputedGridTemplateTracks, ComputedGridTrackBreadth as B, ComputedGridTrackList,
    ComputedGridTrackListComponent as C, ComputedGridTrackRepeat, ComputedGridTrackSize as S,
    ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P, ComputedValues,
    SmolStr, property::*,
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
    sync::Arc,
};
use taffy::{
    BlockContainerStyle as BC, BlockItemStyle as BI, CoreStyle, FlexboxContainerStyle as FC,
    FlexboxItemStyle as FI, GenericRepetition, GridContainerStyle as GC, GridItemStyle as GI,
    OofItemStyle as OI,
};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static COUNT: Cell<usize> = const { Cell::new(0) };
}
struct Counting;
fn record() {
    if ENABLED.try_with(Cell::get).unwrap_or(false) {
        let _ = COUNT.try_with(|c| c.set(c.get() + 1));
    }
}
// SAFETY: Every operation forwards the caller's original layout and pointer to System.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Forwarding the allocator contract unchanged.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            record();
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: Forwarding the allocator contract unchanged.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            record();
        }
        p
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: Forwarding the allocator contract unchanged.
        let p = unsafe { System.realloc(p, layout, size) };
        if !p.is_null() {
            record();
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        // SAFETY: Forwarding the allocator contract unchanged.
        unsafe {
            System.dealloc(p, layout);
        }
    }
}
#[global_allocator]
static ALLOCATOR: Counting = Counting;
struct Disable;
impl Drop for Disable {
    fn drop(&mut self) {
        ENABLED.with(|e| e.set(false));
    }
}
fn allocations(f: impl FnOnce()) -> usize {
    COUNT.with(|c| c.set(0));
    ENABLED.with(|e| e.set(true));
    let guard = Disable;
    f();
    drop(guard);
    COUNT.with(Cell::get)
}
fn consume_template(tracks: TemplateTracks<'_>) {
    for component in tracks.clone().chain(tracks) {
        match component {
            taffy::GenericGridTemplateComponent::Single(track) => {
                black_box(track);
            }
            taffy::GenericGridTemplateComponent::Repeat(repeat) => {
                black_box(repeat.count());
                black_box(repeat.track_count());
                let tracks = repeat.tracks();
                for track in tracks.clone().chain(tracks) {
                    black_box(track);
                }
                let names = repeat.lines_names();
                for set in names.clone().chain(names) {
                    for name in set.clone().chain(set) {
                        black_box(name);
                    }
                }
            }
        }
    }
}
fn read_all(view: &LayoutStyleView<'_>) {
    let v = &TaffyStyle::new(*view);
    macro_rules! query {
        ($t:ident: $($method:ident),+ $(,)?) => { $(black_box($t::$method(v));)+ };
    }
    query!(CoreStyle: box_generation_mode,is_block,is_compressible_replaced,is_replaced,box_sizing,direction,overflow,scrollbar_width,position,inset,is_containing_block,size,min_size,max_size,aspect_ratio,margin,padding,border,contain);
    query!(BC: text_align,align_content,justify_items);
    query!(BI: is_table,align_self,justify_self,align_content,float,clear);
    query!(OI: align_self,justify_self,is_table,grid_row,grid_column);
    black_box(view.bridge_align_self());
    black_box(view.bridge_justify_self());
    black_box(view.bridge_align_content());
    query!(FC: flex_direction,flex_wrap,gap,align_content,align_items,justify_content);
    query!(FI: flex_basis,flex_grow,flex_shrink,align_self);
    query!(GC: grid_auto_flow,gap,align_content,justify_content,align_items,justify_items,grid_template_area_row_count,grid_template_area_column_count);
    query!(GI: grid_row,grid_column,align_self,justify_self);
    for tracks in [GC::grid_template_columns(v), GC::grid_template_rows(v)]
        .into_iter()
        .flatten()
    {
        consume_template(tracks);
    }
    for tracks in [GC::grid_auto_columns(v), GC::grid_auto_rows(v)] {
        for track in tracks.clone().chain(tracks) {
            black_box(track);
        }
    }
    for names in [
        GC::grid_template_column_names(v),
        GC::grid_template_row_names(v),
    ]
    .into_iter()
    .flatten()
    {
        for set in names.clone().chain(names) {
            for name in set.clone().chain(set) {
                black_box(name);
            }
        }
    }
    if let Some(areas) = GC::grid_template_areas(v) {
        for area in areas.clone().chain(areas) {
            black_box(area);
        }
    }
}
#[test]
fn all_getters_and_iterator_clones_allocate_nothing() {
    let control = allocations(|| {
        black_box(Box::new([1u8; 128]));
    });
    assert!(control > 0);

    let name = SmolStr::new("an-area-and-line-name-long-enough-to-use-shared-storage");
    let expr = CalcLengthPercentage {
        px: 3.0,
        percent: 25.0,
    };
    let payload = Arc::new(AlignedCalc::new(expr, true));
    let mut cv = ComputedValues::initial();
    cv.width = L::Calc(expr);
    cv.padding.top = P::Calc(expr);
    cv.grid_column_start = GridLineValue::Named(name.clone());
    cv.grid_row_end = GridLineValue::SpanNamed(name.clone(), 2);
    cv.grid_template_columns = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![name.clone()], vec![], vec![name.clone()]],
        components: vec![
            C::Size(S::Breadth(B::Px(11.0))),
            C::Repeat(ComputedGridTrackRepeat {
                count: GridRepeatCount::AutoFill,
                line_names: vec![vec![name.clone()], vec![name.clone()]],
                tracks: vec![S::MinMax(B::Calc(expr), B::Calc(expr))],
            }),
        ],
    }));
    cv.grid_template_rows = ComputedGridTemplateTracks::List(Arc::new(ComputedGridTrackList {
        line_names: vec![vec![name.clone()], vec![]],
        components: vec![C::Size(S::Breadth(B::Flex(1.0)))],
    }));
    cv.grid_template_areas = GridTemplateAreasValue::Areas(Arc::new(GridTemplateAreas {
        row_strings: vec![name.clone()],
        row_count: 1,
        column_count: 1,
        areas: vec![GridTemplateAreaEntry {
            name,
            row_start: 1,
            row_end: 2,
            column_start: 1,
            column_end: 2,
        }],
    }));
    let bindings = [
        PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::Width),
            payload: &payload,
        },
        PreparedCalc {
            slot: CalcSlot::Property(LengthProperty::PaddingTop),
            payload: &payload,
        },
        PreparedCalc {
            slot: CalcSlot::Grid {
                axis: Axis::X,
                list: GridList::Template,
                component: 1,
                repeat_track: Some(0),
                part: TrackPart::Min,
            },
            payload: &payload,
        },
        PreparedCalc {
            slot: CalcSlot::Grid {
                axis: Axis::X,
                list: GridList::Template,
                component: 1,
                repeat_track: Some(0),
                part: TrackPart::Max,
            },
            payload: &payload,
        },
    ];
    let prepared = PreparedStyle::new(&cv, StyleMetadata::default(), &bindings).unwrap();
    let owned = PreparedStyle::prepare(&cv, StyleMetadata::default());
    let shared = PreparedStyle::prepare_shared(Arc::new(cv.clone()), StyleMetadata::default());
    let data = Arc::new(
        PreparedData::prepare_with(&cv, |_, expression, range| {
            Arc::new(AlignedCalc::new(expression, range))
        })
        .unwrap(),
    );
    let from_data = PreparedStyle::from_data(&cv, StyleMetadata::default(), data.clone()).unwrap();
    let from_shared_data = PreparedStyle::from_shared_data(
        Arc::new(cv.clone()),
        StyleMetadata::default(),
        data.clone(),
    )
    .unwrap();
    let owned_pass = [
        OwnedPropertyOverride::aspect_ratio(Some(2.0)),
        OwnedPropertyOverride::overflow(Axis::X, OverflowValue::Hidden),
        OwnedPropertyOverride::flex_shrink(0.5),
        OwnedPropertyOverride::length(LengthProperty::MarginLeft, L::Calc(expr), false).unwrap(),
    ];
    let owned_scoped = [
        OwnedPropertyOverride::align_self(Some(taffy::AlignItems::CENTER)),
        OwnedPropertyOverride::justify_self(None),
        OwnedPropertyOverride::length(LengthProperty::PaddingBottom, L::Px(7.0), true).unwrap(),
    ];
    let pass = [
        PropertyOverride::aspect_ratio(Some(2.0)),
        PropertyOverride::overflow(Axis::X, OverflowValue::Hidden),
        PropertyOverride::flex_shrink(0.5),
    ];
    let scoped = [
        PropertyOverride::align_self(Some(taffy::AlignItems::CENTER)),
        PropertyOverride::justify_self(None),
        PropertyOverride::length(LengthProperty::PaddingBottom, LengthValue::resolved(7.0))
            .unwrap(),
    ];
    read_all(&LayoutStyleView::new(
        &prepared,
        ViewContext::default(),
        &pass,
        &scoped,
    ));
    let count = allocations(|| {
        for mode in [
            WritingMode::HorizontalTb,
            WritingMode::VerticalRl,
            WritingMode::VerticalLr,
            WritingMode::SidewaysLr,
        ] {
            for basis in [
                PercentBasis::Taffy,
                PercentBasis::Definite(200.0),
                PercentBasis::Indefinite,
            ] {
                let v = LayoutStyleView::new(
                    &prepared,
                    ViewContext {
                        layout_wm: mode,
                        percent_basis: basis,
                        ..ViewContext::default()
                    },
                    &pass,
                    &scoped,
                );
                for _ in 0..100 {
                    read_all(black_box(&v));
                    read_all(black_box(&v.for_flex_grid()));
                    for right_to_left in [false, true] {
                        read_all(black_box(&v.with_context(ViewContext {
                            bridge_projection: BridgeProjection::BlockTranspose { right_to_left },
                            block_item_basis: Some(123.0),
                            resolved_padding: Some(taffy::Rect {
                                top: 1.0,
                                right: 2.0,
                                bottom: 3.0,
                                left: 4.0,
                            }),
                            claims_policy: ClaimsPolicy::Positioning,
                            ..ViewContext::default()
                        })));
                    }
                    black_box(v);
                    for prepared in [&owned, &shared, &from_data, &from_shared_data] {
                        let v = LayoutStyleView::with_owned_overrides(
                            prepared,
                            ViewContext {
                                layout_wm: mode,
                                percent_basis: basis,
                                ..ViewContext::default()
                            },
                            &owned_pass,
                            &owned_scoped,
                        );
                        read_all(black_box(&v));
                        read_all(black_box(&v.for_flex_grid()));
                        black_box(v);
                    }
                }
            }
        }
    });
    assert_eq!(count, 0);
}
