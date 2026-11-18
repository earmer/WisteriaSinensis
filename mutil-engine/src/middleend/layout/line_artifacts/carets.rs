//! A line's caret stops (spec §6.1, CRT-1 to CRT-3, INT-4 to INT-6, INT-9).

use super::super::line_builder::LineBuildState;
use super::super::{Layout, PieceKind, PlacedPiece};
use crate::frontend::GlueKind;
use crate::middleend::tolerance::approx_eq;
use crate::middleend::{
    CaretAffinity, CaretStop, HangingWhitespaceMode, PositionedInlineBox, VisualGlyphRun,
    resolve_ligature_carets,
};
use crate::types::{AtomFlags, BidiLevel};

/// A candidate caret stop, with the affinity and level it takes at a
/// direction boundary (CRT-3): upstream at its piece's end, downstream at its
/// start, and the level of its entity if that renders.
#[derive(Clone, Copy)]
struct RawStop {
    x: f32,
    offset: usize,
    affinity: CaretAffinity,
    level: Option<BidiLevel>,
}

impl Layout<'_, '_> {
    /// The line's caret stops in visual order (spec §6.1, CRT-1 to CRT-3,
    /// INT-4 to INT-6, INT-9), given the end of its visible bytes, where its
    /// content starts, and its glyph runs and boxes.
    pub(in crate::middleend::layout) fn caret_stops(
        &self,
        line: &LineBuildState<'_>,
        visible_end: usize,
        content_start: f32,
        runs: &[VisualGlyphRun],
        boxes: &[PositionedInlineBox],
    ) -> Vec<CaretStop> {
        let bytes = line.byte_range;
        // Under `Collapsible`, a soft wrap hides the trailing white space,
        // whose offsets belong to the next line (INT-6). Otherwise it all
        // sits beyond the far edge of the base direction (HNG-3), so the end
        // of a hard-broken line or of the paragraph stays reachable.
        let hides_trailing =
            line.soft_wrap && self.options.whitespace == HangingWhitespaceMode::Collapsible;
        let mut raw = Vec::new();
        if bytes.is_empty() || line.body_len == 0 && (line.placed.is_empty() || hides_trailing) {
            raw.push(RawStop {
                x: content_start,
                offset: bytes.start(),
                affinity: CaretAffinity::Downstream,
                level: None,
            });
        }
        let trailing = if hides_trailing {
            &[][..]
        } else {
            &line.placed[line.body_len..]
        };
        let (forward, backward) = match self.base.is_rtl() {
            false => (trailing, &[][..]),
            true => (&[][..], trailing),
        };
        let body = line.order.iter().map(|&k| &line.placed[k]);
        for &piece in body.chain(forward).chain(backward.iter().rev()) {
            self.push_piece_stops(piece, &mut raw);
        }
        // A hard break's own bytes start the next line.
        let drops_end = !line.soft_wrap
            && !bytes.is_empty()
            && line
                .end_break
                .is_some_and(|b| b.mandatory && !b.bytes.is_empty());
        let belongs_here = |offset: usize| {
            bytes.start() <= offset
                && offset <= bytes.end()
                && !(drops_end && offset == bytes.end())
                && !self
                    .ast
                    .atomic_span_containing(offset)
                    .is_some_and(|span| span.is_interior(offset))
                && !(hides_trailing && offset > visible_end && offset != bytes.start())
        };
        // One stop per distinct `(x, offset)`.
        let mut stops: Vec<RawStop> = Vec::new();
        for stop in raw.into_iter().filter(|s| belongs_here(s.offset)) {
            let recent = &stops[stops.len().saturating_sub(4)..];
            if !recent
                .iter()
                .any(|s| s.offset == stop.offset && approx_eq(s.x, stop.x))
            {
                stops.push(stop);
            }
        }
        resolve_shared_offsets(&mut stops, line.soft_wrap);
        // The level of an entity touching `offset`, else the paragraph's.
        let level_at = |offset: usize| {
            let glyphs = runs.iter().flat_map(|r| {
                r.glyphs
                    .iter()
                    .filter_map(|g| Some((g.byte_range?, r.bidi_level)))
            });
            glyphs
                .chain(boxes.iter().map(|b| (b.byte_range, b.bidi_level)))
                .find(|(r, _)| r.start() <= offset && offset <= r.end())
                .map_or_else(|| self.base.base_level(), |(_, level)| level)
        };
        let to_stop = |s: &RawStop| CaretStop {
            x: s.x,
            offset: s.offset,
            affinity: s.affinity,
            level: s.level.unwrap_or_else(|| level_at(s.offset)),
            line: line.index,
        };
        stops.iter().map(to_stop).collect()
    }

