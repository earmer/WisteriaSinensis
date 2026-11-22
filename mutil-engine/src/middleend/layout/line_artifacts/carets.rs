//! A line's caret stops (spec §6.1, CRT-1 to CRT-3, INT-4 to INT-6, INT-9).

use crate::frontend::GlueKind;
use crate::middleend::layout::line_builder::AlignedLine;
use crate::middleend::layout::{Layout, PieceKind, PlacedPiece, Reshapes};
use crate::middleend::tolerance::approx_eq_f64;
use crate::middleend::{
    CaretAffinity, CaretStop, HangingWhitespaceMode, PositionedInlineBox, VisualGlyphRun,
    resolve_ligature_carets,
};
use crate::types::{AtomFlags, BidiLevel, GlyphOrientation, TextRange};
use smallvec::SmallVec;

/// A candidate caret stop, with the affinity and level it takes at a
/// direction boundary (CRT-3): upstream at its piece's end, downstream at its
/// start, and the level of its entity if that renders.
#[derive(Clone, Copy)]
struct RawStop {
    x: f64,
    offset: usize,
    affinity: CaretAffinity,
    level: Option<BidiLevel>,
    /// Set by deduplication or shared-offset merging before `retain`.
    dropped: bool,
}

/// The working buffers of [`Layout::caret_stops`], kept across the lines
/// of a paragraph.
#[derive(Default)]
pub(in crate::middleend::layout) struct CaretScratch {
    raw: Vec<RawStop>,
    /// `(offset, x, index)` tuples sorted by offset for non-monotone dedup.
    by_offset: Vec<(usize, f64, usize)>,
    /// `(offset, index)` pairs sorted by offset.
    pairs: Vec<(usize, usize)>,
    entities: Vec<Entity>,
}

impl Layout<'_, '_> {
    /// The line's caret stops in visual order (spec §6.1, CRT-1 to CRT-3,
    /// INT-4 to INT-6, INT-9), given the end of its visible bytes, where its
    /// content starts, and its glyph runs and boxes.
    pub(in crate::middleend::layout) fn caret_stops(
        &self,
        line: &AlignedLine<'_, '_>,
        visible_end: usize,
        runs: &[VisualGlyphRun],
        boxes: &[PositionedInlineBox],
        pending: &Reshapes,
        scratch: &mut CaretScratch,
    ) -> Vec<CaretStop> {
        let CaretScratch {
            raw,
            by_offset,
            pairs,
            entities,
        } = scratch;
        let bytes = line.byte_range;
        // Under `Collapsible`, a soft wrap hides the trailing white space,
        // whose offsets belong to the next line (INT-6). Otherwise it all
        // sits beyond the far edge of the base direction (HNG-3), so the end
        // of a hard-broken line or of the paragraph stays reachable.
        let hides_trailing =
            line.soft_wrap() && self.options.whitespace == HangingWhitespaceMode::Collapsible;
        raw.clear();
        if bytes.is_empty() || line.body_len() == 0 && (line.placed.is_empty() || hides_trailing) {
            raw.push(RawStop {
                x: line.content.start,
                offset: bytes.start(),
                affinity: CaretAffinity::Downstream,
                level: None,
                dropped: false,
            });
        }
        let trailing: &[PlacedPiece<'_>] = if hides_trailing {
            &[]
        } else {
            line.placed.get(line.body_len()..).unwrap_or_default()
        };
        let (forward, backward): (&[PlacedPiece<'_>], &[PlacedPiece<'_>]) = match self.base.is_rtl()
        {
            false => (trailing, &[]),
            true => (&[], trailing),
        };
        let body = line.order.pick(line.placed);
        for &piece in body.chain(forward).chain(backward.iter().rev()) {
            self.push_piece_stops(piece, raw, pending);
        }
        // A hard break's own bytes start the next line.
        let drops_end = !line.soft_wrap()
            && !bytes.is_empty()
            && line
                .end_break
                .is_some_and(|b| b.is_mandatory() && !b.byte_range.is_empty());
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
        raw.retain(|s| belongs_here(s.offset));
        distinct_stops(raw, by_offset);
        resolve_shared_offsets(raw, pairs, line.soft_wrap());
        // The level of an entity touching `offset`, else the paragraph's.
        // When every run and box shares the base level, every stop takes it
        // directly without indexing entities.
        let base_level = self.base.base_level();
        if runs.iter().all(|r| r.bidi_level == base_level)
            && boxes.iter().all(|b| b.bidi_level == base_level)
        {
            return raw
                .iter()
                .map(|s| CaretStop {
                    x: s.x as f32,
                    offset: s.offset,
                    affinity: s.affinity,
                    level: s.level.unwrap_or(base_level),
                })
                .collect();
        }
        let mut levels = EntityLevels {
            sorted: entities,
            built: false,
        };
        let mut level_at = |offset: usize| levels.at(runs, boxes, offset).unwrap_or(base_level);
        raw.iter()
            .map(|s| CaretStop {
                x: s.x as f32,
                offset: s.offset,
                affinity: s.affinity,
                level: s.level.unwrap_or_else(|| level_at(s.offset)),
            })
            .collect()
    }

