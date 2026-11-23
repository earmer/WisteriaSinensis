//! Line breaking and measurement for [`Layout`] (spec §5 Stage 1).

use super::hanging::LineEdge;
use super::justify::CJK_GAP_STRETCH;
use super::trim::{TrimInfo, is_trim_transparent, line_start_lead_trim, pair_trim};
use super::{Layout, LineCursor, LineEnd, Piece, PieceKind, Reshaped, Reshapes, Token};
use crate::frontend::{BoxOverflowPolicy, DiscretionaryNode, GlueKind, InlineObjectBox};
use crate::middleend::badness::{self, TierCapacities};
use crate::middleend::shaper_gate::{checked_font_metrics, checked_reshape_window};
use crate::middleend::tolerance::tolerance_f64;
use crate::middleend::validated::{ClusterId, RunId, TokenIdx};
use crate::middleend::{InlineSlotConstraint, LayoutInputError};
use crate::shaping::ReshapeRequest;
use crate::types::{AtomFlags, FontMetrics, RawShapedGlyph, TextRange};
use smallvec::SmallVec;

pub(super) fn ladder_tier(d: &DiscretionaryNode) -> usize {
    if d.is_mandatory() {
        return 0;
    }
    let net = d.penalty - (d.cloned_spans.len() as i32) * DiscretionaryNode::CLONED_SPAN_PENALTY;
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

#[derive(Clone, Copy, Default)]
struct LineWalk {
    pen: f64,
    body: f64,
    has_start: bool,
    has_content: bool,
    shrink: f64,
    visible_shrink: f64,
    end_hang: f64,
    prev_trim: Option<(TrimInfo, f64)>,
    pending_punct_glue_shrink: f64,
    stretch: [f64; 3],
    shrink_capacity: [f64; 2],
    visible_stretch: [f64; 3],
    visible_shrink_cap: [f64; 2],
    pending_cjk_gap: f64,
    content_clusters: usize,
}

impl LineWalk {
    fn new(post: f64) -> Self {
        Self {
            pen: post,
            body: post,
            ..Self::default()
        }
    }

    fn capacities(&self) -> TierCapacities {
        TierCapacities {
            stretch: self.visible_stretch.map(|v| v as f32),
            shrink: self.visible_shrink_cap.map(|v| v as f32),
        }
    }

    fn reset_capacities(&mut self) {
        self.stretch = [0.0; 3];
        self.shrink_capacity = [0.0; 2];
        self.visible_stretch = [0.0; 3];
        self.visible_shrink_cap = [0.0; 2];
        self.pending_cjk_gap = 0.0;
    }

    fn used(&self) -> f64 {
        self.body - self.end_hang - self.visible_shrink
    }

    fn advance_break(&mut self, no_break_width: f64) {
        self.pen += no_break_width;
        if no_break_width != 0.0 {
            self.prev_trim = None;
            self.pending_cjk_gap = 0.0;
        }
    }

    fn used_at_break(&self, pre: f64, shrinks: bool) -> f64 {
        let hang = if pre == 0.0 { self.end_hang } else { 0.0 };
        self.body + pre - hang - if shrinks { self.visible_shrink } else { 0.0 }
    }
}

const MAX_SEAM_CHAIN: usize = 8;

pub(super) fn within(used: f64, limit: f64) -> bool {
    used <= limit + tolerance_f64(used, limit)
}

fn same_end(a: LineEnd<'_>, b: LineEnd<'_>) -> bool {
    match (a, b) {
        (LineEnd::At { token: a, .. }, LineEnd::At { token: b, .. })
        | (LineEnd::Before(a), LineEnd::Before(b)) => a == b,
        _ => false,
    }
}

#[derive(Clone, Copy)]
struct FitCandidate<'a> {
    end: LineEnd<'a>,
    used: f64,
    capacities: TierCapacities,
    content_clusters: usize,
}

struct Fits<'a, 'c> {
    tiers: [Option<LineEnd<'a>>; 3],
    candidates: &'c mut SmallVec<[FitCandidate<'a>; 16]>,
}

