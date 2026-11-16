//! Line-level fragments, decorations, and caret stops (spec §5 Stage 5, §6.1).

use super::super::tolerance::approx_eq;
use super::super::{
    AtomicFragment, CaretAffinity, CaretStop, DecorationKind, DecorationLine,
    HangingWhitespaceMode, LayoutInputError, PositionedInlineBox, PositionedSpanFragment,
    PositionedTab, VisualGlyphRun, resolve_ligature_carets,
};
use super::line_builder::LineBounds;
use super::{Break, Layout, Piece, PieceKind, PlacedPiece};
use crate::frontend::InlineDecorationSpan;
use crate::types::{AtomFlags, BidiLevel, FontInstanceRef, TextRange};

pub(super) struct SpanContext<'a> {
    pub(super) placed: &'a [PlacedPiece],
    pub(super) order: &'a [usize],
    pub(super) bounds: LineBounds,
    pub(super) end_break: Option<&'a Break>,
    pub(super) post: f32,
    pub(super) line_bytes: TextRange,
    pub(super) align_shift: f32,
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
    pub(super) tabs: &'a [PositionedTab],
}

impl<'i, 'a> Layout<'i, 'a> {
    /// Walks each visual group of each decoration span on the line (spec ST2-3,
    /// FRG-1), calling `emit(span, x, width, bytes)`.
    pub(super) fn for_each_span_group(
        &self,
        ctx: &SpanContext<'_>,
        mut emit: impl FnMut(&InlineDecorationSpan, f32, f32, TextRange),
    ) {
        for span in &self.ast.decoration_spans {
            let range = span.byte_range;
            if range.end() < ctx.line_bytes.start() || range.start() > ctx.line_bytes.end() {
                continue;
            }
            let member = |p: &Piece| match p.kind {
                PieceKind::SpanEdge { span_id } => span_id == span.span_id,
                _ => {
                    !p.bytes.is_empty()
                        && range.start() <= p.bytes.start()
                        && p.bytes.end() <= range.end()
                }
            };
            let cloned_at_end = ctx
                .end_break
                .is_some_and(|b| b.cloned.contains(&span.span_id));
            let cloned_at_start = ctx.post > 0.0
                && range.start() < ctx.line_bytes.start()
                && ctx.line_bytes.start() < range.end();
            let mut group: Option<(f32, f32, TextRange)> = None;
            let mut flush = |group: &mut Option<(f32, f32, TextRange)>| {
                if let Some((a, b, bytes)) = group.take() {
                    emit(span, a, (b - a).max(0.0), bytes);
                }
            };
            for &k in ctx.order {
                let p = ctx.placed[k];
                // Another span's edge neither joins nor splits this span's
                // fragment: a nested span's padding lies inside it.
                if matches!(p.piece.kind, PieceKind::SpanEdge { span_id } if span_id != span.span_id)
                {
                    continue;
                }
                if member(&p.piece) {
                    let (a, b) = (p.x, p.x + p.width);
                    group = Some(match group {
                        Some((ga, gb, bytes)) => (ga.min(a), gb.max(b), bytes.cover(p.piece.bytes)),
                        None => (a, b, p.piece.bytes),
                    });
                } else {
                    flush(&mut group);
                }
            }
            if let Some((a, b, bytes)) = group.as_mut() {
                if cloned_at_end {
                    *b = b.max(ctx.bounds.body_hi);
                }
                if cloned_at_start {
                    *a = a
                        .min(ctx.bounds.body_lo - ctx.post)
                        .max(ctx.align_shift.min(*a));
                }
                // A fragment's bytes stay inside the span and the line.
                let start = bytes.start().max(range.start()).max(ctx.line_bytes.start());
                let end = bytes
                    .end()
                    .min(range.end())
                    .min(ctx.line_bytes.end())
                    .max(start);
                *bytes = TextRange::new(start, end);
            }
            flush(&mut group);
        }
    }

