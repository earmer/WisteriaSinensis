//! Hanging punctuation (spec ST1-3, HNG-1 to HNG-4).

use super::scratch::LogicalPiece;
use super::{Layout, Piece, PieceKind, Reshapes};
use crate::middleend::{DecorationLine, HangEndMode, HangSide, PositionedSpanFragment};
use crate::types::AtomFlags;

// Full-width CJK marks keep ink in one half, so 0.85 protrudes the ink itself (HNG-1).
pub(super) const HANG_FRACTION: f64 = 0.85;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum LineEdge {
    Start,
    End,
}

#[derive(Clone, Copy)]
pub(super) struct EndHang {
    pub(super) pre: f64,
    pub(super) start: Option<(usize, f64)>,
    pub(super) fits_without: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct LineHangs {
    pub(super) start: Option<(usize, f64)>,
    pub(super) end: Option<(usize, f64)>,
}

impl LineHangs {
    pub(super) fn protrusion(hang: Option<(usize, f64)>) -> f64 {
        hang.map_or(0.0, |(_, protrusion)| protrusion)
    }

    pub(super) fn physical(&self, rtl: bool) -> (f64, f64) {
        let (start, end) = (Self::protrusion(self.start), Self::protrusion(self.end));
        if rtl { (end, start) } else { (start, end) }
    }
}

impl Layout<'_, '_> {
    pub(super) fn hang_protrusion(
        &self,
        piece: &Piece<'_>,
        edge: LineEdge,
        first_line: bool,
        is_soft_wrap: bool,
        pending: Option<&Reshapes>,
    ) -> Option<f64> {
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
        let c = self.inputs.cluster(cluster);
        (c.flags.contains(flag)
            && piece.level == self.base.base_level()
            && self.options.hang_mode.allows(side, self.base, first_line))
        .then(|| {
            let raw = self.cluster_glyphs_over(pending, cluster).1;
            let outer_trim = match edge {
                LineEdge::Start => self.start_edge_trim(piece, is_soft_wrap, pending),
                LineEdge::End => 0.0,
            };
            (raw * HANG_FRACTION - outer_trim).max(0.0)
        })
    }

    fn outermost<'p>(
        body: &'p [LogicalPiece<'_>],
        edge: LineEdge,
    ) -> Option<(usize, &'p LogicalPiece<'p>)> {
        let visible = |(_, p): &(usize, &LogicalPiece<'_>)| !p.piece.is_edge_transparent();
        match edge {
            LineEdge::Start => body.iter().enumerate().find(visible),
            LineEdge::End => body.iter().enumerate().rev().find(visible),
        }
    }

    fn hang_at(
        &self,
        body: &[LogicalPiece<'_>],
        edge: LineEdge,
        first_line: bool,
        is_soft_wrap: bool,
        allowed: impl FnOnce(usize) -> bool,
        pending: &Reshapes,
    ) -> Option<(usize, f64)> {
        let (k, item) = Self::outermost(body, edge).filter(|&(k, _)| allowed(k))?;
        Some((
            k,
            self.hang_protrusion(&item.piece, edge, first_line, is_soft_wrap, Some(pending))?,
        ))
    }

    pub(super) fn start_hang(
        &self,
        body: &[LogicalPiece<'_>],
        first_line: bool,
        is_soft_wrap: bool,
        post: f64,
        pending: &Reshapes,
    ) -> Option<(usize, f64)> {
        self.hang_at(
            body,
            LineEdge::Start,
            first_line,
            is_soft_wrap,
            |_| post == 0.0,
            pending,
        )
    }

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
    ) -> Option<(usize, f64)> {
        self.hang_at(
            body,
            LineEdge::End,
            first_line,
            false,
            |k| {
                pre == 0.0
                    && start.is_none_or(|(s, _)| s != k)
                    && (self.options.hang_mode.end == HangEndMode::Force || !fits_without)
            },
            pending,
        )
    }
}

pub(super) fn clip_to_content(
    (hang_left, hang_right): (f64, f64),
    (left, right): (f64, f64),
    fragments: &mut [PositionedSpanFragment],
    decorations: &mut [DecorationLine],
) {
    let clip = |x: &mut f32, width: &mut f32| {
        let (mut lo, mut hi) = (f64::from(*x), f64::from(*x) + f64::from(*width));
        if hang_left > 0.0 {
            lo = lo.max(left);
        }
        if hang_right > 0.0 {
            hi = hi.min(right);
        }
        *x = lo as f32;
        *width = (hi - lo).max(0.0) as f32;
    };
    for f in fragments {
        clip(&mut f.x, &mut f.width);
    }
    for d in decorations {
        clip(&mut d.x, &mut d.width);
    }
}
