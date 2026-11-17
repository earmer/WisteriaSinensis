//! Line-breaking and measurement helpers for [`Layout`] (spec §5 Stage 1).

use super::super::shaper_gate::{checked_font_metrics, checked_reshape_window, resolved_line_box};
use super::super::{InlineSlotConstraint, LayoutInputError, StrutMetrics};
use super::hanging::LineEdge;
use super::{FIT_EPSILON, Layout, LineEnd, Piece, PieceKind, Reshaped, Token};
use crate::frontend::{BoxNode, BoxOverflowPolicy, DiscretionaryNode, WordNode};
use crate::types::{AtomFlags, FontInstanceRef, FontMetrics, RawShapedGlyph};
use smallvec::SmallVec;

/// The tier of a discretionary on the overflow ladder (spec ST1-1): its
/// penalty net of cloned-span penalties (WN-21) in the URL band (WN-19)
/// below its maximum is tier 1, at the maximum tier 2; any other is tier 0.
pub(super) fn ladder_tier(d: &DiscretionaryNode) -> usize {
    if d.is_mandatory() {
        return 0;
    }
    let clones = i32::try_from(d.cloned_spans.len()).unwrap_or(i32::MAX);
    let net = d
        .penalty
        .saturating_sub(clones.saturating_mul(DiscretionaryNode::CLONED_SPAN_PENALTY));
    match net {
        DiscretionaryNode::URL_PENALTY_MAX => 2,
        n if (DiscretionaryNode::URL_PENALTY_MIN..DiscretionaryNode::URL_PENALTY_MAX)
            .contains(&n) =>
        {
            1
        }
        _ => 0,
    }
}

/// Where a line starts: its first token, the cloned padding before it and
/// whether it is the paragraph's first line.
#[derive(Clone, Copy)]
pub(super) struct LineStart {
    pub(super) token: usize,
    pub(super) post: f32,
    pub(super) first_line: bool,
}

impl<'i, 'a> Layout<'i, 'a> {
    /// The next tab stop after `distance`, measured from the slot's
    /// line-start edge along the base direction, if it lies inside `slot`
    /// (spec ST3-1): a stop further than the slot's width is not reachable.
    pub(super) fn reachable_stop(&self, distance: f32, slot: InlineSlotConstraint) -> Option<f32> {
        let width = slot.width();
        self.inputs
            .constraints()
            .tab_stops
            .next_stop(distance)
            .filter(|&stop| stop <= width + super::super::tolerance::tolerance(stop, width))
    }

    /// The advance and the reach (furthest pen end, relative to the start)
    /// of a piece whose start lies `pen` from the slot's line-start edge.
    pub(super) fn measure(
        &self,
        piece: &Piece,
        pen: f32,
        slot: InlineSlotConstraint,
    ) -> (f32, f32) {
        match piece.kind {
            PieceKind::Tab => {
                // A tab with no reachable stop takes no advance (ST3-1, AX-6).
                let width = self
                    .reachable_stop(pen, slot)
                    .map_or(0.0, |stop| stop - pen);
                (width, width)
            }
            PieceKind::Cluster { cluster, .. } => {
                let (glyphs, width) = self.cluster_glyphs(cluster);
                let mut x = 0.0_f32;
                let mut reach = width;
                for g in glyphs {
                    x += g.x_advance;
                    reach = reach.max(x);
                }
                (width, reach)
            }
            PieceKind::Object { item } => {
                let (width, _, _) = self.fitted_object(item, slot);
                (width, width)
            }
            _ => (piece.width, piece.width),
        }
    }

