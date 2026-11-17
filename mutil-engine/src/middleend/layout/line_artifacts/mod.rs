//! Line-level fragments, decorations, and caret stops (spec §5 Stage 5, §6.1).
//!
//! This module groups each span's pieces into fragments (ST2-3, FRG-1,
//! FRG-3); `decorations` merges underline and strike-through segments
//! (ST5-1) and `carets` derives the caret stops (§6.1).

mod carets;
mod decorations;

pub(super) use carets::CaretContext;

use super::super::tolerance::tolerance;
use super::super::{AtomicFragment, PositionedSpanFragment};
use super::line_builder::LineBounds;
use super::{Break, Layout, Piece, PieceKind, PlacedPiece};
use crate::frontend::InlineDecorationSpan;
use crate::types::{BidiLevel, TextRange};

pub(super) struct SpanContext<'a> {
    pub(super) placed: &'a [PlacedPiece],
    pub(super) order: &'a [usize],
    pub(super) bounds: LineBounds,
    pub(super) end_break: Option<&'a Break>,
    pub(super) post: f32,
    pub(super) line_bytes: TextRange,
}

/// One group of a span's pieces on a line: contiguous in visual order and in
/// logical order (spec ST2-3, FRG-1, FRG-3).
struct VisualGroup {
    lo: f32,
    hi: f32,
    bytes: TextRange,
    /// The level of its first piece in visual order.
    level: BidiLevel,
    /// Whether it holds only zero-width span boundaries.
    is_bare_boundary: bool,
}

/// Where cloned span padding lies on the line (ST2-3): the padding on each
/// physical side and the outer edges it reaches. The line end is on the
/// right in LTR and on the left in RTL.
struct PaddingEdges {
    left_pad: f32,
    right_pad: f32,
    line_lo: f32,
    line_hi: f32,
}

/// The visual groups, left to right, of the pieces `is_member` selects. A
/// piece outside the selection splits a group, except another span's
/// boundary, which neither joins nor splits it: a nested span's padding lies
/// inside the enclosing one. Visually adjacent members join only when they
/// are also adjacent among the members in logical order, `placed` being in
/// logical order.
fn visual_groups(
    placed: &[PlacedPiece],
    order: &[usize],
    is_member: impl Fn(&Piece) -> bool,
) -> Vec<VisualGroup> {
    let mut rank = vec![None; placed.len()];
    for (r, k) in (0..placed.len())
        .filter(|&k| is_member(&placed[k].piece))
        .enumerate()
    {
        rank[k] = Some(r);
    }
    let mut groups: Vec<VisualGroup> = Vec::new();
    let mut previous: Option<usize> = None;
    for &k in order {
        let p = &placed[k];
        let is_edge = matches!(p.piece.kind, PieceKind::SpanEdge { .. });
        let Some(r) = rank[k] else {
            if !is_edge {
                previous = None;
            }
            continue;
        };
        let (lo, hi) = (p.x, p.x + p.width);
        let is_bare_boundary = is_edge && p.width == 0.0;
        match groups
            .last_mut()
            .filter(|_| previous.is_some_and(|q| q.abs_diff(r) == 1))
        {
            Some(g) => {
                g.lo = g.lo.min(lo);
                g.hi = g.hi.max(hi);
                g.bytes = g.bytes.cover(p.piece.bytes);
                g.is_bare_boundary &= is_bare_boundary;
            }
            None => groups.push(VisualGroup {
                lo,
                hi,
                bytes: p.piece.bytes,
                level: p.piece.level,
                is_bare_boundary,
            }),
        }
        previous = Some(r);
    }
    groups
}

