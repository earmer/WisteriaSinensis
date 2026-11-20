//! The line box (spec §5 Stage 4, BAS-1): the line's ascent and descent
//! from the strut and the items on it, and each item's baseline shift.

use super::scratch::LogicalPiece;
use super::{Layout, PieceKind};
use crate::middleend::shaper_gate::resolved_line_box;
use crate::middleend::{InlineSlotConstraint, LayoutInputError, LineHeight, StrutMetrics};
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

/// Where an item's baseline goes, classified once from its alignment
/// (BAS-1): against an edge of the finished line box, or shifted from the
/// strut's baseline, with the extent it adds to the line when it counts
/// (under `Normal`, and always for an object).
enum BaselinePlacement {
    LineEdge(LineRelativeItem),
    StrutRelative {
        shift: f32,
        extent: Option<(f32, f32)>,
    },
}

/// Single-entry metrics cache inside [`Layout::compute_line_box`].
#[derive(Clone, Copy)]
struct CachedClusterMetrics {
    font_id: u32,
    scale_bits: u32,
    run: usize,
    own: FontMetrics,
    line_box: Option<StrutMetrics>,
}

pub(super) struct LineBoxMetrics {
    pub(super) ascent: f32,
    pub(super) descent: f32,
    pub(super) line_height: f32,
}

impl Layout<'_, '_> {
    /// Computes line-box ascent/descent and fills `y_shifts` with each body
    /// piece's baseline shift, 0 for the others (spec §5 Stage 4, BAS-1).
    pub(super) fn compute_line_box(
        &mut self,
        placed: &[LogicalPiece<'_>],
        order: &[usize],
        y_shifts: &mut Vec<f32>,
        byte_range: TextRange,
        slot: InlineSlotConstraint,
    ) -> Result<LineBoxMetrics, LayoutInputError> {
        y_shifts.clear();
        y_shifts.resize(placed.len(), 0.0);
        let normal = matches!(self.options.line_height, LineHeight::Normal);
        let (mut above, mut below) = half_leaded(self.strut);
        let mut line_relative: Vec<LineRelativeItem> = Vec::new();
        let mut cached: Option<CachedClusterMetrics> = None;
        for &k in order {
            let item = |edge, (above, below)| {
                BaselinePlacement::LineEdge(LineRelativeItem {
                    piece_index: k,
                    edge,
                    above,
                    below,
                })
            };
            let placement = match placed[k].piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    let align = self.ast.runs[run].baseline_align;
                    let edge = LineRelativeEdge::of(align);
                    let needs_box = edge.is_some() || normal;
                    let font = self.ast.clusters[cluster].font;
                    let scale_bits = font.scale.to_bits();
                    let (own, resolved_box) = match cached {
                        Some(entry)
                            if entry.font_id == font.font_id.0
                                && entry.scale_bits == scale_bits
                                && entry.run == run
                                && (!needs_box || entry.line_box.is_some()) =>
                        {
                            (entry.own, entry.line_box)
                        }
                        _ => {
                            let own = self.cluster_metrics(run, cluster)?;
                            let line_box = if needs_box {
                                Some(resolved_line_box(
                                    &own,
                                    self.ast.runs[run].font_size,
                                    self.options.line_height,
                                    self.ast.writing_mode,
                                )?)
                            } else {
                                None
                            };
                            cached = Some(CachedClusterMetrics {
                                font_id: font.font_id.0,
                                scale_bits,
                                run,
                                own,
                                line_box,
                            });
                            (own, line_box)
                        }
                    };
                    match edge {
                        Some(edge) => item(edge, half_leaded(resolved_box.unwrap_or(self.strut))),
                        None => BaselinePlacement::StrutRelative {
                            shift: baseline_shift(align, &self.ast.strut_metrics, Some(&own)),
                            extent: resolved_box.map(half_leaded),
                        },
                    }
                }
                PieceKind::Object(object) => {
                    let (_, a, d) = Self::fitted_object(object, slot);
                    match LineRelativeEdge::of(object.baseline_align) {
                        Some(edge) => item(edge, (a, d)),
                        None => BaselinePlacement::StrutRelative {
                            shift: baseline_shift(
                                object.baseline_align,
                                &self.ast.strut_metrics,
                                None,
                            ),
                            extent: Some((a, d)),
                        },
                    }
                }
                _ => continue,
            };
            match placement {
                BaselinePlacement::LineEdge(item) => line_relative.push(item),
                BaselinePlacement::StrutRelative { shift, extent } => {
                    y_shifts[k] = shift;
                    if let Some((item_above, item_below)) = extent {
                        above = above.max(item_above + shift);
                        below = below.max(item_below - shift);
                    }
                }
            }
        }
        if normal {
            deepen_for_line_relative(&line_relative, &mut above, &mut below);
        }
        let metrics = self.resolve_line_box(above, below, byte_range);
        for item in &line_relative {
            y_shifts[item.piece_index] = match item.edge {
                LineRelativeEdge::Top => metrics.ascent - item.above,
                LineRelativeEdge::Bottom => item.below - metrics.descent,
            };
        }
        Ok(metrics)
    }

    /// The line box from the content's `above` and `below` the baseline: the
    /// strut's under a fixed line height, or for an empty line (spec MET-3).
    fn resolve_line_box(&self, above: f32, below: f32, byte_range: TextRange) -> LineBoxMetrics {
        let (ascent, descent, line_height) = match self.options.line_height {
            LineHeight::Fixed(_) => (
                self.strut.ascent,
                self.strut.descent,
                self.strut.line_height(),
            ),
            LineHeight::Normal if byte_range.is_empty() => {
                let (a, d) = half_leaded(self.strut);
                (a, d, self.strut.line_height())
            }
            LineHeight::Normal => (above, below, above + below),
        };
        LineBoxMetrics {
            ascent,
            descent,
            line_height,
        }
    }
}

/// A strut's or box's ascent and descent with half its line gap added to each.
fn half_leaded(m: StrutMetrics) -> (f32, f32) {
    (m.ascent + m.line_gap / 2.0, m.descent + m.line_gap / 2.0)
}

/// A line-relative item taller than the line deepens it on its free side
/// under `Normal` (spec BAS-1).
fn deepen_for_line_relative(items: &[LineRelativeItem], above: &mut f32, below: &mut f32) {
    for item in items {
        let room = *above + *below;
        let item_height = item.above + item.below;
        if item_height > room {
            match item.edge {
                LineRelativeEdge::Top => *below += item_height - room,
                LineRelativeEdge::Bottom => *above += item_height - room,
            }
        }
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
