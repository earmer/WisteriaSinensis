//! Per-paragraph working buffers for line placement (spec §5 Stages 2–4).
//!
//! `Layout::place_lines` owns one [`LayoutScratch`] and lends it to each line.
//! Every buffer is cleared, never freed, between lines, so a paragraph
//! allocates its working memory once instead of once per line. A line's
//! phases fill the columns in order (`logical`, then the level, width,
//! expansion, position and shift columns), and the line's [`PlacedPiece`]s
//! are built once, in one literal, when every column is final.
//!
//! Deliberate deviation from ADR-0003 rule 2: the measuring and placement
//! phases fill these columns through `&mut` instead of returning a value,
//! since returning one would allocate per line, which this scratch exists to
//! avoid. Only the orchestrator, `place`, holds the scratch. It lends each
//! phase only the columns that phase reads or fills, in the order above, so
//! a column is final before any later phase reads it.

use super::line_artifacts::{CaretScratch, VisualIndex};
use super::{Piece, PlacedPiece};
use crate::middleend::validated::VisualOrder;
use crate::types::BidiLevel;

/// A piece of the current line in logical order, with the no-break gap
/// before it: what is known before the line is measured.
#[derive(Clone, Copy, Debug)]
pub(super) struct LogicalPiece<'a> {
    pub(super) piece: Piece<'a>,
    pub(super) gap: f64,
}

/// The reusable columns and buffers of one line's placement.
#[derive(Default)]
pub(super) struct LayoutScratch<'a> {
    /// The line's pieces in logical order (UAX #9 L1 levels once applied).
    pub(super) logical: Vec<LogicalPiece<'a>>,
    /// The body's levels, for UAX #9 L1 and L2.
    pub(super) levels: Vec<BidiLevel>,
    /// The body's visual order (UAX #9 L2).
    pub(super) order: VisualOrder,
    /// Each piece's advance, by logical index.
    pub(super) widths: Vec<f64>,
    /// Each piece's justification inside its advance (ST3-2).
    pub(super) expansions: Vec<f64>,
    /// Each piece's inline position.
    pub(super) xs: Vec<f64>,
    /// Each piece's baseline shift (BAS-1).
    pub(super) y_shifts: Vec<f64>,
    /// Each piece's CJK punctuation leading trim.
    pub(super) lead_trims: Vec<f64>,
    /// Each piece's CJK punctuation trailing trim.
    pub(super) trail_trims: Vec<f64>,
    /// The finished pieces, built from the columns above.
    pub(super) placed: Vec<PlacedPiece<'a>>,
    /// The visual order indexed for span grouping (ST2-3).
    pub(super) visual: VisualIndex,
    /// The caret-stop working buffers (§6.1).
    pub(super) carets: CaretScratch,
}

/// Rebuilds `placed` from the line's final columns, one literal per piece:
/// each piece of `logical` with its `x`, width, expansion, baseline shift,
/// and CJK punctuation trims.
pub(super) fn build_placed<'a>(
    placed: &mut Vec<PlacedPiece<'a>>,
    logical: &[LogicalPiece<'a>],
    [xs, widths, expansions, y_shifts, lead_trims, trail_trims]: [&[f64]; 6],
) {
    placed.clear();
    placed.extend(
        logical
            .iter()
            .zip(xs)
            .zip(widths)
            .zip(expansions)
            .zip(y_shifts)
            .zip(lead_trims)
            .zip(trail_trims)
            .map(
                |((((((p, &x), &width), &expansion), &y_shift), &lead_trim), &trail_trim)| {
                    PlacedPiece {
                        piece: p.piece,
                        x,
                        width,
                        expansion,
                        y_shift,
                        lead_trim,
                        trail_trim,
                    }
                },
            ),
    );
}
