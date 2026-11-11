//! Writing modes and resolved glyph geometry in canonical flow space.
use super::{Point, Rect};

/// Orthogonal mapping from inline/block coordinates to paragraph-local pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WritingMode {
    /// Inline right, block down.
    #[default]
    HorizontalTb,
    /// Inline down, block left.
    VerticalRl,
    /// Inline down, block right.
    VerticalLr,
}

impl WritingMode {
    /// Whether the inline axis is physical vertical.
    pub const fn is_vertical(self) -> bool {
        !matches!(self, Self::HorizontalTb)
    }

    /// Converts a flow point to paragraph-local physical coordinates.
    pub fn flow_to_physical_point(self, p: Point, total_block_size: f32) -> Point {
        match self {
            Self::HorizontalTb => p,
            Self::VerticalLr => Point { x: p.y, y: p.x },
            Self::VerticalRl => Point {
                x: total_block_size - p.y,
                y: p.x,
            },
        }
    }

    /// Converts paragraph-local physical coordinates to flow coordinates.
    pub fn physical_to_flow_point(self, p: Point, total_block_size: f32) -> Point {
        match self {
            Self::HorizontalTb => p,
            Self::VerticalLr => Point { x: p.y, y: p.x },
            Self::VerticalRl => Point {
                x: p.y,
                y: total_block_size - p.x,
            },
        }
    }

    /// Transforms an axis-aligned flow rectangle, normalizing its corners.
    pub fn flow_to_physical_rect(self, r: Rect, total_block_size: f32) -> Rect {
        let a = self.flow_to_physical_point(Point { x: r.x, y: r.y }, total_block_size);
        let b = self.flow_to_physical_point(
            Point {
                x: r.right(),
                y: r.bottom(),
            },
            total_block_size,
        );
        Rect::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }

    /// Inverse of `flow_to_physical_rect` for a physical rectangle.
    pub fn physical_to_flow_rect(self, r: Rect, total_block_size: f32) -> Rect {
        let a = self.physical_to_flow_point(Point { x: r.x, y: r.y }, total_block_size);
        let b = self.physical_to_flow_point(
            Point {
                x: r.right(),
                y: r.bottom(),
            },
            total_block_size,
        );
        Rect::new(
            a.x.min(b.x),
            a.y.min(b.y),
            (a.x - b.x).abs(),
            (a.y - b.y).abs(),
        )
    }

    /// Converts a positive line-over offset into a signed block-axis offset.
    pub const fn over_to_block(self, offset: f32) -> f32 {
        match self {
            Self::VerticalLr => offset,
            _ => -offset,
        }
    }

    /// Block-axis bounds for a baseline and line-over/under extents.
    pub fn line_bounds(self, baseline: f32, ascent: f32, descent: f32) -> (f32, f32) {
        match self {
            Self::VerticalLr => (baseline - descent, baseline + ascent),
            _ => (baseline - ascent, baseline + descent),
        }
    }
}

/// Frontend policy for resolving vertical orientation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextOrientation {
    /// Resolve each character using vertical orientation properties.
    #[default]
    Mixed,
    /// Force upright glyphs and an LTR paragraph.
    Upright,
    /// Shape horizontally and rotate clockwise.
    Sideways,
}

/// Resolved orientation of one shaping unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum GlyphOrientation {
    /// Horizontal shaping in horizontal text.
    #[default]
    Horizontal,
    /// Upright vertical shaping.
    VerticalUpright,
    /// Horizontal shaping rotated clockwise.
    VerticalSideways,
    /// One indivisible horizontal group embedded in vertical text
    /// (tate-chu-yoko).
    ///
    /// A run with this orientation is shaped into exactly one cluster covering
    /// all of its atoms (see [`TextShaper::shape_run`](crate::TextShaper::shape_run)),
    /// and is referenced by exactly one text box that spans the whole run, so
    /// no line break can fall inside it.
    HorizontalInVertical,
}

impl GlyphOrientation {
    /// Checks writing-mode compatibility and the required level parity.
    pub const fn is_valid(self, mode: WritingMode, level: u8) -> bool {
        let compatible = mode.is_vertical() != matches!(self, Self::Horizontal);
        compatible
            && (!matches!(self, Self::VerticalUpright | Self::HorizontalInVertical)
                || level % 2 == 0)
    }
}

/// Baseline selected for an inline entity; offsets are positive toward line-over.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum BaselineAlignment {
    /// Alphabetic horizontally; central vertically.
    #[default]
    Auto,
    /// Alphabetic baseline.
    Alphabetic,
    /// Ideographic baseline.
    Ideographic,
    /// Central baseline.
    Central,
    /// Hanging baseline.
    Hanging,
    /// Mathematical baseline.
    Mathematical,
    /// Align to the line-over edge.
    LineTop,
    /// Align to the line-under edge.
    LineBottom,
    /// Explicit finite offset toward line-over.
    Offset(f32),
}

impl BaselineAlignment {
    /// Explicit offsets must be finite.
    pub fn is_valid(self) -> bool {
        !matches!(self, Self::Offset(v) if !v.is_finite())
    }
}
