//! Line placement, line-box calculation, alignment, and visual run assembly
//! (spec §5 Stages 2–4).

use super::breaking::within;
use super::hanging::{EndHang, LineHangs, clip_to_content};
use super::visual_runs::VisualRuns;
use super::{Layout, LineCursor, LineEnd, PieceKind, PlacedPiece, Reshapes, Token};
use crate::frontend::bidi::{LineClass, reset_line_levels, visual_order as bidi_order};
use crate::frontend::{DiscretionaryNode, SpanEdge};
use crate::middleend::{
    DecorationKind, DecorationLine, HangingWhitespaceMode, InlineSlotConstraint, LayoutInputError,
    LineGeometry, PositionedLine, PositionedSlot, VisualGlyphRun,
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

/// A line placed in logical order, before alignment: the pieces and their
/// visual order, and what the line's end and hanging marks leave. Its `x`
/// coordinates are relative to the body's start.
pub(super) struct PlacedLine<'a> {
    pub(super) placed: Vec<PlacedPiece<'a>>,
    pub(super) order: Vec<usize>,
    pub(super) byte_range: TextRange,
    pub(super) bounds: LineBounds,
    pub(super) end_break: Option<&'a DiscretionaryNode>,
    pub(super) post: f32,
    hangs: LineHangs,
    is_justified: bool,
    block_top: f32,
}

impl PlacedLine<'_> {
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
pub(super) struct AlignedLine<'a> {
    line: PlacedLine<'a>,
    pub(super) content: ContentBlock,
}