impl<'a, 'c> Fits<'a, 'c> {
    fn record(&mut self, token: TokenIdx, brk: &'a DiscretionaryNode, walk: &LineWalk, used: f64) {
        let end = LineEnd::At { token, brk };
        if let Some(slot) = self.tiers.get_mut(ladder_tier(brk)) {
            *slot = Some(end);
        }
        self.candidates.push(FitCandidate {
            end,
            used,
            capacities: walk.capacities(),
            content_clusters: walk.content_clusters,
        });
    }

    fn best(&self) -> Option<LineEnd<'a>> {
        let [t0, t1, t2] = self.tiers;
        t0.or(t1).or(t2)
    }
}

#[derive(Clone, Copy)]
struct SearchState<'a> {
    cursor: LineCursor<'a>,
    last_end: LineEnd<'a>,
    cumulative: f64,
    previous: Option<(bool, u8)>,
    first_end: LineEnd<'a>,
    at_end: bool,
}

impl<'i, 'a> Layout<'i, 'a> {
    pub(super) fn reachable_stop(&self, distance: f64, slot: InlineSlotConstraint) -> Option<f64> {
        let width = f64::from(slot.width());
        let step = f64::from(self.inputs.constraints().tab_stops.step());
        let next = ((distance / step).floor() + 1.0) * step;
        [next, distance + step]
            .into_iter()
            .find(|&stop| stop.is_finite() && stop > distance)
            .filter(|&stop| within(stop, width))
    }

    pub(super) fn measure(
        &self,
        piece: &Piece<'_>,
        pen: f64,
        slot: InlineSlotConstraint,
        pending: Option<&Reshapes>,
    ) -> (f64, f64) {
        match piece.kind {
            PieceKind::Tab => {
                let width = self
                    .reachable_stop(pen, slot)
                    .map_or(0.0, |stop| stop - pen);
                (width, width)
            }
            PieceKind::Cluster { cluster, .. } => {
                let (glyphs, width) = self.cluster_glyphs_over(pending, cluster);
                let mut x = 0.0_f64;
                let mut reach = width;
                for g in glyphs {
                    x += f64::from(g.x_advance) * self.font_scale;
                    reach = reach.max(x);
                }
                (width, reach)
            }
            PieceKind::Object(object) => {
                let (width, _, _) = Self::fitted_object(object, slot);
                (width, width)
            }
            PieceKind::Glue(_) => {
                let w = piece.width * self.font_scale;
                (w, w)
            }
            _ => (piece.width, piece.width),
        }
    }

    pub(super) fn fitted_object(
        object: &InlineObjectBox,
        slot: InlineSlotConstraint,
    ) -> (f64, f64, f64) {
        let (width, ascent, descent) = (
            f64::from(object.width),
            f64::from(object.ascent),
            f64::from(object.descent),
        );
        let reference = f64::from(slot.width());
        match object.overflow_policy {
            BoxOverflowPolicy::AllowOverflow => (width, ascent, descent),
            _ if width <= reference => (width, ascent, descent),
            BoxOverflowPolicy::Clip => (reference, ascent, descent),
            BoxOverflowPolicy::ScaleToFit => {
                let s = reference / width;
                (reference, ascent * s, descent * s)
            }
        }
    }

