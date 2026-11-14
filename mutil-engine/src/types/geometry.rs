//! Minimal geometric primitives used by the middle-end output and the
//! interaction types (spec §6.4, §6.6). All values are in `f32` layout units
//! (the engine is unit-agnostic); the y axis grows downwards.

/// The largest magnitude a primary measurement may have, in layout units
/// (spec MSR-1).
///
/// The gates accept no width, extent, offset or metric beyond it. A paragraph
/// has fewer than 2^64 records, so any sum the engine forms over them stays
/// below `2^64 · 10^15 · 10^4 ≈ 1.8e38 < f32::MAX`: no layout arithmetic can
/// overflow, and code behind the gates does not check for it.
pub const MAX_MEASUREMENT: f32 = 1.0e15;

/// `true` when `value` is a measurement: finite with `|value| <=
/// MAX_MEASUREMENT` (spec MSR-1). Signed: offsets and deltas may be negative.
#[must_use]
pub fn is_measurement(value: f32) -> bool {
    value.is_finite() && value.abs() <= MAX_MEASUREMENT
}

/// `true` when `value` is a non-negative measurement (spec MSR-1): a width,
/// an extent or a capacity.
#[must_use]
pub fn is_extent(value: f32) -> bool {
    is_measurement(value) && value >= 0.0
}

/// A point. `x` grows rightwards, `y` downwards.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

/// A width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

/// An axis-aligned rectangle given by its top-left corner and extent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left edge.
    pub x: f32,
    /// Top edge.
    pub y: f32,
    /// Horizontal extent.
    pub width: f32,
    /// Vertical extent.
    pub height: f32,
}

impl Rect {
    /// Builds a rectangle from its top-left corner and extent.
    #[inline]
    #[must_use]
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Right edge (`x + width`).
    #[inline]
    #[must_use]
    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    /// Bottom edge (`y + height`).
    #[inline]
    #[must_use]
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(r.right(), 4.0);
        assert_eq!(r.bottom(), 6.0);
    }
}
