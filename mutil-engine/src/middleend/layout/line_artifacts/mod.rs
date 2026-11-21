//! Line-level fragments (ST2-3, FRG-1, FRG-3), decorations (ST5-1) and, in
//! `carets`, caret stops (§6.1).

mod carets;

pub(super) use carets::CaretScratch;

use super::line_builder::{AlignedLine, PlacedLine};
use super::{Layout, Piece, PieceKind, PlacedPiece};
use crate::frontend::{DiscretionaryNode, InlineDecorationSpan, MAX_DECORATION_NESTING_DEPTH};
use crate::middleend::tolerance::tolerance;
use crate::middleend::validated::{BodyIdx, SpanId, VisualOrder};
use crate::middleend::{
    AtomicFragment, DecorationKind, DecorationLine, LayoutInputError, PositionedSpanFragment,
    ValidatedInputs,
};
use crate::types::{BidiLevel, FontMetrics, TextDecoration, TextRange};
use smallvec::SmallVec;
use std::ops::Range;

/// One group of a span's pieces on a line: contiguous in visual order and in
/// logical order (spec ST2-3, FRG-1, FRG-3).
pub(super) struct VisualGroup {
    pub(super) lo: f32,
    pub(super) hi: f32,
    bytes: TextRange,
    /// Visual-order bounds `(first_v, last_v)` in `line.order`; `None` for a
    /// synthetic trailing-only or cloned-padding group.
    visual_span: Option<(usize, usize)>,
    /// The level of its first piece in visual order.
    level: BidiLevel,
    /// Whether it holds only zero-width span boundaries.
    is_bare_boundary: bool,
}

/// A line's visual order, indexed for grouping (ST2-3): each piece's
/// visual position, how many pieces other than span boundaries come before
/// each visual position, and the outermost positive-width body pieces for
/// O(1) cloned-padding checks. Rebuilt once per line in the paragraph's
/// scratch.
#[derive(Default)]
pub(in crate::middleend::layout) struct VisualIndex {
    /// The visual position of each piece in logical order; `None` for the
    /// trailing whitespace, which has none.
    position: Vec<Option<usize>>,
    /// `solid_before[v]`: the pieces at visual positions below `v` that are
    /// not span boundaries.
    solid_before: Vec<usize>,
    /// First and last body pieces in visual order with `width > 0.0`, as
    /// `(visual_pos, body_idx)`.
    first_wide: Option<(usize, BodyIdx)>,
    last_wide: Option<(usize, BodyIdx)>,
    /// Whether every body piece has `width >= 0.0` and starts at or after the
    /// previous body piece's end, so `first_wide` and `last_wide` bound all
    /// positive-width non-members in O(1).
    ordered_x: bool,
}

