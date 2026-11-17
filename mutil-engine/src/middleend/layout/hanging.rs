//! Hanging punctuation (spec ST1-3, HNG-1 to HNG-4): which mark hangs at
//! each edge of a line, by half its advance.

use super::super::tolerance::tolerance;
use super::super::{DecorationLine, HangEndMode, HangSide, PositionedSpanFragment, VisualGlyphRun};
use super::{Layout, Piece, PieceKind, PlacedPiece};
use crate::types::{AtomFlags, BidiLevel};

/// The logical edge of a line a mark hangs from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LineEdge {
    Start,
    End,
}

/// The marks hanging on one line: for each, its index into the placed
/// pieces and the half of its advance that protrudes (ST1-3).
#[derive(Clone, Copy, Default)]
pub(super) struct LineHangs {
    pub(super) start: Option<(usize, f32)>,
    pub(super) end: Option<(usize, f32)>,
}

impl LineHangs {
    /// The protruding half of a mark, or 0.
    pub(super) fn protrusion(hang: Option<(usize, f32)>) -> f32 {
        hang.map_or(0.0, |(_, half)| half)
    }

    /// The protrusion at the physical left and right edges.
    pub(super) fn physical(&self, rtl: bool) -> (f32, f32) {
        let (start, end) = (Self::protrusion(self.start), Self::protrusion(self.end));
        if rtl { (end, start) } else { (start, end) }
    }
}

impl Layout<'_, '_> {
    /// The protruding half of `piece` if it may hang at `edge` of a line
    /// (the first line when `first_line`): a mark of the edge's class whose
    /// post-L1 `level` is the paragraph's, on a side the mode allows (ST1-3, HNG-2). Whether
    /// anything lies between it and the edge is the caller's to judge.
    pub(super) fn hang_half(
        &self,
        piece: &Piece,
        level: BidiLevel,
        edge: LineEdge,
        first_line: bool,
    ) -> Option<f32> {
        let PieceKind::Cluster { cluster, .. } = piece.kind else {
            return None;
        };
        let flag = match edge {
            LineEdge::Start => AtomFlags::HANGABLE_START,
            LineEdge::End => AtomFlags::HANGABLE_END,
        };
        let rtl = self.base.is_rtl();
        let side = match (edge == LineEdge::Start) != rtl {
            true => HangSide::Left,
            false => HangSide::Right,
        };
        let c = &self.ast.clusters[cluster];
        (c.flags.contains(flag)
            && level == self.base.base_level()
            && self.options.hang_mode.allows(side, self.base, first_line))
        .then(|| self.cluster_glyphs(cluster).1 / 2.0)
    }

    /// The index of the first or last visible piece of `body`: only
    /// zero-width span boundaries may lie between a mark and its edge.
    fn outermost(body: &[PlacedPiece], edge: LineEdge) -> Option<usize> {
        let visible = |k: &usize| {
            let p = &body[*k].piece;
            !(matches!(p.kind, PieceKind::SpanEdge { .. }) && p.width == 0.0)
        };
        match edge {
            LineEdge::Start => (0..body.len()).find(visible),
            LineEdge::End => (0..body.len()).rev().find(visible),
        }
    }

    /// The mark hanging at the start of a line whose body is `body`, with
    /// no cloned padding (`post`) before it (ST1-3).
    pub(super) fn start_hang(
        &self,
        body: &[PlacedPiece],
        first_line: bool,
        post: f32,
    ) -> Option<(usize, f32)> {
        let k = Self::outermost(body, LineEdge::Start).filter(|_| post == 0.0)?;
        let p = &body[k];
        Some((
            k,
            self.hang_half(&p.piece, p.level, LineEdge::Start, first_line)?,
        ))
    }

    /// The mark hanging at the end of a line whose body is `body`, with no
    /// cloned padding (`pre`) after it and not the start mark: under `Allow`
    /// only when the line does not fit without it (`fits_without`, ST1-3).
    pub(super) fn end_hang(
        &self,
        body: &[PlacedPiece],
        first_line: bool,
        pre: f32,
        start: Option<(usize, f32)>,
        fits_without: bool,
    ) -> Option<(usize, f32)> {
        let k = Self::outermost(body, LineEdge::End).filter(|&k| {
            pre == 0.0
                && start.is_none_or(|(s, _)| s != k)
                && (self.options.hang_mode.end == HangEndMode::Force || !fits_without)
        })?;
        let p = &body[k];
        Some((
            k,
            self.hang_half(&p.piece, p.level, LineEdge::End, first_line)?,
        ))
    }
}

/// Flags the glyphs of the hanging marks that overflow the content edges
/// `(left, right)` beyond tolerance (OUT-4, HNG-1).
pub(super) fn mark_hanging(
    runs: &mut [VisualGlyphRun],
    placed: &[PlacedPiece],
    hangs: LineHangs,
    (left, right): (f32, f32),
) {
    for (k, _) in [hangs.start, hangs.end].into_iter().flatten() {
        let bytes = placed[k].piece.bytes;
        for g in runs.iter_mut().flat_map(|r| r.glyphs.iter_mut()) {
            let (lo, hi) = g.pen_interval();
            let overflows = lo < left - tolerance(lo, left) || hi > right + tolerance(hi, right);
            if g.byte_range == Some(bytes) && g.advance != 0.0 && overflows {
                g.is_hanging = true;
            }
        }
    }
}

/// Stops span fragments and decorations at the content edge on each side
/// where a mark hangs, so that only the mark protrudes (ST1-3, ST5-1).
pub(super) fn clip_to_content(
    (hang_left, hang_right): (f32, f32),
    (left, right): (f32, f32),
    fragments: &mut [PositionedSpanFragment],
    decorations: &mut [DecorationLine],
) {
    let clip = |x: &mut f32, width: &mut f32| {
        let (mut lo, mut hi) = (*x, *x + *width);
        if hang_left > 0.0 {
            lo = lo.max(left);
        }
        if hang_right > 0.0 {
            hi = hi.min(right);
        }
        *x = lo;
        *width = (hi - lo).max(0.0);
    };
    for f in fragments {
        clip(&mut f.x, &mut f.width);
    }
    for d in decorations {
        clip(&mut d.x, &mut d.width);
    }
}
