//! A placed line's glyph runs in visual order, inline boxes and tabs
//! (spec §5 Stage 4, OUT-3).

use super::line_builder::PlacedLine;
use super::{Layout, PieceKind, PlacedPiece, Reshapes};
use crate::middleend::{
    InlineSlotConstraint, PositionedGlyph, PositionedInlineBox, PositionedTab, VisualGlyphRun,
};
use crate::types::{RawShapedGlyph, TextRange};

/// A line's glyph runs before alignment, the inline extent of its visible
/// entities (`None` when it has none), and the end of its last visible
/// source byte.
pub(super) struct VisualRuns {
    pub(super) runs: Vec<VisualGlyphRun>,
    pub(super) extent: Option<(f32, f32)>,
    pub(super) visible_end: usize,
}

impl VisualRuns {
    fn extend(&mut self, a: f32, b: f32) {
        extend_extent(&mut self.extent, a, b);
    }
}

/// Widens `extent` to hold `[a, b]` in either order.
fn extend_extent(extent: &mut Option<(f32, f32)>, a: f32, b: f32) {
    let (lo, hi) = (a.min(b), a.max(b));
    *extent = Some(extent.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
}

/// How one cluster's glyphs are placed: from pen position `pen`, raised by
/// the cluster's baseline shift, owning its bytes unless synthesized.
struct ClusterGlyphs {
    pen: f32,
    y_shift: f32,
    byte_range: TextRange,
    /// An LTR cluster's justification, on its last glyph in visual order.
    ltr_expansion: Option<f32>,
}

impl ClusterGlyphs {
    /// Appends `glyphs`, already in visual order, to `target`, widening
    /// `extent` by each shaped advance (not the expansion).
    fn push<'g>(
        &self,
        target: &mut Vec<PositionedGlyph>,
        glyphs: impl Iterator<Item = &'g RawShapedGlyph>,
        extent: &mut Option<(f32, f32)>,
    ) {
        let mut pen = self.pen;
        target.extend(glyphs.map(|g| {
            if g.x_advance != 0.0 {
                extend_extent(extent, pen, pen + g.x_advance);
            }
            let glyph = PositionedGlyph {
                glyph_id: g.glyph_id,
                x: pen,
                x_offset: g.x_offset,
                y_offset: g.y_offset + self.y_shift,
                advance: g.x_advance,
                byte_range: (!g.is_synthesized).then_some(self.byte_range),
            };
            pen += g.x_advance;
            glyph
        }));
        if let (Some(expansion), Some(last)) = (self.ltr_expansion, target.last_mut()) {
            last.advance += expansion;
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    /// The line's glyph runs in visual order (OUT-3), the inline extent of
    /// their glyphs, objects and tabs, and the end of the last visible
    /// source byte, in one pass over the body.
    pub(super) fn build_visual_runs(
        &self,
        line: &PlacedLine<'_>,
        pending: &Reshapes,
    ) -> VisualRuns {
        let mut out = VisualRuns {
            runs: Vec::new(),
            extent: None,
            visible_end: line.byte_range.start(),
        };
        for &k in &line.order {
            let p = line.placed[k];
            match p.piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    self.push_cluster_glyphs(&mut out, &p, (run, cluster), pending)
                }
                PieceKind::Object(_) | PieceKind::Tab => {
                    out.extend(p.x, p.x + p.width);
                    out.visible_end = out.visible_end.max(p.piece.bytes.end());
                }
                PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => {}
            }
        }
        out
    }

    /// Appends the glyphs of placed cluster `p` to the line's runs, merging
    /// into the last run when font, level, size, orientation and variations
    /// agree. The glyphs go straight into that run, in visual order.
    fn push_cluster_glyphs(
        &self,
        out: &mut VisualRuns,
        p: &PlacedPiece<'_>,
        (run, cluster): (usize, usize),
        pending: &Reshapes,
    ) {
        let r = &self.ast.runs[run];
        let c = &self.ast.clusters[cluster];
        let (glyphs, _) = self.cluster_glyphs(pending, cluster);
        if glyphs.is_empty() {
            return;
        }
        // Justification after an RTL cluster lies before its first glyph
        // in visual order, after an LTR one on its last glyph (ST3-2).
        let level = p.piece.level;
        let rtl_cluster = level.direction().is_rtl();
        let pen = p.x + if rtl_cluster { p.expansion } else { 0.0 };
        if !glyphs.iter().all(|g| g.is_synthesized) {
            out.visible_end = out.visible_end.max(p.piece.bytes.end());
        }
        let cluster_glyphs = ClusterGlyphs {
            pen,
            y_shift: p.y_shift,
            byte_range: c.byte_range,
            ltr_expansion: (!rtl_cluster).then_some(p.expansion),
        };
        let extent = &mut out.extent;
        let mut place = |target: &mut Vec<PositionedGlyph>| {
            if rtl_cluster {
                cluster_glyphs.push(target, glyphs.iter().rev(), extent);
            } else {
                cluster_glyphs.push(target, glyphs.iter(), extent);
            }
        };
        let joins_last = |v: &&mut VisualGlyphRun| {
            v.font == c.font
                && v.bidi_level == level
                && v.font_size == r.font_size
                && v.orientation == c.orientation
                && v.variations.as_slice() == r.features.variations()
        };
        if let Some(v) = out.runs.last_mut().filter(joins_last) {
            v.byte_range = v.byte_range.cover(c.byte_range);
            place(&mut v.glyphs);
        } else {
            let mut positioned = Vec::with_capacity(glyphs.len());
            place(&mut positioned);
            out.runs.push(VisualGlyphRun {
                orientation: c.orientation,
                font_size: r.font_size,
                variations: r.features.variations().iter().copied().collect(),
                font: c.font,
                bidi_level: level,
                byte_range: c.byte_range,
                glyphs: positioned,
            });
        }
    }

    /// Boxes and tabs are built in logical order directly.
    pub(super) fn build_boxes_and_tabs(
        &self,
        placed: &[PlacedPiece<'_>],
        slot: InlineSlotConstraint,
    ) -> (Vec<PositionedInlineBox>, Vec<PositionedTab>) {
        let mut visual_boxes: Vec<PositionedInlineBox> = Vec::new();
        let mut tabs: Vec<PositionedTab> = Vec::new();
        for p in placed {
            match p.piece.kind {
                PieceKind::Object(object) => {
                    let (w, a, d) = Self::fitted_object(object, slot);
                    visual_boxes.push(PositionedInlineBox {
                        baseline_align: object.baseline_align,
                        box_id: object.box_id,
                        byte_range: object.byte_range,
                        x: p.x,
                        y: -p.y_shift,
                        width: w,
                        ascent: a,
                        descent: d,
                        overflow_policy: object.overflow_policy,
                        bidi_level: object.bidi_level,
                    });
                }
                PieceKind::Tab => tabs.push(PositionedTab {
                    byte_range: p.piece.bytes,
                    x: p.x,
                    width: p.width,
                }),
                PieceKind::Cluster { .. } | PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => {}
            }
        }
        (visual_boxes, tabs)
    }
}