impl<'a> Deref for AlignedLine<'a> {
    type Target = PlacedLine<'a>;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

/// What a line's [`LineEnd`] names.
struct LineEndFacts<'a> {
    last_piece: usize,
    end_break: Option<&'a DiscretionaryNode>,
    byte_end: usize,
    pre: f32,
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
    /// Places the line `(cursor.token_start, end]` in the first slot of `geometry`.
    pub(super) fn place(
        &mut self,
        cursor: &LineCursor,
        end: LineEnd<'a>,
        geometry: &LineGeometry,
        pending: &Reshapes,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.slots[0];
        let LineEndFacts {
            last_piece,
            end_break,
            byte_end,
            pre,
        } = self.end_facts(end);
        let soft_wrap = soft_wraps(end_break);

        let mut placed = self.collect_placed_pieces(cursor.token_start, last_piece);
        let body_len = placed
            .iter()
            .rposition(|p| p.piece.is_content())
            .map_or(0, |k| k + 1);
        self.apply_l1(&mut placed);
        let order = visual_order(&placed[..body_len]);

        let rtl = self.base.is_rtl();
        let post = cursor.post_break_width;
        let pads = Pads::new(rtl, pre, post);
        let first_line = cursor.first_line();
        let start = self.start_hang(&placed[..body_len], first_line, post, pending);
        // Widths in logical order: a tab's depends on its distance from the
        // line-start edge (spec ST3-1), the start mark's protrusion outside.
        let distance = self.measure_widths(
            &mut placed[..body_len],
            post - LineHangs::protrusion(start),
            slot,
            pending,
        );
        // A soft-wrapped line is justified; one that ends the paragraph or at
        // a hard break is only ever compressed, and only on request (ST3-3).
        let compressible = self.options.justify && (soft_wrap || self.options.compress_last_lines);
        let shrink = if compressible {
            Self::shrink_capacity(&placed[..body_len])
        } else {
            0.0
        };
        let fits_without = within(distance + pre - shrink, slot.width());
        let end = self.end_hang(
            &placed[..body_len],
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
        let is_justified = compressible
            && self.justify_slack(&mut placed[..body_len], slot, slack, soft_wrap, pending);
        let body_end = self.place_visual(&mut placed, &order, body_len, pads, hangs);

        self.build_line(
            PlacedLine {
                placed,
                order,
                byte_range: TextRange::new(cursor.byte_start, byte_end),
                bounds: LineBounds {
                    body_lo: pads.left,
                    body_hi: body_end,
                },
                end_break,
                post,
                hangs,
                is_justified,
                block_top: cursor.block_top,
            },
            geometry,
            pending,
        )
    }

    /// Justifies a compressible line whose slot is finite when it is
    /// soft-wrapped or overfull; whether it did (ST3-3).
    fn justify_slack(
        &self,
        body: &mut [PlacedPiece<'a>],
        slot: InlineSlotConstraint,
        slack: f32,
        soft_wrap: bool,
        pending: &Reshapes,
    ) -> bool {
        slot.width().is_finite() && (soft_wrap || slack < 0.0) && self.justify(body, slack, pending)
    }

    /// Sets each piece's width in logical order from the pen position
    /// `distance` and returns the position after the last one.
    fn measure_widths(
        &self,
        body: &mut [PlacedPiece<'a>],
        mut distance: f32,
        slot: InlineSlotConstraint,
        pending: &Reshapes,
    ) -> f32 {
        for p in body {
            distance += p.gap;
            p.width = self.measure(&p.piece, distance, slot, Some(pending)).0;
            distance += p.width;
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
                byte_end: self.tokens[token].bytes().start(),
                pre: 0.0,
            },
        }
    }

    /// Assigns each body piece its `x` in visual `order` and places the
    /// trailing whitespace; returns the body's far end including the right
    /// pad.
    fn place_visual(
        &self,
        placed: &mut [PlacedPiece<'a>],
        order: &[usize],
        body_len: usize,
        pads: Pads,
        hangs: LineHangs,
    ) -> f32 {
        let rtl = self.base.is_rtl();
        let mut x = pads.left - hangs.physical(rtl).0;
        for &k in order {
            x += placed[k].gap;
            placed[k].x = x;
            x += placed[k].width;
        }
        let body_end = x + pads.right;
        // In RTL the trailing whitespace lies left of everything on the line,
        // a hanging mark's protrusion included (HNG-3).
        let far_edge = if rtl {
            -hangs.physical(rtl).0
        } else {
            body_end
        };
        self.place_trailing_whitespace(&mut placed[body_len..], far_edge, rtl);
        body_end
    }

    /// UAX #9 L1 on the line (spec ST2-1): tabs, the whitespace before them
    /// and the whitespace at the line end take the paragraph level, read
    /// from glue classes, cluster flags and tabs, never from the text. A span
    /// boundary is transparent and then takes the level of its span's next
    /// (leading) or previous (trailing) item on the line.
    fn apply_l1(&self, placed: &mut [PlacedPiece<'_>]) {
        let class = |p: &PlacedPiece<'_>| match p.piece.kind {
            PieceKind::Tab => LineClass::Separator,
            PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => LineClass::Whitespace,
            PieceKind::Cluster { cluster, .. }
                if self.ast.clusters[cluster]
                    .flags
                    .contains(AtomFlags::L1_RESETTABLE) =>
            {
                LineClass::Whitespace
            }
            PieceKind::Cluster { .. } | PieceKind::Object(_) => LineClass::Other,
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
            let is_item = |p: &&PlacedPiece<'_>| {
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

    fn collect_placed_pieces(&self, start: usize, last_piece: usize) -> Vec<PlacedPiece<'a>> {
        let mut placed = Vec::new();
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
                Token::Break(b) => gap += b.no_break_width,
            }
        }
        placed
    }

    /// Trailing whitespace beyond the far edge `far_edge` (UAX #9 L1, HNG-3); synthetic
    /// glue collapses there under both modes (WN-10, ST1-12).
    fn place_trailing_whitespace(
        &self,
        trailing: &mut [PlacedPiece<'_>],
        far_edge: f32,
        rtl: bool,
    ) {
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        let trailing_width = |p: &PlacedPiece<'_>| match p.piece.kind {
            PieceKind::Glue(glue) => {
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
        mut line: PlacedLine<'a>,
        geometry: &LineGeometry,
        pending: &Reshapes,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.slots[0];
        let line_box =
            self.compute_line_box(&mut line.placed, &line.order, line.byte_range, slot)?;
        let VisualRuns {
            mut runs,
            extent,
            visible_end,
        } = self.build_visual_runs(&line, pending);
        let line = self.align_line_content(line, &mut runs, extent, slot);
        let content = &line.content;
        let content_edges = content.edges();
        let (visual_boxes, tabs) = self.build_boxes_and_tabs(&line.placed, slot);
        let mut span_fragments = self.span_fragments(&line);

        let mut decorations: Vec<DecorationLine> = Vec::new();
        for kind in [DecorationKind::Underline, DecorationKind::Strikethrough] {
            self.decorations(kind, &line, &mut decorations)?;
        }
        let atomic_fragments = self.atomic_fragments(&line);
        clip_to_content(
            line.hangs.physical(self.base.is_rtl()),
            content_edges,
            &mut span_fragments,
            &mut decorations,
        );
        let caret_stops = self.caret_stops(&line, visible_end, &runs, &visual_boxes, pending);

        // The line fills its first slot; the others stay empty (ME-2).
        let slots: SmallVec<[PositionedSlot; 1]> = geometry
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
    fn align_line_content(
        &self,
        mut line: PlacedLine<'a>,
        runs: &mut [VisualGlyphRun],
        extent: Option<(f32, f32)>,
        slot: InlineSlotConstraint,
    ) -> AlignedLine<'a> {
        let (mut lo, mut hi) = extent.unwrap_or((f32::INFINITY, f32::NEG_INFINITY));
        self.for_each_span_group(&line, |_, g| {
            lo = lo.min(g.lo.min(g.hi));
            hi = hi.max(g.lo.max(g.hi));
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
        for p in &mut line.placed {
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
/// ST2-2), through the frontend's shared rule.
pub(super) fn visual_order(placed: &[PlacedPiece<'_>]) -> Vec<usize> {
    let levels: SmallVec<[BidiLevel; 64]> = placed.iter().map(|p| p.piece.level).collect();
    bidi_order(&levels)
}