    /// One fragment per visual group of each decoration span on the line
    /// (spec ST2-3, FRG-1), padding included; a span cloned across this
    /// line's breaks reaches the line's edge.
    pub(super) fn span_fragments(&self, ctx: &SpanContext<'_>) -> Vec<PositionedSpanFragment> {
        let mut fragments = Vec::new();
        let (ascent, descent) = (self.strut.ascent.max(0.0), self.strut.descent.max(0.0));
        self.for_each_span_group(ctx, |span, x, width, byte_range| {
            fragments.push(PositionedSpanFragment {
                span_id: span.span_id,
                span_range: span.byte_range,
                byte_range,
                x,
                width,
                ascent,
                descent,
                bidi_level: self.base.base_level(),
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
            let mut groups: Vec<(f32, f32, TextRange)> = Vec::new();
            let mut open = false;
            for &k in order {
                let p = placed[k];
                let inside = !p.piece.bytes.is_empty()
                    && p.piece.bytes.start() < full.end()
                    && p.piece.bytes.end() > full.start();
                if inside {
                    let (a, b) = (p.x, p.x + p.width);
                    if open {
                        let g = groups.last_mut().expect("open group");
                        *g = (g.0.min(a), g.1.max(b), g.2.cover(p.piece.bytes));
                    } else {
                        groups.push((a, b, p.piece.bytes));
                        open = true;
                    }
                } else {
                    open = false;
                }
            }
            if groups.is_empty() {
                continue;
            }
            groups.sort_by_key(|g| g.2.start());
            let n = groups.len();
            let mut covered = part_start;
            for (i, (a, b, bytes)) in groups.into_iter().enumerate() {
                let end = if i + 1 == n {
                    part_end
                } else {
                    bytes.end().clamp(covered + 1, part_end)
                };
                if covered >= end {
                    continue;
                }
                out.push(AtomicFragment {
                    atomic_id: span.span_id,
                    full_byte_range: full,
                    fragment_byte_range: TextRange::new(covered, end),
                    x: a.min(b),
                    width: (b - a).abs(),
                    bidi_level: self.base.base_level(),
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
            let decorated = match p.kind {
                PieceKind::Cluster { run, .. } => {
                    let d = self.ast.runs[run].decoration;
                    match kind {
                        DecorationKind::Underline => d.underline,
                        DecorationKind::Strikethrough => d.strikethrough,
                    }
                }
                PieceKind::Glue => {
                    if open.is_some() {
                        let end = placed_piece.x + placed_piece.width;
                        pending_glue = Some(match pending_glue {
                            Some((_, bytes)) => (end, bytes.cover(p.bytes)),
                            None => (end, p.bytes),
                        });
                    }
                    continue;
                }
                _ => false,
            };
            if !decorated {
                out.extend(open.take());
                pending_glue = None;
                continue;
            }
            let PieceKind::Cluster { run, cluster } = p.kind else {
                unreachable!("only clusters are decorated")
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

    /// The line's caret stops in visual order (spec §6.1, CRT-1, CRT-2,
    /// INT-4 to INT-6, INT-9).
    pub(super) fn caret_stops(&self, ctx: &CaretContext<'_>) -> Vec<CaretStop> {
        let base_level = self.base.base_level();
        // The level of an entity touching `offset`, else the paragraph's.
        let level_at = |offset: usize| -> BidiLevel {
            ctx.runs
                .iter()
                .find(|r| {
                    r.glyphs.iter().any(|g| {
                        g.byte_range
                            .is_some_and(|b| b.start() <= offset && offset <= b.end())
                    })
                })
                .map(|r| r.bidi_level)
                .or_else(|| {
                    ctx.boxes
                        .iter()
                        .find(|b| b.byte_range.start() <= offset && offset <= b.byte_range.end())
                        .map(|b| b.bidi_level)
                })
                .or_else(|| {
                    ctx.tabs
                        .iter()
                        .any(|t| t.byte_range.start() <= offset && offset <= t.byte_range.end())
                        .then_some(base_level)
                })
                .unwrap_or(base_level)
        };
        let mut raw: Vec<(f32, usize)> = Vec::new();
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        if ctx.byte_range.is_empty()
            || ctx.body_len == 0 && (ctx.placed.is_empty() || collapsible && ctx.soft_wrap)
        {
            raw.push((ctx.content_start, ctx.byte_range.start()));
        }
        let push_piece = |k: usize, raw: &mut Vec<(f32, usize)>| {
            let placed_piece = ctx.placed[k];
            let p = placed_piece.piece;
            let (a, b) = (placed_piece.x, placed_piece.x + placed_piece.width);
            let rtl = placed_piece.level.direction().is_rtl();
            let (left, right) = if rtl {
                (p.bytes.end(), p.bytes.start())
            } else {
                (p.bytes.start(), p.bytes.end())
            };
            raw.push((a, left));
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
                    let positions = resolve_ligature_carets(width, k_atoms, &carets);
                    let pairs = positions.iter().zip(&atoms[1..]).map(|(&pos, atom)| {
                        let x = if rtl { a + width - pos } else { a + pos };
                        (x, atom.byte_range.start())
                    });
                    if rtl {
                        raw.extend(pairs.rev());
                    } else {
                        raw.extend(pairs);
                    }
                }
            }
            raw.push((b, right));
        };
        for &k in ctx.order {
            push_piece(k, &mut raw);
        }
        let trailing_stops = !collapsible || !ctx.soft_wrap;
        if trailing_stops && ctx.body_len < ctx.placed.len() {
            // Beyond the far edge of the base direction (HNG-3, INT-6). Under
            // `Collapsible` they all sit at the edge, so that the end of a
            // hard-broken line or of the paragraph stays reachable.
            if self.base.is_rtl() {
                for k in (ctx.body_len..ctx.placed.len()).rev() {
                    push_piece(k, &mut raw);
                }
            } else {
                for k in ctx.body_len..ctx.placed.len() {
                    push_piece(k, &mut raw);
                }
            }
        }
        // Visual order, one stop per distinct (x, offset).
        let mut stops: Vec<CaretStop> = Vec::new();
        for (x, offset) in raw {
            if !(ctx.byte_range.start() <= offset && offset <= ctx.byte_range.end()) {
                continue;
            }
            if self
                .ast
                .atomic_span_containing(offset)
                .is_some_and(|span| span.is_interior(offset))
            {
                continue;
            }
            // Offsets past the visible end of a soft-wrapped collapsible line
            // belong to the next line (INT-6).
            if collapsible
                && ctx.soft_wrap
                && offset > ctx.visible_end
                && offset != ctx.byte_range.start()
            {
                continue;
            }
            let duplicate = stops
                .iter()
                .rev()
                .take(4)
                .any(|s| s.offset == offset && approx_eq(s.x, x));
            if duplicate {
                continue;
            }
            stops.push(CaretStop {
                x,
                offset,
                affinity: CaretAffinity::Downstream,
                level: level_at(offset),
                line: ctx.index,
            });
        }
        // A hard break's own bytes start the next line.
        if !ctx.soft_wrap && !ctx.byte_range.is_empty() {
            let hard_end = ctx.byte_range.end();
            let is_terminated = ctx
                .end_break
                .is_some_and(|b| b.mandatory && !b.bytes.is_empty());
            if is_terminated {
                stops.retain(|s| s.offset != hard_end);
            }
        }
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
        // At a soft wrap the line's last logical stop is upstream (INT-1, INT-9).
        if ctx.soft_wrap
            && let Some(last) = stops.iter_mut().max_by_key(|s| s.offset)
        {
            last.affinity = CaretAffinity::Upstream;
        }
        stops
    }
}