    /// An object's width, ascent and descent under its overflow policy in
    /// `slot` (spec WN-7): `Clip` clamps to the reference width,
    /// `ScaleToFit` scales uniformly to it, `AllowOverflow` keeps its size.
    pub(super) fn fitted_object(&self, item: usize, slot: InlineSlotConstraint) -> (f32, f32, f32) {
        let WordNode::Box(BoxNode::InlineObject {
            width,
            ascent,
            descent,
            overflow_policy,
            ..
        }) = &self.ast.items[item]
        else {
            unreachable!("an object piece names an object item")
        };
        let (width, ascent, descent) = (*width, *ascent, *descent);
        let reference = slot.width();
        match overflow_policy {
            BoxOverflowPolicy::AllowOverflow => (width, ascent, descent),
            _ if width <= reference => (width, ascent, descent),
            BoxOverflowPolicy::Clip => (reference, ascent, descent),
            BoxOverflowPolicy::ScaleToFit => {
                let s = reference / width;
                (reference, ascent * s, descent * s)
            }
        }
    }

    /// The glyphs and width of a cluster, after any emergency re-shape.
    pub(super) fn cluster_glyphs(&self, cluster: usize) -> (&[RawShapedGlyph], f32) {
        if let Some(r) = self.reshaped.get(&cluster) {
            return (&r.glyphs, r.width);
        }
        let c = &self.ast.clusters[cluster];
        (self.inputs.cluster_glyphs(c), c.natural_width)
    }

    /// Greedy fitting (spec ST1-1, ST1-5, ST1-7, WN-1): the last break of
    /// the lowest ladder tier that fits before the content overflows, a
    /// mandatory break, or an emergency cluster break when nothing fits.
    /// Under `justify` a line fits while its width less the shrink of its
    /// visible glue after the last tab fits; a mark that may hang is credited
    /// with its protrusion (ST1-3).
    pub(super) fn fit(&mut self, line: LineStart, slot: InlineSlotConstraint) -> LineEnd {
        let slot_width = slot.width();
        let LineStart {
            token: start,
            post,
            first_line,
        } = line;
        let mut pen = post;
        let mut body = post;
        let mut has_content = false;
        // Whether a piece other than a zero-width span boundary has been
        // seen: the start mark is the first such piece (ST1-3).
        let mut has_visible = false;
        // The shrink of the glue since the last tab, and of the visible part.
        let (mut shrink, mut visible_shrink) = (0.0_f32, 0.0_f32);
        // The protrusion of the last visible piece, if it may hang.
        let mut end_hang = 0.0;
        // The last fitting break of each ladder tier.
        let mut fits: [Option<usize>; 3] = [None; 3];
        let best = |fits: &[Option<usize>; 3]| fits.iter().find_map(|&b| b);
        let mut t = start;
        loop {
            match &self.tokens[t] {
                Token::Break(b) => {
                    let hang = if b.pre == 0.0 { end_hang } else { 0.0 };
                    if b.mandatory {
                        // A line ending here is compressed only under
                        // `compress_last_lines` (ST3-3): otherwise, if only
                        // shrink would fit it, it ends at an earlier break.
                        let shrink = if self.options.compress_last_lines {
                            visible_shrink
                        } else {
                            0.0
                        };
                        let used = body + b.pre - hang - shrink;
                        let overflows = used
                            > slot_width + super::super::tolerance::tolerance(used, slot_width);
                        return match best(&fits).filter(|_| overflows) {
                            Some(earlier) => LineEnd::At(earlier),
                            None => LineEnd::At(t),
                        };
                    }
                    let used = body + b.pre - hang - visible_shrink;
                    if used <= slot_width + super::super::tolerance::tolerance(used, slot_width) {
                        fits[b.tier] = Some(t);
                    }
                    pen += b.no_break;
                }
                Token::Piece(piece)
                    if matches!(piece.kind, PieceKind::Tab)
                        && self.reachable_stop(pen, slot).is_none() =>
                {
                    // No reachable stop (spec ST3-1, AX-6): break before the
                    // tab at an earlier opportunity, else consume it with no
                    // advance and end the line right after it.
                    if has_content && let Some(b) = best(&fits) {
                        return LineEnd::At(b);
                    }
                    return match self.tokens.get(t + 1) {
                        Some(Token::Break(_)) => LineEnd::At(t + 1),
                        _ => LineEnd::Before(t + 1),
                    };
                }
                Token::Piece(piece) => {
                    let piece = *piece;
                    match piece.kind {
                        PieceKind::Glue { item } if self.options.justify => {
                            shrink += self.glue(item).max_shrink;
                        }
                        PieceKind::Tab => shrink = 0.0,
                        _ => {}
                    }
                    if piece.is_content() {
                        if !has_visible && post == 0.0 {
                            pen -= self
                                .hang_protrusion(&piece, piece.level, LineEdge::Start, first_line)
                                .unwrap_or(0.0);
                        }
                        has_visible = true;
                        visible_shrink = shrink;
                        end_hang = self
                            .hang_protrusion(&piece, piece.level, LineEdge::End, first_line)
                            .unwrap_or(0.0);
                    }
                    let (advance, reach) = self.measure(&piece, pen, slot);
                    if piece.is_content() {
                        let end = pen + reach.max(advance);
                        let used = end - end_hang - visible_shrink;
                        if used > slot_width + super::super::tolerance::tolerance(used, slot_width)
                        {
                            return match best(&fits) {
                                Some(b) => LineEnd::At(b),
                                None => self.emergency(start, t, post, slot),
                            };
                        }
                        body = end;
                        has_content = true;
                    }
                    pen += advance;
                }
            }
            t += 1;
        }
    }

