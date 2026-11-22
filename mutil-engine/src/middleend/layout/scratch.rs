//! Per-paragraph reusable line-placement buffers (spec §5 Stages 2–4).
// Deliberate deviation from ADR-0003 rule 2: phases mutate these buffers in place to avoid per-line allocations.

use super::line_artifacts::{CaretScratch, VisualIndex};
use super::{Piece, PlacedPiece};
use crate::middleend::validated::VisualOrder;
use crate::types::BidiLevel;

#[derive(Clone, Copy, Debug)]
pub(super) struct LogicalPiece<'a> {
    pub(super) piece: Piece<'a>,
    pub(super) gap: f64,
}

#[derive(Default)]
pub(super) struct LayoutScratch<'a> {
    pub(super) logical: Vec<LogicalPiece<'a>>,
    pub(super) levels: Vec<BidiLevel>,
    pub(super) order: VisualOrder,
    pub(super) widths: Vec<f64>,
    pub(super) expansions: Vec<f64>,
    pub(super) xs: Vec<f64>,
    pub(super) y_shifts: Vec<f64>,
    pub(super) lead_trims: Vec<f64>,
    pub(super) trail_trims: Vec<f64>,
    pub(super) placed: Vec<PlacedPiece<'a>>,
    pub(super) visual: VisualIndex,
    pub(super) carets: CaretScratch,
}

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
