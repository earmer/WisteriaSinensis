//! Visual glyph runs, inline boxes, and tabs (spec §5 Stage 4, OUT-3).

use super::line_builder::{AlignedLine, PlacedLine};
use super::{Layout, PieceKind, PlacedPiece, Reshapes};
use crate::middleend::validated::{ClusterId, RunId};
use crate::middleend::{
    InlineSlotConstraint, PositionedGlyph, PositionedInlineBox, PositionedTab, VisualGlyphRun,
};
use crate::types::{
    BidiLevel, FontRef, FontVariation, GlyphOrientation, RawShapedGlyph, TextRange,
};
use smallvec::SmallVec;

fn extend_extent(extent: &mut Option<(f64, f64)>, a: f64, b: f64) {
    let (lo, hi) = (a.min(b), a.max(b));
    *extent = Some(extent.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
}

#[derive(Clone, PartialEq)]
struct RunKey {
    font: FontRef,
    level: BidiLevel,
    font_size: f32,
    x_scale: f32,
    orientation: GlyphOrientation,
    variations: SmallVec<[FontVariation; 2]>,
}

impl RunKey {
    fn of_run(v: &VisualGlyphRun) -> Self {
        Self {
            font: v.font,
            level: v.bidi_level,
            font_size: v.font_size,
            x_scale: v.x_scale,
            orientation: v.orientation,
            variations: v.variations.clone(),
        }
    }
}

struct ClusterGlyphs {
    pen: f64,
    y_shift: f64,
    byte_range: TextRange,
    ltr_expansion: Option<f64>,
    left_trim: f64,
    total_trim: f64,
    x_mul: f64,
    y_mul: f64,
}

fn trimmed_glyphs<'g>(
    mut pen: f64,
    mut remaining_trim: f64,
    x_mul: f64,
    glyphs: impl Iterator<Item = &'g RawShapedGlyph>,
) -> impl Iterator<Item = (&'g RawShapedGlyph, f64, f64)> {
    glyphs.map(move |g| {
        let x_adv = f64::from(g.x_advance) * x_mul;
        let trim = remaining_trim.min(x_adv.max(0.0));
        remaining_trim -= trim;
        let advance = x_adv - trim;
        let start = pen;
        pen += advance;
        (g, start, advance)
    })
}

