//! Line-level fragments (ST2-3, FRG-1, FRG-3), decorations (ST5-1) and, in
//! `carets`, caret stops (§6.1).

mod carets;

use super::line_builder::{AlignedLine, PlacedLine};
use super::{Layout, Piece, PieceKind, PlacedPiece};
use crate::frontend::InlineDecorationSpan;
use crate::middleend::tolerance::tolerance;
use crate::middleend::{
    AtomicFragment, DecorationKind, DecorationLine, LayoutInputError, PositionedSpanFragment,
};
use crate::types::{BidiLevel, FontMetrics, TextDecoration, TextRange};

/// One group of a span's pieces on a line: contiguous in visual order and in
/// logical order (spec ST2-3, FRG-1, FRG-3).
pub(super) struct VisualGroup {
    pub(super) lo: f32,
    pub(super) hi: f32,
    bytes: TextRange,
    /// The level of its first piece in visual order.
    level: BidiLevel,
    /// Whether it holds only zero-width span boundaries.
    is_bare_boundary: bool,
}

/// The visual groups, left to right, of the pieces `is_member` selects. A
/// piece outside the selection splits a group, except another span's
/// boundary, which neither joins nor splits it: a nested span's padding lies
/// inside the enclosing one. Visually adjacent members join only when they
/// are also adjacent among the members in logical order, `placed` being in
/// logical order.
fn visual_groups(
    placed: &[PlacedPiece<'_>],
    order: &[usize],
    is_member: impl Fn(&Piece<'_>) -> bool,
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

/// The cloned padding on one physical side of a line (ST2-3): whether the
/// span is cloned there, its width, and the line's empty edge bytes there.
struct PaddingSide {
    cloned: bool,
    width: f32,
    bytes: TextRange,
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Walks each fragment of each decoration span on the line, left to
    /// right (spec ST2-3, FRG-1), calling `emit(span, group)`. Groups of bare
    /// zero-width boundaries are dropped beside any other group; a fragment's
    /// bytes stay inside its span and the line.
    pub(super) fn for_each_span_group(
        &self,
        line: &PlacedLine<'_>,
        mut emit: impl FnMut(&InlineDecorationSpan, &VisualGroup),
    ) {
        let bytes = line.byte_range;
        for (index, span) in self.ast.decoration_spans.iter().enumerate() {
            let range = span.byte_range;
            if range.end() < bytes.start() || range.start() > bytes.end() {
                continue;
            }
            let is_member = |p: &Piece<'_>| match p.kind {
                PieceKind::SpanEdge { span, .. } => span == index,
                _ => !p.bytes.is_empty() && range.contains_range(p.bytes),
            };
            let mut groups = visual_groups(&line.placed, &line.order, is_member);
            if groups.iter().any(|g| !g.is_bare_boundary) {
                groups.retain(|g| !g.is_bare_boundary);
            }
            self.attach_padding(line, span, &is_member, &mut groups);
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
    /// outermost groups with the padding cloned at the line's start or end:
    /// padding joins the outermost group only if no other content lies
    /// between them, else it is a fragment of its own at the line's edge
    /// offset on that side (ST2-3, AX-5).
    fn attach_padding(
        &self,
        line: &PlacedLine<'_>,
        span: &InlineDecorationSpan,
        is_member: &impl Fn(&Piece<'_>) -> bool,
        groups: &mut Vec<VisualGroup>,
    ) {
        let rtl = self.base.is_rtl();
        let at_start = PaddingSide {
            cloned: line.post > 0.0
                && span.byte_range.start() < line.byte_range.start()
                && line.byte_range.start() < span.byte_range.end(),
            width: line.post,
            bytes: TextRange::empty_at(line.byte_range.start()),
        };
        let at_end = PaddingSide {
            cloned: line
                .end_break
                .is_some_and(|b| b.cloned_spans.contains(&span.span_id)),
            width: line.end_break.map_or(0.0, |b| b.pre_break_width),
            bytes: TextRange::empty_at(line.byte_range.end()),
        };
        let (left, right) = if rtl {
            (at_end, at_start)
        } else {
            (at_start, at_end)
        };
        let (line_lo, line_hi) = (line.bounds.body_lo - left.width, line.bounds.body_hi);
        let group = |lo: f32, hi: f32, bytes: TextRange| VisualGroup {
            lo,
            hi,
            bytes,
            level: self.base.base_level(),
            is_bare_boundary: false,
        };
        if let Some(bytes) = line.placed[line.body_len()..]
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
        let mut other_content = line
            .order
            .iter()
            .map(|&k| &line.placed[k])
            .filter(|p| p.width > 0.0 && !is_member(&p.piece));
        if left.cloned
            && let Some(first) = groups.first_mut()
        {
            if other_content
                .clone()
                .any(|p| p.x < first.lo - tolerance(p.x, first.lo))
            {
                groups.insert(0, group(line_lo, line_lo + left.width, left.bytes));
            } else {
                first.lo = first.lo.min(line_lo);
            }
        }
        if right.cloned
            && let Some(last) = groups.last_mut()
        {
            if other_content.any(|p| {
                let end = p.x + p.width;
                end > last.hi + tolerance(end, last.hi)
            }) {
                groups.push(group(line_hi - right.width, line_hi, right.bytes));
            } else {
                last.hi = last.hi.max(line_hi);
            }
        }
    }

    /// One fragment per visual group of each decoration span on the line
    /// (spec ST2-3, FRG-1), padding included; a span cloned across this
    /// line's breaks reaches the line's edge.
    pub(super) fn span_fragments(&self, line: &AlignedLine<'_>) -> Vec<PositionedSpanFragment> {
        let mut fragments = Vec::new();
        let (ascent, descent) = (self.strut.ascent.max(0.0), self.strut.descent.max(0.0));
        self.for_each_span_group(line, |span, g| {
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
    pub(super) fn atomic_fragments(&self, line: &AlignedLine<'_>) -> Vec<AtomicFragment> {
        let mut out = Vec::new();
        for span in &self.ast.atomic_spans {
            let full = span.byte_range;
            let part_end = full.end().min(line.byte_range.end());
            let mut covered = full.start().max(line.byte_range.start());
            if covered >= part_end {
                continue;
            }
            let mut groups = visual_groups(&line.placed, &line.order, |p| {
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
        line: &AlignedLine<'_>,
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
        for &k in &line.order {
            let p = line.placed[k];
            let bytes = p.piece.bytes;
            match p.piece.kind {
                PieceKind::Cluster { run, cluster } if drawn(self.ast.runs[run].decoration) => {
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
