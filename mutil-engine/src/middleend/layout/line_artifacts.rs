//! Line-level fragments, decorations, and caret stops (spec §5 Stage 5, §6.1).

use super::super::tolerance::{approx_eq, tolerance};
use super::super::{
    AtomicFragment, CaretAffinity, CaretStop, DecorationKind, DecorationLine,
    HangingWhitespaceMode, LayoutInputError, PositionedInlineBox, PositionedSpanFragment,
    VisualGlyphRun, resolve_ligature_carets,
};
use super::line_builder::LineBounds;
use super::{Break, Layout, Piece, PieceKind, PlacedPiece};
use crate::frontend::GlueKind;
use crate::frontend::InlineDecorationSpan;
use crate::types::{AtomFlags, BidiLevel, FontInstanceRef, TextRange};

pub(super) struct SpanContext<'a> {
    pub(super) placed: &'a [PlacedPiece],
    pub(super) order: &'a [usize],
    pub(super) bounds: LineBounds,
    pub(super) end_break: Option<&'a Break>,
    pub(super) post: f32,
    pub(super) line_bytes: TextRange,
}

pub(super) struct CaretContext<'a> {
    pub(super) index: usize,
    pub(super) placed: &'a [PlacedPiece],
    pub(super) order: &'a [usize],
    pub(super) body_len: usize,
    pub(super) soft_wrap: bool,
    pub(super) byte_range: TextRange,
    pub(super) visible_end: usize,
    pub(super) content_start: f32,
    pub(super) end_break: Option<&'a Break>,
    pub(super) runs: &'a [VisualGlyphRun],
    pub(super) boxes: &'a [PositionedInlineBox],
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
                level: p.level,
                is_bare_boundary,
            }),
        }
        previous = Some(r);
    }
    groups
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
        let rtl = self.base.is_rtl();
        let pre = ctx.end_break.map_or(0.0, |b| b.pre);
        // The cloned padding on each physical side: the line end is on the
        // right in LTR and on the left in RTL (ST2-3).
        let (left_pad, right_pad) = if rtl {
            (pre, ctx.post)
        } else {
            (ctx.post, pre)
        };
        let line_lo = ctx.bounds.body_lo - left_pad;
        let line_hi = ctx.bounds.body_hi;
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
            let trailing_bytes = ctx.placed[ctx.order.len()..]
                .iter()
                .filter(|p| is_member(&p.piece) && !p.piece.bytes.is_empty())
                .fold(None::<TextRange>, |acc, p| {
                    Some(acc.map_or(p.piece.bytes, |r| r.cover(p.piece.bytes)))
                });
            if let Some(tb) = trailing_bytes {
                if let Some(last) = groups.iter_mut().max_by_key(|g| g.bytes.end()) {
                    last.bytes = last.bytes.cover(tb);
                    last.is_bare_boundary = false;
                } else {
                    let edge = if rtl { line_lo } else { line_hi };
                    groups.push(VisualGroup {
                        lo: edge,
                        hi: edge,
                        bytes: tb,
                        level: self.base.base_level(),
                        is_bare_boundary: false,
                    });
                }
            }
            if groups.is_empty() {
                continue;
            }
            let cloned_at_end = ctx
                .end_break
                .is_some_and(|b| b.cloned.contains(&span.span_id));
            let cloned_at_start = ctx.post > 0.0
                && range.start() < ctx.line_bytes.start()
                && ctx.line_bytes.start() < range.end();
            let (at_left, at_right) = if rtl {
                (cloned_at_end, cloned_at_start)
            } else {
                (cloned_at_start, cloned_at_end)
            };
            // Padding joins the outermost group only if nothing else lies
            // between them; otherwise it is a fragment of its own, at the
            // line's edge offset on that side (AX-5).
            let outside = |at: f32, leftwards: bool| {
                ctx.order.iter().map(|&k| &ctx.placed[k]).any(|p| {
                    p.width > 0.0
                        && !is_member(&p.piece)
                        && if leftwards {
                            p.x < at - tolerance(p.x, at)
                        } else {
                            p.x + p.width > at + tolerance(p.x + p.width, at)
                        }
                })
            };
            let padding = |lo: f32, hi: f32, at_end: bool| VisualGroup {
                lo,
                hi,
                bytes: TextRange::empty_at(if at_end {
                    ctx.line_bytes.end()
                } else {
                    ctx.line_bytes.start()
                }),
                level: self.base.base_level(),
                is_bare_boundary: false,
            };
            if at_left {
                if outside(groups[0].lo, true) {
                    groups.insert(0, padding(line_lo, line_lo + left_pad, rtl));
                } else {
                    groups[0].lo = groups[0].lo.min(line_lo);
                }
            }
            if at_right {
                let last = groups.len() - 1;
                if outside(groups[last].hi, false) {
                    groups.push(padding(line_hi - right_pad, line_hi, !rtl));
                } else {
                    groups[last].hi = groups[last].hi.max(line_hi);
                }
            }
            for g in &mut groups {
                // A fragment's bytes stay inside the span and the line.
                let start = g
                    .bytes
                    .start()
                    .max(range.start())
                    .max(ctx.line_bytes.start());
                let end = g
                    .bytes
                    .end()
                    .min(range.end())
                    .min(ctx.line_bytes.end())
                    .max(start);
                g.bytes = TextRange::new(start, end);
                emit(span, g);
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
    /// FRG-3), together covering exactly the part of the span on the line.
    pub(super) fn atomic_fragments(
        &self,
        placed: &[PlacedPiece],
        order: &[usize],
        line_bytes: TextRange,
    ) -> Vec<AtomicFragment> {
        let mut out = Vec::new();
        for span in &self.ast.atomic_spans {
            let full = span.byte_range;
            let part_start = full.start().max(line_bytes.start());
            let part_end = full.end().min(line_bytes.end());
            if part_start >= part_end {
                continue;
            }
            let mut groups = visual_groups(placed, order, |p| {
                !p.bytes.is_empty() && p.bytes.start() < full.end() && p.bytes.end() > full.start()
            });
            groups.sort_by_key(|g| g.bytes.start());
            let n = groups.len();
            let mut covered = part_start;
            for (i, g) in groups.into_iter().enumerate() {
                if covered >= part_end {
                    break;
                }
                let end = if i + 1 == n {
                    part_end
                } else {
                    g.bytes.end().clamp(covered + 1, part_end)
                };
                if covered >= end {
                    continue;
                }
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
    /// clusters and the glue between them (spec ST5-1).
    pub(super) fn decorations(
        &mut self,
        kind: DecorationKind,
        placed: &[PlacedPiece],
        order: &[usize],
        out: &mut Vec<DecorationLine>,
    ) -> Result<(), LayoutInputError> {
        let mut open: Option<DecorationLine> = None;
        let mut pending_glue: Option<(f32, TextRange)> = None;
        for &k in order {
            let placed_piece = placed[k];
            let p = placed_piece.piece;
            let decorated = |run: usize| {
                let d = self.ast.runs[run].decoration;
                match kind {
                    DecorationKind::Underline => d.underline,
                    DecorationKind::Strikethrough => d.strikethrough,
                }
            };
            let (run, cluster) = match p.kind {
                PieceKind::Cluster { run, cluster } if decorated(run) => (run, cluster),
                PieceKind::Glue { .. } => {
                    if open.is_some() {
                        let end = placed_piece.x + placed_piece.width;
                        pending_glue = Some(match pending_glue {
                            Some((_, bytes)) => (end, bytes.cover(p.bytes)),
                            None => (end, p.bytes),
                        });
                    }
                    continue;
                }
                PieceKind::SpanEdge { .. } => continue,
                _ => {
                    out.extend(open.take());
                    pending_glue = None;
                    continue;
                }
            };
            let m = self.cluster_metrics(run, cluster)?;
            let shift = placed_piece.y_shift;
            let (position, thickness) = match kind {
                DecorationKind::Underline => (m.underline_position, m.underline_thickness),
                DecorationKind::Strikethrough => (m.strikeout_position, m.underline_thickness),
            };
            let (a, b) = (placed_piece.x, placed_piece.x + placed_piece.width);
            match open.as_mut() {
                Some(line) => {
                    if let Some((_, bytes)) = pending_glue.take() {
                        line.byte_range = line.byte_range.cover(bytes);
                    }
                    line.width = b.max(line.x + line.width) - line.x;
                    line.byte_range = line.byte_range.cover(p.bytes);
                    // The lowest underline of the fonts it crosses (ST5-1).
                    if kind == DecorationKind::Underline && position + shift < line.y_offset {
                        line.y_offset = position + shift;
                        line.thickness = thickness;
                    }
                }
                None => {
                    open = Some(DecorationLine {
                        kind,
                        byte_range: p.bytes,
                        x: a,
                        width: b - a,
                        y_offset: position + shift,
                        thickness,
                    });
                }
            }
        }
        out.extend(open);
        Ok(())
    }

    /// A piece's caret stops, left to right: its two visual edges and the
    /// INT-5 sub-slots of a ligature between them.
    fn push_piece_stops(&self, placed_piece: PlacedPiece, raw: &mut Vec<RawStop>) {
        let p = placed_piece.piece;
        // A compression pool squeezed below zero overlaps its neighbours and
        // owns no bytes: its edges would only duplicate theirs (CRT-3).
        if placed_piece.width < 0.0
            && matches!(p.kind, PieceKind::Glue { item }
                if self.glue(item).kind == GlueKind::CjkPunctCompression)
        {
            return;
        }
        let (a, b) = (placed_piece.x, placed_piece.x + placed_piece.width);
        let rtl = placed_piece.level.direction().is_rtl();
        // A glyph-less piece lends no level of its own (spec CRT-2).
        let level = match p.kind {
            PieceKind::Cluster { .. } | PieceKind::Object { .. } | PieceKind::Tab => {
                Some(placed_piece.level)
            }
            PieceKind::Glue { .. } | PieceKind::SpanEdge { .. } => None,
        };
        let stop = |x: f32, edge: PieceEdge| RawStop {
            x,
            offset: match edge {
                PieceEdge::Start => p.bytes.start(),
                PieceEdge::End => p.bytes.end(),
            },
            edge,
            level,
        };
        let (left, right) = if rtl {
            (PieceEdge::End, PieceEdge::Start)
        } else {
            (PieceEdge::Start, PieceEdge::End)
        };
        raw.push(stop(a, left));
        if let PieceKind::Cluster { run, cluster } = p.kind {
            let c = &self.ast.clusters[cluster];
            let k_atoms = c.atom_count();
            if k_atoms > 1 && !c.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE) {
                let atoms = &self.inputs.run_atoms(run)[c.atom_range.clone()];
                let (glyphs, width) = self.cluster_glyphs(cluster);
                let first_glyph = glyphs.first().map_or(0, |g| g.glyph_id);
                let carets = self.shaper.ligature_carets(
                    FontInstanceRef {
                        font: c.font,
                        font_size: self.ast.runs[run].font_size,
                        variations: self.ast.runs[run].features.variations(),
                    },
                    first_glyph,
                    placed_piece.level.direction(),
                );
                // Positions increase from the left edge (SHP-7); in RTL they
                // pair with the atoms in reverse logical order (INT-5).
                let mut offsets: Vec<usize> = atoms[1..]
                    .iter()
                    .map(|atom| atom.byte_range.start())
                    .collect();
                if rtl {
                    offsets.reverse();
                }
                let positions = resolve_ligature_carets(width, k_atoms, &carets);
                // An RTL cluster's justification lies before its glyphs.
                let origin = a + if rtl { placed_piece.expansion } else { 0.0 };
                raw.extend(positions.iter().zip(offsets).map(|(&pos, offset)| RawStop {
                    x: origin + pos,
                    offset,
                    edge: PieceEdge::Start,
                    level,
                }));
            }
        }
        raw.push(stop(b, right));
    }

    /// The line's candidate stops in visual order, before filtering: the
    /// content start of an empty line, the body's pieces, then the trailing
    /// whitespace beyond the far edge of the base direction (HNG-3, INT-6).
    fn raw_caret_stops(&self, ctx: &CaretContext<'_>, collapsible: bool) -> Vec<RawStop> {
        let mut raw = Vec::new();
        if ctx.byte_range.is_empty()
            || ctx.body_len == 0 && (ctx.placed.is_empty() || collapsible && ctx.soft_wrap)
        {
            raw.push(RawStop {
                x: ctx.content_start,
                offset: ctx.byte_range.start(),
                edge: PieceEdge::Start,
                level: None,
            });
        }
        for &k in ctx.order {
            self.push_piece_stops(ctx.placed[k], &mut raw);
        }
        let trailing_stops = !collapsible || !ctx.soft_wrap;
        if trailing_stops && ctx.body_len < ctx.placed.len() {
            // Under `Collapsible` they all sit at the edge, so that the end of
            // a hard-broken line or of the paragraph stays reachable.
            let trailing = &ctx.placed[ctx.body_len..];
            if self.base.is_rtl() {
                for &p in trailing.iter().rev() {
                    self.push_piece_stops(p, &mut raw);
                }
            } else {
                for &p in trailing {
                    self.push_piece_stops(p, &mut raw);
                }
            }
        }
        raw
    }

    /// The line's caret stops in visual order (spec §6.1, CRT-1 to CRT-3,
    /// INT-4 to INT-6, INT-9).
    pub(super) fn caret_stops(&self, ctx: &CaretContext<'_>) -> Vec<CaretStop> {
        let base_level = self.base.base_level();
        // The level of an entity touching `offset`, else the paragraph's.
        let level_at = |offset: usize| -> BidiLevel {
            let touches = |r: TextRange| r.start() <= offset && offset <= r.end();
            ctx.runs
                .iter()
                .find(|r| r.glyphs.iter().any(|g| g.byte_range.is_some_and(touches)))
                .map(|r| r.bidi_level)
                .or_else(|| {
                    ctx.boxes
                        .iter()
                        .find(|b| touches(b.byte_range))
                        .map(|b| b.bidi_level)
                })
                .unwrap_or(base_level)
        };
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        // A hard break's own bytes start the next line.
        let is_terminated = ctx
            .end_break
            .is_some_and(|b| b.mandatory && !b.bytes.is_empty());
        let drops_end = !ctx.soft_wrap && !ctx.byte_range.is_empty() && is_terminated;
        // Visual order, one stop per distinct (x, offset).
        let mut stops: Vec<RawStop> = Vec::new();
        for stop in self.raw_caret_stops(ctx, collapsible) {
            let offset = stop.offset;
            let keep = ctx.byte_range.start() <= offset
                && offset <= ctx.byte_range.end()
                && !(drops_end && offset == ctx.byte_range.end())
                && !self
                    .ast
                    .atomic_span_containing(offset)
                    .is_some_and(|span| span.is_interior(offset))
                // Offsets past the visible end of a soft-wrapped collapsible
                // line belong to the next line (INT-6).
                && !(collapsible
                    && ctx.soft_wrap
                    && offset > ctx.visible_end
                    && offset != ctx.byte_range.start())
                && !stops
                    .iter()
                    .rev()
                    .take(4)
                    .any(|s| s.offset == offset && approx_eq(s.x, stop.x));
            if keep {
                stops.push(stop);
            }
        }
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
        collapse_same_level_overlaps(&mut stops);
        let split = split_offsets(&stops);
        let mut out: Vec<CaretStop> = stops
            .iter()
            .zip(&split)
            .map(|(stop, &is_split)| {
                // A direction boundary's two stops (CRT-3).
                let (affinity, level) = match (is_split, stop.edge) {
                    (true, PieceEdge::End) => (CaretAffinity::Upstream, stop.level),
                    (true, PieceEdge::Start) => (CaretAffinity::Downstream, stop.level),
                    (false, _) => (CaretAffinity::Downstream, None),
                };
                CaretStop {
                    x: stop.x,
                    offset: stop.offset,
                    affinity,
                    level: level.unwrap_or_else(|| level_at(stop.offset)),
                    line: ctx.index,
                }
            })
            .collect();
        // At a soft wrap the line's last logical stop is upstream (INT-1,
        // INT-9), unless CRT-3 has split its offset.
        if ctx.soft_wrap
            && let Some(last) = (0..out.len()).max_by_key(|&i| out[i].offset)
            && !split[last]
        {
            out[last].affinity = CaretAffinity::Upstream;
        }
        out
    }
}

/// Which logical edge of its piece a caret stop marks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PieceEdge {
    /// Where the piece starts: a following entity's leading edge.
    Start,
    /// Where the piece ends: a preceding entity's trailing edge.
    End,
}

/// A candidate caret stop, with the edge it marks and the level of the
/// entity it belongs to, if that entity renders (spec CRT-3).
#[derive(Clone, Copy)]
struct RawStop {
    x: f32,
    offset: usize,
    edge: PieceEdge,
    level: Option<BidiLevel>,
}

/// Merges stops at the same `offset` that share the same bidi level (e.g.
/// when negative-width `CjkPunctCompression` overlaps two adjacent clusters)
/// into one stop at their midpoint, so `split_offsets` only splits genuine
/// direction boundaries (CRT-3).
fn collapse_same_level_overlaps(stops: &mut Vec<RawStop>) {
    let mut by_offset: Vec<usize> = (0..stops.len()).collect();
    by_offset.sort_by_key(|&i| stops[i].offset);
    let mut remove = vec![false; stops.len()];
    let mut updates: Vec<(usize, f32)> = Vec::new();
    for same in by_offset.chunk_by(|&i, &j| stops[i].offset == stops[j].offset) {
        if same.len() < 2 {
            continue;
        }
        let first_level = stops[same[0]].level;
        if first_level.is_none() || same.iter().any(|&i| stops[i].level != first_level) {
            continue;
        }
        let min_x = same
            .iter()
            .map(|&i| stops[i].x)
            .fold(f32::INFINITY, f32::min);
        let max_x = same
            .iter()
            .map(|&i| stops[i].x)
            .fold(f32::NEG_INFINITY, f32::max);
        if approx_eq(min_x, max_x) {
            continue;
        }
        updates.push((same[0], (min_x + max_x) / 2.0));
        for &i in &same[1..] {
            remove[i] = true;
        }
    }
    if !updates.is_empty() {
        for (i, x) in updates {
            stops[i].x = x;
        }
        *stops = core::mem::take(stops)
            .into_iter()
            .zip(remove)
            .filter_map(|(stop, removed)| (!removed).then_some(stop))
            .collect();
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
}

/// For each stop, whether its offset has stops at two positions on the line:
/// a direction boundary (spec CRT-3).
fn split_offsets(stops: &[RawStop]) -> Vec<bool> {
    let mut by_offset: Vec<usize> = (0..stops.len()).collect();
    by_offset.sort_by_key(|&i| stops[i].offset);
    let mut split = vec![false; stops.len()];
    for same in by_offset.chunk_by(|&i, &j| stops[i].offset == stops[j].offset) {
        let x = stops[same[0]].x;
        if same.iter().any(|&i| !approx_eq(stops[i].x, x)) {
            for &i in same {
                split[i] = true;
            }
        }
    }
    split
}