impl ClusterGlyphs {
    fn push<'g>(
        &self,
        target: &mut Vec<PositionedGlyph>,
        glyphs: impl Iterator<Item = &'g RawShapedGlyph>,
    ) {
        let start_len = target.len();
        let mut last_advance = 0.0_f64;
        target.extend(
            trimmed_glyphs(self.pen, self.total_trim, self.x_mul, glyphs).map(
                |(g, pen, advance)| {
                    last_advance = advance;
                    PositionedGlyph {
                        glyph_id: g.glyph_id,
                        x: pen as f32,
                        x_offset: (f64::from(g.x_offset) * self.x_mul - self.left_trim) as f32,
                        y_offset: (f64::from(g.y_offset) * self.y_mul + self.y_shift) as f32,
                        advance: advance as f32,
                        byte_range: (!g.is_synthesized).then_some(self.byte_range),
                    }
                },
            ),
        );
        if let Some(expansion) = self.ltr_expansion
            && target.len() > start_len
            && let Some(last) = target.last_mut()
        {
            last.advance = (last_advance + expansion) as f32;
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    pub(super) fn visual_extent(
        &self,
        line: &PlacedLine<'_, '_>,
        pending: &Reshapes,
    ) -> (Option<(f64, f64)>, usize) {
        let mut extent = None;
        let mut visible_end = line.byte_range.start();
        for &p in line.order.pick(line.placed) {
            match p.piece.kind {
                PieceKind::Cluster { cluster, .. } => {
                    let (glyphs, _) = self.cluster_glyphs(pending, cluster);
                    if glyphs.is_empty() {
                        continue;
                    }
                    let rtl = p.piece.level.direction().is_rtl();
                    let pen = p.x + if rtl { p.expansion } else { 0.0 };
                    if glyphs.iter().any(|g| !g.is_synthesized) {
                        visible_end = visible_end.max(p.piece.bytes.end());
                    }
                    let trim = p.lead_trim + p.trail_trim;
                    let ltr_expansion = if rtl { 0.0 } else { p.expansion };
                    let x_mul = self.font_scale * line.x_scale;
                    let mut scan = |it: &mut dyn Iterator<Item = &RawShapedGlyph>| {
                        let mut last = None;
                        for (_, s, adv) in trimmed_glyphs(pen, trim, x_mul, it) {
                            if adv != 0.0 {
                                extend_extent(&mut extent, s, s + adv);
                            }
                            last = Some((s, adv));
                        }
                        if ltr_expansion != 0.0
                            && let Some((s, adv)) = last
                        {
                            extend_extent(&mut extent, s, s + adv + ltr_expansion);
                        }
                    };
                    if rtl {
                        scan(&mut glyphs.iter().rev());
                    } else {
                        scan(&mut glyphs.iter());
                    }
                }
                PieceKind::Object(_) | PieceKind::Tab => {
                    extend_extent(&mut extent, p.x, p.x + p.width);
                    visible_end = visible_end.max(p.piece.bytes.end());
                }
                PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => {}
            }
        }
        (extent, visible_end)
    }

    pub(super) fn build_visual_runs(
        &self,
        line: &AlignedLine<'_, '_>,
        pending: &Reshapes,
    ) -> Vec<VisualGlyphRun> {
        let sizes = self.run_sizes(line, pending);
        let mut runs = Vec::with_capacity(sizes.len());
        let mut sizes = sizes.into_iter();
        for &p in line.order.pick(line.placed) {
            if let PieceKind::Cluster { run, cluster } = p.piece.kind {
                self.push_cluster_glyphs(
                    &mut runs,
                    &p,
                    (run, cluster),
                    line.x_scale,
                    pending,
                    &mut sizes,
                );
            }
        }
        runs
    }

    fn run_key(
        &self,
        run: RunId,
        cluster: ClusterId,
        level: BidiLevel,
        x_scale: f64,
        pending: &Reshapes,
    ) -> RunKey {
        let (r, c) = (self.inputs.run(run), self.inputs.cluster(cluster));
        let mut variations: SmallVec<[FontVariation; 2]> =
            r.features.variations().iter().copied().collect();
        if let Some(var) = pending
            .get(&cluster)
            .or_else(|| self.reshaped.get(&cluster))
            .and_then(|res| res.wdth)
            .and_then(|w| FontVariation::try_new(*b"wdth", w))
        {
            variations.retain(|v| v.tag() != *b"wdth");
            variations.push(var);
        }
        RunKey {
            font: c.font,
            level,
            font_size: (f64::from(r.font_size) * self.font_scale) as f32,
            x_scale: x_scale as f32,
            orientation: c.orientation,
            variations,
        }
    }

    fn run_sizes(&self, line: &PlacedLine<'_, '_>, pending: &Reshapes) -> SmallVec<[usize; 8]> {
        let mut sizes = SmallVec::new();
        let mut last = None;
        for p in line.order.pick(line.placed) {
            let PieceKind::Cluster { run, cluster } = p.piece.kind else {
                continue;
            };
            let count = self.cluster_glyphs(pending, cluster).0.len();
            if count == 0 {
                continue;
            }
            let key = self.run_key(run, cluster, p.piece.level, line.x_scale, pending);
            match sizes.last_mut().filter(|_| last.as_ref() == Some(&key)) {
                Some(size) => *size += count,
                None => sizes.push(count),
            }
            last = Some(key);
        }
        sizes
    }

    fn push_cluster_glyphs(
        &self,
        runs: &mut Vec<VisualGlyphRun>,
        p: &PlacedPiece<'_>,
        (run, cluster): (RunId, ClusterId),
        x_scale: f64,
        pending: &Reshapes,
        sizes: &mut impl Iterator<Item = usize>,
    ) {
        let c = self.inputs.cluster(cluster);
        let (glyphs, _) = self.cluster_glyphs(pending, cluster);
        if glyphs.is_empty() {
            return;
        }
        let level = p.piece.level;
        let rtl_cluster = level.direction().is_rtl();
        let pen = p.x + if rtl_cluster { p.expansion } else { 0.0 };
        let cluster_glyphs = ClusterGlyphs {
            pen,
            y_shift: p.y_shift,
            byte_range: c.byte_range,
            ltr_expansion: (!rtl_cluster).then_some(p.expansion),
            left_trim: if rtl_cluster {
                p.trail_trim
            } else {
                p.lead_trim
            },
            total_trim: p.lead_trim + p.trail_trim,
            x_mul: self.font_scale * x_scale,
            y_mul: self.font_scale,
        };
        let place = |target: &mut Vec<PositionedGlyph>| {
            if rtl_cluster {
                cluster_glyphs.push(target, glyphs.iter().rev());
            } else {
                cluster_glyphs.push(target, glyphs.iter());
            }
        };
        let key = self.run_key(run, cluster, level, x_scale, pending);
        let joins_last = |v: &&mut VisualGlyphRun| RunKey::of_run(v) == key;
        if let Some(v) = runs.last_mut().filter(joins_last) {
            v.byte_range = v.byte_range.cover(c.byte_range);
            place(&mut v.glyphs);
        } else {
            let mut positioned = Vec::with_capacity(sizes.next().unwrap_or(glyphs.len()));
            place(&mut positioned);
            runs.push(VisualGlyphRun {
                orientation: c.orientation,
                font_size: key.font_size,
                x_scale: key.x_scale,
                variations: key.variations,
                font: c.font,
                bidi_level: level,
                byte_range: c.byte_range,
                glyphs: positioned,
            });
        }
    }

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
                    let (_, a, d) = Self::fitted_object(object, slot);
                    visual_boxes.push(PositionedInlineBox {
                        baseline_align: object.baseline_align,
                        box_id: object.box_id,
                        byte_range: object.byte_range,
                        x: p.x as f32,
                        y: (-p.y_shift) as f32,
                        width: p.width as f32,
                        ascent: a as f32,
                        descent: d as f32,
                        overflow_policy: object.overflow_policy,
                        bidi_level: object.bidi_level,
                    });
                }
                PieceKind::Tab => tabs.push(PositionedTab {
                    byte_range: p.piece.bytes,
                    x: p.x as f32,
                    width: p.width as f32,
                }),
                PieceKind::Cluster { .. } | PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => {}
            }
        }
        (visual_boxes, tabs)
    }
}
