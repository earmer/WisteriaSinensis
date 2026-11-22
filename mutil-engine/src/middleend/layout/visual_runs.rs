//! A placed line's glyph runs in visual order, inline boxes and tabs
//! (spec §5 Stage 4, OUT-3).

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

/// Widens `extent` to hold `[a, b]` in either order.
fn extend_extent(extent: &mut Option<(f64, f64)>, a: f64, b: f64) {
    let (lo, hi) = (a.min(b), a.max(b));
    *extent = Some(extent.map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))));
}

/// What a run's glyphs share: consecutive clusters with equal keys merge
/// into one run (OUT-3).
#[derive(Clone, Copy, PartialEq)]
struct RunKey<'k> {
    font: FontRef,
    level: BidiLevel,
    font_size: f32,
    orientation: GlyphOrientation,
    variations: &'k [FontVariation],
}

impl<'k> RunKey<'k> {
    fn of_run(v: &'k VisualGlyphRun) -> Self {
        Self {
            font: v.font,
            level: v.bidi_level,
            font_size: v.font_size,
            orientation: v.orientation,
            variations: &v.variations,
        }
    }
}

/// How one cluster's glyphs are placed: from aligned pen position `pen` in
/// `f64`, raised by the cluster's baseline shift, owning its bytes unless
/// synthesized.
struct ClusterGlyphs {
    pen: f64,
    y_shift: f64,
    byte_range: TextRange,
    /// An LTR cluster's justification, on its last glyph in visual order.
    ltr_expansion: Option<f64>,
    /// Visual-left trim (lead_trim in LTR, trail_trim in RTL).
    left_trim: f64,
    /// Total side-slot trim (lead_trim + trail_trim).
    total_trim: f64,
}

/// Yields `(glyph, pen_start, advance)` for each glyph in `glyphs` in `f64`,
/// deducting `total_trim` from positive advances.
fn trimmed_glyphs<'g>(
    mut pen: f64,
    mut remaining_trim: f64,
    glyphs: impl Iterator<Item = &'g RawShapedGlyph>,
) -> impl Iterator<Item = (&'g RawShapedGlyph, f64, f64)> {
    glyphs.map(move |g| {
        let x_adv = f64::from(g.x_advance);
        let trim = remaining_trim.min(x_adv.max(0.0));
        remaining_trim -= trim;
        let advance = x_adv - trim;
        let start = pen;
        pen += advance;
        (g, start, advance)
    })
}

impl ClusterGlyphs {
    /// Appends `glyphs`, already in visual order, to `target`, advancing `pen`
    /// in `f64` and converting to `f32` once at emission.
    fn push<'g>(
        &self,
        target: &mut Vec<PositionedGlyph>,
        glyphs: impl Iterator<Item = &'g RawShapedGlyph>,
    ) {
        let start_len = target.len();
        let mut last_advance = 0.0_f64;
        target.extend(trimmed_glyphs(self.pen, self.total_trim, glyphs).map(
            |(g, pen, advance)| {
                last_advance = advance;
                PositionedGlyph {
                    glyph_id: g.glyph_id,
                    x: pen as f32,
                    x_offset: (f64::from(g.x_offset) - self.left_trim) as f32,
                    y_offset: (f64::from(g.y_offset) + self.y_shift) as f32,
                    advance: advance as f32,
                    byte_range: (!g.is_synthesized).then_some(self.byte_range),
                }
            },
        ));
        if let Some(expansion) = self.ltr_expansion
            && target.len() > start_len
            && let Some(last) = target.last_mut()
        {
            last.advance = (last_advance + expansion) as f32;
        }
    }
}

