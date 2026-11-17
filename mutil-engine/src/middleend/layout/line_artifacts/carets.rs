//! A line's caret stops (spec §6.1, CRT-1 to CRT-3, INT-4 to INT-6, INT-9).

use super::super::{Break, Layout, PieceKind, PlacedPiece};
use crate::frontend::GlueKind;
use crate::middleend::tolerance::approx_eq;
use crate::middleend::{
    CaretAffinity, CaretStop, HangingWhitespaceMode, PositionedInlineBox, VisualGlyphRun,
    resolve_ligature_carets,
};
use crate::types::{AtomFlags, BidiLevel, FontInstanceRef, TextRange};

pub(in crate::middleend::layout) struct CaretContext<'a> {
    pub(in crate::middleend::layout) index: usize,
    pub(in crate::middleend::layout) placed: &'a [PlacedPiece],
    pub(in crate::middleend::layout) order: &'a [usize],
    pub(in crate::middleend::layout) body_len: usize,
    pub(in crate::middleend::layout) soft_wrap: bool,
    pub(in crate::middleend::layout) byte_range: TextRange,
    pub(in crate::middleend::layout) visible_end: usize,
    pub(in crate::middleend::layout) content_start: f32,
    pub(in crate::middleend::layout) end_break: Option<&'a Break>,
    pub(in crate::middleend::layout) runs: &'a [VisualGlyphRun],
    pub(in crate::middleend::layout) boxes: &'a [PositionedInlineBox],
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

