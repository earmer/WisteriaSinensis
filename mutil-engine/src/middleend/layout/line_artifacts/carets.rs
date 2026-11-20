//! A line's caret stops (spec §6.1, CRT-1 to CRT-3, INT-4 to INT-6, INT-9).

use crate::frontend::GlueKind;
use crate::middleend::layout::line_builder::AlignedLine;
use crate::middleend::layout::{Layout, PieceKind, PlacedPiece, Reshapes};
use crate::middleend::tolerance::approx_eq;
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
    x: f32,
    offset: usize,
    affinity: CaretAffinity,
    level: Option<BidiLevel>,
}

/// The working buffers of [`Layout::caret_stops`], kept across the lines
/// of a paragraph.
#[derive(Default)]
pub(in crate::middleend::layout) struct CaretScratch {
    raw: Vec<RawStop>,
    /// Stop indices sorted by offset.
    by_offset: Vec<usize>,
    /// `(offset, index)` pairs sorted by offset.
    pairs: Vec<(usize, usize)>,
    /// One mark per stop: a duplicate, or merged away.
    marks: Vec<bool>,
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
            marks,
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
            });
        }
        let trailing = if hides_trailing {
            &[][..]
        } else {
            &line.placed[line.body_len()..]
        };
        let (forward, backward) = match self.base.is_rtl() {
            false => (trailing, &[][..]),
            true => (&[][..], trailing),
        };
        let body = line.order.iter().map(|&k| &line.placed[k]);
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
        distinct_stops(raw, by_offset, marks);
        resolve_shared_offsets(raw, pairs, marks, line.soft_wrap());
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
                    x: s.x,
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
                x: s.x,
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
            // A tate-chu-yoko cluster merges several atoms but is no ligature.
            && c.orientation != GlyphOrientation::HorizontalInVertical
        {
            let direction = p.level.direction();
            let (glyphs, width) = self.cluster_glyphs(pending, cluster);
            // A cluster with no glyph has no ligature to ask GDEF about; it
            // takes the equal slots (INT-5).
            let carets = glyphs.first().map_or_else(SmallVec::new, |glyph| {
                let instance = self.ast.runs[run].font_instance(c.font);
                self.shaper
                    .ligature_carets(instance, glyph.glyph_id, direction)
            });
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

/// One stop per distinct `(x, offset)` (INT-20), the first emitted of each,
/// in emission order. Two pieces that meet at an offset need not be emitted
/// next to each other: an RTL line's trailing white space is emitted after
/// its whole body but sits at its visual left, beside the body's last
/// logical edge. So every stop is compared with all earlier stops of its
/// offset, not with its recent neighbours.
///
/// `by_offset` and `duplicate` are working buffers.
fn distinct_stops(raw: &mut Vec<RawStop>, by_offset: &mut Vec<usize>, duplicate: &mut Vec<bool>) {
    // Fast path: when offsets are already monotone (pure LTR or pure RTL),
    // equal offsets are contiguous and in emission order, so deduplication is
    // one linear in-place pass with no sort.
    let monotone = raw.windows(2).all(|w| w[0].offset <= w[1].offset)
        || raw.windows(2).all(|w| w[0].offset >= w[1].offset);
    if monotone {
        let mut write = 0;
        let mut group_start = 0;
        for read in 0..raw.len() {
            let stop = raw[read];
            if write > group_start && raw[group_start].offset != stop.offset {
                group_start = write;
            }
            if !raw[group_start..write]
                .iter()
                .any(|kept| approx_eq(kept.x, stop.x))
            {
                raw[write] = stop;
                write += 1;
            }
        }
        raw.truncate(write);
        return;
    }
    by_offset.clear();
    by_offset.extend(0..raw.len());
    // Stable: emission order within an offset.
    by_offset.sort_by_key(|&i| raw[i].offset);
    duplicate.clear();
    duplicate.resize(raw.len(), false);
    for same in by_offset.chunk_by(|&a, &b| raw[a].offset == raw[b].offset) {
        for (n, &i) in same.iter().enumerate() {
            duplicate[i] = same[..n]
                .iter()
                .any(|&j| !duplicate[j] && approx_eq(raw[j].x, raw[i].x));
        }
    }
    let mut duplicate = duplicate.iter();
    raw.retain(|_| duplicate.next() == Some(&false));
}

/// Fast path for [`resolve_shared_offsets`] when stops are already ordered by
/// both `x` and `offset`: equal offsets are contiguous so no sort is needed.
fn resolve_ordered_shared_offsets(stops: &mut Vec<RawStop>, soft_wrap: bool) {
    let last_offset = stops.last().map(|s| s.offset);
    let mut any_merged = false;
    let mut write = 0;
    let mut start = 0;
    while start < stops.len() {
        let offset = stops[start].offset;
        let mut end = start + 1;
        while end < stops.len() && stops[end].offset == offset {
            end += 1;
        }
        let x = stops[start].x;
        let level = stops[start].level;
        let last_x = stops[end - 1].x;
        let merges = level.is_some()
            && stops[start..end].iter().all(|s| s.level == level)
            && !approx_eq(x, last_x);
        if merges {
            let mut kept = stops[start];
            kept.x = (x + last_x) / 2.0;
            kept.affinity = if soft_wrap && Some(offset) == last_offset {
                CaretAffinity::Upstream
            } else {
                CaretAffinity::Downstream
            };
            kept.level = None;
            stops[write] = kept;
            write += 1;
            any_merged = true;
        } else {
            let distinct_x = stops[start..end].iter().any(|s| !approx_eq(s.x, x));
            for i in start..end {
                let mut stop = stops[i];
                if !distinct_x {
                    stop.affinity = CaretAffinity::Downstream;
                    stop.level = None;
                    if soft_wrap && Some(offset) == last_offset && i + 1 == end {
                        stop.affinity = CaretAffinity::Upstream;
                    }
                }
                stops[write] = stop;
                write += 1;
            }
        }
        start = end;
    }
    if any_merged {
        stops.truncate(write);
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    }
}

/// Resolves the stops, in visual order, that share an offset (CRT-3). Those
/// all of one level, e.g. where a negative-width `CjkPunctCompression`
/// overlaps two clusters, merge into one at their midpoint. Only stops at two
/// positions, a direction boundary, keep their own affinity and level; the
/// others lean downstream, except the line's last logical stop at a soft
/// wrap, which is upstream (INT-1, INT-9). `by_offset` and `merged` are
/// working buffers.
fn resolve_shared_offsets(
    stops: &mut Vec<RawStop>,
    by_offset: &mut Vec<(usize, usize)>,
    merged: &mut Vec<bool>,
    soft_wrap: bool,
) {
    if stops
        .windows(2)
        .all(|w| w[0].x.total_cmp(&w[1].x).is_le() && w[0].offset <= w[1].offset)
    {
        resolve_ordered_shared_offsets(stops, soft_wrap);
        return;
    }
    stops.sort_by(|a, b| a.x.total_cmp(&b.x));
    by_offset.clear();
    by_offset.extend(stops.iter().enumerate().map(|(i, s)| (s.offset, i)));
    by_offset.sort_unstable();
    let last_offset = by_offset.last().map(|&(offset, _)| offset);
    merged.clear();
    merged.resize(stops.len(), false);
    let mut any_merged = false;
    for same in by_offset.chunk_by(|a, b| a.0 == b.0) {
        let (first, last) = (same[0].1, same[same.len() - 1].1);
        let (x, level) = (stops[first].x, stops[first].level);
        let merges = level.is_some()
            && same.iter().all(|&(_, i)| stops[i].level == level)
            && !approx_eq(x, stops[last].x);
        if merges {
            stops[first].x = (x + stops[last].x) / 2.0;
            same[1..].iter().for_each(|&(_, i)| merged[i] = true);
            any_merged = true;
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
    if any_merged {
        let mut merged = merged.iter();
        stops.retain(|_| merged.next() == Some(&false));
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
        self.sorted[..upto]
            .iter()
            .rev()
            .take_while(|e| e.reach >= offset)
            .filter(|e| e.range.end() >= offset)
            .min_by_key(|e| e.rank)
            .map(|e| e.level)
    }
}