    /// The break token right after `t` and its trailing glue, if any.
    fn next_break_after(&self, t: usize) -> Option<usize> {
        let mut e = t + 1;
        while let Some(Token::Piece(p)) = self.tokens.get(e) {
            if !matches!(p.kind, PieceKind::Glue { .. }) {
                return None;
            }
            e += 1;
        }
        matches!(self.tokens.get(e), Some(Token::Break(_))).then_some(e)
    }

    /// The emergency break (spec ST1-5) of a line from `start` whose piece
    /// `t` overflows with no fitting opportunity: before `t` when something
    /// visible precedes it, else after `t` alone (one indivisible entity on
    /// its own overflowing line, SLT-5), together with the break right after
    /// it if only glue separates them.
    fn emergency(
        &mut self,
        start: usize,
        t: usize,
        post: f32,
        slot: InlineSlotConstraint,
    ) -> LineEnd {
        let first_content =
            (start..t).find(|&k| matches!(&self.tokens[k], Token::Piece(p) if p.is_content()));
        if let Some(first) = first_content {
            // Re-shaping an unsafe seam can widen its sides (kerning across
            // it is lost); while the line then overflows, the seam moves one
            // piece back and the abandoned re-shape is dropped.
            let available = slot.width();
            let mut t = t;
            loop {
                // A dash, ellipsis or middle-dot run (FE-15) moves whole to
                // the next line when something precedes it on this one. A
                // run that starts the line and is still wider than the slot
                // is split like any other unit (ST1-5): only one cluster may
                // overflow a slot (SLT-5).
                let run_start = self.dash_run_start(t);
                if run_start > first {
                    t = run_start;
                }
                let added = self.repair_seam(t);
                if t <= first + 1
                    || self.content_end(start, t, post, slot) <= available + FIT_EPSILON
                {
                    return LineEnd::Before(t);
                }
                for c in added {
                    self.reshaped.remove(&c);
                }
                t -= 1;
                while t > first + 1 && matches!(self.tokens[t], Token::Break(_)) {
                    t -= 1;
                }
            }
        }
        if let Some(b) = self.next_break_after(t) {
            return LineEnd::At(b);
        }
        self.repair_seam(t + 1);
        LineEnd::Before(t + 1)
    }

