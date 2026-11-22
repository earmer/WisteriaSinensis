//! Line-box metrics and baseline shifts (spec §5 Stage 4, BAS-1).

use super::scratch::LogicalPiece;
use super::{Layout, PieceKind};
use crate::middleend::shaper_gate::resolved_line_box;
use crate::middleend::validated::{BodyIdx, RunId, VisualOrder};
use crate::middleend::{InlineSlotConstraint, LayoutInputError, LineHeight, StrutMetrics};
use crate::types::{BaselineAlignment, FontMetrics, TextRange};

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineRelativeEdge {
    Top,
    Bottom,
}

impl LineRelativeEdge {
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
    piece_index: BodyIdx,
    edge: LineRelativeEdge,
    above: f64,
    below: f64,
}

enum BaselinePlacement {
    LineEdge(LineRelativeItem),
    StrutRelative {
        shift: f64,
        extent: Option<(f64, f64)>,
    },
}

#[derive(Clone, Copy)]
struct CachedClusterMetrics {
    font_id: u32,
    scale_bits: u32,
    run: RunId,
    own: FontMetrics,
    line_box: Option<StrutMetrics>,
}

pub(super) struct LineBoxMetrics {
    pub(super) ascent: f64,
    pub(super) descent: f64,
    pub(super) line_height: f64,
}

impl Layout<'_, '_> {
    pub(super) fn compute_line_box(
        &mut self,
        placed: &[LogicalPiece<'_>],
        order: &VisualOrder,
        y_shifts: &mut Vec<f64>,
        byte_range: TextRange,
        slot: InlineSlotConstraint,
        extra: Option<(f64, f64)>,
    ) -> Result<LineBoxMetrics, LayoutInputError> {
        y_shifts.clear();
        y_shifts.resize(placed.len(), 0.0);
        let placed_t = order.table(placed);
        let mut shifts_t = order.table_mut(y_shifts);
        let normal = matches!(self.options.line_height, LineHeight::Normal);
        let (mut above, mut below) = half_leaded(self.strut);
        if let Some((extra_above, extra_below)) = extra {
            above = above.max(extra_above);
            below = below.max(extra_below);
        }
        let mut line_relative: Vec<LineRelativeItem> = Vec::new();
        let mut cached: Option<CachedClusterMetrics> = None;
        for k in order.indices() {
            let item = |edge, (above, below)| {
                BaselinePlacement::LineEdge(LineRelativeItem {
                    piece_index: k,
                    edge,
                    above,
                    below,
                })
            };
            let placement = match placed_t.get(k).piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    let run_ref = self.inputs.run(run);
                    let align = run_ref.baseline_align;
                    let edge = LineRelativeEdge::of(align);
                    let needs_box = edge.is_some() || normal;
                    let font = self.inputs.cluster(cluster).font;
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
                                    run_ref.font_size,
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
                    shifts_t.set(k, shift);
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
            shifts_t.set(
                item.piece_index,
                match item.edge {
                    LineRelativeEdge::Top => metrics.ascent - item.above,
                    LineRelativeEdge::Bottom => item.below - metrics.descent,
                },
            );
        }
        Ok(metrics)
    }

    fn resolve_line_box(&self, above: f64, below: f64, byte_range: TextRange) -> LineBoxMetrics {
        let strut_height = self.strut_height();
        let (ascent, descent, line_height) = match self.options.line_height {
            LineHeight::Fixed(_) => (
                f64::from(self.strut.ascent),
                f64::from(self.strut.descent),
                strut_height,
            ),
            LineHeight::Normal if byte_range.is_empty() => {
                let (a, d) = half_leaded(self.strut);
                (a, d, strut_height)
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

pub(super) fn half_leaded(m: StrutMetrics) -> (f64, f64) {
    let half_gap = f64::from(m.line_gap) / 2.0;
    (
        f64::from(m.ascent) + half_gap,
        f64::from(m.descent) + half_gap,
    )
}

fn deepen_for_line_relative(items: &[LineRelativeItem], above: &mut f64, below: &mut f64) {
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

fn baseline_offset(align: BaselineAlignment, m: &FontMetrics) -> Option<f64> {
    match align {
        BaselineAlignment::Auto | BaselineAlignment::Alphabetic => Some(0.0),
        BaselineAlignment::Ideographic => Some(f64::from(m.ideographic_baseline_offset)),
        BaselineAlignment::Central => Some(f64::from(m.central_baseline_offset)),
        BaselineAlignment::Hanging => Some(f64::from(m.hanging_baseline_offset)),
        BaselineAlignment::Mathematical => Some(f64::from(m.mathematical_baseline_offset)),
        BaselineAlignment::LineTop
        | BaselineAlignment::LineBottom
        | BaselineAlignment::Offset(_) => None,
    }
}

fn baseline_shift(align: BaselineAlignment, strut: &FontMetrics, own: Option<&FontMetrics>) -> f64 {
    match align {
        BaselineAlignment::Offset(offset) => f64::from(offset),
        _ => baseline_offset(align, strut).map_or(0.0, |s| {
            s - own.and_then(|m| baseline_offset(align, m)).unwrap_or(0.0)
        }),
    }
}
