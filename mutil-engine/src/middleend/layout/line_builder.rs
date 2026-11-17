//! Line placement, line-box calculation, alignment, and visual run assembly
//! (spec §5 Stages 2–4).

use super::super::{
    DecorationKind, DecorationLine, HangingWhitespaceMode, InlineSlotConstraint, LayoutInputError,
    LineGeometry, LineHeight, PositionedGlyph, PositionedInlineBox, PositionedLine, PositionedSlot,
    PositionedTab, StrutMetrics, VisualGlyphRun,
};
use super::hanging::{LineHangs, clip_to_content, mark_hanging};
use super::line_artifacts::{CaretContext, SpanContext};
use super::{Break, Layout, LineCursor, LineEnd, PieceKind, PlacedPiece, Token};
use crate::frontend::bidi::{LineClass, reset_line_levels, visual_order as bidi_order};
use crate::frontend::{BoxNode, SpanEdge, WordNode};
use crate::types::{
    AtomFlags, BaselineAlignment, BidiLevel, FontMetrics, RawShapedGlyph, TextRange, WritingMode,
};
use smallvec::SmallVec;

/// Horizontal extent of the line's body (excluding trailing whitespace) in the
/// current coordinate frame.
#[derive(Clone, Copy)]
pub(super) struct LineBounds {
    pub(super) body_lo: f32,
    pub(super) body_hi: f32,
}

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

struct LineBoxMetrics {
    ascent: f32,
    descent: f32,
    line_height: f32,
}

