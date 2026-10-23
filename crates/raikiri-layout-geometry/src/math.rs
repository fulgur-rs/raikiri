use crate::Size;

/// Minimum, maximum, clamp, addition and subtraction where one side may be
/// missing.
///
/// When the left-hand value is `None` the result is `None`. When only the
/// right-hand value is `None`, it is treated as if it were not there: the
/// left-hand value is returned unchanged.
///
/// Implemented for `Option<f32>` and `f32` on the left, and for [`Size`] of
/// such values, applying the operation to the width and to the height.
pub trait MaybeMath<In, Out> {
    /// The smaller of `self` and `rhs`.
    fn maybe_min(self, rhs: In) -> Out;

    /// The larger of `self` and `rhs`.
    fn maybe_max(self, rhs: In) -> Out;

    /// `self` limited to at most `max`, then to at least `min`.
    fn maybe_clamp(self, min: In, max: In) -> Out;

    /// `self` plus `rhs`.
    fn maybe_add(self, rhs: In) -> Out;

    /// `self` minus `rhs`.
    fn maybe_sub(self, rhs: In) -> Out;
}

impl MaybeMath<Option<f32>, Option<f32>> for Option<f32> {
    #[inline]
    fn maybe_min(self, rhs: Option<f32>) -> Option<f32> {
        match (self, rhs) {
            (Some(l), Some(r)) => Some(l.min(r)),
            (Some(_l), None) => self,
            (None, Some(_r)) => None,
            (None, None) => None,
        }
    }

    #[inline]
    fn maybe_max(self, rhs: Option<f32>) -> Option<f32> {
        match (self, rhs) {
            (Some(l), Some(r)) => Some(l.max(r)),
            (Some(_l), None) => self,
            (None, Some(_r)) => None,
            (None, None) => None,
        }
    }

    #[inline]
    fn maybe_clamp(self, min: Option<f32>, max: Option<f32>) -> Option<f32> {
        match (self, min, max) {
            (Some(base), Some(min), Some(max)) => Some(base.min(max).max(min)),
            (Some(base), None, Some(max)) => Some(base.min(max)),
            (Some(base), Some(min), None) => Some(base.max(min)),
            (Some(_), None, None) => self,
            (None, _, _) => None,
        }
    }

    #[inline]
    fn maybe_add(self, rhs: Option<f32>) -> Option<f32> {
        match (self, rhs) {
            (Some(l), Some(r)) => Some(l + r),
            (Some(_l), None) => self,
            (None, Some(_r)) => None,
            (None, None) => None,
        }
    }

    #[inline]
    fn maybe_sub(self, rhs: Option<f32>) -> Option<f32> {
        match (self, rhs) {
            (Some(l), Some(r)) => Some(l - r),
            (Some(_l), None) => self,
            (None, Some(_r)) => None,
            (None, None) => None,
        }
    }
}

impl MaybeMath<f32, Option<f32>> for Option<f32> {
    #[inline]
    fn maybe_min(self, rhs: f32) -> Option<f32> {
        self.map(|val| val.min(rhs))
    }

    #[inline]
    fn maybe_max(self, rhs: f32) -> Option<f32> {
        self.map(|val| val.max(rhs))
    }

    #[inline]
    fn maybe_clamp(self, min: f32, max: f32) -> Option<f32> {
        self.map(|val| val.min(max).max(min))
    }

    #[inline]
    fn maybe_add(self, rhs: f32) -> Option<f32> {
        self.map(|val| val + rhs)
    }

    #[inline]
    fn maybe_sub(self, rhs: f32) -> Option<f32> {
        self.map(|val| val - rhs)
    }
}

impl MaybeMath<Option<f32>, f32> for f32 {
    #[inline]
    fn maybe_min(self, rhs: Option<f32>) -> f32 {
        match rhs {
            Some(val) => self.min(val),
            None => self,
        }
    }

    #[inline]
    fn maybe_max(self, rhs: Option<f32>) -> f32 {
        match rhs {
            Some(val) => self.max(val),
            None => self,
        }
    }

    #[inline]
    fn maybe_clamp(self, min: Option<f32>, max: Option<f32>) -> f32 {
        match (min, max) {
            (Some(min), Some(max)) => self.min(max).max(min),
            (None, Some(max)) => self.min(max),
            (Some(min), None) => self.max(min),
            (None, None) => self,
        }
    }

    #[inline]
    fn maybe_add(self, rhs: Option<f32>) -> f32 {
        match rhs {
            Some(val) => self + val,
            None => self,
        }
    }

    #[inline]
    fn maybe_sub(self, rhs: Option<f32>) -> f32 {
        match rhs {
            Some(val) => self - val,
            None => self,
        }
    }
}

impl<In, Out, T: MaybeMath<In, Out>> MaybeMath<Size<In>, Size<Out>> for Size<T> {
    #[inline]
    fn maybe_min(self, rhs: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_min(rhs.width),
            height: self.height.maybe_min(rhs.height),
        }
    }

    #[inline]
    fn maybe_max(self, rhs: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_max(rhs.width),
            height: self.height.maybe_max(rhs.height),
        }
    }

    #[inline]
    fn maybe_clamp(self, min: Size<In>, max: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_clamp(min.width, max.width),
            height: self.height.maybe_clamp(min.height, max.height),
        }
    }

    #[inline]
    fn maybe_add(self, rhs: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_add(rhs.width),
            height: self.height.maybe_add(rhs.height),
        }
    }

    #[inline]
    fn maybe_sub(self, rhs: Size<In>) -> Size<Out> {
        Size {
            width: self.width.maybe_sub(rhs.width),
            height: self.height.maybe_sub(rhs.height),
        }
    }
}