    /// A piece's caret stops, left to right: its two visual edges and, between
    /// them, those between a ligature's atoms (INT-5) from the font's caret
    /// positions, none for a single atom or an emoji sequence. Positions
    /// increase from the left edge (SHP-7); in RTL they pair with the atoms
    /// in reverse logical order, and the justification lies before the glyphs.
    fn push_piece_stops(
        &self,
        placed: PlacedPiece<'_>,
        raw: &mut Vec<RawStop>,
        pending: &Reshapes,
    ) {
        let p = placed.piece;
        // A compression pool squeezed below zero overlaps its neighbours and
        // owns no bytes: its edges would only duplicate theirs (CRT-3).
        if placed.width < 0.0
            && matches!(p.kind, PieceKind::Glue(glue)
                if glue.kind == GlueKind::CjkPunctCompression)
        {
            return;
        }
        // A glyph-less piece lends no level of its own (spec CRT-2).
        let level = match p.kind {
            PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => None,
            _ => Some(p.level),
        };
        let edge = |x: f64, affinity: CaretAffinity| RawStop {
            x,
            offset: match affinity {
                CaretAffinity::Downstream => p.bytes.start(),
                CaretAffinity::Upstream => p.bytes.end(),
            },
            affinity,
            level,
            dropped: false,
        };
        let (left, right) = if p.level.direction().is_rtl() {
            (CaretAffinity::Upstream, CaretAffinity::Downstream)
        } else {
            (CaretAffinity::Downstream, CaretAffinity::Upstream)
        };
        raw.push(edge(placed.x, left));
        if let PieceKind::Cluster { run, cluster } = p.kind
            && let c = self.inputs.cluster(cluster)
            && c.atom_count() > 1
            && !c.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE)
            // A tate-chu-yoko cluster merges several atoms but is no ligature.
            && c.orientation != GlyphOrientation::HorizontalInVertical
        {
            let direction = p.level.direction();
            let (glyphs, width) = self.cluster_glyphs(pending, cluster);
            // A cluster with no glyph has no ligature to ask GDEF about; it
            // takes the equal slots (INT-5).
            let carets = glyphs.first().map_or_else(SmallVec::new, |glyph| {
                let instance = self.inputs.run(run).font_instance(c.font);
                self.shaper
                    .ligature_carets(instance, glyph.glyph_id, direction)
            });
            let atoms = self.inputs.cluster_atoms(run, cluster);
            let mut offsets: Vec<usize> =
                atoms.iter().skip(1).map(|a| a.byte_range.start()).collect();
            let rtl = direction.is_rtl();
            if rtl {
                offsets.reverse();
            }
            let origin = placed.x + if rtl { placed.expansion } else { 0.0 };
            let positions = resolve_ligature_carets(width as f32, c.atom_count(), &carets);
            raw.extend(positions.iter().zip(offsets).map(|(&pos, offset)| RawStop {
                x: origin + f64::from(pos),
                offset,
                affinity: CaretAffinity::Downstream,
                level,
                dropped: false,
            }));
        }
        raw.push(edge(placed.x + placed.width, right));
    }
}

