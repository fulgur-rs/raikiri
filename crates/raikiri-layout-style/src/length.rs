use crate::*;
use raikiri_style::{ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P};
use taffy::{Dimension, LengthPercentage as LP, LengthPercentageAuto as LPA};

pub(crate) fn sanitize(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(-1e7, 1e7)
    }
}
impl LengthValue<'_> {
    fn pointer(self) -> *const () {
        self.payload.expect("calc has a prepared payload").pointer()
    }
    pub(crate) fn dimension(self) -> Dimension {
        match self.value {
            L::Auto => Dimension::auto(),
            L::Px(v) => Dimension::length(sanitize(v)),
            L::Percent(v) => Dimension::percent(sanitize(v / 100.0)),
            L::MinContent => Dimension::min_content(),
            L::MaxContent => Dimension::max_content(),
            L::FitContent => Dimension::fit_content(),
            L::Calc(_) => Dimension::calc(self.pointer()),
            L::FitContentOf(P::Px(v)) => Dimension::fit_content_px(sanitize(v)),
            L::FitContentOf(P::Percent(v)) => Dimension::fit_content_percent(sanitize(v / 100.0)),
            L::FitContentOf(P::Calc(_)) => Dimension::fit_content_calc(self.pointer()),
        }
    }
    pub(crate) fn auto(self, margin: bool) -> LPA {
        match self.value {
            L::Px(v) => LPA::length(sanitize(v)),
            L::Percent(v) => LPA::percent(sanitize(v / 100.0)),
            L::Calc(_) => LPA::calc(self.pointer()),
            L::Auto => LPA::auto(),
            _ if margin => LPA::length(0.0),
            _ => LPA::auto(),
        }
    }
    pub(crate) fn lp(self, basis: PercentBasis) -> LP {
        if !self.resolved && basis != PercentBasis::Taffy {
            let basis = match basis {
                PercentBasis::Definite(v) => v,
                _ => 0.0,
            };
            let value = match self.value {
                L::Px(v) => sanitize(v),
                L::Percent(v) => sanitize(v / 100.0) * basis,
                L::Calc(_) => self.payload.expect("prepared calc").resolve(basis),
                _ => 0.0,
            };
            return LP::length(value);
        }
        match self.value {
            L::Px(v) => LP::length(sanitize(v)),
            L::Percent(v) => LP::percent(sanitize(v / 100.0)),
            L::Calc(_) => LP::calc(self.pointer()),
            _ => LP::length(0.0),
        }
    }
}