    /// A piece's caret stops, left to right: its two visual edges and, between
    /// them, those between a ligature's atoms (INT-5) from the font's caret
    /// positions, none for a single atom or an emoji sequence. Positions
    /// increase from the left edge (SHP-7); in RTL they pair with the atoms
    /// in reverse logical order, and the justification lies before the glyphs.
    fn push_piece_stops(&self, placed: PlacedPiece, raw: &mut Vec<RawStop>) {
        let p = placed.piece;
        // A compression pool squeezed below zero overlaps its neighbours and
        // owns no bytes: its edges would only duplicate theirs (CRT-3).
        if placed.width < 0.0
            && matches!(p.kind, PieceKind::Glue { item }
                if self.glue(item).kind == GlueKind::CjkPunctCompression)
        {
            return;
        }
        // A glyph-less piece lends no level of its own (spec CRT-2).
        let level = match p.kind {
            PieceKind::Glue { .. } | PieceKind::SpanEdge { .. } => None,
            _ => Some(p.level),
        };
        let edge = |x: f32, affinity: CaretAffinity| RawStop {
            x,
            offset: match affinity {
                CaretAffinity::Downstream => p.bytes.start(),
                CaretAffinity::Upstream => p.bytes.end(),
            },
            affinity,
            level,
        };
        let (left, right) = if p.level.direction().is_rtl() {
            (CaretAffinity::Upstream, CaretAffinity::Downstream)
        } else {
            (CaretAffinity::Downstream, CaretAffinity::Upstream)
        };
        raw.push(edge(placed.x, left));
        if let PieceKind::Cluster { run, cluster } = p.kind
            && let c = &self.ast.clusters[cluster]
            && c.atom_count() > 1
            && !c.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE)
        {
            let direction = p.level.direction();
            let (glyphs, width) = self.cluster_glyphs(cluster);
            let first_glyph = glyphs.first().map_or(0, |g| g.glyph_id);
            let instance = self.ast.runs[run].font_instance(c.font);
            let carets = self
                .shaper
                .ligature_carets(instance, first_glyph, direction);
            let atoms = &self.inputs.run_atoms(run)[c.atom_range.clone()];
            let mut offsets: Vec<usize> = atoms[1..].iter().map(|a| a.byte_range.start()).collect();
            let rtl = direction.is_rtl();
            if rtl {
                offsets.reverse();
            }
            let origin = placed.x + if rtl { placed.expansion } else { 0.0 };
            let positions = resolve_ligature_carets(width, c.atom_count(), &carets);
            raw.extend(positions.iter().zip(offsets).map(|(&pos, offset)| RawStop {
                x: origin + pos,
                offset,
                affinity: CaretAffinity::Downstream,
                level,
            }));
        }
        raw.push(edge(placed.x + placed.width, right));
    }
}

/// Resolves the stops, in visual order, that share an offset (CRT-3). Those
/// all of one level, e.g. where a negative-width `CjkPunctCompression`
/// overlaps two clusters, merge into one at their midpoint. Only stops at two
/// positions, a direction boundary, keep their own affinity and level; the
/// others lean downstream, except the line's last logical stop at a soft
/// wrap, which is upstream (INT-1, INT-9).
fn resolve_shared_offsets(stops: &mut Vec<RawStop>, soft_wrap: bool) {
    stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    let mut by_offset: Vec<(usize, usize)> = stops
        .iter()
        .enumerate()
        .map(|(i, s)| (s.offset, i))
        .collect();
    by_offset.sort_unstable();
    let last_offset = by_offset.last().map(|&(offset, _)| offset);
    let mut merged = vec![false; stops.len()];
    for same in by_offset.chunk_by(|a, b| a.0 == b.0) {
        let (first, last) = (same[0].1, same[same.len() - 1].1);
        let (x, level) = (stops[first].x, stops[first].level);
        let merges = level.is_some()
            && same.iter().all(|&(_, i)| stops[i].level == level)
            && !approx_eq(x, stops[last].x);
        if merges {
            stops[first].x = (x + stops[last].x) / 2.0;
            same[1..].iter().for_each(|&(_, i)| merged[i] = true);
        } else if same.iter().any(|&(_, i)| !approx_eq(stops[i].x, x)) {
            continue;
        }
        for &(_, i) in same {
            stops[i].affinity = CaretAffinity::Downstream;
            stops[i].level = None;
        }
        if soft_wrap && Some(same[0].0) == last_offset {
            stops[if merges { first } else { last }].affinity = CaretAffinity::Upstream;
        }
    }
    let mut merged = merged.into_iter();
    stops.retain(|_| merged.next() == Some(false));
    stops.sort_by(|a, b| a.x.total_cmp(&b.x));
}