/// Accumulates the `f64` inline extent of one cluster's glyphs (net of
/// side-slot trim, not the expansion).
fn extend_cluster_extent<'g>(
    pen: f64,
    total_trim: f64,
    glyphs: impl Iterator<Item = &'g RawShapedGlyph>,
    extent: &mut Option<(f64, f64)>,
) {
    for (_, start, advance) in trimmed_glyphs(pen, total_trim, glyphs) {
        if advance != 0.0 {
            extend_extent(extent, start, start + advance);
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Computes the pre-alignment `f64` inline extent of the line's visible
    /// glyphs, objects and tabs, and the end of its last visible source byte.
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
                    let rtl_cluster = p.piece.level.direction().is_rtl();
                    let pen = p.x + if rtl_cluster { p.expansion } else { 0.0 };
                    if glyphs.iter().any(|g| !g.is_synthesized) {
                        visible_end = visible_end.max(p.piece.bytes.end());
                    }
                    let total_trim = p.lead_trim + p.trail_trim;
                    if rtl_cluster {
                        extend_cluster_extent(pen, total_trim, glyphs.iter().rev(), &mut extent);
                    } else {
                        extend_cluster_extent(pen, total_trim, glyphs.iter(), &mut extent);
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

    /// The line's glyph runs in visual order (OUT-3), built after alignment so
    /// every glyph's `x` is shifted in `f64` and rounded to `f32` once.
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
                self.push_cluster_glyphs(&mut runs, &p, (run, cluster), pending, &mut sizes);
            }
        }
        runs
    }

    /// The key of the run a cluster's glyphs join, at level `level`.
    fn run_key(&self, run: RunId, cluster: ClusterId, level: BidiLevel) -> RunKey<'a> {
        let (r, c) = (self.inputs.run(run), self.inputs.cluster(cluster));
        RunKey {
            font: c.font,
            level,
            font_size: r.font_size,
            orientation: c.orientation,
            variations: r.features.variations(),
        }
    }

    /// How many glyphs each of the line's runs will hold, in visual order,
    /// so each run's glyphs are allocated once.
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
            let key = self.run_key(run, cluster, p.piece.level);
            match sizes.last_mut().filter(|_| last == Some(key)) {
                Some(size) => *size += count,
                None => sizes.push(count),
            }
            last = Some(key);
        }
        sizes
    }

    /// Appends the glyphs of placed cluster `p` to the line's runs, merging
    /// into the last run when font, level, size, orientation and variations
    /// agree. The glyphs go straight into that run, in visual order; a new
    /// run takes the next of `sizes` as its capacity.
    fn push_cluster_glyphs(
        &self,
        runs: &mut Vec<VisualGlyphRun>,
        p: &PlacedPiece<'_>,
        (run, cluster): (RunId, ClusterId),
        pending: &Reshapes,
        sizes: &mut impl Iterator<Item = usize>,
    ) {
        let r = self.inputs.run(run);
        let c = self.inputs.cluster(cluster);
        let (glyphs, _) = self.cluster_glyphs(pending, cluster);
        if glyphs.is_empty() {
            return;
        }
        // Justification after an RTL cluster lies before its first glyph
        // in visual order, after an LTR one on its last glyph (ST3-2).
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
        };
        let place = |target: &mut Vec<PositionedGlyph>| {
            if rtl_cluster {
                cluster_glyphs.push(target, glyphs.iter().rev());
            } else {
                cluster_glyphs.push(target, glyphs.iter());
            }
        };
        let key = self.run_key(run, cluster, level);
        let joins_last = |v: &&mut VisualGlyphRun| RunKey::of_run(v) == key;
        if let Some(v) = runs.last_mut().filter(joins_last) {
            v.byte_range = v.byte_range.cover(c.byte_range);
            place(&mut v.glyphs);
        } else {
            // `run_sizes` counted this run; the hint only sizes the Vec.
            let mut positioned = Vec::with_capacity(sizes.next().unwrap_or(glyphs.len()));
            place(&mut positioned);
            runs.push(VisualGlyphRun {
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
                        x: p.x as f32,
                        y: (-p.y_shift) as f32,
                        width: w as f32,
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