    pub(super) fn cluster_glyphs<'s>(
        &'s self,
        pending: &'s Reshapes,
        cluster: ClusterId,
    ) -> (&'s [RawShapedGlyph], f64) {
        self.cluster_glyphs_over(Some(pending), cluster)
    }

    #[inline]
    pub(super) fn has_reshapes(&self, pending: Option<&Reshapes>) -> bool {
        !self.reshaped.is_empty() || pending.is_some_and(|p| !p.is_empty())
    }

    #[inline]
    pub(super) fn cluster_glyphs_over<'s>(
        &'s self,
        pending: Option<&'s Reshapes>,
        cluster: ClusterId,
    ) -> (&'s [RawShapedGlyph], f64) {
        if self.has_reshapes(pending)
            && let Some(reshaped) = self.reshaped_cluster(pending, cluster)
        {
            return (&reshaped.glyphs, reshaped.width);
        }
        (
            self.inputs.cluster_glyphs_id(cluster),
            f64::from(self.inputs.cluster(cluster).natural_width) * self.font_scale,
        )
    }

    #[cold]
    #[inline(never)]
    pub(super) fn reshaped_cluster<'s>(
        &'s self,
        pending: Option<&'s Reshapes>,
        cluster: ClusterId,
    ) -> Option<&'s Reshaped> {
        pending
            .and_then(|p| p.get(&cluster))
            .or_else(|| self.reshaped.get(&cluster))
    }

    fn fit(
        &self,
        line: &LineCursor<'_>,
        slot: InlineSlotConstraint,
        candidates: &mut SmallVec<[FitCandidate<'a>; 16]>,
    ) -> (LineEnd<'a>, Reshapes) {
        let slot_width = f64::from(slot.width());
        let min_scale = f64::from(self.options.min_horizontal_scale);
        let mut walk = LineWalk::new(line.post_break_width());
        candidates.clear();
        let mut fits = Fits {
            tiers: [None; 3],
            candidates,
        };
        let mut t = line.token_start;
        let end = loop {
            match *self.tokens.at_or_sentinel(t) {
                Token::Break(b) => {
                    let pre = f64::from(b.pre_break_width);
                    let natural = walk.used_at_break(pre, false);
                    if b.is_mandatory() {
                        let used = walk.used_at_break(pre, self.options.compress_last_lines);
                        let feasible = within(used * min_scale, slot_width);
                        if feasible {
                            fits.record(t, b, &walk, natural);
                        }
                        break fits
                            .best()
                            .filter(|_| !feasible)
                            .unwrap_or(LineEnd::At { token: t, brk: b });
                    }
                    if within(walk.used_at_break(pre, true) * min_scale, slot_width) {
                        fits.record(t, b, &walk, natural);
                    }
                    walk.advance_break(f64::from(b.no_break_width));
                }
                Token::Piece(piece)
                    if matches!(piece.kind, PieceKind::Tab)
                        && self.reachable_stop(walk.pen, slot).is_none() =>
                {
                    let earlier = fits.best().filter(|_| walk.has_content);
                    let next = t.next();
                    let chosen = earlier.unwrap_or(match *self.tokens.at_or_sentinel(next) {
                        Token::Break(brk) => LineEnd::At { token: next, brk },
                        Token::Piece(_) => LineEnd::Before(next),
                    });
                    if earlier.is_none()
                        && let LineEnd::At { token, brk } = chosen
                    {
                        walk.reset_capacities();
                        fits.record(token, brk, &walk, walk.used_at_break(0.0, false));
                    }
                    break chosen;
                }
                Token::Piece(piece) => {
                    let used = self.walk_piece(&mut walk, &piece, line, slot, None);
                    if !within(used * min_scale, slot_width) && piece.is_content() {
                        match fits.best() {
                            Some(end) => break end,
                            None => return self.emergency(line, t, slot),
                        }
                    }
                }
            }
            t = t.next();
        };
        (end, Reshapes::new())
    }

    fn repair_taken_break(
        &self,
        line: &LineCursor<'_>,
        end: LineEnd<'a>,
        slot: InlineSlotConstraint,
    ) -> Reshapes {
        let mut reshapes = Reshapes::new();
        let LineEnd::At { token, brk } = end else {
            return reshapes;
        };
        if self
            .repair_seam(line, token.next(), &mut reshapes)
            .is_empty()
        {
            return reshapes;
        }
        let shrinks = !brk.is_mandatory() || self.options.compress_last_lines;
        let used = self
            .walk_to(line, token, slot, &reshapes)
            .used_at_break(f64::from(brk.pre_break_width), shrinks);
        let min_scale = f64::from(self.options.min_horizontal_scale);
        if !within(used * min_scale, f64::from(slot.width())) {
            reshapes.clear();
        }
        reshapes
    }

    /// One parameterised state machine for every strategy (spec ST1-6, ST1-9, ST1-10).
    pub(super) fn break_line(
        &self,
        cursor: &LineCursor<'a>,
        slot: InlineSlotConstraint,
    ) -> Result<(LineEnd<'a>, Reshapes), LayoutInputError> {
        let params = self.parameters;
        let mut scan_buf: SmallVec<[FitCandidate<'a>; 16]> = SmallVec::new();
        let (first_end, first_reshapes) = self.fit(cursor, slot, &mut scan_buf);
        let mut states: SmallVec<[SearchState<'a>; 8]> = SmallVec::new();
        let mut expanded: SmallVec<[SearchState<'a>; 8]> = SmallVec::new();
        self.push_candidates(cursor, slot, first_end, &scan_buf, None, &mut states);
        for _ in 1..params.window_depth {
            let first = states.first().map(|s| s.first_end);
            if first.is_some_and(|f| states.iter().all(|s| s.at_end || same_end(s.first_end, f))) {
                break;
            }
            expanded.clear();
            for &state in &states {
                if state.at_end {
                    expanded.push(state);
                    continue;
                }
                let mut next = state.cursor;
                next.advance(state.last_end, &self.tokens, self.strut_height());
                let next_slot = self.speculative_slot(&mut next)?;
                let (end, _) = self.fit(&next, next_slot, &mut scan_buf);
                self.push_candidates(
                    &next,
                    next_slot,
                    end,
                    &scan_buf,
                    Some(&state),
                    &mut expanded,
                );
            }
            expanded.sort_by(|a, b| a.cumulative.total_cmp(&b.cumulative));
            expanded.truncate(params.beam_width);
            std::mem::swap(&mut states, &mut expanded);
        }
        let end = states
            .iter()
            .min_by(|a, b| a.cumulative.total_cmp(&b.cumulative))
            .map_or(first_end, |s| s.first_end);
        let reshapes = if same_end(end, first_end) && !first_reshapes.is_empty() {
            first_reshapes
        } else {
            self.repair_taken_break(cursor, end, slot)
        };
        Ok((end, reshapes))
    }

    fn push_candidates(
        &self,
        cursor: &LineCursor<'a>,
        slot: InlineSlotConstraint,
        greedy_end: LineEnd<'a>,
        scan: &[FitCandidate<'a>],
        parent: Option<&SearchState<'a>>,
        out: &mut SmallVec<[SearchState<'a>; 8]>,
    ) {
        let slot_width = f64::from(slot.width());
        let previous = parent.and_then(|p| p.previous);
        let base_cost = parent.map_or(0.0, |p| p.cumulative);
        let mut emit = |c: &FitCandidate<'a>| {
            let (demerits, fitness, flagged, is_paragraph_end) =
                self.score(c, slot_width, previous);
            out.push(SearchState {
                cursor: *cursor,
                last_end: c.end,
                cumulative: base_cost + demerits,
                previous: Some((flagged, fitness)),
                first_end: parent.map_or(c.end, |p| p.first_end),
                at_end: is_paragraph_end,
            });
        };
        let fallback = FitCandidate {
            end: greedy_end,
            used: cursor.post_break_width(),
            capacities: TierCapacities::default(),
            content_clusters: 0,
        };
        emit(
            scan.iter()
                .find(|c| same_end(c.end, greedy_end))
                .unwrap_or(&fallback),
        );
        for c in scan
            .iter()
            .rev()
            .filter(|c| !same_end(c.end, greedy_end))
            .take(self.parameters.beam_width.saturating_sub(1))
        {
            emit(c);
        }
    }

    fn score(
        &self,
        candidate: &FitCandidate<'_>,
        slot_width: f64,
        previous: Option<(bool, u8)>,
    ) -> (f64, u8, bool, bool) {
        let (penalty, flagged, forced, is_paragraph_end) = match candidate.end {
            LineEnd::At { brk, .. } => (
                brk.penalty,
                brk.flagged,
                brk.is_mandatory(),
                brk.is_paragraph_end(),
            ),
            LineEnd::Before(_) => (0, false, false, false),
        };
        let slack = slot_width - candidate.used;
        let mut fit = badness::line_fit(
            slack as f32,
            &candidate.capacities,
            self.ast.strut_font_size,
        );
        if (is_paragraph_end || forced) && fit.ratio > 0.0 {
            fit.badness = 0.0;
        }
        let fitness = badness::fitness_class(fit.ratio);
        let mut demerits =
            badness::line_demerits(&fit, penalty, flagged, forced, fitness, previous);
        if slack < 0.0 && self.options.min_horizontal_scale < 1.0 && candidate.used > 0.0 {
            demerits += 1_000.0 * (1.0 - slot_width / candidate.used).abs();
        }
        if is_paragraph_end && candidate.content_clusters < 4 {
            demerits += badness::RUNT_PENALTY;
        }
        (demerits, fitness, flagged, is_paragraph_end)
    }

    fn speculative_slot(
        &self,
        cursor: &mut LineCursor<'a>,
    ) -> Result<InlineSlotConstraint, LayoutInputError> {
        let mut obstructed = 0;
        Ok(self
            .open_geometry(cursor, self.strut_height(), &mut obstructed)?
            .first_slot())
    }

    pub(super) fn strut_height(&self) -> f64 {
        f64::from(self.strut.ascent)
            + f64::from(self.strut.descent)
            + f64::from(self.strut.line_gap)
    }

    fn walk_piece(
        &self,
        walk: &mut LineWalk,
        piece: &Piece<'_>,
        line: &LineCursor<'_>,
        slot: InlineSlotConstraint,
        pending: Option<&Reshapes>,
    ) -> f64 {
        let is_soft_wrap = line.is_soft_wrapped_start();
        match piece.kind {
            PieceKind::Glue(glue) => {
                let tier = glue.kind.tier() as usize;
                if let Some(stretch) = walk.stretch.get_mut(tier) {
                    *stretch += f64::from(glue.max_stretch);
                }
                if let Some(shrink) = walk.shrink_capacity.get_mut(tier) {
                    *shrink += f64::from(glue.max_shrink);
                }
                if self.options.justify {
                    let shrink = f64::from(glue.max_shrink);
                    walk.shrink += shrink;
                    if glue.kind == GlueKind::CjkPunctCompression {
                        walk.pending_punct_glue_shrink = shrink;
                    }
                }
            }
            PieceKind::Tab => {
                walk.shrink = 0.0;
                walk.reset_capacities();
                walk.pending_punct_glue_shrink = 0.0;
            }
            _ => {}
        }
        if !walk.has_start && !piece.is_edge_transparent() {
            walk.has_start = true;
            if line.post_break_width() == 0.0 {
                walk.pen -= self
                    .hang_protrusion(
                        piece,
                        LineEdge::Start,
                        line.first_line(),
                        is_soft_wrap,
                        pending,
                    )
                    .unwrap_or(0.0);
            }
        }
        let (mut advance, mut reach) = self.measure(piece, walk.pen, slot, pending);
        if !is_trim_transparent(piece) {
            if let PieceKind::Cluster { run, cluster } = piece.kind {
                let info = self.cluster_trim_info(run, cluster, advance);
                let mut lead_trim = 0.0_f64;
                let mut a_trail_delta = 0.0_f64;
                if let Some(b) = info {
                    if !walk.has_content && line.post_break_width() == 0.0 {
                        lead_trim = line_start_lead_trim(b, is_soft_wrap);
                    }
                    if let Some((a, a_trail)) = walk.prev_trim {
                        let (t_a, l_b) = pair_trim(a, b);
                        a_trail_delta =
                            (t_a.min(a.class.max_trail(a.font_size)) - a_trail).max(0.0);
                        lead_trim = (lead_trim + l_b).min(b.class.max_lead(b.font_size));
                    }
                }
                if a_trail_delta > 0.0 {
                    walk.pen -= a_trail_delta;
                }
                let static_trim = a_trail_delta + lead_trim;
                if static_trim > 0.0 && walk.pending_punct_glue_shrink > 0.0 {
                    walk.shrink -= static_trim.min(walk.pending_punct_glue_shrink);
                }
                walk.pending_punct_glue_shrink = 0.0;
                walk.prev_trim = info.map(|b| (b, 0.0));
                advance = (advance - lead_trim).max(0.0);
                reach = (reach - lead_trim).max(0.0);
            } else {
                walk.prev_trim = None;
                walk.pending_punct_glue_shrink = 0.0;
            }
        }
        if piece.is_content() {
            // Tier 2 gap belongs to the cluster it follows once another visible cluster follows it (ST1-7).
            if walk.has_content
                && let Some(tier2) = walk.stretch.get_mut(2)
            {
                *tier2 += std::mem::take(&mut walk.pending_cjk_gap);
            }
            walk.pending_cjk_gap = match piece.kind {
                PieceKind::Cluster { run, cluster }
                    if self
                        .inputs
                        .cluster(cluster)
                        .flags
                        .contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER) =>
                {
                    CJK_GAP_STRETCH * f64::from(self.inputs.run(run).font_size)
                }
                _ => 0.0,
            };
            if matches!(piece.kind, PieceKind::Cluster { .. }) {
                walk.content_clusters += 1;
            }
            walk.visible_shrink = walk.shrink;
            walk.visible_stretch = walk.stretch;
            walk.visible_shrink_cap = walk.shrink_capacity;
            walk.end_hang = self
                .hang_protrusion(piece, LineEdge::End, line.first_line(), false, pending)
                .unwrap_or(0.0);
            walk.body = walk.pen + reach;
            walk.has_content = true;
        }
        walk.pen += advance;
        walk.used()
    }

    fn next_break_after(&self, t: TokenIdx) -> Option<LineEnd<'a>> {
        for (e, token) in self.tokens.iter_from(t.next()) {
            match *token {
                Token::Piece(p) if matches!(p.kind, PieceKind::Glue(_)) => {}
                Token::Break(brk) => return Some(LineEnd::At { token: e, brk }),
                Token::Piece(_) => return None,
            }
        }
        None
    }

    fn emergency(
        &self,
        line: &LineCursor<'_>,
        t: TokenIdx,
        slot: InlineSlotConstraint,
    ) -> (LineEnd<'a>, Reshapes) {
        let start = line.token_start;
        let mut reshapes = Reshapes::new();
        let first_content = self
            .tokens
            .iter_range(start, t)
            .find_map(|(k, tok)| matches!(tok, Token::Piece(p) if p.is_content()).then_some(k));
        if let Some(first) = first_content {
            let available = f64::from(slot.width());
            let min_scale = f64::from(self.options.min_horizontal_scale);
            let floor = first.next();
            let mut t = t;
            loop {
                let run_start = self.dash_run_start(t);
                if run_start > first {
                    t = run_start;
                }
                let added = self.repair_seam(line, t, &mut reshapes);
                if t <= floor
                    || within(
                        self.walk_to(line, t, slot, &reshapes).used() * min_scale,
                        available,
                    )
                {
                    return (LineEnd::Before(t), reshapes);
                }
                for c in added {
                    reshapes.remove(&c);
                }
                while let Some(prev) = t.prev() {
                    t = prev;
                    if t <= floor || !matches!(self.tokens.get(t), Some(Token::Break(_))) {
                        break;
                    }
                }
            }
        }
        if let Some(end) = self.next_break_after(t) {
            return (end, reshapes);
        }
        let next = t.next();
        self.repair_seam(line, next, &mut reshapes);
        (LineEnd::Before(next), reshapes)
    }

    fn dash_run_start(&self, mut t: TokenIdx) -> TokenIdx {
        while let Some(prev) = t.prev() {
            let binds = self.tokens.slice_from(t).iter().find_map(|tok| match tok {
                Token::Piece(Piece {
                    kind: PieceKind::SpanEdge { .. },
                    ..
                }) => None,
                Token::Piece(Piece {
                    kind: PieceKind::Cluster { cluster, .. },
                    ..
                }) => Some(
                    self.inputs
                        .cluster(*cluster)
                        .flags
                        .contains(AtomFlags::BINDS_TO_PREVIOUS),
                ),
                _ => Some(false),
            });
            if binds != Some(true) {
                break;
            }
            t = prev;
        }
        t
    }

    pub(super) fn repair_seam(
        &self,
        line: &LineCursor<'_>,
        t: TokenIdx,
        reshapes: &mut Reshapes,
    ) -> SmallVec<[ClusterId; 4]> {
        let mut added = SmallVec::new();
        let Some((run, after)) = self.cluster_from(t) else {
            return added;
        };
        let after_cluster = self.inputs.cluster(after);
        if after_cluster.safe_to_break_before {
            return added;
        }
        let depends = |c: ClusterId| !self.inputs.cluster(c).safe_to_concat_before;
        let next_side = std::iter::once(after).chain(
            self.inputs
                .run_clusters_after(run, after)
                .take_while(|&c| depends(c))
                .take(MAX_SEAM_CHAIN - 1),
        );
        let mut joined = true;
        let line_first = self
            .tokens
            .slice_from(line.token_start)
            .iter()
            .find_map(|token| match token {
                Token::Piece(Piece {
                    kind: PieceKind::Cluster { cluster, .. },
                    ..
                }) => Some(*cluster),
                _ => None,
            })
            .unwrap_or(after);
        let previous_side = self
            .inputs
            .run_clusters_before(run, line_first, after)
            .take_while(|&c| std::mem::replace(&mut joined, depends(c)))
            .take(MAX_SEAM_CHAIN);
        let run_ref = self.inputs.run(run);
        let atoms = self.inputs.run_atoms_id(run);
        let text = self.inputs.text();
        let seam = after_cluster.byte_range.start();
        let next_context = TextRange::new(seam, text.len());
        let previous_context = TextRange::new(0, seam);
        for (c, context) in next_side
            .map(|c| (c, next_context))
            .chain(previous_side.map(|c| (c, previous_context)))
        {
            let cluster = self.inputs.cluster(c);
            if let Some((glyphs, width)) = checked_reshape_window(
                self.shaper,
                &ReshapeRequest {
                    context,
                    ..ReshapeRequest::plain(text, run_ref, atoms, cluster.byte_range)
                },
                cluster,
            ) && reshapes
                .insert(
                    c,
                    Reshaped {
                        glyphs,
                        width: f64::from(width) * self.font_scale,
                        wdth: None,
                    },
                )
                .is_none()
            {
                added.push(c);
            }
        }
        added
    }

    fn cluster_from(&self, t: TokenIdx) -> Option<(RunId, ClusterId)> {
        self.tokens
            .slice_from(t)
            .iter()
            .find_map(|token| match token {
                Token::Piece(Piece {
                    kind: PieceKind::SpanEdge { .. },
                    ..
                }) => None,
                Token::Piece(Piece {
                    kind: PieceKind::Cluster { run, cluster },
                    ..
                }) => Some(Some((*run, *cluster))),
                _ => Some(None),
            })?
    }

    fn walk_to(
        &self,
        line: &LineCursor<'_>,
        end: TokenIdx,
        slot: InlineSlotConstraint,
        pending: &Reshapes,
    ) -> LineWalk {
        let mut walk = LineWalk::new(line.post_break_width());
        for token in self.tokens.slice(line.token_start, end) {
            match token {
                Token::Break(b) => walk.advance_break(f64::from(b.no_break_width)),
                Token::Piece(piece) => {
                    self.walk_piece(&mut walk, piece, line, slot, Some(pending));
                }
            }
        }
        walk
    }

    pub(super) fn cluster_metrics(
        &mut self,
        run: RunId,
        cluster: ClusterId,
    ) -> Result<FontMetrics, LayoutInputError> {
        let font = self.inputs.cluster(cluster).font;
        let key = (font.font_id.0, font.scale.to_bits(), run);
        if let Some(found) = self.metrics.get(&key) {
            return Ok(*found);
        }
        let metrics = checked_font_metrics(self.shaper, self.inputs.run(run).font_instance(font))?;
        self.metrics.insert(key, metrics);
        Ok(metrics)
    }
}
