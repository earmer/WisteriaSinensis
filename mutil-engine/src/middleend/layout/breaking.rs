//! Line-breaking and measurement helpers for [`Layout`] (spec §5 Stage 1).

use super::hanging::LineEdge;
use super::trim::{TrimInfo, is_trim_transparent, line_start_lead_trim, pair_trim};
use super::{Layout, LineCursor, LineEnd, Piece, PieceKind, Reshaped, Reshapes, Token};
use crate::frontend::{BoxOverflowPolicy, DiscretionaryNode, GlueKind, InlineObjectBox};
use crate::middleend::shaper_gate::{checked_font_metrics, checked_reshape_window};
use crate::middleend::tolerance::tolerance_f64;
use crate::middleend::validated::{ClusterId, RunId, TokenIdx};
use crate::middleend::{InlineSlotConstraint, LayoutInputError};
use crate::shaping::ReshapeRequest;
use crate::types::{AtomFlags, FontMetrics, RawShapedGlyph, TextRange};
use smallvec::SmallVec;

/// The tier of a discretionary on the overflow ladder (spec ST1-1): its
/// penalty net of cloned-span penalties (WN-21) in the URL band (WN-19)
/// below its maximum is tier 1, at the maximum tier 2; any other is tier 0.
pub(super) fn ladder_tier(d: &DiscretionaryNode) -> usize {
    if d.is_mandatory() {
        return 0;
    }
    // G1 proves `cloned_spans.len() <= MAX_DECORATION_NESTING_DEPTH` (8) and
    // `penalty` in `[-10_000, 10_000)`, so this subtraction cannot overflow.
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

/// The running measure of a line prefix: its pen, where its content ends,
/// the shrink of its glue and the protrusion of its last mark (ST1-3).
#[derive(Clone, Copy)]
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
}

impl LineWalk {
    fn new(post: f64) -> Self {
        Self {
            pen: post,
            body: post,
            has_start: false,
            has_content: false,
            shrink: 0.0,
            visible_shrink: 0.0,
            end_hang: 0.0,
            prev_trim: None,
            pending_punct_glue_shrink: 0.0,
        }
    }

    /// The width the line uses if it stops here.
    fn used(&self) -> f64 {
        self.body - self.end_hang - self.visible_shrink
    }

    /// Advances past an untaken break opportunity; a non-zero `no_break_width`
    /// separates the clusters on either side so they no longer pair for trim.
    fn advance_break(&mut self, no_break_width: f64) {
        self.pen += no_break_width;
        if no_break_width != 0.0 {
            self.prev_trim = None;
        }
    }

    /// The width used if the line ends at a break whose cloned padding is
    /// `pre`; a hang needs no padding behind it.
    fn used_at_break(&self, pre: f64, shrinks: bool) -> f64 {
        let hang = if pre == 0.0 { self.end_hang } else { 0.0 };
        self.body + pre - hang - if shrinks { self.visible_shrink } else { 0.0 }
    }
}

/// The most clusters a seam re-shape walks on each side (spec ST1-4): a
/// chain of ligatures joined across the seam rarely holds more, and the bound
/// keeps each emergency try linear (AX-6).
const MAX_SEAM_CHAIN: usize = 8;

/// `used` fits `limit` within the fitting tolerance.
pub(super) fn within(used: f64, limit: f64) -> bool {
    used <= limit + tolerance_f64(used, limit)
}

/// The outcome of fitting a line: where it ends, and the emergency re-shapes
/// of the seam it needs. [`Layout::fit`] only proposes them; the caller
/// places the line with them pending and commits them only if it keeps it.
pub(super) struct LineFit<'a> {
    pub(super) end: LineEnd<'a>,
    pub(super) reshapes: Reshapes,
}

