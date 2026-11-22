//! Line-level fragments (ST2-3, FRG-1, FRG-3), decorations (ST5-1) and caret stops (§6.1).

mod carets;

pub(super) use carets::CaretScratch;

use super::line_box::LineBoxMetrics;
use super::line_builder::{AlignedLine, PlacedLine};
use super::{Layout, Piece, PieceKind, PlacedPiece};
use crate::frontend::{
    BoxDecorationBreak, DiscretionaryNode, InlineDecorationSpan, MAX_DECORATION_NESTING_DEPTH,
};
use crate::middleend::tolerance::tolerance_f64;
use crate::middleend::validated::{BodyIdx, SpanId, VisualOrder};
use crate::middleend::{
    AtomicFragment, DecorationKind, DecorationLine, InlineSlotConstraint, LayoutInputError,
    PositionedSpanFragment, SpanEdgeCut, SpanFragmentEdges, ValidatedInputs,
};
use crate::types::{BidiLevel, BoxHeightStyle, FontMetrics, TextDecoration, TextRange};
use smallvec::SmallVec;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GroupKind {
    Content,
    TrailingOnly,
    ClonedLeft,
    ClonedRight,
}

pub(super) struct VisualGroup {
    pub(super) lo: f64,
    pub(super) hi: f64,
    bytes: TextRange,
    visual_span: Option<(usize, usize)>,
    level: BidiLevel,
    is_bare_boundary: bool,
    kind: GroupKind,
}

#[derive(Default)]
pub(in crate::middleend::layout) struct VisualIndex {
    position: Vec<Option<usize>>,
    solid_before: Vec<usize>,
    first_wide: Option<(usize, BodyIdx)>,
    last_wide: Option<(usize, BodyIdx)>,
    ordered_x: bool,
}

impl VisualIndex {
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
        let mut prev_end = f64::NEG_INFINITY;
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

    fn solid_between(&self, a: usize, b: usize) -> bool {
        self.solid_before.get(b) > self.solid_before.get(a + 1)
    }

    fn has_other_left(
        &self,
        placed: &[PlacedPiece<'_>],
        order: &VisualOrder,
        first_v: Option<usize>,
        limit: f64,
        is_member: &impl Fn(&Piece<'_>) -> bool,
    ) -> bool {
        if self.ordered_x {
            let placed_t = order.table(placed);
            return self.first_wide.is_some_and(|(v, k)| {
                first_v.is_none_or(|fv| v < fv) && {
                    let x = placed_t.get(k).x;
                    x < limit - tolerance_f64(x, limit)
                }
            });
        }
        order.pick(placed).any(|p| {
            p.width > 0.0 && !is_member(&p.piece) && p.x < limit - tolerance_f64(p.x, limit)
        })
    }

    fn has_other_right(
        &self,
        placed: &[PlacedPiece<'_>],
        order: &VisualOrder,
        last_v: Option<usize>,
        limit: f64,
        is_member: &impl Fn(&Piece<'_>) -> bool,
    ) -> bool {
        if self.ordered_x {
            let placed_t = order.table(placed);
            return self.last_wide.is_some_and(|(v, k)| {
                last_v.is_none_or(|lv| v > lv) && {
                    let p = placed_t.get(k);
                    let end = p.x + p.width;
                    end > limit + tolerance_f64(end, limit)
                }
            });
        }
        order.pick(placed).any(|p| {
            let end = p.x + p.width;
            p.width > 0.0 && !is_member(&p.piece) && end > limit + tolerance_f64(end, limit)
        })
    }
}

fn visual_groups(
    placed: &[PlacedPiece<'_>],
    index: &VisualIndex,
    candidates: Range<usize>,
    is_member: impl Fn(&Piece<'_>) -> bool,
) -> SmallVec<[VisualGroup; 2]> {
    let slice = placed.get(candidates.clone()).unwrap_or_default();
    let positions = index.position.get(candidates).unwrap_or_default();
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
                kind: GroupKind::Content,
            }),
        }
        previous = Some((v, rank));
    }
    groups
}

fn pieces_within(placed: &[PlacedPiece<'_>], range: TextRange) -> Range<usize> {
    let lo = placed.partition_point(|p| p.piece.bytes.start() < range.start());
    let hi = placed.partition_point(|p| p.piece.bytes.end() <= range.end());
    lo..hi.max(lo)
}

fn pieces_meeting(placed: &[PlacedPiece<'_>], range: TextRange) -> Range<usize> {
    let lo = placed.partition_point(|p| p.piece.bytes.end() <= range.start());
    let hi = placed.partition_point(|p| p.piece.bytes.start() < range.end());
    lo..hi.max(lo)
}

struct ClonedEdge {
    clones: SmallVec<[(SpanId, f64); MAX_DECORATION_NESTING_DEPTH]>,
    break_width: f64,
    bytes: TextRange,
}

impl ClonedEdge {
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
                    cloned
                        .contains(&span.span_id)
                        .then(|| (id, f64::from(padding(span))))
                })
                .collect()
        };
        Self {
            clones,
            break_width: brk.map_or(0.0, |b| f64::from(break_width(b))),
            bytes: TextRange::empty_at(at),
        }
    }

