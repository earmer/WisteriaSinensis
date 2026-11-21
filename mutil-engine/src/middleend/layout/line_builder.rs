//! Line placement, line-box calculation, alignment, and visual run assembly
//! (spec §5 Stages 2–4).

use super::breaking::within;
use super::hanging::{EndHang, LineHangs, clip_to_content};
use super::line_artifacts::{CaretScratch, VisualIndex};
use super::line_box::LineBoxMetrics;
use super::scratch::{LayoutScratch, LogicalPiece, build_placed};
use super::visual_runs::VisualRuns;
use super::{Layout, LineCursor, LineEnd, PieceKind, PlacedPiece, Reshapes, Token};
use crate::frontend::bidi::{LineClass, reset_line_levels};
use crate::frontend::{DiscretionaryNode, SpanEdge};
use crate::middleend::validated::{TokenIdx, VisualOrder};
use crate::middleend::{
    DecorationKind, DecorationLine, HangingWhitespaceMode, InlineSlotConstraint, LayoutInputError,
    OpenGeometry, PositionedLine, PositionedSlot, VisualGlyphRun,
};
use crate::types::{AtomFlags, BidiLevel, TextRange, WritingMode};
use smallvec::SmallVec;
use std::ops::Deref;

/// Horizontal extent of the line's body (excluding trailing whitespace) in the
/// current coordinate frame.
#[derive(Clone, Copy)]
pub(super) struct LineBounds {
    pub(super) body_lo: f32,
    pub(super) body_hi: f32,
}

/// Where a line's content block lies in its slot (SLT-3), after alignment.
pub(super) struct ContentBlock {
    pub(super) start: f32,
    pub(super) width: f32,
}

impl ContentBlock {
    fn edges(&self) -> (f32, f32) {
        (self.start, self.start + self.width)
    }
}

/// A line placed in logical order, before alignment: the pieces, borrowed
/// from the paragraph's [`LayoutScratch`], their visual order and its index,
/// and what the line's end and hanging marks leave. Its `x` coordinates are
/// relative to the body's start.
pub(super) struct PlacedLine<'l, 'a> {
    pub(super) placed: &'l mut [PlacedPiece<'a>],
    pub(super) order: &'l VisualOrder,
    pub(super) visual: &'l VisualIndex,
    pub(super) byte_range: TextRange,
    pub(super) bounds: LineBounds,
    pub(super) start_break: Option<&'a DiscretionaryNode>,
    pub(super) end_break: Option<&'a DiscretionaryNode>,
    hangs: LineHangs,
    is_justified: bool,
    block_top: f32,
}

impl PlacedLine<'_, '_> {
    /// The pieces before the trailing whitespace: `order` ranks exactly
    /// those.
    pub(super) fn body_len(&self) -> usize {
        self.order.len()
    }

    /// Whether the line wraps at a soft break or before an emergency cluster
    /// seam, as opposed to ending at a hard break or the paragraph.
    pub(super) fn soft_wrap(&self) -> bool {
        soft_wraps(self.end_break)
    }
}

/// Whether a line ending at `end_break` (none: an emergency seam) wraps.
fn soft_wraps(end_break: Option<&DiscretionaryNode>) -> bool {
    end_break.is_none_or(|b| !b.is_mandatory())
}

/// A [`PlacedLine`] after alignment: every coordinate is final, and the
/// content block it lies in is known.
pub(super) struct AlignedLine<'l, 'a> {
    line: PlacedLine<'l, 'a>,
    pub(super) content: ContentBlock,
}

impl<'l, 'a> Deref for AlignedLine<'l, 'a> {
    type Target = PlacedLine<'l, 'a>;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

/// What a line's [`LineEnd`] names.
struct LineEndFacts<'a> {
    last_piece: TokenIdx,
    end_break: Option<&'a DiscretionaryNode>,
    byte_end: usize,
    pre: f32,
}

/// How the body of a line is measured: where it starts, the line-end pad
/// `pre`, whether it soft-wraps, and the slot it fills.
struct BodyRequest<'c, 'a> {
    cursor: &'c LineCursor<'a>,
    pre: f32,
    soft_wrap: bool,
    slot: InlineSlotConstraint,
}

