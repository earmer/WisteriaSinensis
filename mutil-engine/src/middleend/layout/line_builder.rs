//! Line placement, line-box calculation, alignment, and visual run assembly
//! (spec §5 Stages 2–4).

use super::super::{
    DecorationKind, DecorationLine, HangingWhitespaceMode, InlineSlotConstraint, LayoutInputError,
    LineGeometry, PositionedGlyph, PositionedInlineBox, PositionedLine, PositionedSlot,
    PositionedTab, VisualGlyphRun,
};
use super::hanging::{LineHangs, clip_to_content, mark_hanging};
use super::line_artifacts::{CaretContext, SpanContext};
use super::{Break, Layout, LineCursor, LineEnd, PieceKind, PlacedPiece, Token};
use crate::frontend::bidi::{LineClass, reset_line_levels, visual_order as bidi_order};
use crate::frontend::{BoxNode, SpanEdge, WordNode};
use crate::types::{AtomFlags, BidiLevel, RawShapedGlyph, TextRange, WritingMode};
use smallvec::SmallVec;

/// Horizontal extent of the line's body (excluding trailing whitespace) in the
/// current coordinate frame.
#[derive(Clone, Copy)]
pub(super) struct LineBounds {
    pub(super) body_lo: f32,
    pub(super) body_hi: f32,
}

/// A line's glyph runs before alignment, the inline extent of its visible
/// entities (`None` when it has none), and the end of its last visible
/// source byte.
struct VisualRuns {
    runs: Vec<VisualGlyphRun>,
    extent: Option<(f32, f32)>,
    visible_end: usize,
}

impl VisualRuns {
    fn extend(&mut self, a: f32, b: f32) {
        let (lo, hi) = (a.min(b), a.max(b));
        self.extent = Some(
            self.extent
                .map_or((lo, hi), |(l, h)| (l.min(lo), h.max(hi))),
        );
    }
}

/// Where a line's content block lies in its slot (SLT-3), after alignment.
struct ContentBlock {
    start: f32,
    width: f32,
}

impl ContentBlock {
    fn edges(&self) -> (f32, f32) {
        (self.start, self.start + self.width)
    }
}

/// A line placed in logical order and ready for its output: the pieces,
/// their visual order, and what the line's end and hanging marks leave.
struct LineBuildState<'a> {
    index: usize,
    placed: &'a mut [PlacedPiece],
    order: &'a [usize],
    body_len: usize,
    soft_wrap: bool,
    byte_range: TextRange,
    bounds: LineBounds,
    end_break: Option<&'a Break>,
    post: f32,
    hangs: LineHangs,
    is_justified: bool,
    block_top: f32,
    geometry: &'a LineGeometry,
}