impl VisualIndex {
    /// Indexes `order`, the visual order of the body of `placed`.
    pub(in crate::middleend::layout) fn rebuild(
        &mut self,
        placed: &[PlacedPiece<'_>],
        order: &VisualOrder,
    ) {
        self.position.clear();
        self.position.resize(placed.len(), None);
        self.solid_before.clear();
        self.first_wide = None;
        self.last_wide = None;
        self.ordered_x = true;
        let placed_t = order.table(placed);
        let mut pos_t = order.table_mut(&mut self.position);
        let mut solid = 0;
        let mut prev_end = f32::NEG_INFINITY;
        for (v, k) in order.indices().enumerate() {
            let p = placed_t.get(k);
            pos_t.set(k, Some(v));
            self.solid_before.push(solid);
            solid += usize::from(!matches!(p.piece.kind, PieceKind::SpanEdge { .. }));
            if p.width > 0.0 {
                self.first_wide.get_or_insert((v, k));
                self.last_wide = Some((v, k));
            }
            self.ordered_x &= p.width >= 0.0 && prev_end <= p.x;
            prev_end = p.x + p.width;
        }
        self.solid_before.push(solid);
    }

    /// Whether a piece other than a span boundary lies strictly between
    /// visual positions `a < b`.
    fn solid_between(&self, a: usize, b: usize) -> bool {
        self.solid_before.get(b) > self.solid_before.get(a + 1)
    }

    /// Whether any positive-width non-member body piece starts strictly left
    /// of `limit` (beyond tolerance). O(1) when body `x` positions are
    /// ordered; falls back to scanning `order` only on lines with negative
    /// glue or no-break gaps.
    fn has_other_left(
        &self,
        placed: &[PlacedPiece<'_>],
        order: &VisualOrder,
        first_v: Option<usize>,
        limit: f32,
        is_member: &impl Fn(&Piece<'_>) -> bool,
    ) -> bool {
        if self.ordered_x {
            let placed_t = order.table(placed);
            return self.first_wide.is_some_and(|(v, k)| {
                first_v.is_none_or(|fv| v < fv) && {
                    let x = placed_t.get(k).x;
                    x < limit - tolerance(x, limit)
                }
            });
        }
        order
            .pick(placed)
            .any(|p| p.width > 0.0 && !is_member(&p.piece) && p.x < limit - tolerance(p.x, limit))
    }

    /// Whether any positive-width non-member body piece ends strictly right
    /// of `limit` (beyond tolerance). O(1) when body `x` positions are
    /// ordered; falls back to scanning `order` only on lines with negative
    /// glue or no-break gaps.
    fn has_other_right(
        &self,
        placed: &[PlacedPiece<'_>],
        order: &VisualOrder,
        last_v: Option<usize>,
        limit: f32,
        is_member: &impl Fn(&Piece<'_>) -> bool,
    ) -> bool {
        if self.ordered_x {
            let placed_t = order.table(placed);
            return self.last_wide.is_some_and(|(v, k)| {
                last_v.is_none_or(|lv| v > lv) && {
                    let p = placed_t.get(k);
                    let end = p.x + p.width;
                    end > limit + tolerance(end, limit)
                }
            });
        }
        order.pick(placed).any(|p| {
            let end = p.x + p.width;
            p.width > 0.0 && !is_member(&p.piece) && end > limit + tolerance(end, limit)
        })
    }
}

/// The visual groups, left to right, of the pieces `is_member` selects,
/// all of which lie in the logical range `candidates`. A piece outside the
/// selection splits a group, except another span's boundary, which neither
/// joins nor splits it: a nested span's padding lies inside the enclosing
/// one. Visually adjacent members join only when they are also adjacent
/// among the members in logical order, `placed` being in logical order.
///
/// Only the candidates are read, so a line with many spans costs the sum of
/// their pieces, not pieces × spans.
fn visual_groups(
    placed: &[PlacedPiece<'_>],
    index: &VisualIndex,
    candidates: Range<usize>,
    is_member: impl Fn(&Piece<'_>) -> bool,
) -> SmallVec<[VisualGroup; 2]> {
    let slice = placed.get(candidates.clone()).unwrap_or_default();
    let positions = index.position.get(candidates).unwrap_or_default();
    // (visual position, rank among the members in logical order, piece).
    let mut members: SmallVec<[(usize, usize, &PlacedPiece<'_>); 16]> = slice
        .iter()
        .zip(positions)
        .filter(|(p, _)| is_member(&p.piece))
        .enumerate()
        .filter_map(|(rank, (p, &pos))| Some((pos?, rank, p)))
        .collect();
    members.sort_unstable_by_key(|&(v, ..)| v);
    let mut groups: SmallVec<[VisualGroup; 2]> = SmallVec::new();
    let mut previous: Option<(usize, usize)> = None;
    for (v, rank, p) in members {
        let is_edge = matches!(p.piece.kind, PieceKind::SpanEdge { .. });
        let end = p.x + p.width;
        let (lo, hi) = (p.x.min(end), p.x.max(end));
        let is_bare_boundary = is_edge && p.width == 0.0;
        let joins =
            previous.is_some_and(|(pv, pr)| pr.abs_diff(rank) == 1 && !index.solid_between(pv, v));
        match groups.last_mut().filter(|_| joins) {
            Some(g) => {
                g.lo = g.lo.min(lo);
                g.hi = g.hi.max(hi);
                g.bytes = g.bytes.cover(p.piece.bytes);
                if let Some((_, last_v)) = &mut g.visual_span {
                    *last_v = v;
                }
                g.is_bare_boundary &= is_bare_boundary;
            }
            None => groups.push(VisualGroup {
                lo,
                hi,
                bytes: p.piece.bytes,
                visual_span: Some((v, v)),
                level: p.piece.level,
                is_bare_boundary,
            }),
        }
        previous = Some((v, rank));
    }
    groups
}

/// The pieces whose bytes may lie inside `range`, as a logical range: G1
/// proves the items in byte order (AST-3, `ItemsOutOfOrder`), so the pieces'
/// starts and ends both ascend and bisect.
fn pieces_within(placed: &[PlacedPiece<'_>], range: TextRange) -> Range<usize> {
    let lo = placed.partition_point(|p| p.piece.bytes.start() < range.start());
    let hi = placed.partition_point(|p| p.piece.bytes.end() <= range.end());
    lo..hi.max(lo)
}

/// The pieces whose bytes may meet `range`, as a logical range (AST-3).
fn pieces_meeting(placed: &[PlacedPiece<'_>], range: TextRange) -> Range<usize> {
    let lo = placed.partition_point(|p| p.piece.bytes.end() <= range.start());
    let hi = placed.partition_point(|p| p.piece.bytes.start() < range.end());
    lo..hi.max(lo)
}

/// One line edge's break as decoration spans see it (ST2-3): the spans it
/// clones, by validated ID and outer first, each with the padding it lays out
/// there; the break's whole width there (padding and its own edit), which
/// the edge's padding starts from; and the line's empty edge bytes.
struct ClonedEdge {
    clones: SmallVec<[(SpanId, f32); MAX_DECORATION_NESTING_DEPTH]>,
    break_width: f32,
    bytes: TextRange,
}

impl ClonedEdge {
    /// The edge of `brk`, each cloned span laying out `padding` there.
    fn of(
        inputs: &ValidatedInputs<'_>,
        candidates: impl Iterator<Item = SpanId>,
        brk: Option<&DiscretionaryNode>,
        padding: impl Fn(&InlineDecorationSpan) -> f32,
        break_width: impl Fn(&DiscretionaryNode) -> f32,
        at: usize,
    ) -> Self {
        let cloned: &[u64] = brk.map_or(&[], |b| b.cloned_spans.as_slice());
        let clones = if cloned.is_empty() {
            SmallVec::new()
        } else {
            candidates
                .filter_map(|id| {
                    let span = inputs.decoration_span(id);
                    cloned.contains(&span.span_id).then(|| (id, padding(span)))
                })
                .collect()
        };
        Self {
            clones,
            break_width: brk.map_or(0.0, break_width),
            bytes: TextRange::empty_at(at),
        }
    }

    /// The edge as span `index` sees it. The cloned spans all contain the
    /// break, so they nest with the span: those before it in slice order
    /// that contain it lie outside it, and those from it on that it contains
    /// lie inside (equal ranges nest in slice order, AST-8).
    fn side(&self, inputs: &ValidatedInputs<'_>, index: SpanId) -> PaddingSide {
        let span = inputs.decoration_span(index).byte_range;
        let (cloned, outside, inside) = self.clones.iter().fold(
            (false, 0.0, 0.0),
            |(cloned, outside, inside), &(c, width)| {
                let clone = inputs.decoration_span(c).byte_range;
                if c < index && clone.contains_range(span) {
                    (cloned, outside + width, inside)
                } else if c >= index && span.contains_range(clone) {
                    (true, outside, inside + width)
                } else {
                    (cloned, outside, inside)
                }
            },
        );
        PaddingSide {
            cloned,
            outside,
            inside,
            break_width: self.break_width,
            bytes: self.bytes,
        }
    }
}

/// The cloned padding on one physical side of a line (ST2-3), as one span
/// sees it: whether the span or a span inside it is cloned there, the
/// padding of the cloned spans enclosing it (outermost, at the line edge)
/// and of those inside it, the break's whole width there, and the line's
/// empty edge bytes there.
struct PaddingSide {
    cloned: bool,
    outside: f32,
    inside: f32,
    break_width: f32,
    bytes: TextRange,
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Walks each fragment of each decoration span on the line, left to
    /// right (spec ST2-3, FRG-1), calling `emit(span, group)`. Groups of bare
    /// zero-width boundaries are dropped beside any other group; a fragment's
    /// bytes stay inside its span and the line.
    pub(super) fn for_each_span_group(
        &self,
        line: &PlacedLine<'_, '_>,
        mut emit: impl FnMut(&InlineDecorationSpan, &VisualGroup),
    ) {
        let bytes = line.byte_range;
        if self.ast.decoration_spans.is_empty() {
            return;
        }
        let at_start = ClonedEdge::of(
            self.inputs,
            self.spans_touching(TextRange::empty_at(bytes.start())),
            line.start_break,
            |s| s.start_padding,
            |b| b.post_break_width,
            bytes.start(),
        );
        let at_end = ClonedEdge::of(
            self.inputs,
            self.spans_touching(TextRange::empty_at(bytes.end())),
            line.end_break,
            |s| s.end_padding,
            |b| b.pre_break_width,
            bytes.end(),
        );
        let visual = line.visual;
        for index in self.spans_touching(bytes) {
            let span = self.inputs.decoration_span(index);
            let range = span.byte_range;
            if range.end() < bytes.start() {
                continue;
            }
            // The edges of the span and of the spans nested in it: their
            // padding lies inside it.
            let is_member = |p: &Piece<'_>| match p.kind {
                PieceKind::SpanEdge { span, .. } => {
                    span >= index
                        && range.contains_range(self.inputs.decoration_span(span).byte_range)
                }
                _ => !p.bytes.is_empty() && range.contains_range(p.bytes),
            };
            let candidates = pieces_within(line.placed, range);
            let trailing_start = candidates.start.max(line.body_len());
            let trailing = trailing_start..candidates.end.max(trailing_start);
            let mut groups = visual_groups(line.placed, visual, candidates, is_member);
            if groups.iter().any(|g| !g.is_bare_boundary) {
                groups.retain(|g| !g.is_bare_boundary);
            }
            let sides = [
                at_start.side(self.inputs, index),
                at_end.side(self.inputs, index),
            ];
            self.attach_padding(line, trailing, sides, &is_member, &mut groups);
            let (lo, hi) = (
                range.start().max(bytes.start()),
                range.end().min(bytes.end()),
            );
            for g in &mut groups {
                let start = g.bytes.start().max(lo);
                g.bytes = TextRange::new(start, g.bytes.end().min(hi).max(start));
                emit(span, g);
            }
        }
    }

    /// Gives a span's bytes in the line's collapsed trailing white space,
    /// which has no visual order, to its last group by bytes, or to an empty
    /// group at the line end when it has none on the line. Then extends its
    /// outermost groups with the padding cloned at the line's start or end,
    /// its own and that of the spans inside it, which the cloned spans
    /// enclosing it surround (ST2-3): that padding joins the outermost group
    /// only if no other content lies between them, else it is a fragment of
    /// its own at the line's edge offset on that side (AX-5).
    fn attach_padding(
        &self,
        line: &PlacedLine<'_, '_>,
        trailing: Range<usize>,
        [at_start, at_end]: [PaddingSide; 2],
        is_member: &impl Fn(&Piece<'_>) -> bool,
        groups: &mut SmallVec<[VisualGroup; 2]>,
    ) {
        let rtl = self.base.is_rtl();
        let (left, right) = if rtl {
            (at_end, at_start)
        } else {
            (at_start, at_end)
        };
        // The line's outer edges (the body bounds hold each break's whole
        // width), and the span's own edges inside the padding of the cloned
        // spans enclosing it.
        let (line_lo, line_hi) = (line.bounds.body_lo - left.break_width, line.bounds.body_hi);
        let (span_lo, span_hi) = (line_lo + left.outside, line_hi - right.outside);
        let group = |lo: f32, hi: f32, bytes: TextRange| VisualGroup {
            lo,
            hi,
            bytes,
            visual_span: None,
            level: self.base.base_level(),
            is_bare_boundary: false,
        };
        if let Some(bytes) = line
            .placed
            .get(trailing)
            .unwrap_or_default()
            .iter()
            .filter(|p| is_member(&p.piece) && !p.piece.bytes.is_empty())
            .map(|p| p.piece.bytes)
            .reduce(|a, b| a.cover(b))
        {
            if let Some(last) = groups.iter_mut().max_by_key(|g| g.bytes.end()) {
                last.bytes = last.bytes.cover(bytes);
                last.is_bare_boundary = false;
            } else {
                let edge = if rtl { line_lo } else { line_hi };
                groups.push(group(edge, edge, bytes));
            }
        }
        if left.cloned
            && let Some(first) = groups.first_mut()
        {
            let first_v = first.visual_span.map(|(v, _)| v);
            if line
                .visual
                .has_other_left(line.placed, line.order, first_v, first.lo, is_member)
            {
                groups.insert(0, group(span_lo, span_lo + left.inside, left.bytes));
            } else {
                first.lo = first.lo.min(span_lo);
            }
        }
        if right.cloned
            && let Some(last) = groups.last_mut()
        {
            let last_v = last.visual_span.map(|(_, v)| v);
            if line
                .visual
                .has_other_right(line.placed, line.order, last_v, last.hi, is_member)
            {
                groups.push(group(span_hi - right.inside, span_hi, right.bytes));
            } else {
                last.hi = last.hi.max(span_hi);
            }
        }
    }

    /// One fragment per visual group of each decoration span on the line
    /// (spec ST2-3, FRG-1), padding included; a span cloned across this
    /// line's breaks reaches the line's edge.
    pub(super) fn span_fragments(&self, line: &AlignedLine<'_, '_>) -> Vec<PositionedSpanFragment> {
        let mut fragments = Vec::new();
        let (ascent, descent) = (self.strut.ascent.max(0.0), self.strut.descent.max(0.0));
        self.for_each_span_group(line, |span, g| {
            fragments.push(PositionedSpanFragment {
                span_id: span.span_id,
                span_range: span.byte_range,
                byte_range: g.bytes,
                x: g.lo,
                width: g.hi - g.lo,
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
    pub(super) fn atomic_fragments(&self, line: &AlignedLine<'_, '_>) -> Vec<AtomicFragment> {
        let mut out = Vec::new();
        let visual = line.visual;
        for span in self.inputs.atomic_spans_meeting(line.byte_range) {
            let full = span.byte_range;
            let part_end = full.end().min(line.byte_range.end());
            let mut covered = full.start().max(line.byte_range.start());
            if covered >= part_end {
                continue;
            }
            let candidates = pieces_meeting(line.placed, full);
            let mut groups = visual_groups(line.placed, visual, candidates, |p| {
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

    /// Underline or strike-through segments merged across adjacent decorated
    /// clusters and the glue between them (spec ST5-1); an underline takes
    /// the lowest of the fonts it crosses.
    pub(super) fn decorations(
        &mut self,
        kind: DecorationKind,
        line: &AlignedLine<'_, '_>,
        out: &mut Vec<DecorationLine>,
    ) -> Result<(), LayoutInputError> {
        let drawn: fn(TextDecoration) -> bool = match kind {
            DecorationKind::Underline => |d| d.underline,
            DecorationKind::Strikethrough => |d| d.strikethrough,
        };
        let position: fn(FontMetrics) -> f32 = match kind {
            DecorationKind::Underline => |m| m.underline_position,
            DecorationKind::Strikethrough => |m| m.strikeout_position,
        };
        let mut open: Option<DecorationLine> = None;
        // Glue after the open segment joins only if a decorated cluster follows.
        let mut pending_glue: Option<TextRange> = None;
        for &p in line.order.pick(line.placed) {
            let bytes = p.piece.bytes;
            match p.piece.kind {
                PieceKind::Cluster { run, cluster } if drawn(self.inputs.run(run).decoration) => {
                    let m = self.cluster_metrics(run, cluster)?;
                    let (y_offset, thickness) = (position(m) + p.y_shift, m.underline_thickness);
                    let right = p.x + p.width;
                    let Some(segment) = open.as_mut() else {
                        open = Some(DecorationLine {
                            kind,
                            byte_range: bytes,
                            x: p.x,
                            width: right - p.x,
                            y_offset,
                            thickness,
                        });
                        continue;
                    };
                    if let Some(glue) = pending_glue.take() {
                        segment.byte_range = segment.byte_range.cover(glue);
                    }
                    segment.width = right.max(segment.x + segment.width) - segment.x;
                    segment.byte_range = segment.byte_range.cover(bytes);
                    if kind == DecorationKind::Underline && y_offset < segment.y_offset {
                        segment.y_offset = y_offset;
                        segment.thickness = thickness;
                    }
                }
                PieceKind::Glue(_) if open.is_some() => {
                    pending_glue = Some(pending_glue.map_or(bytes, |glue| glue.cover(bytes)));
                }
                PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => {}
                _ => {
                    out.extend(open.take());
                    pending_glue = None;
                }
            }
        }
        out.extend(open);
        Ok(())
    }
}