/// `bytes` clamped inside both the span's `range` and the line's bytes: a
/// fragment never reaches past either.
fn clamp_fragment_bytes(bytes: TextRange, range: TextRange, line_bytes: TextRange) -> TextRange {
    let start = bytes.start().max(range.start()).max(line_bytes.start());
    let end = bytes
        .end()
        .min(range.end())
        .min(line_bytes.end())
        .max(start);
    TextRange::new(start, end)
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Walks each fragment of each decoration span on the line, left to
    /// right (spec ST2-3, FRG-1), calling `emit(span, group)`. Groups of bare
    /// zero-width boundaries are dropped beside any other group, and cloned
    /// padding extends the group at its physical line edge, or stands alone
    /// when other content lies between them.
    fn for_each_span_group(
        &self,
        ctx: &SpanContext<'_>,
        mut emit: impl FnMut(&InlineDecorationSpan, &VisualGroup),
    ) {
        let edges = self.padding_edges(ctx);
        let trailing_edge = if self.base.is_rtl() {
            edges.line_lo
        } else {
            edges.line_hi
        };
        for (index, span) in self.ast.decoration_spans.iter().enumerate() {
            let range = span.byte_range;
            if range.end() < ctx.line_bytes.start() || range.start() > ctx.line_bytes.end() {
                continue;
            }
            let is_member = |p: &Piece| match p.kind {
                PieceKind::SpanEdge { span, .. } => span == index,
                _ => !p.bytes.is_empty() && range.contains_range(p.bytes),
            };
            let mut groups = visual_groups(ctx.placed, ctx.order, is_member);
            if groups.iter().any(|g| !g.is_bare_boundary) {
                groups.retain(|g| !g.is_bare_boundary);
            }
            self.attach_trailing_bytes(ctx, &is_member, trailing_edge, &mut groups);
            self.attach_cloned_padding(ctx, &edges, span, &is_member, &mut groups);
            for g in &mut groups {
                g.bytes = clamp_fragment_bytes(g.bytes, range, ctx.line_bytes);
                emit(span, g);
            }
        }
    }

    /// The line's cloned padding and the edges it reaches (ST2-3).
    fn padding_edges(&self, ctx: &SpanContext<'_>) -> PaddingEdges {
        let pre = ctx.end_break.map_or(0.0, |b| b.pre);
        let (left_pad, right_pad) = if self.base.is_rtl() {
            (pre, ctx.post)
        } else {
            (ctx.post, pre)
        };
        PaddingEdges {
            left_pad,
            right_pad,
            line_lo: ctx.bounds.body_lo - left_pad,
            line_hi: ctx.bounds.body_hi,
        }
    }

    /// Gives a span's bytes in the line's collapsed trailing white space,
    /// which has no visual order, to its last group by bytes, or to an empty
    /// group at `edge`, the line end, when it has none on the line (ST2-3).
    fn attach_trailing_bytes(
        &self,
        ctx: &SpanContext<'_>,
        is_member: &impl Fn(&Piece) -> bool,
        edge: f32,
        groups: &mut Vec<VisualGroup>,
    ) {
        let Some(bytes) = ctx.placed[ctx.order.len()..]
            .iter()
            .filter(|p| is_member(&p.piece) && !p.piece.bytes.is_empty())
            .map(|p| p.piece.bytes)
            .reduce(|a, b| a.cover(b))
        else {
            return;
        };
        if let Some(last) = groups.iter_mut().max_by_key(|g| g.bytes.end()) {
            last.bytes = last.bytes.cover(bytes);
            last.is_bare_boundary = false;
        } else {
            groups.push(VisualGroup {
                lo: edge,
                hi: edge,
                bytes,
                level: self.base.base_level(),
                is_bare_boundary: false,
            });
        }
    }

    /// Extends a span's outermost groups with the padding cloned at the
    /// line's start or end (ST2-3). Padding joins the outermost group only
    /// if no other content lies between them; otherwise it is a fragment of
    /// its own, at the line's edge offset on that side (AX-5).
    fn attach_cloned_padding(
        &self,
        ctx: &SpanContext<'_>,
        edges: &PaddingEdges,
        span: &InlineDecorationSpan,
        is_member: &impl Fn(&Piece) -> bool,
        groups: &mut Vec<VisualGroup>,
    ) {
        let rtl = self.base.is_rtl();
        let range = span.byte_range;
        let cloned_at_end = ctx
            .end_break
            .is_some_and(|b| b.cloned.contains(&span.span_id));
        let cloned_at_start = ctx.post > 0.0
            && range.start() < ctx.line_bytes.start()
            && ctx.line_bytes.start() < range.end();
        let (line_start, line_end) = (ctx.line_bytes.start(), ctx.line_bytes.end());
        let ((at_left, left_offset), (at_right, right_offset)) = if rtl {
            ((cloned_at_end, line_end), (cloned_at_start, line_start))
        } else {
            ((cloned_at_start, line_start), (cloned_at_end, line_end))
        };
        let other_content = || {
            ctx.order
                .iter()
                .map(|&k| &ctx.placed[k])
                .filter(|p| p.width > 0.0 && !is_member(&p.piece))
        };
        let padding = |lo: f32, hi: f32, offset: usize| VisualGroup {
            lo,
            hi,
            bytes: TextRange::empty_at(offset),
            level: self.base.base_level(),
            is_bare_boundary: false,
        };
        if at_left && let Some(first) = groups.first_mut() {
            if other_content().any(|p| p.x < first.lo - tolerance(p.x, first.lo)) {
                let pad = padding(edges.line_lo, edges.line_lo + edges.left_pad, left_offset);
                groups.insert(0, pad);
            } else {
                first.lo = first.lo.min(edges.line_lo);
            }
        }
        if at_right && let Some(last) = groups.last_mut() {
            if other_content().any(|p| {
                let right = p.x + p.width;
                right > last.hi + tolerance(right, last.hi)
            }) {
                let pad = padding(edges.line_hi - edges.right_pad, edges.line_hi, right_offset);
                groups.push(pad);
            } else {
                last.hi = last.hi.max(edges.line_hi);
            }
        }
    }

    /// The left and right edges of the decoration span fragments on the line.
    pub(super) fn span_fragment_extents(
        &self,
        ctx: &SpanContext<'_>,
        mut extend: impl FnMut(f32, f32),
    ) {
        self.for_each_span_group(ctx, |_, g| extend(g.lo, g.hi));
    }

    /// One fragment per visual group of each decoration span on the line
    /// (spec ST2-3, FRG-1), padding included; a span cloned across this
    /// line's breaks reaches the line's edge.
    pub(super) fn span_fragments(&self, ctx: &SpanContext<'_>) -> Vec<PositionedSpanFragment> {
        let mut fragments = Vec::new();
        let (ascent, descent) = (self.strut.ascent.max(0.0), self.strut.descent.max(0.0));
        self.for_each_span_group(ctx, |span, g| {
            fragments.push(PositionedSpanFragment {
                span_id: span.span_id,
                span_range: span.byte_range,
                byte_range: g.bytes,
                x: g.lo,
                width: (g.hi - g.lo).max(0.0),
                ascent,
                descent,
                bidi_level: g.level,
            });
        });
        fragments
    }

    /// One fragment per visual group of each atomic span on the line (spec
    /// FRG-3), together covering exactly the part of the span on the line:
    /// each group takes the bytes from where the previous one stopped to its
    /// own end, at least one byte, and the last takes the rest.
    pub(super) fn atomic_fragments(
        &self,
        placed: &[PlacedPiece],
        order: &[usize],
        line_bytes: TextRange,
    ) -> Vec<AtomicFragment> {
        let mut out = Vec::new();
        for span in &self.ast.atomic_spans {
            let full = span.byte_range;
            let part_end = full.end().min(line_bytes.end());
            let mut covered = full.start().max(line_bytes.start());
            if covered >= part_end {
                continue;
            }
            let mut groups = visual_groups(placed, order, |p| {
                !p.bytes.is_empty() && p.bytes.start() < full.end() && p.bytes.end() > full.start()
            });
            groups.sort_by_key(|g| g.bytes.start());
            let last = groups.len().saturating_sub(1);
            for (i, g) in groups.into_iter().enumerate() {
                if covered >= part_end {
                    break;
                }
                let end = if i == last {
                    part_end
                } else {
                    g.bytes.end().clamp(covered + 1, part_end)
                };
                out.push(AtomicFragment {
                    atomic_id: span.span_id,
                    full_byte_range: full,
                    fragment_byte_range: TextRange::new(covered, end),
                    x: g.lo,
                    width: g.hi - g.lo,
                    bidi_level: g.level,
                });
                covered = end;
            }
        }
        out
    }
}
