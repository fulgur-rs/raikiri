use crate::values::{CalcRef, Dimension, LengthPercentage as LP, LengthPercentageAuto as LPA};
use crate::*;
use raikiri_style::{ComputedLengthPercentageOrAuto as L, ComputedLengthPercentageOrCalc as P};

pub(crate) fn sanitize(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(-1e7, 1e7)
    }
}
impl LengthValue<'_> {
    fn pixel(self, value: f32) -> f32 {
        if self.resolved && value.is_finite() {
            value
        } else {
            sanitize(value)
        }
    }
    fn calc_ref(self) -> CalcRef {
        CalcRef::new(self.payload.expect("calc has a prepared payload"))
    }
    #[inline]
    pub(crate) fn dimension(self) -> Dimension {
        match self.value {
            L::Auto => Dimension::auto(),
            L::Px(v) => Dimension::length(self.pixel(v)),
            L::Percent(v) => Dimension::percent(sanitize(v / 100.0)),
            L::MinContent => Dimension::min_content(),
            L::MaxContent => Dimension::max_content(),
            L::FitContent => Dimension::fit_content(),
            L::Stretch => Dimension::stretch(),
            L::Calc(_) => Dimension::calc(self.calc_ref()),
            L::FitContentOf(P::Px(v)) => Dimension::fit_content_px(sanitize(v)),
            L::FitContentOf(P::Percent(v)) => Dimension::fit_content_percent(sanitize(v / 100.0)),
            L::FitContentOf(P::Calc(_)) => Dimension::fit_content_calc(self.calc_ref()),
        }
    }
    #[inline]
    pub(crate) fn auto(self, margin: bool) -> LPA {
        match self.value {
            L::Px(v) => LPA::length(self.pixel(v)),
            L::Percent(v) => LPA::percent(sanitize(v / 100.0)),
            L::Calc(_) => LPA::calc(self.calc_ref()),
            L::Auto => LPA::auto(),
            _ if margin => LPA::length(0.0),
            _ => LPA::auto(),
        }
    }
    #[inline]
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
            L::Px(v) => LP::length(self.pixel(v)),
            L::Percent(v) => LP::percent(sanitize(v / 100.0)),
            L::Calc(_) => LP::calc(self.calc_ref()),
            _ => LP::length(0.0),
        }
    }
}
