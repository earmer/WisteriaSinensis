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

#[derive(Clone, Copy)]
struct RawStop {
    x: f64,
    offset: usize,
    affinity: CaretAffinity,
    level: Option<BidiLevel>,
    dropped: bool,
}

#[derive(Default)]
pub(in crate::middleend::layout) struct CaretScratch {
    raw: Vec<RawStop>,
    by_offset: Vec<(usize, f64, usize)>,
    pairs: Vec<(usize, usize)>,
    entities: Vec<Entity>,
}

impl Layout<'_, '_> {
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

    fn push_piece_stops(
        &self,
        placed: PlacedPiece<'_>,
        raw: &mut Vec<RawStop>,
        pending: &Reshapes,
    ) {
        let p = placed.piece;
        if placed.width < 0.0
            && matches!(p.kind, PieceKind::Glue(glue)
                if glue.kind == GlueKind::CjkPunctCompression)
        {
            return;
        }
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
            && c.orientation != GlyphOrientation::HorizontalInVertical
        {
            let direction = p.level.direction();
            let (glyphs, width) = self.cluster_glyphs(pending, cluster);
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

fn distinct_stops(raw: &mut Vec<RawStop>, by_offset: &mut Vec<(usize, f64, usize)>) {
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

struct Entity {
    range: TextRange,
    rank: usize,
    level: BidiLevel,
    reach: usize,
}

struct EntityLevels<'s> {
    sorted: &'s mut Vec<Entity>,
    built: bool,
}

impl EntityLevels<'_> {
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