impl LineBuildState<'_> {
    /// The span walk's view of the line in its current coordinates.
    fn span_context(&self) -> SpanContext<'_> {
        SpanContext {
            placed: self.placed,
            order: self.order,
            bounds: self.bounds,
            end_break: self.end_break,
            post: self.post,
            line_bytes: self.byte_range,
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Places the line `(cursor.token_start, end]` in the first slot of `geometry`.
    pub(super) fn place(
        &mut self,
        index: usize,
        cursor: &LineCursor,
        end: LineEnd,
        geometry: &LineGeometry,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.slots[0];
        // The break is cloned: placing the line borrows `self` mutably for
        // its metrics while the break lives in `self.tokens`.
        let (last_piece, end_break, byte_end, soft_wrap, pre) = match end {
            LineEnd::At(t) => {
                let Token::Break(b) = &self.tokens[t] else {
                    unreachable!("a line ends at a break token")
                };
                (t, Some(b.clone()), b.bytes.end(), !b.mandatory, b.pre)
            }
            LineEnd::Before(t) => (t, None, self.piece(t).bytes.start(), true, 0.0),
        };

        let mut placed = self.collect_placed_pieces(cursor.token_start, last_piece);
        let body_len = placed
            .iter()
            .rposition(|p| p.piece.is_content())
            .map_or(0, |k| k + 1);
        self.apply_l1(&mut placed);
        let order = visual_order(&placed[..body_len]);

        let rtl = self.base.is_rtl();
        let post = cursor.post_break_width;
        let (left_pad, right_pad) = if rtl { (pre, post) } else { (post, pre) };
        let first_line = index == 0;
        let start = self.start_hang(&placed[..body_len], first_line, post);
        // Widths in logical order: a tab's depends on its distance from the
        // line-start edge (spec ST3-1), the start mark's protrusion outside.
        let mut distance = post - LineHangs::protrusion(start);
        for p in &mut placed[..body_len] {
            distance += p.gap;
            p.width = self.measure(&p.piece, distance, slot).0;
            distance += p.width;
        }
        // A soft-wrapped line is justified; one that ends the paragraph or at
        // a hard break is only ever compressed, and only on request (ST3-3).
        let compressible = self.options.justify && (soft_wrap || self.options.compress_last_lines);
        let shrink = if compressible {
            self.shrink_capacity(&mut placed[..body_len])
        } else {
            0.0
        };
        let needed = distance + pre - shrink;
        let fits_without =
            needed <= slot.width() + super::super::tolerance::tolerance(needed, slot.width());
        let end = self.end_hang(&placed[..body_len], first_line, pre, start, fits_without);
        let hangs = LineHangs { start, end };
        // The visible width `W`, hanging halves outside (ST3-2).
        let visible_width = distance + pre - LineHangs::protrusion(end);
        let slack = slot.width() - visible_width;
        let is_justified = compressible
            && slot.width().is_finite()
            && (soft_wrap || slack < 0.0)
            && self.justify(&mut placed[..body_len], slack);
        let mut x = left_pad - hangs.physical(rtl).0;
        for &k in &order {
            x += placed[k].gap;
            placed[k].x = x;
            x += placed[k].width;
        }
        let body_end = x + right_pad;
        // In RTL the trailing whitespace lies left of everything on the line,
        // a hanging mark's protrusion included (HNG-3).
        let far_edge = if rtl {
            -hangs.physical(rtl).0
        } else {
            body_end
        };
        self.place_trailing_whitespace(&mut placed[body_len..], far_edge, rtl);

        self.build_line(LineBuildState {
            index,
            placed: &mut placed,
            order: &order,
            body_len,
            soft_wrap,
            byte_range: TextRange::new(cursor.byte_start, byte_end),
            bounds: LineBounds {
                body_lo: left_pad,
                body_hi: body_end,
            },
            end_break: end_break.as_ref(),
            post,
            hangs,
            is_justified,
            block_top: cursor.block_top,
            geometry,
        })
    }

    /// UAX #9 L1 on the line (spec ST2-1): tabs, the whitespace before them
    /// and the whitespace at the line end take the paragraph level, read
    /// from glue classes, cluster flags and tabs, never from the text. A span
    /// boundary is transparent and then takes the level of its span's next
    /// (leading) or previous (trailing) item on the line.
    fn apply_l1(&self, placed: &mut [PlacedPiece]) {
        let class = |p: &PlacedPiece| match p.piece.kind {
            PieceKind::Tab => LineClass::Separator,
            PieceKind::Glue { .. } | PieceKind::SpanEdge { .. } => LineClass::Whitespace,
            PieceKind::Cluster { cluster, .. }
                if self.ast.clusters[cluster]
                    .flags
                    .contains(AtomFlags::L1_RESETTABLE) =>
            {
                LineClass::Whitespace
            }
            PieceKind::Cluster { .. } | PieceKind::Object { .. } => LineClass::Other,
        };
        let mut levels: SmallVec<[BidiLevel; 64]> = placed.iter().map(|p| p.piece.level).collect();
        reset_line_levels(
            placed.iter().map(class),
            &mut levels,
            self.base.base_level(),
        );
        for (p, level) in placed.iter_mut().zip(levels) {
            if !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) {
                p.piece.level = level;
            }
        }
        for k in 0..placed.len() {
            let PieceKind::SpanEdge { span, edge } = placed[k].piece.kind else {
                continue;
            };
            let span = &self.ast.decoration_spans[span];
            let is_item = |p: &&PlacedPiece| {
                !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) && !p.piece.bytes.is_empty()
            };
            let neighbour = match edge {
                SpanEdge::Leading => placed[k + 1..].iter().find(is_item),
                SpanEdge::Trailing => placed[..k].iter().rev().find(is_item),
            };
            placed[k].piece.level = neighbour
                .filter(|n| span.byte_range.contains_range(n.piece.bytes))
                .map_or_else(|| self.base.base_level(), |n| n.piece.level);
        }
    }

    fn collect_placed_pieces(&self, start: usize, last_piece: usize) -> Vec<PlacedPiece> {
        let mut placed: Vec<PlacedPiece> = Vec::new();
        let mut gap = 0.0;
        for token in &self.tokens[start..last_piece] {
            match token {
                Token::Piece(p) => {
                    placed.push(PlacedPiece {
                        piece: *p,
                        gap,
                        x: 0.0,
                        width: 0.0,
                        expansion: 0.0,
                        y_shift: 0.0,
                    });
                    gap = 0.0;
                }
                Token::Break(b) => gap += b.no_break,
            }
        }
        placed
    }

    /// Trailing whitespace beyond the far edge `far_edge` (UAX #9 L1, HNG-3); synthetic
    /// glue collapses there under both modes (WN-10, ST1-12).
    fn place_trailing_whitespace(&self, trailing: &mut [PlacedPiece], far_edge: f32, rtl: bool) {
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        let trailing_width = |p: &PlacedPiece| match p.piece.kind {
            PieceKind::Glue { item } => {
                let glue = self.glue(item);
                if glue.kind.is_synthetic() || (collapsible && glue.is_collapsible) {
                    0.0
                } else {
                    p.piece.width
                }
            }
            _ => p.piece.width,
        };
        // Away from the body: leftwards in RTL, rightwards in LTR.
        let mut tx = far_edge;
        for p in trailing {
            p.width = trailing_width(p);
            if rtl {
                tx -= p.width;
                p.x = tx;
            } else {
                p.x = tx;
                tx += p.width;
            }
        }
    }

    fn build_line(
        &mut self,
        mut state: LineBuildState<'_>,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = state.geometry.slots[0];
        let line_box = self.compute_line_box(state.placed, state.order, state.byte_range, slot)?;
        let VisualRuns {
            mut runs,
            extent,
            visible_end,
        } = self.build_visual_runs(&state);
        let content = self.align_line_content(&mut state, &mut runs, extent, slot);
        let mut visual_runs = runs;
        let content_edges = content.edges();
        mark_hanging(&mut visual_runs, state.placed, state.hangs, content_edges);
        let (visual_boxes, tabs) = self.build_boxes_and_tabs(state.placed, slot);
        let mut span_fragments = self.span_fragments(&state.span_context());

        let mut decorations: Vec<DecorationLine> = Vec::new();
        for kind in [DecorationKind::Underline, DecorationKind::Strikethrough] {
            self.decorations(kind, state.placed, state.order, &mut decorations)?;
        }
        let atomic_fragments = self.atomic_fragments(state.placed, state.order, state.byte_range);
        clip_to_content(
            state.hangs.physical(self.base.is_rtl()),
            content_edges,
            &mut span_fragments,
            &mut decorations,
        );
        let caret_stops = self.caret_stops(&CaretContext {
            index: state.index,
            placed: state.placed,
            order: state.order,
            body_len: state.body_len,
            soft_wrap: state.soft_wrap,
            byte_range: state.byte_range,
            visible_end,
            content_start: content.start,
            end_break: state.end_break,
            runs: &visual_runs,
            boxes: &visual_boxes,
        });

        // The line fills its first slot; the others stay empty (ME-2).
        let slots: SmallVec<[PositionedSlot; 1]> = state
            .geometry
            .slots
            .iter()
            .enumerate()
            .map(|(i, &constraint)| match i {
                0 => PositionedSlot {
                    constraint,
                    content_start: content.start,
                    visual_width: content.width,
                },
                _ => PositionedSlot {
                    constraint,
                    content_start: constraint.start(),
                    visual_width: 0.0,
                },
            })
            .collect();
        let baseline_y = state.block_top
            + if self.ast.writing_mode == WritingMode::VerticalLr {
                line_box.descent
            } else {
                line_box.ascent
            };
        Ok(PositionedLine {
            slots,
            is_ellipsized: false,
            annotations: Vec::new(),
            atomic_fragments,
            byte_range: state.byte_range,
            visible_byte_end: visible_end,
            baseline_y,
            ascent: line_box.ascent,
            descent: line_box.descent,
            line_height: line_box.line_height,
            is_justified: state.is_justified,
            visual_runs,
            visual_boxes,
            tabs,
            span_fragments,
            caret_stops,
            decorations,
        })
    }

    /// Aligns the line in `slot` (SLT-3): finds the content block from the
    /// glyph and box `extent` and the span fragments, then shifts every
    /// placed piece, glyph and the body bounds by the same amount, so every
    /// later stage reads final coordinates.
    fn align_line_content(
        &self,
        state: &mut LineBuildState<'_>,
        runs: &mut [VisualGlyphRun],
        extent: Option<(f32, f32)>,
        slot: InlineSlotConstraint,
    ) -> ContentBlock {
        let (mut lo, mut hi) = extent.unwrap_or((f32::INFINITY, f32::NEG_INFINITY));
        self.span_fragment_extents(&state.span_context(), |a: f32, b: f32| {
            lo = lo.min(a.min(b));
            hi = hi.max(a.max(b));
        });
        let rtl = self.base.is_rtl();
        // A hanging mark's protrusion lies outside the content (ST1-3).
        let (hang_left, hang_right) = state.hangs.physical(rtl);
        let (content_lo, width) = if lo > hi {
            (if rtl { state.bounds.body_hi } else { 0.0 }, 0.0)
        } else if rtl {
            let lo = lo + hang_left;
            (lo, hi.max(state.bounds.body_hi) - hang_right - lo)
        } else {
            (0.0, hi - hang_right)
        };
        // Alignment: the line start of the base direction.
        let start = if rtl && slot.end().is_finite() && width <= slot.width() {
            slot.end() - width
        } else {
            slot.start()
        };
        let shift = start - content_lo;
        for p in state.placed.iter_mut() {
            p.x += shift;
        }
        for g in runs.iter_mut().flat_map(|r| r.glyphs.iter_mut()) {
            g.x += shift;
        }
        state.bounds.body_lo += shift;
        state.bounds.body_hi += shift;
        ContentBlock { start, width }
    }

    /// The line's glyph runs in visual order (OUT-3), the inline extent of
    /// their glyphs, objects and tabs, and the end of the last visible
    /// source byte, in one pass over the body.
    fn build_visual_runs(&self, state: &LineBuildState<'_>) -> VisualRuns {
        let mut out = VisualRuns {
            runs: Vec::new(),
            extent: None,
            visible_end: state.byte_range.start(),
        };
        for &k in state.order {
            let p = state.placed[k];
            match p.piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    self.push_cluster_glyphs(&mut out, &p, run, cluster)
                }
                PieceKind::Object { .. } | PieceKind::Tab => {
                    out.extend(p.x, p.x + p.width);
                    out.visible_end = out.visible_end.max(p.piece.bytes.end());
                }
                PieceKind::Glue { .. } | PieceKind::SpanEdge { .. } => {}
            }
        }
        out
    }

    /// Appends the glyphs of placed cluster `p` to the line's runs, merging
    /// into the last run when font, level, size, orientation and variations
    /// agree.
    fn push_cluster_glyphs(
        &self,
        out: &mut VisualRuns,
        p: &PlacedPiece,
        run: usize,
        cluster: usize,
    ) {
        let r = &self.ast.runs[run];
        let c = &self.ast.clusters[cluster];
        let (glyphs, _) = self.cluster_glyphs(cluster);
        if glyphs.is_empty() {
            return;
        }
        // Justification after an RTL cluster lies before its first glyph
        // in visual order, after an LTR one on its last glyph (ST3-2).
        let level = p.piece.level;
        let rtl_cluster = level.direction().is_rtl();
        let mut pen = p.x + if rtl_cluster { p.expansion } else { 0.0 };
        let mut make_glyph = |g: &RawShapedGlyph| {
            // The extent counts the shaped advance, not the expansion.
            if g.x_advance != 0.0 {
                out.extend(pen, pen + g.x_advance);
            }
            let glyph = PositionedGlyph {
                glyph_id: g.glyph_id,
                x: pen,
                x_offset: g.x_offset,
                y_offset: g.y_offset + p.y_shift,
                advance: g.x_advance,
                byte_range: (!g.is_synthesized).then_some(c.byte_range),
                is_hanging: false,
            };
            pen += g.x_advance;
            glyph
        };
        let mut positioned: Vec<PositionedGlyph> = if rtl_cluster {
            glyphs.iter().rev().map(&mut make_glyph).collect()
        } else {
            glyphs.iter().map(&mut make_glyph).collect()
        };
        if !glyphs.iter().all(|g| g.is_synthesized) {
            out.visible_end = out.visible_end.max(p.piece.bytes.end());
        }
        if !rtl_cluster && let Some(last) = positioned.last_mut() {
            last.advance += p.expansion;
        }
        let same_run = out.runs.last_mut().filter(|v| {
            v.font == c.font
                && v.bidi_level == level
                && v.font_size == r.font_size
                && v.orientation == c.orientation
                && v.variations.as_slice() == r.features.variations()
        });
        if let Some(v) = same_run {
            v.byte_range = v.byte_range.cover(c.byte_range);
            v.glyphs.extend(positioned);
        } else {
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
    fn build_boxes_and_tabs(
        &self,
        placed: &[PlacedPiece],
        slot: InlineSlotConstraint,
    ) -> (Vec<PositionedInlineBox>, Vec<PositionedTab>) {
        let mut visual_boxes: Vec<PositionedInlineBox> = Vec::new();
        let mut tabs: Vec<PositionedTab> = Vec::new();
        for p in placed {
            match p.piece.kind {
                PieceKind::Object { item } => {
                    let WordNode::Box(BoxNode::InlineObject {
                        baseline_align,
                        byte_range,
                        box_id,
                        overflow_policy,
                        bidi_level,
                        ..
                    }) = &self.ast.items[item]
                    else {
                        unreachable!("an object piece names an object item")
                    };
                    let (w, a, d) = self.fitted_object(item, slot);
                    visual_boxes.push(PositionedInlineBox {
                        baseline_align: *baseline_align,
                        box_id: *box_id,
                        byte_range: *byte_range,
                        x: p.x,
                        y: -p.y_shift,
                        width: w,
                        ascent: a,
                        descent: d,
                        overflow_policy: *overflow_policy,
                        bidi_level: *bidi_level,
                    });
                }
                PieceKind::Tab => tabs.push(PositionedTab {
                    byte_range: p.piece.bytes,
                    x: p.x,
                    width: p.width,
                }),
                PieceKind::Cluster { .. } | PieceKind::Glue { .. } | PieceKind::SpanEdge { .. } => {
                }
            }
        }
        (visual_boxes, tabs)
    }
}

/// Visual indices of a line's pieces from their levels (UAX #9 L2, spec
/// ST2-2), through the frontend's shared rule.
pub(super) fn visual_order(placed: &[PlacedPiece]) -> Vec<usize> {
    let levels: SmallVec<[BidiLevel; 64]> = placed.iter().map(|p| p.piece.level).collect();
    bidi_order(&levels)
}