    fn side(&self, inputs: &ValidatedInputs<'_>, index: SpanId) -> PaddingSide {
        let span = inputs.decoration_span(index).byte_range;
        let (cloned, own_cloned, outside, inside) = self.clones.iter().fold(
            (false, false, 0.0, 0.0),
            |(cloned, own_cloned, outside, inside), &(c, width)| {
                let clone = inputs.decoration_span(c).byte_range;
                if c < index && clone.contains_range(span) {
                    (cloned, own_cloned, outside + width, inside)
                } else if c >= index && span.contains_range(clone) {
                    (true, own_cloned || c == index, outside, inside + width)
                } else {
                    (cloned, own_cloned, outside, inside)
                }
            },
        );
        PaddingSide {
            cloned,
            own_cloned,
            outside,
            inside,
            break_width: self.break_width,
            bytes: self.bytes,
        }
    }
}

struct PaddingSide {
    cloned: bool,
    own_cloned: bool,
    outside: f64,
    inside: f64,
    break_width: f64,
    bytes: TextRange,
}

fn wrap_cut(span: &InlineDecorationSpan, own_cloned: bool) -> SpanEdgeCut {
    if span.break_mode == BoxDecorationBreak::Clone && own_cloned {
        SpanEdgeCut::ClonedWrap
    } else {
        SpanEdgeCut::SlicedWrap
    }
}