/// One stop per distinct `(x, offset)` (INT-20), the first emitted of each,
/// in emission order. Two pieces that meet at an offset need not be emitted
/// next to each other: an RTL line's trailing white space is emitted after
/// its whole body but sits at its visual left, beside the body's last
/// logical edge. So every stop is compared with all earlier stops of its
/// offset, not with its recent neighbours.
///
/// `by_offset` is a working buffer.
fn distinct_stops(raw: &mut Vec<RawStop>, by_offset: &mut Vec<(usize, f64, usize)>) {
    // Fast path: when offsets are already monotone (pure LTR or pure RTL),
    // equal offsets are contiguous and in emission order, so deduplication is
    // one linear in-place pass with no sort.
    let monotone =
        raw.is_sorted_by_key(|s| s.offset) || raw.is_sorted_by(|a, b| a.offset >= b.offset);
    if monotone {
        let mut current_offset: Option<usize> = None;
        let mut kept_xs: SmallVec<[f64; 4]> = SmallVec::new();
        raw.retain(|stop| {
            if current_offset != Some(stop.offset) {
                current_offset = Some(stop.offset);
                kept_xs.clear();
            }
            if kept_xs.iter().any(|&x| approx_eq_f64(x, stop.x)) {
                return false;
            }
            kept_xs.push(stop.x);
            true
        });
        return;
    }
    by_offset.clear();
    by_offset.extend(raw.iter().enumerate().map(|(i, s)| (s.offset, s.x, i)));
    // Stable: emission order within an offset.
    by_offset.sort_by_key(|&(offset, ..)| offset);
    let mut kept_xs: SmallVec<[f64; 4]> = SmallVec::new();
    for same in by_offset.chunk_by(|a, b| a.0 == b.0) {
        kept_xs.clear();
        for &(_, x, i) in same {
            if kept_xs.iter().any(|&kept| approx_eq_f64(kept, x)) {
                if let Some(stop) = raw.get_mut(i) {
                    stop.dropped = true;
                }
            } else {
                kept_xs.push(x);
            }
        }
    }
    raw.retain(|s| !s.dropped);
}

