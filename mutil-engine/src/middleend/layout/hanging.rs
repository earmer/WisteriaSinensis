//! Hanging punctuation (spec ST1-3, HNG-1 to HNG-4): which mark hangs at
//! each edge of a line, by [`HANG_FRACTION`] of its advance.

use super::scratch::LogicalPiece;
use super::{Layout, Piece, PieceKind, Reshapes};
use crate::middleend::{DecorationLine, HangEndMode, HangSide, PositionedSpanFragment};
use crate::types::AtomFlags;

/// The part of a hanging mark's advance that protrudes past the content
/// block (ST1-3). A full-width CJK mark keeps its ink in one half of its em
/// box, so a half-advance hang would push only the blank half out; at 0.85
/// the ink itself protrudes, and 0.15 of the advance stays inside (HNG-1).
pub(super) const HANG_FRACTION: f32 = 0.85;

/// The logical edge of a line a mark hangs from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LineEdge {
    Start,
    End,
}

/// What the end mark's hang depends on besides the body (ST1-3).
#[derive(Clone, Copy)]
pub(super) struct EndHang {
    /// Cloned padding after the mark.
    pub(super) pre: f32,
    /// The start mark, which the end mark may not be.
    pub(super) start: Option<(usize, f32)>,
    /// Whether the line fits without hanging the mark.
    pub(super) fits_without: bool,
}

/// The marks hanging on one line: for each, its index into the placed
/// pieces and the part of its advance that protrudes (ST1-3).
#[derive(Clone, Copy, Default)]
pub(super) struct LineHangs {
    pub(super) start: Option<(usize, f32)>,
    pub(super) end: Option<(usize, f32)>,
}

impl LineHangs {
    /// The protrusion of a mark, or 0.
    pub(super) fn protrusion(hang: Option<(usize, f32)>) -> f32 {
        hang.map_or(0.0, |(_, protrusion)| protrusion)
    }

    /// The protrusion at the physical left and right edges.
    pub(super) fn physical(&self, rtl: bool) -> (f32, f32) {
        let (start, end) = (Self::protrusion(self.start), Self::protrusion(self.end));
        if rtl { (end, start) } else { (start, end) }
    }
}

impl Layout<'_, '_> {
    /// The protruding part of `piece` if it may hang at `edge` of a line
    /// (the first line when `first_line`): a mark of the edge's class whose
    /// post-L1 level is the paragraph's, on a side the mode allows (ST1-3,
    /// HNG-2). Whether anything lies between it and the edge is the caller's
    /// to judge. `pending` re-shapes, not yet committed, apply to its width.
    pub(super) fn hang_protrusion(
        &self,
        piece: &Piece<'_>,
        edge: LineEdge,
        first_line: bool,
        pending: Option<&Reshapes>,
    ) -> Option<f32> {
        let PieceKind::Cluster { cluster, .. } = piece.kind else {
            return None;
        };
        let flag = match edge {
            LineEdge::Start => AtomFlags::HANGABLE_START,
            LineEdge::End => AtomFlags::HANGABLE_END,
        };
        let rtl = self.base.is_rtl();
        let side = if (edge == LineEdge::Start) != rtl {
            HangSide::Left
        } else {
            HangSide::Right
        };
        let c = &self.ast.clusters[cluster];
        (c.flags.contains(flag)
            && piece.level == self.base.base_level()
            && self.options.hang_mode.allows(side, self.base, first_line))
        .then(|| self.cluster_glyphs_over(pending, cluster).1 * HANG_FRACTION)
    }

    /// The index of the first or last visible piece of `body`: only
    /// zero-width span boundaries may lie between a mark and its edge.
    fn outermost(body: &[LogicalPiece<'_>], edge: LineEdge) -> Option<usize> {
        let visible = |k: &usize| !body[*k].piece.is_edge_transparent();
        match edge {
            LineEdge::Start => (0..body.len()).find(visible),
            LineEdge::End => (0..body.len()).rev().find(visible),
        }
    }

    /// The mark at `edge` of `body` and its protrusion, if the index of the
    /// outermost visible piece passes `allowed`.
    fn hang_at(
        &self,
        body: &[LogicalPiece<'_>],
        edge: LineEdge,
        first_line: bool,
        allowed: impl FnOnce(usize) -> bool,
        pending: &Reshapes,
    ) -> Option<(usize, f32)> {
        let k = Self::outermost(body, edge).filter(|&k| allowed(k))?;
        Some((
            k,
            self.hang_protrusion(&body[k].piece, edge, first_line, Some(pending))?,
        ))
    }

    /// The mark hanging at the start of a line whose body is `body`, with
    /// no cloned padding (`post`) before it (ST1-3).
    pub(super) fn start_hang(
        &self,
        body: &[LogicalPiece<'_>],
        first_line: bool,
        post: f32,
        pending: &Reshapes,
    ) -> Option<(usize, f32)> {
        self.hang_at(body, LineEdge::Start, first_line, |_| post == 0.0, pending)
    }

    /// The mark hanging at the end of a line whose body is `body`, with no
    /// cloned padding (`pre`) after it and not the start mark: under `Allow`
    /// only when the line does not fit without it (`fits_without`, ST1-3).
    pub(super) fn end_hang(
        &self,
        body: &[LogicalPiece<'_>],
        first_line: bool,
        EndHang {
            pre,
            start,
            fits_without,
        }: EndHang,
        pending: &Reshapes,
    ) -> Option<(usize, f32)> {
        self.hang_at(
            body,
            LineEdge::End,
            first_line,
            |k| {
                pre == 0.0
                    && start.is_none_or(|(s, _)| s != k)
                    && (self.options.hang_mode.end == HangEndMode::Force || !fits_without)
            },
            pending,
        )
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