impl<'i, 'a> Layout<'i, 'a> {
    /// The line's caret stops in visual order (spec §6.1, CRT-1 to CRT-3,
    /// INT-4 to INT-6, INT-9).
    pub(in crate::middleend::layout) fn caret_stops(
        &self,
        ctx: &CaretContext<'_>,
    ) -> Vec<CaretStop> {
        // Under `Collapsible`, a soft wrap hides the trailing white space,
        // whose offsets belong to the next line (INT-6).
        let hides_trailing =
            ctx.soft_wrap && self.options.whitespace == HangingWhitespaceMode::Collapsible;
        let mut stops = self.kept_stops(ctx, hides_trailing);
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
        collapse_same_level_overlaps(&mut stops);
        let split = split_offsets(&stops);
        let base_level = self.base.base_level();
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
                    level: level.unwrap_or_else(|| level_at(ctx, stop.offset, base_level)),
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

    /// The candidate stops the line keeps, in visual order, one per distinct
    /// `(x, offset)`: those inside the line's bytes and outside any atomic
    /// span's interior.
    fn kept_stops(&self, ctx: &CaretContext<'_>, hides_trailing: bool) -> Vec<RawStop> {
        // A hard break's own bytes start the next line.
        let drops_end = !ctx.soft_wrap
            && !ctx.byte_range.is_empty()
            && ctx
                .end_break
                .is_some_and(|b| b.mandatory && !b.bytes.is_empty());
        let belongs_here = |offset: usize| {
            ctx.byte_range.start() <= offset
                && offset <= ctx.byte_range.end()
                && !(drops_end && offset == ctx.byte_range.end())
                && !self
                    .ast
                    .atomic_span_containing(offset)
                    .is_some_and(|span| span.is_interior(offset))
                && !(hides_trailing && offset > ctx.visible_end && offset != ctx.byte_range.start())
        };
        let mut stops: Vec<RawStop> = Vec::new();
        for stop in self.raw_caret_stops(ctx, hides_trailing) {
            let is_duplicate = stops
                .iter()
                .rev()
                .take(4)
                .any(|s| s.offset == stop.offset && approx_eq(s.x, stop.x));
            if belongs_here(stop.offset) && !is_duplicate {
                stops.push(stop);
            }
        }
        stops
    }

    /// The line's candidate stops in visual order, before filtering: the
    /// content start of an empty line, the body's pieces, then the trailing
    /// whitespace beyond the far edge of the base direction (HNG-3, INT-6).
    fn raw_caret_stops(&self, ctx: &CaretContext<'_>, hides_trailing: bool) -> Vec<RawStop> {
        let mut raw = Vec::new();
        if ctx.byte_range.is_empty()
            || ctx.body_len == 0 && (ctx.placed.is_empty() || hides_trailing)
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
        if !hides_trailing {
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
        // A glyph-less piece lends no level of its own (spec CRT-2).
        let level = match p.kind {
            PieceKind::Cluster { .. } | PieceKind::Object { .. } | PieceKind::Tab => Some(p.level),
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
        let (left, right) = if p.level.direction().is_rtl() {
            (PieceEdge::End, PieceEdge::Start)
        } else {
            (PieceEdge::Start, PieceEdge::End)
        };
        raw.push(stop(placed_piece.x, left));
        if let PieceKind::Cluster { run, cluster } = p.kind {
            self.push_ligature_stops(placed_piece, run, cluster, level, raw);
        }
        raw.push(stop(placed_piece.x + placed_piece.width, right));
    }

    /// The stops between a ligature's atoms (INT-5), from the font's caret
    /// positions; none for a single atom or an emoji sequence.
    fn push_ligature_stops(
        &self,
        placed_piece: PlacedPiece,
        run: usize,
        cluster: usize,
        level: Option<BidiLevel>,
        raw: &mut Vec<RawStop>,
    ) {
        let c = &self.ast.clusters[cluster];
        let k_atoms = c.atom_count();
        if k_atoms <= 1 || c.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE) {
            return;
        }
        let direction = placed_piece.piece.level.direction();
        let rtl = direction.is_rtl();
        let (glyphs, width) = self.cluster_glyphs(cluster);
        let first_glyph = glyphs.first().map_or(0, |g| g.glyph_id);
        let carets = self.shaper.ligature_carets(
            FontInstanceRef {
                font: c.font,
                font_size: self.ast.runs[run].font_size,
                variations: self.ast.runs[run].features.variations(),
            },
            first_glyph,
            direction,
        );
        // Positions increase from the left edge (SHP-7); in RTL they pair
        // with the atoms in reverse logical order (INT-5).
        let atoms = &self.inputs.run_atoms(run)[c.atom_range.clone()];
        let mut offsets: Vec<usize> = atoms[1..]
            .iter()
            .map(|atom| atom.byte_range.start())
            .collect();
        if rtl {
            offsets.reverse();
        }
        let positions = resolve_ligature_carets(width, k_atoms, &carets);
        // An RTL cluster's justification lies before its glyphs.
        let origin = placed_piece.x + if rtl { placed_piece.expansion } else { 0.0 };
        raw.extend(positions.iter().zip(offsets).map(|(&pos, offset)| RawStop {
            x: origin + pos,
            offset,
            edge: PieceEdge::Start,
            level,
        }));
    }
}

/// The level of an entity touching `offset`, else the paragraph's.
fn level_at(ctx: &CaretContext<'_>, offset: usize, base_level: BidiLevel) -> BidiLevel {
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
}

/// Indices of `stops` ordered by offset, ties in their current order, for
/// walking the stops that share an offset with `chunk_by`.
fn indices_by_offset(stops: &[RawStop]) -> Vec<usize> {
    let mut by_offset: Vec<usize> = (0..stops.len()).collect();
    by_offset.sort_by_key(|&i| stops[i].offset);
    by_offset
}

/// Merges stops at the same `offset` that share the same bidi level (e.g.
/// when negative-width `CjkPunctCompression` overlaps two adjacent clusters)
/// into one stop at their midpoint, so `split_offsets` only splits genuine
/// direction boundaries (CRT-3).
fn collapse_same_level_overlaps(stops: &mut Vec<RawStop>) {
    let mut remove = vec![false; stops.len()];
    let mut midpoints: Vec<(usize, f32)> = Vec::new();
    let by_offset = indices_by_offset(stops);
    for same in by_offset.chunk_by(|&i, &j| stops[i].offset == stops[j].offset) {
        let &[first, ref rest @ ..] = same else {
            continue;
        };
        let level = stops[first].level;
        if rest.is_empty() || level.is_none() || rest.iter().any(|&i| stops[i].level != level) {
            continue;
        }
        let xs = same.iter().map(|&i| stops[i].x);
        let min_x = xs.clone().fold(f32::INFINITY, f32::min);
        let max_x = xs.fold(f32::NEG_INFINITY, f32::max);
        if approx_eq(min_x, max_x) {
            continue;
        }
        midpoints.push((first, (min_x + max_x) / 2.0));
        for &i in rest {
            remove[i] = true;
        }
    }
    if midpoints.is_empty() {
        return;
    }
    for (i, x) in midpoints {
        stops[i].x = x;
    }
    *stops = core::mem::take(stops)
        .into_iter()
        .zip(remove)
        .filter_map(|(stop, removed)| (!removed).then_some(stop))
        .collect();
    stops.sort_by(|a, b| a.x.total_cmp(&b.x));
}

/// For each stop, whether its offset has stops at two positions on the line:
/// a direction boundary (spec CRT-3).
fn split_offsets(stops: &[RawStop]) -> Vec<bool> {
    let mut split = vec![false; stops.len()];
    let by_offset = indices_by_offset(stops);
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