/// Fast path for [`resolve_shared_offsets`] when stops are already ordered by
/// both `x` and `offset`: equal offsets are contiguous so no sort is needed.
fn resolve_ordered_shared_offsets(stops: &mut Vec<RawStop>, soft_wrap: bool) {
    let last_offset = stops.last().map(|s| s.offset);
    let mut any_merged = false;
    for group in stops.chunk_by_mut(|a, b| a.offset == b.offset) {
        let Some((first, rest)) = group.split_first_mut() else {
            continue;
        };
        let offset = first.offset;
        let x = first.x;
        let level = first.level;
        let last_x = rest.last().map_or(x, |s| s.x);
        let merges =
            level.is_some() && rest.iter().all(|s| s.level == level) && !approx_eq_f64(x, last_x);
        if merges {
            first.x = (x + last_x) / 2.0;
            first.affinity = if soft_wrap && Some(offset) == last_offset {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            };
            first.level = None;
            for dropped in rest {
                dropped.dropped = true;
            }
            any_merged = true;
        } else if !rest.iter().any(|s| !approx_eq_f64(s.x, x)) {
            first.affinity = CaretAffinity::Downstream;
            first.level = None;
            for s in rest.iter_mut() {
                s.affinity = CaretAffinity::Downstream;
                s.level = None;
            }
            if soft_wrap && Some(offset) == last_offset {
                rest.last_mut().unwrap_or(first).affinity = CaretAffinity::Upstream;
            }
        }
    }
    if any_merged {
        stops.retain(|s| !s.dropped);
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
}

/// Resolves the stops, in visual order, that share an offset (CRT-3). Those
/// all of one level, e.g. where a negative-width `CjkPunctCompression`
/// overlaps two clusters, merge into one at their midpoint. Only stops at two
/// positions, a direction boundary, keep their own affinity and level; the
/// others lean downstream, except the line's last logical stop at a soft
/// wrap, which is upstream (INT-1, INT-9). `by_offset` is a working buffer.
fn resolve_shared_offsets(
    stops: &mut Vec<RawStop>,
    by_offset: &mut Vec<(usize, usize)>,
    soft_wrap: bool,
) {
    if stops.is_sorted_by(|a, b| a.x.total_cmp(&b.x).is_le() && a.offset <= b.offset) {
        resolve_ordered_shared_offsets(stops, soft_wrap);
        return;
    }
    stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    by_offset.clear();
    by_offset.extend(stops.iter().enumerate().map(|(i, s)| (s.offset, i)));
    by_offset.sort_unstable();
    let last_offset = by_offset.last().map(|&(offset, _)| offset);
    let mut any_merged = false;
    for same in by_offset.chunk_by(|a, b| a.0 == b.0) {
        let (Some(&(offset, first)), Some(&(_, last))) = (same.first(), same.last()) else {
            continue;
        };
        let (Some(&first_stop), Some(&last_stop)) = (stops.get(first), stops.get(last)) else {
            continue;
        };
        let (x, level) = (first_stop.x, first_stop.level);
        let merges = level.is_some()
            && same
                .iter()
                .all(|&(_, i)| stops.get(i).is_some_and(|s| s.level == level))
            && !approx_eq_f64(x, last_stop.x);
        if merges {
            if let Some(s) = stops.get_mut(first) {
                s.x = (x + last_stop.x) / 2.0;
            }
            for &(_, i) in same.iter().skip(1) {
                if let Some(s) = stops.get_mut(i) {
                    s.dropped = true;
                }
            }
            any_merged = true;
        } else if same
            .iter()
            .any(|&(_, i)| stops.get(i).is_some_and(|s| !approx_eq_f64(s.x, x)))
        {
            continue;
        }
        for &(_, i) in same {
            if let Some(s) = stops.get_mut(i) {
                s.affinity = CaretAffinity::Downstream;
                s.level = None;
            }
        }
        if soft_wrap
            && Some(offset) == last_offset
            && let Some(s) = stops.get_mut(if merges { first } else { last })
        {
            s.affinity = CaretAffinity::Upstream;
        }
    }
    if any_merged {
        stops.retain(|s| !s.dropped);
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
}

/// One glyph cluster or box of a line, as [`EntityLevels`] indexes it.
struct Entity {
    range: TextRange,
    /// Position in visual order (glyphs of all runs, then boxes).
    rank: usize,
    level: BidiLevel,
    /// The greatest end of this entity and all those sorted before it.
    reach: usize,
}

/// The levels of the entities of a line, by byte range, so that "the first
/// entity in visual order whose closed range holds `offset`" is a binary
/// search, not a scan of every glyph. Consecutive glyphs of one cluster share
/// a range and a level, so only the first is indexed: a multi-glyph cluster
/// costs one entry, not one per glyph. The index lives in the paragraph's
/// scratch and is built for a line when its first stop needs it.
struct EntityLevels<'s> {
    /// Sorted by range start, then rank.
    sorted: &'s mut Vec<Entity>,
    built: bool,
}

impl EntityLevels<'_> {
    /// Indexes the glyphs of `runs` and then `boxes`.
    fn build(&mut self, runs: &[VisualGlyphRun], boxes: &[PositionedInlineBox]) {
        let glyphs = runs.iter().flat_map(|r| {
            r.glyphs
                .iter()
                .filter_map(|g| Some((g.byte_range?, r.bidi_level)))
        });
        let mut previous = None;
        let sorted = &mut *self.sorted;
        sorted.clear();
        sorted.extend(
            glyphs
                .chain(boxes.iter().map(|b| (b.byte_range, b.bidi_level)))
                .enumerate()
                .filter(|&(_, entity)| previous.replace(entity) != Some(entity))
                .map(|(rank, (range, level))| Entity {
                    range,
                    rank,
                    level,
                    reach: 0,
                }),
        );
        sorted.sort_unstable_by_key(|e| (e.range.start(), e.rank));
        let mut reach = 0;
        for entity in sorted.iter_mut() {
            reach = reach.max(entity.range.end());
            entity.reach = reach;
        }
        self.built = true;
    }

    /// The level of the first entity in visual order whose closed range
    /// holds `offset`, building the index on first use.
    fn at(
        &mut self,
        runs: &[VisualGlyphRun],
        boxes: &[PositionedInlineBox],
        offset: usize,
    ) -> Option<BidiLevel> {
        if !self.built {
            self.build(runs, boxes);
        }
        let upto = self.sorted.partition_point(|e| e.range.start() <= offset);
        self.sorted
            .iter()
            .take(upto)
            .rev()
            .take_while(|e| e.reach >= offset)
            .filter(|e| e.range.end() >= offset)
            .min_by_key(|e| e.rank)
            .map(|e| e.level)
    }
}
