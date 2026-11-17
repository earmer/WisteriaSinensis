//! Underline and strike-through segments (spec ST5-1).

use super::super::{Layout, PieceKind, PlacedPiece};
use crate::middleend::{DecorationKind, DecorationLine, LayoutInputError};
use crate::types::TextRange;

/// Where one decorated cluster draws its line, from its font's metrics.
struct Stroke {
    y_offset: f32,
    thickness: f32,
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Underline or strike-through segments merged across adjacent decorated
    /// clusters and the glue between them (spec ST5-1).
    pub(in crate::middleend::layout) fn decorations(
        &mut self,
        kind: DecorationKind,
        placed: &[PlacedPiece],
        order: &[usize],
        out: &mut Vec<DecorationLine>,
    ) -> Result<(), LayoutInputError> {
        let mut open: Option<DecorationLine> = None;
        // Glue after the open segment: it joins only if another decorated
        // cluster follows.
        let mut pending_glue: Option<TextRange> = None;
        for &k in order {
            let placed_piece = placed[k];
            match placed_piece.piece.kind {
                PieceKind::Cluster { run, cluster } if self.is_decorated(kind, run) => {
                    let stroke = self.stroke(kind, run, cluster, placed_piece.y_shift)?;
                    match open.as_mut() {
                        Some(line) => extend(line, placed_piece, stroke, pending_glue.take()),
                        None => open = Some(start(kind, placed_piece, stroke)),
                    }
                }
                PieceKind::Glue { .. } => {
                    if open.is_some() {
                        let bytes = placed_piece.piece.bytes;
                        pending_glue = Some(pending_glue.map_or(bytes, |glue| glue.cover(bytes)));
                    }
                }
                PieceKind::SpanEdge { .. } => {}
                _ => {
                    out.extend(open.take());
                    pending_glue = None;
                }
            }
        }
        out.extend(open);
        Ok(())
    }

    /// Whether the run's style draws `kind`.
    fn is_decorated(&self, kind: DecorationKind, run: usize) -> bool {
        let d = self.ast.runs[run].decoration;
        match kind {
            DecorationKind::Underline => d.underline,
            DecorationKind::Strikethrough => d.strikethrough,
        }
    }

    /// The stroke of `kind` under a cluster raised by `shift`.
    fn stroke(
        &mut self,
        kind: DecorationKind,
        run: usize,
        cluster: usize,
        shift: f32,
    ) -> Result<Stroke, LayoutInputError> {
        let m = self.cluster_metrics(run, cluster)?;
        let position = match kind {
            DecorationKind::Underline => m.underline_position,
            DecorationKind::Strikethrough => m.strikeout_position,
        };
        Ok(Stroke {
            y_offset: position + shift,
            thickness: m.underline_thickness,
        })
    }
}

/// A new segment over one decorated cluster.
fn start(kind: DecorationKind, placed_piece: PlacedPiece, stroke: Stroke) -> DecorationLine {
    let (a, b) = (placed_piece.x, placed_piece.x + placed_piece.width);
    DecorationLine {
        kind,
        byte_range: placed_piece.piece.bytes,
        x: a,
        width: b - a,
        y_offset: stroke.y_offset,
        thickness: stroke.thickness,
    }
}

/// Extends `line` over the pending `glue` and the next decorated cluster.
fn extend(
    line: &mut DecorationLine,
    placed_piece: PlacedPiece,
    stroke: Stroke,
    glue: Option<TextRange>,
) {
    if let Some(bytes) = glue {
        line.byte_range = line.byte_range.cover(bytes);
    }
    let right = placed_piece.x + placed_piece.width;
    line.width = right.max(line.x + line.width) - line.x;
    line.byte_range = line.byte_range.cover(placed_piece.piece.bytes);
    // The lowest underline of the fonts it crosses (ST5-1).
    if line.kind == DecorationKind::Underline && stroke.y_offset < line.y_offset {
        line.y_offset = stroke.y_offset;
        line.thickness = stroke.thickness;
    }
}
