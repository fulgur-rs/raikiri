use super::{ComparisonStats, ReadContext};
use std::fmt::Debug;

#[derive(Clone, Copy)]
pub(super) struct Diagnostic<'a> {
    context: ReadContext,
    pub(super) stats: &'a ComparisonStats,
    property: &'static str,
    component: &'static str,
    indices: [Option<usize>; 3],
    basis: Option<f32>,
}
impl<'a> Diagnostic<'a> {
    pub(super) fn new(
        context: ReadContext,
        stats: &'a ComparisonStats,
        property: &'static str,
    ) -> Self {
        Self {
            context,
            stats,
            property,
            component: "",
            indices: [None; 3],
            basis: None,
        }
    }
    pub(super) fn component(mut self, component: &'static str) -> Self {
        self.component = component;
        self
    }
    pub(super) fn indexed(mut self, index: usize) -> Self {
        let slot = self
            .indices
            .iter_mut()
            .find(|slot| slot.is_none())
            .expect("at most three nested Grid indices");
        *slot = Some(index);
        self
    }
    pub(super) fn element(self) {
        self.stats
            .iterator_reads
            .set(self.stats.iterator_reads.get() + 1);
    }
    pub(super) fn getter(self, property: &'static str) {
        self.stats.getter(property, self.context);
    }
    pub(super) fn equal<T: Debug + PartialEq>(self, legacy: &T, view: &T) {
        if legacy != view {
            self.mismatch(legacy, view);
        }
    }
    pub(super) fn mismatch<A: Debug, B: Debug>(self, legacy: &A, view: &B) -> ! {
        panic!(
            "style view mismatch node={:?} property={} component={} indices={:?} basis={:?} legacy={legacy:?} view={view:?} context={:?} role={:?} scope=(pass={},depth={},restored={})",
            self.context.node,
            self.property,
            self.component,
            self.indices,
            self.basis,
            self.context.context,
            self.context.role,
            self.context.pass_entries,
            self.context.scope_depth,
            self.context.was_scoped
        )
    }
    #[allow(unsafe_code)]
    fn calc(self, legacy: *const (), view: *const ()) {
        // SAFETY: CheckedStyle receives retained legacy handles; the document or test owns ArenaCalc.
        let expression = unsafe { &*legacy.cast::<crate::taffy_impl::ArenaCalc>() }.value;
        // Include either side of a positive zero crossing, beyond the fixed sample bases.
        let crossing = (-f64::from(expression.px) * 100.0 / f64::from(expression.percent))
            .abs()
            .min(f64::from(f32::MAX) / 2.0) as f32;
        for basis in [
            0.0,
            1.0,
            37.0,
            100.0,
            1000.0,
            crossing * 0.5,
            crossing * 2.0,
        ] {
            self.stats
                .calc_comparisons
                .set(self.stats.calc_comparisons.get() + 1);
            let old = crate::taffy_impl::resolve_calc(legacy, basis);
            // SAFETY: The candidate getter's PreparedStyle and override owners remain borrowed.
            let new = unsafe { raikiri_layout_style::resolve_calc(view, basis) };
            Self {
                basis: Some(basis),
                ..self
            }
            .equal(&old, &new);
        }
    }
}

pub(super) trait CheckedValue: Debug {
    fn check(&self, candidate: &Self, diagnostic: Diagnostic<'_>);
}
macro_rules! plain {
    ($($value:ty),+ $(,)?) => { $(impl CheckedValue for $value {
        fn check(&self, candidate: &Self, diagnostic: Diagnostic<'_>) { diagnostic.equal(self, candidate); }
    })+ };
}
plain!(
    bool,
    f32,
    u16,
    raikiri_style::SmolStr,
    taffy::BoxGenerationMode,
    taffy::BoxSizing,
    taffy::Direction,
    taffy::Overflow,
    taffy::Position,
    taffy::ContainingBlockClaims,
    taffy::Contain,
    taffy::FlexDirection,
    taffy::FlexWrap,
    taffy::AlignContent,
    taffy::AlignItems,
    taffy::GridAutoFlow,
    taffy::RepetitionCount,
    taffy::GridPlacement<raikiri_style::SmolStr>,
    taffy::GridTemplateArea<raikiri_style::SmolStr>
);

impl<T: CheckedValue> CheckedValue for Option<T> {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        diag.equal(&self.is_some(), &candidate.is_some());
        if let (Some(old), Some(new)) = (self, candidate) {
            old.check(new, diag);
        }
    }
}
impl<T: CheckedValue> CheckedValue for &T {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        (*self).check(*candidate, diag);
    }
}
impl<T: CheckedValue> CheckedValue for taffy::Size<T> {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        self.width.check(&candidate.width, diag.component("width"));
        self.height
            .check(&candidate.height, diag.component("height"));
    }
}
impl<T: CheckedValue> CheckedValue for taffy::Rect<T> {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        self.left.check(&candidate.left, diag.component("left"));
        self.right.check(&candidate.right, diag.component("right"));
        self.top.check(&candidate.top, diag.component("top"));
        self.bottom
            .check(&candidate.bottom, diag.component("bottom"));
    }
}
impl<T: CheckedValue> CheckedValue for taffy::Point<T> {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        self.x.check(&candidate.x, diag.component("x"));
        self.y.check(&candidate.y, diag.component("y"));
    }
}
impl<T: CheckedValue> CheckedValue for taffy::Line<T> {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        self.start.check(&candidate.start, diag.component("start"));
        self.end.check(&candidate.end, diag.component("end"));
    }
}
macro_rules! length {
    ($value:ty, $expanded:ident $(, $additional:ident)?) => {
        impl CheckedValue for $value {
            fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
                use taffy::$expanded as E;
                match (self.expand(), candidate.expand()) {
                    (E::Calc(old), E::Calc(new)) $(| (E::$additional(old), E::$additional(new)))? => diag.calc(old, new),
                    (old, new) => diag.equal(&old, &new),
                }
            }
        }
    };
}
length!(taffy::Dimension, ExpandedDimension, FitContentCalc);
length!(taffy::LengthPercentage, ExpandedLengthPercentage);
length!(taffy::LengthPercentageAuto, ExpandedLengthPercentageAuto);
length!(
    taffy::MinTrackSizingFunction,
    ExpandedMinTrackSizingFunction
);
length!(
    taffy::MaxTrackSizingFunction,
    ExpandedMaxTrackSizingFunction,
    FitContentCalc
);
impl CheckedValue for taffy::TrackSizingFunction {
    fn check(&self, candidate: &Self, diag: Diagnostic<'_>) {
        self.min.check(&candidate.min, diag.component("min"));
        self.max.check(&candidate.max, diag.component("max"));
    }
}