/// The last fitting break of each ladder tier (spec ST1-1), with its token.
#[derive(Default)]
struct Fits<'a> {
    tier0: Option<(TokenIdx, &'a DiscretionaryNode)>,
    tier1: Option<(TokenIdx, &'a DiscretionaryNode)>,
    tier2: Option<(TokenIdx, &'a DiscretionaryNode)>,
}

impl<'a> Fits<'a> {
    fn record(&mut self, token: TokenIdx, brk: &'a DiscretionaryNode) {
        let entry = Some((token, brk));
        match ladder_tier(brk) {
            0 => self.tier0 = entry,
            1 => self.tier1 = entry,
            _ => self.tier2 = entry,
        }
    }

    /// The break of the lowest tier that fits.
    fn best(&self) -> Option<LineEnd<'a>> {
        self.tier0
            .or(self.tier1)
            .or(self.tier2)
            .map(|(token, brk)| LineEnd::At { token, brk })
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    /// The next tab stop after `distance`, measured from the slot's
    /// line-start edge along the base direction, if it lies inside `slot`
    /// (spec ST3-1): a stop further than the slot's width is not reachable.
    pub(super) fn reachable_stop(&self, distance: f64, slot: InlineSlotConstraint) -> Option<f64> {
        let width = f64::from(slot.width());
        self.inputs
            .constraints()
            .tab_stops
            .next_stop(distance as f32)
            .map(f64::from)
            .filter(|&stop| within(stop, width))
    }

    /// The advance and the reach (furthest pen end, relative to the start)
    /// of a piece whose start lies `pen` from the slot's line-start edge.
    pub(super) fn measure(
        &self,
        piece: &Piece<'_>,
        pen: f64,
        slot: InlineSlotConstraint,
        pending: Option<&Reshapes>,
    ) -> (f64, f64) {
        match piece.kind {
            PieceKind::Tab => {
                // A tab with no reachable stop takes no advance (ST3-1, AX-6).
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
                    x += f64::from(g.x_advance);
                    reach = reach.max(x);
                }
                (width, reach)
            }
            PieceKind::Object(object) => {
                let (width, _, _) = Self::fitted_object(object, slot);
                (width, width)
            }
            _ => (piece.width, piece.width),
        }
    }

    /// An object's width, ascent and descent under its overflow policy in
    /// `slot` (spec WN-7): `Clip` clamps to the reference width,
    /// `ScaleToFit` scales uniformly to it, `AllowOverflow` keeps its size.
    pub(super) fn fitted_object(
        object: &InlineObjectBox,
        slot: InlineSlotConstraint,
    ) -> (f64, f64, f64) {
        let width = f64::from(object.width);
        let ascent = f64::from(object.ascent);
        let descent = f64::from(object.descent);
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

    /// The glyphs and width of a cluster, after any emergency re-shape,
    /// `pending` ones taking precedence over the committed.
    pub(super) fn cluster_glyphs<'s>(
        &'s self,
        pending: &'s Reshapes,
        cluster: ClusterId,
    ) -> (&'s [RawShapedGlyph], f64) {
        self.cluster_glyphs_over(Some(pending), cluster)
    }

    /// [`cluster_glyphs`](Self::cluster_glyphs) as `pending` re-shapes, not
    /// yet committed, would change it.
    pub(super) fn cluster_glyphs_over<'s>(
        &'s self,
        pending: Option<&'s Reshapes>,
        cluster: ClusterId,
    ) -> (&'s [RawShapedGlyph], f64) {
        let reshaped = pending
            .and_then(|pending| pending.get(&cluster))
            .or_else(|| self.reshaped.get(&cluster));
        if let Some(r) = reshaped {
            return (&r.glyphs, r.width);
        }
        let c = self.inputs.cluster(cluster);
        (
            self.inputs.cluster_glyphs_id(cluster),
            f64::from(c.natural_width),
        )
    }

    /// Greedy fitting (spec ST1-1, ST1-5, ST1-7, WN-1): the last break of
    /// the lowest ladder tier that fits before the content overflows, a
    /// mandatory break, or an emergency cluster break when nothing fits.
    /// Under `justify` a line fits while its width less the shrink of its
    /// visible glue after the last tab fits; a mark that may hang is credited
    /// with its protrusion (ST1-3).
    pub(super) fn fit(&self, line: &LineCursor<'_>, slot: InlineSlotConstraint) -> LineFit<'a> {
        let slot_width = f64::from(slot.width());
        let mut walk = LineWalk::new(line.post_break_width());
        let mut fits = Fits::default();
        let mut t = line.token_start;
        let end = loop {
            match *self.tokens.at_or_sentinel(t) {
                Token::Break(b) => {
                    let pre = f64::from(b.pre_break_width);
                    if b.is_mandatory() {
                        // A line ending here is compressed only under
                        // `compress_last_lines` (ST3-3): otherwise, if only
                        // shrink would fit it, it ends at an earlier break.
                        let used = walk.used_at_break(pre, self.options.compress_last_lines);
                        break fits
                            .best()
                            .filter(|_| !within(used, slot_width))
                            .unwrap_or(LineEnd::At { token: t, brk: b });
                    }
                    if within(walk.used_at_break(pre, true), slot_width) {
                        fits.record(t, b);
                    }
                    walk.advance_break(f64::from(b.no_break_width));
                }
                Token::Piece(piece)
                    if matches!(piece.kind, PieceKind::Tab)
                        && self.reachable_stop(walk.pen, slot).is_none() =>
                {
                    // No reachable stop (spec ST3-1, AX-6): break before the
                    // tab at an earlier opportunity, else consume it with no
                    // advance and end the line right after it. Since `t` is a
                    // piece and `tokens` ends with the sentinel break, `t.next()`
                    // is in bounds.
                    let earlier = fits.best().filter(|_| walk.has_content);
                    let next = t.next();
                    break earlier.unwrap_or(match *self.tokens.at_or_sentinel(next) {
                        Token::Break(brk) => LineEnd::At { token: next, brk },
                        Token::Piece(_) => LineEnd::Before(next),
                    });
                }
                Token::Piece(piece) => {
                    let used = self.walk_piece(&mut walk, &piece, line, slot, None);
                    if !within(used, slot_width) && piece.is_content() {
                        match fits.best() {
                            Some(end) => break end,
                            None => return self.emergency(line, t, slot),
                        }
                    }
                }
            }
            t = t.next();
        };
        LineFit {
            end,
            reshapes: self.repair_taken_break(line, end, slot),
        }
    }

    /// The re-shapes a line ending at `end` needs at its seam: a taken break
    /// whose seam is not safe to break re-shapes it (spec ST1-10, ME-2), when
    /// the line still fits with the re-shaped widths; otherwise it keeps the
    /// glyphs it was fitted with.
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
        // Measured as `fit` measured the break: with shrink, except at a
        // mandatory break outside `compress_last_lines` (ST3-3).
        let shrinks = !brk.is_mandatory() || self.options.compress_last_lines;
        let used = self
            .walk_to(line, token, slot, &reshapes)
            .used_at_break(f64::from(brk.pre_break_width), shrinks);
        if !within(used, f64::from(slot.width())) {
            reshapes.clear();
        }
        reshapes
    }

    /// Advances `walk` over `piece`, the one place that decides what a piece
    /// does to a line's measure (shrink, start and end hang, tab pen, reach),
    /// and returns the line's used width up to and including it. `pending`
    /// re-shapes, not yet committed, apply to the clusters measured.
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
            PieceKind::Glue(glue) if self.options.justify => {
                let shrink = f64::from(glue.max_shrink);
                walk.shrink += shrink;
                if glue.kind == GlueKind::CjkPunctCompression {
                    walk.pending_punct_glue_shrink = shrink;
                }
            }
            PieceKind::Tab => {
                walk.shrink = 0.0;
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
            walk.visible_shrink = walk.shrink;
            walk.end_hang = self
                .hang_protrusion(piece, LineEdge::End, line.first_line(), false, pending)
                .unwrap_or(0.0);
            walk.body = walk.pen + reach;
            walk.has_content = true;
        }
        walk.pen += advance;
        walk.used()
    }

    /// The break token right after `t` and its trailing glue, if any.
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

    /// The emergency break (spec ST1-5) of a line from `line` whose piece
    /// `t` overflows with no fitting opportunity: before `t` when something
    /// visible precedes it, else after `t` alone (one indivisible entity on
    /// its own overflowing line, SLT-5), together with the break right after
    /// it if only glue separates them.
    fn emergency(
        &self,
        line: &LineCursor<'_>,
        t: TokenIdx,
        slot: InlineSlotConstraint,
    ) -> LineFit<'a> {
        let start = line.token_start;
        let mut reshapes = Reshapes::new();
        let first_content = self
            .tokens
            .iter_range(start, t)
            .find_map(|(k, tok)| matches!(tok, Token::Piece(p) if p.is_content()).then_some(k));
        if let Some(first) = first_content {
            // Re-shaping an unsafe seam can widen its sides (kerning across
            // it is lost); while the line then overflows, the seam moves one
            // piece back and the abandoned re-shape is dropped.
            let available = f64::from(slot.width());
            let floor = first.next();
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
                let added = self.repair_seam(line, t, &mut reshapes);
                if t <= floor || within(self.used_before(line, t, slot, &reshapes), available) {
                    return LineFit {
                        end: LineEnd::Before(t),
                        reshapes,
                    };
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
            return LineFit { end, reshapes };
        }
        let next = t.next();
        self.repair_seam(line, next, &mut reshapes);
        LineFit {
            end: LineEnd::Before(next),
            reshapes,
        }
    }

    /// Whether the seam before token `t` lies inside a binding punctuation run
    /// such as consecutive U+2014, U+2026 or U+00B7 (spec FE-15).
    fn inside_dash_run(&self, t: TokenIdx) -> bool {
        for token in self.tokens.slice_from(t) {
            let Token::Piece(p) = token else {
                return false;
            };
            match p.kind {
                PieceKind::SpanEdge { .. } => {}
                PieceKind::Cluster { cluster, .. } => {
                    return self
                        .inputs
                        .cluster(cluster)
                        .flags
                        .contains(AtomFlags::BINDS_TO_PREVIOUS);
                }
                _ => return false,
            }
        }
        false
    }

    /// The first token of the run whose seam `t` lies inside, or `t`.
    fn dash_run_start(&self, mut t: TokenIdx) -> TokenIdx {
        while let Some(prev) = t.prev() {
            if !self.inside_dash_run(t) {
                break;
            }
            t = prev;
        }
        t
    }

    /// A seam before token `t` that is not safe to break re-shapes the
    /// clusters whose shaping depended on it (spec ST1-4, ST1-5, ST1-10),
    /// each on its own with no edit. The seam lies between the clusters on
    /// either side of `t`, past any span edges, whose padding is no glyph. On
    /// each side the re-shape walks away from the seam along the clusters
    /// that are not safe to concatenate before, as the HarfBuzz recipe does,
    /// up to [`MAX_SEAM_CHAIN`] clusters, and never back past the first
    /// cluster of `line`, whose earlier lines are placed. A re-shape that does
    /// not return exactly one cluster is not used. The re-shapes go into
    /// `reshapes`; the result names the clusters it newly added there.
    fn repair_seam(
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
        // After the seam: `after`, and each next cluster joined to it.
        let next_side = std::iter::once(after).chain(
            self.inputs
                .run_clusters_after(run, after)
                .take_while(|&c| depends(c))
                .take(MAX_SEAM_CHAIN - 1),
        );
        // Before it: the cluster that ends there, and each one it is joined
        // to.
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
        // Each side shapes with no context across the seam, which becomes a
        // line edge (a joining letter takes its final or initial form there).
        let seam = after_cluster.byte_range.start();
        let next_context = TextRange::new(seam, text.len());
        let previous_context = TextRange::new(0, seam);
        let sides = next_side
            .map(|c| (c, next_context))
            .chain(previous_side.map(|c| (c, previous_context)));
        for (c, context) in sides {
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
                        width: f64::from(width),
                    },
                )
                .is_none()
            {
                added.push(c);
            }
        }
        added
    }

    /// The cluster that starts at token `t`, past any span edges, and its
    /// run.
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

    /// The used width of a line from `line.token_start` that stops before
    /// token `end`, measured exactly as [`fit`](Self::fit) measures it, with
    /// the `pending` re-shapes applied.
    fn used_before(
        &self,
        line: &LineCursor<'_>,
        end: TokenIdx,
        slot: InlineSlotConstraint,
        pending: &Reshapes,
    ) -> f64 {
        self.walk_to(line, end, slot, pending).used()
    }

    /// The walk of a line from `line.token_start` up to token `end`, as
    /// [`fit`](Self::fit) walks it, with the `pending` re-shapes applied.
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

    /// A cluster's font metrics, through the shaper gate (spec G2).
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
