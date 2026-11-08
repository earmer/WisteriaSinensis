//! Interactive types: caret, hit-test, movement and selection data (spec §6).
//!
//! Data only. The queries that produce and consume them (`hit_test`,
//! `caret_geometry`, `move_cursor`, `selection_geometry`) arrive in later phases.

use crate::types::{BidiLevel, ByteIndex, Rect};

/// Which side of an ambiguous boundary a caret belongs to (spec §6.1).
///
/// Flutter's definition: upstream leans toward the start of the string. At a
/// soft wrap, upstream is the end of the first line and downstream the start of
/// the second; affinity also resolves a bidi boundary. Explicit newlines are
/// unambiguous `[C1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    /// Leans toward the following text.
    #[default]
    Downstream,
    /// Leans toward the preceding text.
    Upstream,
}

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

/// One visual boundary on a line, in visual order (spec §6.1). Two stops sit at
/// each direction boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretStop {
    /// Line-local x position.
    pub x: f32,
    /// Logical byte offset at this stop.
    pub offset: ByteIndex,
    /// Affinity this stop represents.
    pub affinity: CaretAffinity,
    /// Bidi level of the slot.
    pub level: BidiLevel,
    /// Index into `PositionedParagraph::lines`.
    pub line: usize,
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
    /// **Forward contract (Phase 1/2).** Selection geometry is not computed in
    /// Phase 0; when it lands it must query `TextShaper::font_metrics` for every
    /// selected instance and union their ascent/descent.
    #[default]
    Tight,
    /// Includes line gap and maximum inline-box height.
    Max,
    /// The paragraph's resolved strut (`ParagraphAst::strut_metrics` after `LineHeight::resolve_strut`).
    Strut,
}

use super::output::AtomicFragment;

impl AtomicFragment {
    /// Snaps to the entire span's outer endpoints by midpoint rule.
    /// Even level: left half → start/Downstream, right half → end/Upstream.
    /// Odd level: halves swap. Midpoint itself counts as right half.
    pub fn hit_test(&self, x: f32) -> CursorPosition {
        let in_left_half = x < self.x_start + 0.5 * (self.x_end - self.x_start);
        // even: left→start, right→end ; odd: left→end, right→start
        let snaps_to_start = in_left_half == (self.bidi_level & 1 == 0);
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
    // A non-finite or non-positive width cannot describe a ligature; never
    // accept supplied positions against it.
    if atom_count <= 1 || !width.is_finite() || width <= 0.0 {
        return Vec::new();
    }
    let fallback = || -> Vec<f32> {
        (1..atom_count)
            // Scale the index before multiplying, so a large finite width
            // cannot overflow the intermediate.
            .map(|i| width * (i as f32 / atom_count as f32))
            .collect()
    };
    if carets.len() != atom_count - 1 {
        return fallback();
    }
    if carets
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0 || *v >= width)
    {
        return fallback();
    }
    if carets.windows(2).any(|w| w[1] <= w[0]) {
        return fallback();
    }
    carets.to_vec()
}
