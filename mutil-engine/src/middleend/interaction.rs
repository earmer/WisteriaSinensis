//! Interactive types: caret, hit-test, movement and selection data (spec §6).
//!
//! Data only. The queries that produce and consume them (`hit_test`,
//! `caret_geometry`, `move_cursor`, `selection_geometry`) are not implemented yet.

use super::output::{AtomicFragment, CaretAffinity};
use crate::types::{BidiLevel, ByteIndex, Rect};

/// A caret location: offset plus affinity (spec §6.1).
///
/// Invariant: `byte_offset` is a grapheme atom boundary. Deletion ranges are
/// not so constrained (§6.5), but they are not carets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorPosition {
    /// Atom-aligned byte offset.
    pub byte_offset: ByteIndex,
    /// Affinity at wraps and bidi boundaries.
    pub affinity: CaretAffinity,
}

/// One visual piece of a selection (spec §6.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRect {
    /// Rectangle in paragraph coordinates.
    pub rect: Rect,
    /// Index into `PositionedParagraph::lines`.
    pub line: usize,
    /// Bidi level of the covered slots.
    pub level: BidiLevel,
    /// `true` for the optional extension drawn for a selected newline.
    pub is_line_end_extension: bool,
}

/// Result of `hit_test` (spec §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HitResult {
    /// Nearest caret position.
    pub position: CursorPosition,
    /// `false` when the point was clamped to the first or last line, or to a
    /// line edge (DirectWrite's `isInside` convention `[C1]`).
    pub inside: bool,
}

/// Caret rectangles for one position (spec §6.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretVisual {
    /// Base-direction insertion point (Pango/Android "strong" caret).
    pub primary_rect: Rect,
    /// Opposite-direction insertion point; `Some` only if its x differs from the
    /// primary's.
    pub secondary_rect: Option<Rect>,
    /// Caret slant in degrees clockwise from vertical (italic fonts); 0 if upright.
    pub slant_angle: f32,
}

/// A cursor movement request (spec §6.5). `Left`/`Right` are physical and
/// visual; `Previous`/`Next` are logical, because Firefox's word commands mapped
/// to the wrong direction in RTL text `[C2]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    /// One visual stop to the left.
    Left,
    /// One visual stop to the right.
    Right,
    /// One atom backwards in logical order.
    Previous,
    /// One atom forwards in logical order.
    Next,
    /// Previous UAX #29 word boundary, physically to the left.
    WordLeft,
    /// Next UAX #29 word boundary, physically to the right.
    WordRight,
    /// Previous UAX #29 word boundary in logical order.
    WordPrevious,
    /// Next UAX #29 word boundary in logical order.
    WordNext,
    /// Start of the visual line.
    LineStart,
    /// End of the visual line.
    LineEnd,
    /// One line up, keeping the preferred x held by the caller.
    Up,
    /// One line down, keeping the preferred x held by the caller.
    Down,
}

/// A selection stored logically as anchor and focus (spec §6.5). Shift+arrow
/// moves `focus` visually; the anchor stays put. `anchor == focus` is a caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Selection {
    /// The fixed end.
    pub anchor: CursorPosition,
    /// The moving end.
    pub focus: CursorPosition,
}

/// Height policy for selection rectangles, after Flutter's `BoxHeightStyle`
/// `[C1]` (spec §6.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoxHeightStyle {
    /// Union of selected font-instance ascent/descent, excluding gap and leading.
    ///
    /// **Forward contract.** Selection geometry is not computed yet; when it
    /// is, it must query `TextShaper::font_metrics` for every
    /// selected instance and union their ascent/descent.
    #[default]
    Tight,
    /// Includes line gap and maximum inline-box height.
    Max,
    /// The paragraph's resolved strut (`ParagraphAst::strut_metrics` after `LineHeight::resolve_strut`).
    Strut,
}

impl AtomicFragment {
    /// Snaps to the entire span's outer endpoints by midpoint rule.
    /// Even level: left half → start/Downstream, right half → end/Upstream.
    /// Odd level: halves swap. Midpoint itself counts as right half.
    pub fn hit_test(&self, x: f32) -> CursorPosition {
        let midpoint = self.x_start + 0.5 * (self.x_end - self.x_start);
        let is_in_left_half = x < midpoint;
        let is_even_level = self.bidi_level & 1 == 0;
        // Even levels map the left half to the start; odd levels swap halves.
        let snaps_to_start = is_in_left_half == is_even_level;
        if snaps_to_start {
            CursorPosition {
                byte_offset: self.full_byte_range.start,
                affinity: CaretAffinity::Downstream,
            }
        } else {
            CursorPosition {
                byte_offset: self.full_byte_range.end,
                affinity: CaretAffinity::Upstream,
            }
        }
    }
}
/// Validates GDEF subdivisions or falls back to K equal atom slots.
pub fn resolve_ligature_carets(width: f32, atom_count: usize, carets: &[f32]) -> Vec<f32> {
    // A non-finite or non-positive width cannot describe a ligature, so no
    // supplied position is accepted against it.
    if atom_count <= 1 || !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }

    if carets_are_usable(width, atom_count, carets) {
        carets.to_vec()
    } else {
        equal_caret_slots(width, atom_count)
    }
}

fn carets_are_usable(width: f32, atom_count: usize, carets: &[f32]) -> bool {
    if carets.len() != atom_count - 1 {
        return false;
    }
    let within_width = carets
        .iter()
        .all(|position| position.is_finite() && *position > 0.0 && *position < width);
    let strictly_increasing = carets.windows(2).all(|pair| pair[0] < pair[1]);
    within_width && strictly_increasing
}

/// K equal slots. The index is scaled before multiplying so a large finite
/// width cannot overflow the intermediate.
fn equal_caret_slots(width: f32, atom_count: usize) -> Vec<f32> {
    (1..atom_count)
        .map(|index| width * (index as f32 / atom_count as f32))
        .collect()
}

/// A physical direction resolved once at the interaction boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlowMotion {
    /// Previous visual inline stop.
    InlinePrev,
    /// Next visual inline stop.
    InlineNext,
    /// Previous block-axis line.
    BlockPrev,
    /// Next block-axis line.
    BlockNext,
}
impl Motion {
    /// Resolves the four physical arrows; logical and word commands return `None`.
    pub const fn flow_direction(self, mode: crate::types::WritingMode) -> Option<FlowMotion> {
        use crate::types::WritingMode::*;
        use FlowMotion::*;
        match (mode, self) {
            (HorizontalTb, Self::Left) | (VerticalRl | VerticalLr, Self::Up) => Some(InlinePrev),
            (HorizontalTb, Self::Right) | (VerticalRl | VerticalLr, Self::Down) => Some(InlineNext),
            (HorizontalTb, Self::Up) | (VerticalRl, Self::Right) | (VerticalLr, Self::Left) => {
                Some(BlockPrev)
            }
            (HorizontalTb, Self::Down) | (VerticalRl, Self::Left) | (VerticalLr, Self::Right) => {
                Some(BlockNext)
            }
            _ => None,
        }
    }
}