struct LineAlignment {
    visible_end: usize,
    content_start: f32,
    content_width: f32,
}

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
        let (last_piece, end_break, byte_end) = match end {
            LineEnd::At(t) => {
                let Token::Break(b) = &self.tokens[t] else {
                    unreachable!("a line ends at a break token")
                };
                (t, Some(b.clone()), b.bytes.end())
            }
            LineEnd::Before(t) => (t, None, self.piece(t).bytes.start()),
        };
        let soft_wrap = end_break.as_ref().is_none_or(|b| !b.mandatory);
        let pre = end_break.as_ref().map_or(0.0, |b| b.pre);

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
        let classes: Vec<LineClass> = placed.iter().map(class).collect();
        let mut levels: Vec<BidiLevel> = placed.iter().map(|p| p.level).collect();
        reset_line_levels(classes.into_iter(), &mut levels, self.base.base_level());
        for (p, level) in placed.iter_mut().zip(&levels) {
            if !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) {
                p.level = *level;
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
            placed[k].level = neighbour
                .filter(|n| span.byte_range.contains_range(n.piece.bytes))
                .map_or_else(|| self.base.base_level(), |n| n.level);
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
                        level: p.level,
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
        if rtl {
            let mut tx = far_edge;
            for p in trailing {
                let w = trailing_width(p);
                tx -= w;
                p.x = tx;
                p.width = w;
            }
        } else {
            let mut tx = far_edge;
            for p in trailing {
                let w = trailing_width(p);
                p.x = tx;
                p.width = w;
                tx += w;
            }
        }
    }

    fn build_line(
        &mut self,
        mut state: LineBuildState<'_>,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = state.geometry.slots[0];
        let line_box = self.compute_line_box(state.placed, state.order, state.byte_range, slot)?;
        let alignment = self.align_line_content(&mut state, slot);

        let mut visual_runs = self.build_visual_runs(state.placed, state.order);
        let content_edges = (
            alignment.content_start,
            alignment.content_start + alignment.content_width,
        );
        mark_hanging(&mut visual_runs, state.placed, state.hangs, content_edges);
        let (visual_boxes, tabs) = self.build_boxes_and_tabs(state.placed, slot);
        let span_ctx = SpanContext {
            placed: state.placed,
            order: state.order,
            bounds: state.bounds,
            end_break: state.end_break,
            post: state.post,
            line_bytes: state.byte_range,
        };
        let mut span_fragments = self.span_fragments(&span_ctx);

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
            visible_end: alignment.visible_end,
            content_start: alignment.content_start,
            end_break: state.end_break,
            runs: &visual_runs,
            boxes: &visual_boxes,
        });

        let mut slots: SmallVec<[PositionedSlot; 1]> = state
            .geometry
            .slots
            .iter()
            .map(|&constraint| PositionedSlot {
                constraint,
                content_start: constraint.start(),
                visual_width: 0.0,
            })
            .collect();
        slots[0].content_start = alignment.content_start;
        slots[0].visual_width = alignment.content_width;
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
            visible_byte_end: alignment.visible_end,
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

    /// Computes line-box ascent/descent and assigns `y_shift` to every piece
    /// (spec §5 Stage 4, BAS-1).
    fn compute_line_box(
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
            let (align, item_above, item_below) = match piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    let align = self.ast.runs[run].baseline_align;
                    let (a, d) = if normal || LineRelativeEdge::of(align).is_some() {
                        half(self.cluster_box(run, cluster)?)
                    } else {
                        (0.0, 0.0)
                    };
                    (align, a, d)
                }
                PieceKind::Object { item } => {
                    let WordNode::Box(BoxNode::InlineObject { baseline_align, .. }) =
                        &self.ast.items[item]
                    else {
                        unreachable!("an object piece names an object item")
                    };
                    let (_, a, d) = self.fitted_object(item, slot);
                    (*baseline_align, a, d)
                }
                _ => continue,
            };
            let is_object = matches!(piece.kind, PieceKind::Object { .. });
            match LineRelativeEdge::of(align) {
                Some(edge) => line_relative.push(LineRelativeItem {
                    piece_index: k,
                    edge,
                    above: item_above,
                    below: item_below,
                }),
                None => {
                    let shift = match piece.kind {
                        PieceKind::Cluster { run, cluster } => {
                            let own = self.cluster_metrics(run, cluster)?;
                            run_shift(align, &self.ast.strut_metrics, &own)
                        }
                        _ => object_shift(align, &self.ast.strut_metrics),
                    };
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

    /// Computes content bounds (SLT-3) and shifts all placed pieces to align
    /// within `slot`.
    fn align_line_content(
        &self,
        state: &mut LineBuildState<'_>,
        slot: InlineSlotConstraint,
    ) -> LineAlignment {
        let mut visible_end = state.byte_range.start();
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut extend = |a: f32, b: f32| {
            lo = lo.min(a.min(b));
            hi = hi.max(a.max(b));
        };
        for &k in state.order {
            let p = state.placed[k];
            match p.piece.kind {
                PieceKind::Cluster { cluster, .. } => {
                    let (glyphs, _) = self.cluster_glyphs(cluster);
                    if glyphs.is_empty() {
                        continue;
                    }
                    let rtl_cluster = p.level.direction().is_rtl();
                    let mut pen = p.x + if rtl_cluster { p.expansion } else { 0.0 };
                    let mut track_pen = |g: &RawShapedGlyph| {
                        if g.x_advance != 0.0 {
                            extend(pen, pen + g.x_advance);
                        }
                        pen += g.x_advance;
                    };
                    if rtl_cluster {
                        glyphs.iter().rev().for_each(&mut track_pen);
                    } else {
                        glyphs.iter().for_each(&mut track_pen);
                    }
                    if !glyphs.iter().all(|g| g.is_synthesized) {
                        visible_end = visible_end.max(p.piece.bytes.end());
                    }
                }
                PieceKind::Object { .. } | PieceKind::Tab => {
                    extend(p.x, p.x + p.width);
                    visible_end = visible_end.max(p.piece.bytes.end());
                }
                PieceKind::Glue { .. } | PieceKind::SpanEdge { .. } => {}
            }
        }
        let span_ctx = SpanContext {
            placed: state.placed,
            order: state.order,
            bounds: state.bounds,
            end_break: state.end_break,
            post: state.post,
            line_bytes: state.byte_range,
        };
        self.span_fragment_extents(&span_ctx, &mut extend);

        let has_content = lo <= hi;
        let rtl = self.base.is_rtl();
        // A hanging mark's protruding half lies outside the content (ST1-3).
        let (hang_left, hang_right) = state.hangs.physical(rtl);
        let (content_lo_rel, content_width) = if !has_content {
            (if rtl { state.bounds.body_hi } else { 0.0 }, 0.0)
        } else if rtl {
            let lo = lo + hang_left;
            (lo, hi.max(state.bounds.body_hi) - hang_right - lo)
        } else {
            (0.0, hi - hang_right)
        };
        // Alignment: the line start of the base direction.
        let content_start = if rtl && slot.end().is_finite() && content_width <= slot.width() {
            slot.end() - content_width
        } else {
            slot.start()
        };
        let align_shift = content_start - content_lo_rel;
        for p in state.placed.iter_mut() {
            p.x += align_shift;
        }
        state.bounds.body_lo += align_shift;
        state.bounds.body_hi += align_shift;
        LineAlignment {
            visible_end,
            content_start,
            content_width,
        }
    }

    fn build_visual_runs(&self, placed: &[PlacedPiece], order: &[usize]) -> Vec<VisualGlyphRun> {
        let mut visual_runs: Vec<VisualGlyphRun> = Vec::new();
        for &k in order {
            let p = placed[k];
            let PieceKind::Cluster { run, cluster } = p.piece.kind else {
                continue;
            };
            let r = &self.ast.runs[run];
            let c = &self.ast.clusters[cluster];
            let (glyphs, _) = self.cluster_glyphs(cluster);
            if glyphs.is_empty() {
                continue;
            }
            // Justification after an RTL cluster lies before its first glyph
            // in visual order, after an LTR one on its last glyph (ST3-2).
            let rtl_cluster = p.level.direction().is_rtl();
            let mut pen = p.x + if rtl_cluster { p.expansion } else { 0.0 };
            let mut make_glyph = |g: &RawShapedGlyph| {
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
            if !rtl_cluster && let Some(last) = positioned.last_mut() {
                last.advance += p.expansion;
            }
            let level = p.level;
            let same_run = visual_runs.last_mut().filter(|v| {
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
                visual_runs.push(VisualGlyphRun {
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
        visual_runs
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

/// The shift of a run towards line-over (spec BAS-1): its font's named
/// baseline onto the strut's, or an explicit offset.
fn run_shift(align: BaselineAlignment, strut: &FontMetrics, own: &FontMetrics) -> f32 {
    match align {
        BaselineAlignment::Offset(offset) => offset,
        _ => baseline_offset(align, strut)
            .zip(baseline_offset(align, own))
            .map_or(0.0, |(s, o)| s - o),
    }
}

/// The shift of an object's baseline towards line-over (spec BAS-1): to
/// the strut's named baseline, or an explicit offset.
fn object_shift(align: BaselineAlignment, strut: &FontMetrics) -> f32 {
    match align {
        BaselineAlignment::Offset(offset) => offset,
        _ => baseline_offset(align, strut).unwrap_or(0.0),
    }
}

/// Visual indices of a line's pieces from their levels (UAX #9 L2, spec
/// ST2-2), through the frontend's shared rule.
pub(super) fn visual_order(placed: &[PlacedPiece]) -> Vec<usize> {
    let levels: Vec<BidiLevel> = placed.iter().map(|p| p.level).collect();
    bidi_order(&levels)
}
