//! Minimal geometric primitives used by the middle-end output and the
//! interaction types (spec §6.4, §6.6). All values are in `f32` layout units
//! (the engine is unit-agnostic); the y axis grows downwards.

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
