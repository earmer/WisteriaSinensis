//! The line box (spec §5 Stage 4, BAS-1): the line's ascent and descent
//! from the strut and the items on it, and each item's baseline shift.

use super::super::{InlineSlotConstraint, LayoutInputError, LineHeight, StrutMetrics};
use super::{Layout, PieceKind, PlacedPiece};
use crate::frontend::{BoxNode, WordNode};
use crate::types::{BaselineAlignment, FontMetrics, TextRange};

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineRelativeEdge {
    Top,
    Bottom,
}

impl LineRelativeEdge {
    /// The line-box edge an alignment is placed against, if it is placed
    /// against the finished line box at all.
    fn of(align: BaselineAlignment) -> Option<Self> {
        match align {
            BaselineAlignment::LineTop => Some(Self::Top),
            BaselineAlignment::LineBottom => Some(Self::Bottom),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
struct LineRelativeItem {
    piece_index: usize,
    edge: LineRelativeEdge,
    above: f32,
    below: f32,
}

pub(super) struct LineBoxMetrics {
    pub(super) ascent: f32,
    pub(super) descent: f32,
    pub(super) line_height: f32,
}

impl Layout<'_, '_> {
    /// Computes line-box ascent/descent and assigns `y_shift` to every piece
    /// (spec §5 Stage 4, BAS-1).
    pub(super) fn compute_line_box(
        &mut self,
        placed: &mut [PlacedPiece],
        order: &[usize],
        byte_range: TextRange,
        slot: InlineSlotConstraint,
    ) -> Result<LineBoxMetrics, LayoutInputError> {
        let normal = matches!(self.options.line_height, LineHeight::Normal);
        let half = |m: StrutMetrics| (m.ascent + m.line_gap / 2.0, m.descent + m.line_gap / 2.0);
        let (mut above, mut below) = half(self.strut);
        let mut line_relative: Vec<LineRelativeItem> = Vec::new();
        for &k in order {
            let piece = placed[k].piece;
            // The shift is only read for an item on the baseline.
            let (align, item_above, item_below, shift, is_object) = match piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    let align = self.ast.runs[run].baseline_align;
                    let edge = LineRelativeEdge::of(align);
                    let (a, d) = if normal || edge.is_some() {
                        half(self.cluster_box(run, cluster)?)
                    } else {
                        (0.0, 0.0)
                    };
                    let shift = match edge {
                        None => {
                            let own = self.cluster_metrics(run, cluster)?;
                            baseline_shift(align, &self.ast.strut_metrics, Some(&own))
                        }
                        Some(_) => 0.0,
                    };
                    (align, a, d, shift, false)
                }
                PieceKind::Object { item } => {
                    let WordNode::Box(BoxNode::InlineObject { baseline_align, .. }) =
                        &self.ast.items[item]
                    else {
                        unreachable!("an object piece names an object item")
                    };
                    let (_, a, d) = self.fitted_object(item, slot);
                    let shift = baseline_shift(*baseline_align, &self.ast.strut_metrics, None);
                    (*baseline_align, a, d, shift, true)
                }
                _ => continue,
            };
            match LineRelativeEdge::of(align) {
                Some(edge) => line_relative.push(LineRelativeItem {
                    piece_index: k,
                    edge,
                    above: item_above,
                    below: item_below,
                }),
                None => {
                    placed[k].y_shift = shift;
                    if normal || is_object {
                        above = above.max(item_above + shift);
                        below = below.max(item_below - shift);
                    }
                }
            }
        }
        // A line-relative item taller than the line deepens it on its free
        // side under `Normal` (spec BAS-1).
        if normal {
            for item in &line_relative {
                let room = above + below;
                let item_height = item.above + item.below;
                if item_height > room {
                    match item.edge {
                        LineRelativeEdge::Top => below += item_height - room,
                        LineRelativeEdge::Bottom => above += item_height - room,
                    }
                }
            }
        }
        let (ascent, descent, line_height) = match self.options.line_height {
            LineHeight::Fixed(_) => (
                self.strut.ascent,
                self.strut.descent,
                self.strut.line_height(),
            ),
            LineHeight::Normal if byte_range.is_empty() => {
                let (a, d) = half(self.strut);
                (a, d, self.strut.line_height())
            }
            LineHeight::Normal => (above, below, above + below),
        };
        for item in &line_relative {
            placed[item.piece_index].y_shift = match item.edge {
                LineRelativeEdge::Top => ascent - item.above,
                LineRelativeEdge::Bottom => item.below - descent,
            };
        }
        Ok(LineBoxMetrics {
            ascent,
            descent,
            line_height,
        })
    }
}

/// A font's offset of a named baseline from its alphabetic one, positive
/// towards line-over (FNT-4); `None` for the other alignments.
fn baseline_offset(align: BaselineAlignment, m: &FontMetrics) -> Option<f32> {
    match align {
        BaselineAlignment::Auto | BaselineAlignment::Alphabetic => Some(0.0),
        BaselineAlignment::Ideographic => Some(m.ideographic_baseline_offset),
        BaselineAlignment::Central => Some(m.central_baseline_offset),
        BaselineAlignment::Hanging => Some(m.hanging_baseline_offset),
        BaselineAlignment::Mathematical => Some(m.mathematical_baseline_offset),
        BaselineAlignment::LineTop
        | BaselineAlignment::LineBottom
        | BaselineAlignment::Offset(_) => None,
    }
}

/// The shift of an item towards line-over (spec BAS-1): its own named
/// baseline (a run's font's; an object's is its alphabetic one, `own` is
/// `None`) onto the strut's, or an explicit offset.
fn baseline_shift(align: BaselineAlignment, strut: &FontMetrics, own: Option<&FontMetrics>) -> f32 {
    match align {
        BaselineAlignment::Offset(offset) => offset,
        _ => baseline_offset(align, strut).map_or(0.0, |s| {
            s - own.and_then(|m| baseline_offset(align, m)).unwrap_or(0.0)
        }),
    }
}