fn classify_edges(
    span: &InlineDecorationSpan,
    line_bytes: TextRange,
    base_rtl: bool,
    [at_start_own_cloned, at_end_own_cloned]: [bool; 2],
    group: &VisualGroup,
) -> SpanFragmentEdges {
    let (left_cloned, right_cloned) = if base_rtl {
        (at_end_own_cloned, at_start_own_cloned)
    } else {
        (at_start_own_cloned, at_end_own_cloned)
    };
    match group.kind {
        GroupKind::ClonedLeft => SpanFragmentEdges {
            left: wrap_cut(span, left_cloned),
            right: SpanEdgeCut::BidiSplit,
        },
        GroupKind::ClonedRight => SpanFragmentEdges {
            left: SpanEdgeCut::BidiSplit,
            right: wrap_cut(span, right_cloned),
        },
        GroupKind::Content | GroupKind::TrailingOnly => {
            let rtl = group.level.direction().is_rtl();
            let logical_start = if group.bytes.start() == span.byte_range.start() {
                SpanEdgeCut::LogicalEnd
            } else if group.bytes.start() > line_bytes.start() {
                SpanEdgeCut::BidiSplit
            } else {
                wrap_cut(span, if rtl { right_cloned } else { left_cloned })
            };
            let logical_end = if group.bytes.end() == span.byte_range.end() {
                SpanEdgeCut::LogicalEnd
            } else if group.bytes.end() < line_bytes.end() {
                SpanEdgeCut::BidiSplit
            } else {
                wrap_cut(span, if rtl { left_cloned } else { right_cloned })
            };
            if rtl {
                SpanFragmentEdges {
                    left: logical_end,
                    right: logical_start,
                }
            } else {
                SpanFragmentEdges {
                    left: logical_start,
                    right: logical_end,
                }
            }
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    pub(super) fn for_each_span_group(
        &self,
        line: &PlacedLine<'_, '_>,
        mut emit: impl FnMut(&InlineDecorationSpan, &VisualGroup, SpanFragmentEdges),
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
            let start_side = at_start.side(self.inputs, index);
            let end_side = at_end.side(self.inputs, index);
            let (at_start_own_cloned, at_end_own_cloned) =
                (start_side.own_cloned, end_side.own_cloned);
            self.attach_padding(
                line,
                trailing,
                [start_side, end_side],
                &is_member,
                &mut groups,
            );
            let (lo, hi) = (
                range.start().max(bytes.start()),
                range.end().min(bytes.end()),
            );
            for g in &mut groups {
                let start = g.bytes.start().max(lo);
                g.bytes = TextRange::new(start, g.bytes.end().min(hi).max(start));
                let edges = classify_edges(
                    span,
                    bytes,
                    self.base.is_rtl(),
                    [at_start_own_cloned, at_end_own_cloned],
                    g,
                );
                emit(span, g, edges);
            }
        }
    }

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
        let (line_lo, line_hi) = (line.bounds.body_lo - left.break_width, line.bounds.body_hi);
        let (span_lo, span_hi) = (line_lo + left.outside, line_hi - right.outside);
        let group = |lo: f64, hi: f64, bytes: TextRange, kind: GroupKind| VisualGroup {
            lo,
            hi,
            bytes,
            visual_span: None,
            level: self.base.base_level(),
            is_bare_boundary: false,
            kind,
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
                groups.push(group(edge, edge, bytes, GroupKind::TrailingOnly));
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
                groups.insert(
                    0,
                    group(
                        span_lo,
                        span_lo + left.inside,
                        left.bytes,
                        GroupKind::ClonedLeft,
                    ),
                );
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
                groups.push(group(
                    span_hi - right.inside,
                    span_hi,
                    right.bytes,
                    GroupKind::ClonedRight,
                ));
            } else {
                last.hi = last.hi.max(span_hi);
            }
        }
    }

    fn fragment_vertical_extent(
        &self,
        span: &InlineDecorationSpan,
        g: &VisualGroup,
        line: &AlignedLine<'_, '_>,
        line_box: &LineBoxMetrics,
        slot: InlineSlotConstraint,
    ) -> (f64, f64) {
        let strut = (f64::from(self.strut.ascent), f64::from(self.strut.descent));
        match span.height_style {
            BoxHeightStyle::Strut => strut,
            BoxHeightStyle::Max => (line_box.ascent, line_box.descent),
            BoxHeightStyle::Tight => {
                let Some((first_v, last_v)) = g.visual_span else {
                    return strut;
                };
                let (mut ascent, mut descent) = (0.0_f64, 0.0_f64);
                let mut any_item = false;
                for p in line.order.pick_range(line.placed, first_v, last_v) {
                    match p.piece.kind {
                        PieceKind::Cluster { run, cluster } => {
                            let font = self.inputs.cluster(cluster).font;
                            if let Some(m) =
                                self.metrics
                                    .get(&(font.font_id.0, font.scale.to_bits(), run))
                            {
                                let (a, d) = if self.ast.writing_mode.is_vertical() {
                                    (f64::from(m.vertical_ascent), f64::from(m.vertical_descent))
                                } else {
                                    (f64::from(m.ascent), f64::from(m.descent))
                                };
                                ascent = ascent.max(a + p.y_shift);
                                descent = descent.max(d - p.y_shift);
                                any_item = true;
                            }
                        }
                        PieceKind::Object(object) => {
                            let (_, a, d) = Self::fitted_object(object, slot);
                            ascent = ascent.max(a + p.y_shift);
                            descent = descent.max(d - p.y_shift);
                            any_item = true;
                        }
                        _ => {}
                    }
                }
                if any_item { (ascent, descent) } else { strut }
            }
        }
    }

    pub(super) fn span_fragments(
        &self,
        line: &AlignedLine<'_, '_>,
        line_box: &LineBoxMetrics,
        slot: InlineSlotConstraint,
    ) -> Vec<PositionedSpanFragment> {
        let mut fragments = Vec::new();
        self.for_each_span_group(line, |span, g, edges| {
            let (ascent, descent) = self.fragment_vertical_extent(span, g, line, line_box, slot);
            fragments.push(PositionedSpanFragment {
                span_id: span.span_id,
                span_range: span.byte_range,
                byte_range: g.bytes,
                x: g.lo as f32,
                width: (g.hi - g.lo) as f32,
                ascent: ascent as f32,
                descent: descent as f32,
                bidi_level: g.level,
                edges,
                ribbon_offset: 0.0,
                total_span_advance: 0.0,
            });
        });
        fragments
    }

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
                    x: g.lo as f32,
                    width: (g.hi - g.lo) as f32,
                    bidi_level: g.level,
                });
                covered = end;
            }
        }
        out
    }

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
        let (mut open_x, mut open_right) = (0.0_f64, 0.0_f64);
        let mut pending_glue: Option<TextRange> = None;
        for &p in line.order.pick(line.placed) {
            let bytes = p.piece.bytes;
            match p.piece.kind {
                PieceKind::Cluster { run, cluster } if drawn(self.inputs.run(run).decoration) => {
                    let m = self.cluster_metrics(run, cluster)?;
                    let (y_offset, thickness) = (
                        (f64::from(position(m)) + p.y_shift) as f32,
                        m.underline_thickness,
                    );
                    let right = p.x + p.width;
                    let Some(segment) = open.as_mut() else {
                        (open_x, open_right) = (p.x, right);
                        open = Some(DecorationLine {
                            kind,
                            byte_range: bytes,
                            x: p.x as f32,
                            width: (right - p.x) as f32,
                            y_offset,
                            thickness,
                        });
                        continue;
                    };
                    if let Some(glue) = pending_glue.take() {
                        segment.byte_range = segment.byte_range.cover(glue);
                    }
                    open_right = right.max(open_right);
                    segment.width = (open_right - open_x) as f32;
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