    /// Whether the seam before token `t` lies inside a binding punctuation run
    /// such as consecutive U+2014, U+2026 or U+00B7 (spec FE-15).
    fn inside_dash_run(&self, mut t: usize) -> bool {
        while let Some(Token::Piece(p)) = self.tokens.get(t) {
            match p.kind {
                PieceKind::SpanEdge { .. } => t += 1,
                PieceKind::Cluster { cluster, .. } => {
                    return self.ast.clusters[cluster]
                        .flags
                        .contains(AtomFlags::BINDS_TO_PREVIOUS);
                }
                _ => return false,
            }
        }
        false
    }

    /// The first token of the run whose seam `t` lies inside, or `t`.
    fn dash_run_start(&self, mut t: usize) -> usize {
        while t > 0 && self.inside_dash_run(t) {
            t -= 1;
        }
        t
    }

    /// An emergency break before piece `t` that is not safe to break
    /// re-shapes the clusters on both sides of the seam (spec ST1-5), with no
    /// edit. A re-shape that does not return exactly one cluster per side is
    /// not used.
    fn repair_seam(&mut self, t: usize) -> SmallVec<[usize; 2]> {
        let mut added = SmallVec::new();
        let Some(Token::Piece(Piece {
            kind: PieceKind::Cluster { run, cluster },
            ..
        })) = self.tokens.get(t)
        else {
            return added;
        };
        let (run, after) = (*run, *cluster);
        if self.ast.clusters[after].safe_to_break_before {
            return added;
        }
        let run_clusters = &self.ast.runs[run].cluster_range;
        let before = (after > run_clusters.start).then(|| after - 1);
        let atoms = self.inputs.run_atoms(run);
        let text = self.inputs.text();
        for c in std::iter::once(after).chain(before) {
            let cluster = &self.ast.clusters[c];
            if let Some((glyphs, width)) = checked_reshape_window(
                self.shaper,
                text,
                &self.ast.runs[run],
                atoms,
                cluster.byte_range,
                cluster.atom_range.clone(),
            ) && self
                .reshaped
                .insert(c, Reshaped { glyphs, width })
                .is_none()
            {
                added.push(c);
            }
        }
        added
    }

    /// Where the content of a line from `start` ends when it stops before
    /// token `end`, measured as [`fit`](Self::fit) measures it.
    fn content_end(&self, start: usize, end: usize, post: f32, slot: InlineSlotConstraint) -> f32 {
        let mut pen = post;
        let mut body = post;
        for token in &self.tokens[start..end] {
            match token {
                Token::Break(b) => pen += b.no_break,
                Token::Piece(piece) => {
                    let (advance, reach) = self.measure(piece, pen, slot);
                    if piece.is_content() {
                        body = pen + reach.max(advance);
                    }
                    pen += advance;
                }
            }
        }
        body
    }

    /// The line box of a cluster's font instance (spec MET-1, MET-2), through
    /// the shaper gate.
    pub(super) fn cluster_box(
        &mut self,
        run: usize,
        cluster: usize,
    ) -> Result<StrutMetrics, LayoutInputError> {
        let metrics = self.cluster_metrics(run, cluster)?;
        resolved_line_box(
            &metrics,
            self.ast.runs[run].font_size,
            self.options.line_height,
            self.ast.writing_mode,
        )
    }

    /// A cluster's font metrics, through the shaper gate (spec G2).
    pub(super) fn cluster_metrics(
        &mut self,
        run: usize,
        cluster: usize,
    ) -> Result<FontMetrics, LayoutInputError> {
        let font = self.ast.clusters[cluster].font;
        let key = (font.font_id.0, font.scale.to_bits(), run);
        if let Some(found) = self.metrics.get(&key) {
            return Ok(*found);
        }
        let r = &self.ast.runs[run];
        let metrics = checked_font_metrics(
            self.shaper,
            FontInstanceRef {
                font,
                font_size: r.font_size,
                variations: r.features.variations(),
            },
        )?;
        self.metrics.insert(key, metrics);
        Ok(metrics)
    }
}