/// What measuring the body settled: the marks that hang and whether the
/// line was justified flush.
struct BodyFit {
    hangs: LineHangs,
    is_justified: bool,
}

/// The pads between a line's body and its left and right edges (physical).
#[derive(Clone, Copy)]
struct Pads {
    left: f32,
    right: f32,
}

impl Pads {
    /// The line-end pad `pre` and line-start pad `post` on their physical sides.
    fn new(rtl: bool, pre: f32, post: f32) -> Self {
        if rtl {
            Self {
                left: pre,
                right: post,
            }
        } else {
            Self {
                left: post,
                right: pre,
            }
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Places the line `(cursor.token_start, end]` in the first slot of
    /// `geometry`, working in `scratch`: the pieces are collected, their L1
    /// levels and visual order resolved, the body measured, justified and
    /// placed, the line box found, and only then are the placed pieces built.
    pub(super) fn place(
        &mut self,
        cursor: &LineCursor<'a>,
        end: LineEnd<'a>,
        geometry: &OpenGeometry,
        pending: &Reshapes,
        scratch: &mut LayoutScratch<'a>,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.first_slot();
        let LineEndFacts {
            last_piece,
            end_break,
            byte_end,
            pre,
        } = self.end_facts(end);
        let LayoutScratch {
            logical,
            levels,
            order,
            widths,
            expansions,
            xs,
            y_shifts,
            placed,
            visual,
            carets,
        } = scratch;
        self.collect_logical(logical, cursor.token_start, last_piece);
        let body_len = logical
            .iter()
            .rposition(|p| p.piece.is_content())
            .map_or(0, |k| k + 1);
        self.apply_l1(logical, levels);
        let (body_logical, _) = logical.split_at(body_len);
        visual_order_into(body_logical, levels, order);
        let request = BodyRequest {
            cursor,
            pre,
            soft_wrap: soft_wraps(end_break),
            slot,
        };
        let BodyFit {
            hangs,
            is_justified,
        } = self.measure_line(logical, body_len, widths, expansions, &request, pending);
        let pads = Pads::new(self.base.is_rtl(), pre, cursor.post_break_width());
        let body_end = self.place_visual(logical, order, widths, xs, pads, hangs);
        let byte_range = TextRange::new(cursor.byte_start, byte_end);
        let line_box = self.compute_line_box(logical, order, y_shifts, byte_range, slot)?;
        build_placed(placed, logical, [xs, widths, expansions, y_shifts]);
        visual.rebuild(placed, order);
        let line = PlacedLine {
            placed,
            order,
            visual,
            byte_range,
            bounds: LineBounds {
                body_lo: pads.left,
                body_hi: body_end,
            },
            start_break: cursor.start_break,
            end_break,
            hangs,
            is_justified,
            block_top: cursor.block_top,
        };
        self.build_line(line, &line_box, carets, geometry, pending)
    }

    /// Measures the line in logical order, finds the marks that hang,
    /// justifies a compressible body (ST1-3, ST3-1 to ST3-4), and appends the
    /// trailing whitespace's collapsed/preserved widths (WN-10, ST1-12):
    /// fills both `widths` and `expansions` for every piece of `logical`.
    fn measure_line(
        &self,
        logical: &[LogicalPiece<'a>],
        body_len: usize,
        widths: &mut Vec<f32>,
        expansions: &mut Vec<f32>,
        request: &BodyRequest<'_, 'a>,
        pending: &Reshapes,
    ) -> BodyFit {
        let (body, trailing) = logical.split_at(body_len);
        // Only the body is justified; the trailing whitespace has none.
        expansions.clear();
        expansions.resize(logical.len(), 0.0);
        let BodyRequest {
            cursor,
            pre,
            soft_wrap,
            slot,
        } = *request;
        let post = cursor.post_break_width();
        let first_line = cursor.first_line();
        let start = self.start_hang(body, first_line, post, pending);
        // Widths in logical order: a tab's depends on its distance from the
        // line-start edge (spec ST3-1), the start mark's protrusion outside.
        let distance = self.measure_widths(
            body,
            widths,
            post - LineHangs::protrusion(start),
            slot,
            pending,
        );
        // A soft-wrapped line is justified; one that ends the paragraph or at
        // a hard break is only ever compressed, and only on request (ST3-3).
        let compressible = self.options.justify && (soft_wrap || self.options.compress_last_lines);
        let shrink = if compressible {
            Self::shrink_capacity(body)
        } else {
            0.0
        };
        let fits_without = within(distance + pre - shrink, slot.width());
        let end = self.end_hang(
            body,
            first_line,
            EndHang {
                pre,
                start,
                fits_without,
            },
            pending,
        );
        let hangs = LineHangs { start, end };
        // The visible width `W`, hanging halves outside (ST3-2).
        let visible_width = distance + pre - LineHangs::protrusion(end);
        let slack = slot.width() - visible_width;
        let (body_expansions, _) = expansions.split_at_mut(body_len);
        let is_justified = compressible
            && slot.width().is_finite()
            && (soft_wrap || slack < 0.0)
            && self.justify(body, widths, body_expansions, slack, pending);
        self.append_trailing_widths(trailing, widths);
        BodyFit {
            hangs,
            is_justified,
        }
    }

    /// Appends each trailing whitespace piece's width to `widths`: synthetic
    /// glue collapses under both modes, collapsible glue under `Collapsible`
    /// (WN-10, ST1-12).
    fn append_trailing_widths(&self, trailing: &[LogicalPiece<'_>], widths: &mut Vec<f32>) {
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        widths.extend(trailing.iter().map(|p| match p.piece.kind {
            PieceKind::Glue(glue)
                if glue.kind.is_synthetic() || (collapsible && glue.is_collapsible) =>
            {
                0.0
            }
            _ => p.piece.width,
        }));
    }

    /// Fills `widths` with each body piece's width in logical order from
    /// the pen position `distance` and returns the position after the last
    /// one.
    fn measure_widths(
        &self,
        body: &[LogicalPiece<'a>],
        widths: &mut Vec<f32>,
        mut distance: f32,
        slot: InlineSlotConstraint,
        pending: &Reshapes,
    ) -> f32 {
        widths.clear();
        for p in body {
            distance += p.gap;
            let width = self.measure(&p.piece, distance, slot, Some(pending)).0;
            widths.push(width);
            distance += width;
        }
        distance
    }

    /// What the line's end names: its last piece, the break it ends at, the
    /// byte it ends before and the break's pre-break width.
    fn end_facts(&self, end: LineEnd<'a>) -> LineEndFacts<'a> {
        match end {
            LineEnd::At { token, brk } => LineEndFacts {
                last_piece: token,
                end_break: Some(brk),
                byte_end: brk.byte_range.end(),
                pre: brk.pre_break_width,
            },
            LineEnd::Before(token) => LineEndFacts {
                last_piece: token,
                end_break: None,
                byte_end: self.tokens.at_or_sentinel(token).bytes().start(),
                pre: 0.0,
            },
        }
    }

    /// Fills `xs` with each body piece's `x` in visual `order`, which ranks
    /// exactly the body, and places the trailing whitespace beyond the far
    /// edge; returns the body's far end including the right pad.
    fn place_visual(
        &self,
        logical: &[LogicalPiece<'a>],
        order: &VisualOrder,
        widths: &[f32],
        xs: &mut Vec<f32>,
        pads: Pads,
        hangs: LineHangs,
    ) -> f32 {
        let rtl = self.base.is_rtl();
        let body_len = order.len();
        xs.clear();
        xs.resize(logical.len(), 0.0);
        let logical_t = order.table(logical);
        let widths_t = order.table(widths);
        let mut xs_t = order.table_mut(xs);
        let mut x = pads.left - hangs.physical(rtl).0;
        for k in order.indices() {
            x += logical_t.get(k).gap;
            xs_t.set(k, x);
            x += *widths_t.get(k);
        }
        let body_end = x + pads.right;
        // In RTL the trailing whitespace lies left of everything on the line,
        // a hanging mark's protrusion included (HNG-3).
        let far_edge = if rtl {
            -hangs.physical(rtl).0
        } else {
            body_end
        };
        let (_, trailing_widths) = widths.split_at(body_len);
        let (_, trailing_xs) = xs.split_at_mut(body_len);
        self.place_trailing_whitespace(trailing_widths, trailing_xs, far_edge);
        body_end
    }

    /// UAX #9 L1 on the line (spec ST2-1): tabs, the whitespace before them
    /// and the whitespace at the line end take the paragraph level, read
    /// from glue classes, cluster flags and tabs, never from the text. A span
    /// boundary is transparent and then takes the level of its span's next
    /// (leading) or previous (trailing) item on the line.
    fn apply_l1(&self, placed: &mut [LogicalPiece<'_>], levels: &mut Vec<BidiLevel>) {
        let class = |p: &LogicalPiece<'_>| match p.piece.kind {
            PieceKind::Tab => LineClass::Separator,
            PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => LineClass::Whitespace,
            PieceKind::Cluster { cluster, .. }
                if self
                    .inputs
                    .cluster(cluster)
                    .flags
                    .contains(AtomFlags::L1_RESETTABLE) =>
            {
                LineClass::Whitespace
            }
            PieceKind::Cluster { .. } | PieceKind::Object(_) => LineClass::Other,
        };
        levels.clear();
        levels.extend(placed.iter().map(|p| p.piece.level));
        reset_line_levels(placed.iter().map(class), levels, self.base.base_level());
        for (p, &level) in placed.iter_mut().zip(levels.iter()) {
            if !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) {
                p.piece.level = level;
            }
        }
        for k in 0..placed.len() {
            let (before, rest) = placed.split_at_mut(k);
            let Some((current, after)) = rest.split_first_mut() else {
                continue;
            };
            let PieceKind::SpanEdge { span, edge } = current.piece.kind else {
                continue;
            };
            let span = self.inputs.decoration_span(span);
            let is_item = |p: &&LogicalPiece<'_>| {
                !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) && !p.piece.bytes.is_empty()
            };
            let neighbour = match edge {
                SpanEdge::Leading => after.iter().find(is_item),
                SpanEdge::Trailing => before.iter().rev().find(is_item),
            };
            current.piece.level = neighbour
                .filter(|n| span.byte_range.contains_range(n.piece.bytes))
                .map_or_else(|| self.base.base_level(), |n| n.piece.level);
        }
    }

    /// Refills `logical` with the line's pieces `[start, last_piece)`, each
    /// with the no-break width of the breaks before it.
    fn collect_logical(
        &self,
        logical: &mut Vec<LogicalPiece<'a>>,
        start: TokenIdx,
        last_piece: TokenIdx,
    ) {
        logical.clear();
        let mut gap = 0.0;
        for token in self.tokens.slice(start, last_piece) {
            match token {
                Token::Piece(p) => {
                    logical.push(LogicalPiece { piece: *p, gap });
                    gap = 0.0;
                }
                Token::Break(b) => gap += b.no_break_width,
            }
        }
    }

    /// Trailing whitespace beyond the far edge `far_edge` (UAX #9 L1, HNG-3),
    /// its positions written to `xs`.
    fn place_trailing_whitespace(&self, widths: &[f32], xs: &mut [f32], far_edge: f32) {
        let rtl = self.base.is_rtl();
        // Away from the body: leftwards in RTL, rightwards in LTR.
        let mut tx = far_edge;
        for (&width, x) in widths.iter().zip(xs) {
            if rtl {
                tx -= width;
                *x = tx;
            } else {
                *x = tx;
                tx += width;
            }
        }
    }

    /// The positioned line from a placed one and its line box: its glyph
    /// runs, alignment, boxes, fragments, decorations and caret stops.
    fn build_line(
        &mut self,
        line: PlacedLine<'_, 'a>,
        line_box: &LineBoxMetrics,
        carets: &mut CaretScratch,
        geometry: &OpenGeometry,
        pending: &Reshapes,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.first_slot();
        let VisualRuns {
            mut runs,
            extent,
            visible_end,
        } = self.build_visual_runs(&line, pending);
        let line = self.align_line_content(line, &mut runs, extent, slot);
        let content = &line.content;
        let content_edges = content.edges();
        let (visual_boxes, tabs) = self.build_boxes_and_tabs(line.placed, slot);
        let mut span_fragments = self.span_fragments(&line);

        let mut decorations: Vec<DecorationLine> = Vec::new();
        if self.has_underline {
            self.decorations(DecorationKind::Underline, &line, &mut decorations)?;
        }
        if self.has_strikethrough {
            self.decorations(DecorationKind::Strikethrough, &line, &mut decorations)?;
        }
        let atomic_fragments = self.atomic_fragments(&line);
        clip_to_content(
            line.hangs.physical(self.base.is_rtl()),
            content_edges,
            &mut span_fragments,
            &mut decorations,
        );
        let caret_stops =
            self.caret_stops(&line, visible_end, &runs, &visual_boxes, pending, carets);

        // The line fills its first slot; the others stay empty (ME-2).
        let (first_slot, rest_slots) = geometry.split_slots();
        let slots: SmallVec<[PositionedSlot; 1]> = core::iter::once(PositionedSlot {
            constraint: first_slot,
            content_start: content.start,
            visual_width: content.width,
        })
        .chain(rest_slots.iter().map(|&constraint| PositionedSlot {
            constraint,
            content_start: constraint.start(),
            visual_width: 0.0,
        }))
        .collect();
        let baseline_y = line.block_top
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
            byte_range: line.byte_range,
            visible_byte_end: visible_end,
            baseline_y,
            ascent: line_box.ascent,
            descent: line_box.descent,
            line_height: line_box.line_height,
            is_justified: line.is_justified,
            visual_runs: runs,
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
    fn align_line_content<'l>(
        &self,
        mut line: PlacedLine<'l, 'a>,
        runs: &mut [VisualGlyphRun],
        extent: Option<(f32, f32)>,
        slot: InlineSlotConstraint,
    ) -> AlignedLine<'l, 'a> {
        let (mut lo, mut hi) = extent.unwrap_or((f32::INFINITY, f32::NEG_INFINITY));
        self.for_each_span_group(&line, |_, g| {
            lo = lo.min(g.lo);
            hi = hi.max(g.hi);
        });
        let rtl = self.base.is_rtl();
        // A hanging mark's protrusion lies outside the content (ST1-3).
        let (hang_left, hang_right) = line.hangs.physical(rtl);
        let (content_lo, width) = if lo > hi {
            (if rtl { line.bounds.body_hi } else { 0.0 }, 0.0)
        } else if rtl {
            let lo = lo + hang_left;
            (lo, hi.max(line.bounds.body_hi) - hang_right - lo)
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
        for p in line.placed.iter_mut() {
            p.x += shift;
        }
        for g in runs.iter_mut().flat_map(|r| r.glyphs.iter_mut()) {
            g.x += shift;
        }
        line.bounds.body_lo += shift;
        line.bounds.body_hi += shift;
        AlignedLine {
            line,
            content: ContentBlock { start, width },
        }
    }
}

/// Visual indices of a line's pieces from their levels (UAX #9 L2, spec
/// ST2-2), through the frontend's shared rule; `levels` and `order` are working
/// buffers.
pub(super) fn visual_order_into(
    body: &[LogicalPiece<'_>],
    levels: &mut Vec<BidiLevel>,
    order: &mut VisualOrder,
) {
    levels.clear();
    levels.extend(body.iter().map(|p| p.piece.level));
    order.rebuild(levels);
}

#[cfg(test)]
pub(super) fn visual_order(body: &[LogicalPiece<'_>], levels: &mut Vec<BidiLevel>) -> Vec<usize> {
    let mut order = VisualOrder::default();
    visual_order_into(body, levels, &mut order);
    order.as_raw().to_vec()
}
