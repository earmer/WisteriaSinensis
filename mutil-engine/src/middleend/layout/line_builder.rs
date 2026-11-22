//! Line placement, line-box calculation, alignment, and visual run assembly (spec §5 Stages 2–4).

use super::breaking::within;
use super::clamp::SynthesizedEllipsis;
use super::hanging::{EndHang, LineHangs, clip_to_content};
use super::line_artifacts::{CaretScratch, VisualIndex};
use super::line_box::LineBoxMetrics;
use super::scratch::{LayoutScratch, LogicalPiece, build_placed};
use super::trim::{TrimInfo, is_trim_transparent, line_start_lead_trim, pair_trim};
use super::{Layout, LineCursor, LineEnd, PieceKind, PlacedPiece, Reshapes, Token};
use crate::frontend::bidi::{LineClass, reset_line_levels};
use crate::frontend::{DiscretionaryNode, SpanEdge};
use crate::middleend::validated::{TokenIdx, VisualOrder};
use crate::middleend::{
    DecorationKind, DecorationLine, HangingWhitespaceMode, InlineSlotConstraint, LayoutInputError,
    OpenGeometry, PositionedLine, PositionedSlot, VisualGlyphRun,
};
use crate::types::{AtomFlags, BidiLevel, TextRange, WritingMode};
use smallvec::SmallVec;
use std::ops::Deref;

#[derive(Clone, Copy)]
pub(super) struct LineBounds {
    pub(super) body_lo: f64,
    pub(super) body_hi: f64,
}

pub(super) struct ContentBlock {
    pub(super) start: f64,
    pub(super) width: f64,
}

impl ContentBlock {
    fn edges(&self) -> (f64, f64) {
        (self.start, self.start + self.width)
    }
}

pub(super) struct PlacedLine<'l, 'a> {
    pub(super) placed: &'l mut [PlacedPiece<'a>],
    pub(super) order: &'l VisualOrder,
    pub(super) visual: &'l VisualIndex,
    pub(super) byte_range: TextRange,
    pub(super) bounds: LineBounds,
    pub(super) start_break: Option<&'a DiscretionaryNode>,
    pub(super) end_break: Option<&'a DiscretionaryNode>,
    hangs: LineHangs,
    is_justified: bool,
    pub(super) x_scale: f64,
    block_top: f64,
    pub(super) truncated: bool,
}

impl PlacedLine<'_, '_> {
    pub(super) fn body_len(&self) -> usize {
        self.order.len()
    }

    pub(super) fn soft_wrap(&self) -> bool {
        !self.truncated && soft_wraps(self.end_break)
    }
}

fn soft_wraps(end_break: Option<&DiscretionaryNode>) -> bool {
    end_break.is_none_or(|b| !b.is_mandatory())
}

pub(super) struct AlignedLine<'l, 'a> {
    line: PlacedLine<'l, 'a>,
    pub(super) content: ContentBlock,
}

impl<'l, 'a> Deref for AlignedLine<'l, 'a> {
    type Target = PlacedLine<'l, 'a>;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

struct LineEndFacts<'a> {
    last_piece: TokenIdx,
    end_break: Option<&'a DiscretionaryNode>,
    byte_end: usize,
    pre: f64,
}

struct BodyRequest<'c, 'a> {
    cursor: &'c LineCursor<'a>,
    post: f64,
    pre: f64,
    soft_wrap: bool,
    has_suffix: bool,
    slot: InlineSlotConstraint,
}

struct BodyFit {
    hangs: LineHangs,
    is_justified: bool,
    x_scale: f64,
}

#[derive(Clone, Copy)]
struct Pads {
    left: f64,
    right: f64,
}

impl Pads {
    fn new(rtl: bool, pre: f64, post: f64) -> Self {
        if rtl {
            Self {
                left: pre,
                right: post,
            }
        } else {
            Self {
                left: post,
                right: pre,
            }
        }
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    pub(super) fn place(
        &mut self,
        cursor: &LineCursor<'a>,
        end: LineEnd<'a>,
        geometry: &OpenGeometry,
        pending: &Reshapes,
        scratch: &mut LayoutScratch<'a>,
        ellipsis: Option<Option<&SynthesizedEllipsis>>,
    ) -> Result<(PositionedLine, f64), LayoutInputError> {
        let slot = geometry.first_slot();
        let LineEndFacts {
            last_piece,
            end_break,
            byte_end,
            pre,
        } = self.end_facts(end);
        let LayoutScratch {
            logical,
            levels,
            order,
            widths,
            expansions,
            xs,
            y_shifts,
            lead_trims,
            trail_trims,
            placed,
            visual,
            carets,
        } = scratch;
        self.collect_logical(logical, cursor.token_start, last_piece);
        let body_len = logical
            .iter()
            .rposition(|p| p.piece.is_content())
            .map_or(0, |k| k + 1);
        self.apply_l1(logical, levels);
        let (body_logical, _) = logical.split_at(body_len);
        visual_order_into(body_logical, levels, order);
        let post = if ellipsis.is_some() && byte_end == cursor.byte_start {
            0.0
        } else {
            cursor.post_break_width()
        };
        let request = BodyRequest {
            cursor,
            post,
            pre,
            soft_wrap: ellipsis.is_none() && soft_wraps(end_break),
            has_suffix: ellipsis.flatten().is_some(),
            slot,
        };
        let mut line_pending = pending.clone();
        let BodyFit {
            hangs,
            is_justified,
            x_scale,
        } = self.measure_line(
            logical,
            body_len,
            [widths, expansions, lead_trims, trail_trims],
            &request,
            &mut line_pending,
        );
        let mut pads = Pads::new(self.base.is_rtl(), pre, post);
        let mut hangs = hangs;
        if x_scale < 1.0 {
            pads.left *= x_scale;
            pads.right *= x_scale;
            if let Some(s) = &mut hangs.start {
                s.1 *= x_scale;
            }
            if let Some(e) = &mut hangs.end {
                e.1 *= x_scale;
            }
        }
        let body_end = self.place_visual(logical, order, widths, xs, pads, hangs);
        let byte_range = TextRange::new(cursor.byte_start, byte_end);
        let suffix_extent = ellipsis.flatten().map(|s| (s.ascent, s.descent));
        let line_box =
            self.compute_line_box(logical, order, y_shifts, byte_range, slot, suffix_extent)?;
        build_placed(
            placed,
            logical,
            [xs, widths, expansions, y_shifts, lead_trims, trail_trims],
        );
        visual.rebuild(placed, order);
        let line = PlacedLine {
            placed,
            order,
            visual,
            byte_range,
            bounds: LineBounds {
                body_lo: pads.left,
                body_hi: body_end,
            },
            start_break: cursor.start_break,
            end_break,
            hangs,
            is_justified,
            x_scale,
            block_top: cursor.block_top,
            truncated: ellipsis.is_some(),
        };
        let positioned =
            self.build_line(line, &line_box, carets, geometry, &line_pending, ellipsis)?;
        Ok((positioned, line_box.line_height))
    }

    fn measure_line(
        &self,
        logical: &[LogicalPiece<'a>],
        body_len: usize,
        [widths, expansions, lead_trims, trail_trims]: [&mut Vec<f64>; 4],
        request: &BodyRequest<'_, 'a>,
        pending: &mut Reshapes,
    ) -> BodyFit {
        let (body, trailing) = logical.split_at(body_len);
        expansions.clear();
        expansions.resize(logical.len(), 0.0);
        let BodyRequest {
            cursor,
            post,
            pre,
            soft_wrap,
            has_suffix,
            slot,
        } = *request;
        let first_line = cursor.first_line();
        let is_soft_wrap = cursor.is_soft_wrapped_start();
        let start = self.start_hang(body, first_line, is_soft_wrap, post, pending);
        let distance = self.measure_widths(
            body,
            [widths, lead_trims, trail_trims],
            post - LineHangs::protrusion(start),
            (post == 0.0, is_soft_wrap),
            slot,
            pending,
        );
        let compressible = self.options.justify && (soft_wrap || self.options.compress_last_lines);
        let shrink = if compressible {
            Self::shrink_capacity(body, lead_trims, trail_trims)
        } else {
            0.0
        };
        let slot_width = f64::from(slot.width());
        let fits_without = within(distance + pre - shrink, slot_width);
        let end = if has_suffix {
            None
        } else {
            self.end_hang(
                body,
                first_line,
                EndHang {
                    pre,
                    start,
                    fits_without,
                },
                pending,
            )
        };
        let hangs = LineHangs { start, end };
        let visible_width = distance + pre - LineHangs::protrusion(end);
        let slack = slot_width - visible_width;
        let (body_expansions, _) = expansions.split_at_mut(body_len);
        let can_compress =
            self.options.min_horizontal_scale < 1.0 || self.options.allow_variable_width_axis;
        let try_tier1 =
            (compressible && (soft_wrap || slack < 0.0)) || (can_compress && slack < 0.0);
        let is_justified = try_tier1
            && slot_width.is_finite()
            && self.justify(
                body,
                widths,
                body_expansions,
                [lead_trims, trail_trims],
                slack,
                pending,
            );
        let mut cur_width = post - LineHangs::protrusion(start)
            + body
                .iter()
                .zip(widths.iter())
                .map(|(p, &w)| p.gap + w)
                .sum::<f64>()
            + pre
            - LineHangs::protrusion(end);
        if slot_width.is_finite()
            && !within(cur_width, slot_width)
            && self.options.allow_variable_width_axis
            && self.try_wdth_condense(body, slot_width / cur_width, pending)
        {
            let d2 = self.measure_widths(
                body,
                [widths, lead_trims, trail_trims],
                post - LineHangs::protrusion(start),
                (post == 0.0, is_soft_wrap),
                slot,
                pending,
            );
            cur_width = d2 + pre - LineHangs::protrusion(end);
            if !within(cur_width, slot_width) {
                body_expansions.fill(0.0);
                self.justify(
                    body,
                    widths,
                    body_expansions,
                    [lead_trims, trail_trims],
                    slot_width - cur_width,
                    pending,
                );
                cur_width = post - LineHangs::protrusion(start)
                    + body
                        .iter()
                        .zip(widths.iter())
                        .map(|(p, &w)| p.gap + w)
                        .sum::<f64>()
                    + pre
                    - LineHangs::protrusion(end);
            }
        }
        let fixed_w: f64 = body
            .iter()
            .zip(widths.iter())
            .map(|(p, &w)| {
                p.gap
                    + if matches!(p.piece.kind, PieceKind::Object(_)) {
                        w
                    } else {
                        0.0
                    }
            })
            .sum();
        let scalable_w = cur_width - fixed_w;
        let x_scale = if slot_width.is_finite()
            && !within(cur_width, slot_width)
            && scalable_w > 0.0
            && self.options.min_horizontal_scale < 1.0
        {
            let s = ((slot_width - fixed_w).max(0.0) / scalable_w)
                .clamp(f64::from(self.options.min_horizontal_scale), 1.0);
            for (p, w) in body.iter().zip(widths.iter_mut()) {
                if !matches!(p.piece.kind, PieceKind::Object(_)) {
                    *w *= s;
                }
            }
            for v in [&mut *lead_trims, &mut *trail_trims] {
                for x in v.iter_mut() {
                    *x *= s;
                }
            }
            for x in body_expansions.iter_mut() {
                *x *= s;
            }
            s
        } else {
            1.0
        };
        self.append_trailing_widths(trailing, widths);
        lead_trims.resize(logical.len(), 0.0);
        trail_trims.resize(logical.len(), 0.0);
        BodyFit {
            hangs,
            is_justified,
            x_scale,
        }
    }

    fn try_wdth_condense(
        &self,
        body: &[LogicalPiece<'a>],
        ratio: f64,
        pending: &mut Reshapes,
    ) -> bool {
        let mut changed = false;
        for p in body {
            let PieceKind::Cluster { run, cluster } = p.piece.kind else {
                continue;
            };
            let c = self.inputs.cluster(cluster);
            let Some((min_w, def_w, _)) =
                self.shaper.variation_axis_range(c.font.font_id, *b"wdth")
            else {
                continue;
            };
            let mut r = self.inputs.run(run).clone();
            let cur_w = r
                .features
                .variations()
                .iter()
                .find(|v| v.tag() == *b"wdth")
                .map_or(def_w, |v| v.value());
            if min_w >= cur_w {
                continue;
            }
            let target = (cur_w * ratio as f32).clamp(min_w, cur_w);
            let Some(var) = crate::types::FontVariation::try_new(*b"wdth", target) else {
                continue;
            };
            r.features.push_variation(var);
            let req = crate::shaping::ReshapeRequest::plain(
                self.inputs.text(),
                &r,
                self.inputs.run_atoms_id(run),
                c.byte_range,
            );
            if let Some((glyphs, width)) =
                crate::middleend::shaper_gate::checked_reshape_window(self.shaper, &req, c)
            {
                pending.insert(
                    cluster,
                    super::Reshaped {
                        glyphs,
                        width: f64::from(width) * self.font_scale,
                        wdth: Some(target),
                    },
                );
                changed = true;
            }
        }
        changed
    }

    fn append_trailing_widths(&self, trailing: &[LogicalPiece<'_>], widths: &mut Vec<f64>) {
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        widths.extend(trailing.iter().map(|p| match p.piece.kind {
            PieceKind::Glue(glue)
                if glue.kind.is_synthetic() || (collapsible && glue.is_collapsible) =>
            {
                0.0
            }
            _ => p.piece.width,
        }));
    }

    fn measure_widths(
        &self,
        body: &[LogicalPiece<'a>],
        [widths, lead_trims, trail_trims]: [&mut Vec<f64>; 3],
        mut distance: f64,
        (no_post_pad, is_soft_wrap): (bool, bool),
        slot: InlineSlotConstraint,
        pending: &Reshapes,
    ) -> f64 {
        widths.clear();
        lead_trims.clear();
        trail_trims.clear();
        let mut prev_trim: Option<(usize, TrimInfo, f64)> = None;
        let mut has_content = false;
        for (idx, p) in body.iter().enumerate() {
            distance += p.gap;
            if p.gap != 0.0 {
                prev_trim = None;
            }
            let raw_width = self.measure(&p.piece, distance, slot, Some(pending)).0;
            let mut lead_trim = 0.0_f64;
            if !is_trim_transparent(&p.piece) {
                if let PieceKind::Cluster { run, cluster } = p.piece.kind {
                    let info = self.cluster_trim_info(run, cluster, raw_width);
                    if let Some(b) = info {
                        if !has_content && no_post_pad {
                            lead_trim = line_start_lead_trim(b, is_soft_wrap);
                        }
                        if let Some((prev_idx, a, a_trail)) = prev_trim {
                            let (t_a, l_b) = pair_trim(a, b);
                            let a_delta =
                                (t_a.min(a.class.max_trail(a.font_size)) - a_trail).max(0.0);
                            if a_delta > 0.0 {
                                if let Some(w) = widths.get_mut(prev_idx) {
                                    *w = (*w - a_delta).max(0.0);
                                }
                                if let Some(t) = trail_trims.get_mut(prev_idx) {
                                    *t += a_delta;
                                }
                                distance -= a_delta;
                            }
                            lead_trim = (lead_trim + l_b).min(b.class.max_lead(b.font_size));
                        }
                    }
                    prev_trim = info.map(|b| (idx, b, 0.0));
                } else {
                    prev_trim = None;
                }
            }
            if p.piece.is_content() {
                has_content = true;
            }
            let width = (raw_width - lead_trim).max(0.0);
            widths.push(width);
            lead_trims.push(lead_trim);
            trail_trims.push(0.0);
            distance += width;
        }
        distance
    }

    fn end_facts(&self, end: LineEnd<'a>) -> LineEndFacts<'a> {
        match end {
            LineEnd::At { token, brk } => LineEndFacts {
                last_piece: token,
                end_break: Some(brk),
                byte_end: brk.byte_range.end(),
                pre: f64::from(brk.pre_break_width),
            },
            LineEnd::Before(token) => LineEndFacts {
                last_piece: token,
                end_break: None,
                byte_end: self.tokens.at_or_sentinel(token).bytes().start(),
                pre: 0.0,
            },
        }
    }

    fn place_visual(
        &self,
        logical: &[LogicalPiece<'a>],
        order: &VisualOrder,
        widths: &[f64],
        xs: &mut Vec<f64>,
        pads: Pads,
        hangs: LineHangs,
    ) -> f64 {
        let rtl = self.base.is_rtl();
        let body_len = order.len();
        xs.clear();
        xs.resize(logical.len(), 0.0);
        let logical_t = order.table(logical);
        let widths_t = order.table(widths);
        let mut xs_t = order.table_mut(xs);
        let mut x = pads.left - hangs.physical(rtl).0;
        for k in order.indices() {
            x += logical_t.get(k).gap;
            xs_t.set(k, x);
            x += *widths_t.get(k);
        }
        let body_end = x + pads.right;
        let far_edge = if rtl {
            -hangs.physical(rtl).0
        } else {
            body_end
        };
        let (_, trailing_widths) = widths.split_at(body_len);
        let (_, trailing_xs) = xs.split_at_mut(body_len);
        self.place_trailing_whitespace(trailing_widths, trailing_xs, far_edge);
        body_end
    }

    fn apply_l1(&self, placed: &mut [LogicalPiece<'_>], levels: &mut Vec<BidiLevel>) {
        let class = |p: &LogicalPiece<'_>| match p.piece.kind {
            PieceKind::Tab => LineClass::Separator,
            PieceKind::Glue(_) | PieceKind::SpanEdge { .. } => LineClass::Whitespace,
            PieceKind::Cluster { cluster, .. }
                if self
                    .inputs
                    .cluster(cluster)
                    .flags
                    .contains(AtomFlags::L1_RESETTABLE) =>
            {
                LineClass::Whitespace
            }
            PieceKind::Cluster { .. } | PieceKind::Object(_) => LineClass::Other,
        };
        levels.clear();
        levels.extend(placed.iter().map(|p| p.piece.level));
        reset_line_levels(placed.iter().map(class), levels, self.base.base_level());
        for (p, &level) in placed.iter_mut().zip(levels.iter()) {
            if !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) {
                p.piece.level = level;
            }
        }
        for k in 0..placed.len() {
            let (before, rest) = placed.split_at_mut(k);
            let Some((current, after)) = rest.split_first_mut() else {
                continue;
            };
            let PieceKind::SpanEdge { span, edge } = current.piece.kind else {
                continue;
            };
            let span = self.inputs.decoration_span(span);
            let is_item = |p: &&LogicalPiece<'_>| {
                !matches!(p.piece.kind, PieceKind::SpanEdge { .. }) && !p.piece.bytes.is_empty()
            };
            let neighbour = match edge {
                SpanEdge::Leading => after.iter().find(is_item),
                SpanEdge::Trailing => before.iter().rev().find(is_item),
            };
            current.piece.level = neighbour
                .filter(|n| span.byte_range.contains_range(n.piece.bytes))
                .map_or_else(|| self.base.base_level(), |n| n.piece.level);
        }
    }

    fn collect_logical(
        &self,
        logical: &mut Vec<LogicalPiece<'a>>,
        start: TokenIdx,
        last_piece: TokenIdx,
    ) {
        logical.clear();
        let mut gap = 0.0;
        for token in self.tokens.slice(start, last_piece) {
            match token {
                Token::Piece(p) => {
                    logical.push(LogicalPiece { piece: *p, gap });
                    gap = 0.0;
                }
                Token::Break(b) => gap += f64::from(b.no_break_width),
            }
        }
    }

    fn place_trailing_whitespace(&self, widths: &[f64], xs: &mut [f64], far_edge: f64) {
        let rtl = self.base.is_rtl();
        let mut tx = far_edge;
        for (&width, x) in widths.iter().zip(xs) {
            if rtl {
                tx -= width;
                *x = tx;
            } else {
                *x = tx;
                tx += width;
            }
        }
    }

    fn build_line(
        &mut self,
        line: PlacedLine<'_, 'a>,
        line_box: &LineBoxMetrics,
        carets: &mut CaretScratch,
        geometry: &OpenGeometry,
        pending: &Reshapes,
        ellipsis: Option<Option<&SynthesizedEllipsis>>,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.first_slot();
        let (mut extent, visible_end) = self.visual_extent(&line, pending);
        if let Some(suffix) = ellipsis.flatten() {
            let (lo, hi) = if self.base.is_rtl() {
                (line.bounds.body_lo - suffix.width, line.bounds.body_lo)
            } else {
                (line.bounds.body_hi, line.bounds.body_hi + suffix.width)
            };
            extent = Some(extent.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        }
        let line = self.align_line_content(line, extent, slot);
        let runs = self.build_visual_runs(&line, pending);
        let runs = self.with_ellipsis(runs, &line, ellipsis.flatten());
        let content = &line.content;
        let content_edges = content.edges();
        let (visual_boxes, tabs) = self.build_boxes_and_tabs(line.placed, slot);
        let mut span_fragments = self.span_fragments(&line, line_box, slot);

        let mut decorations: Vec<DecorationLine> = Vec::new();
        if self.has_underline {
            self.decorations(DecorationKind::Underline, &line, &mut decorations)?;
        }
        if self.has_strikethrough {
            self.decorations(DecorationKind::Strikethrough, &line, &mut decorations)?;
        }
        let atomic_fragments = self.atomic_fragments(&line);
        clip_to_content(
            line.hangs.physical(self.base.is_rtl()),
            content_edges,
            &mut span_fragments,
            &mut decorations,
        );
        let caret_stops =
            self.caret_stops(&line, visible_end, &runs, &visual_boxes, pending, carets);

        let (first_slot, rest_slots) = geometry.split_slots();
        let slots: SmallVec<[PositionedSlot; 1]> = core::iter::once(PositionedSlot {
            constraint: first_slot,
            content_start: content.start as f32,
            visual_width: content.width as f32,
        })
        .chain(rest_slots.iter().map(|&constraint| PositionedSlot {
            constraint,
            content_start: constraint.start(),
            visual_width: 0.0,
        }))
        .collect();
        let baseline_y = line.block_top
            + if self.ast.writing_mode == WritingMode::VerticalLr {
                line_box.descent
            } else {
                line_box.ascent
            };
        Ok(PositionedLine {
            slots,
            is_ellipsized: ellipsis.is_some(),
            annotations: Vec::new(),
            atomic_fragments,
            byte_range: line.byte_range,
            visible_byte_end: visible_end,
            baseline_y: baseline_y as f32,
            ascent: line_box.ascent as f32,
            descent: line_box.descent as f32,
            line_height: line_box.line_height as f32,
            is_justified: line.is_justified,
            visual_runs: runs,
            visual_boxes,
            tabs,
            span_fragments,
            caret_stops,
            decorations,
        })
    }

    fn with_ellipsis(
        &self,
        mut runs: Vec<VisualGlyphRun>,
        line: &AlignedLine<'_, '_>,
        suffix: Option<&SynthesizedEllipsis>,
    ) -> Vec<VisualGlyphRun> {
        let Some(suffix) = suffix else {
            return runs;
        };
        let end = line.byte_range.end();
        let rtl = self.base.is_rtl();
        let mut pen = if rtl {
            line.content.start
        } else {
            line.content.start + line.content.width - suffix.width
        };
        let mut synth = suffix.runs.clone();
        if rtl {
            synth.reverse();
        }
        for r in &mut synth {
            r.byte_range = TextRange::empty_at(end);
            if rtl {
                r.glyphs.reverse();
            }
            for g in &mut r.glyphs {
                g.x = pen as f32;
                pen += f64::from(g.advance);
            }
        }
        let at = if rtl { 0 } else { runs.len() };
        runs.splice(at..at, synth);
        runs
    }

    fn align_line_content<'l>(
        &self,
        mut line: PlacedLine<'l, 'a>,
        extent: Option<(f64, f64)>,
        slot: InlineSlotConstraint,
    ) -> AlignedLine<'l, 'a> {
        let (mut lo, mut hi) = extent.unwrap_or((f64::INFINITY, f64::NEG_INFINITY));
        self.for_each_span_group(&line, |_, g, _| {
            lo = lo.min(g.lo);
            hi = hi.max(g.hi);
        });
        let rtl = self.base.is_rtl();
        let (hang_left, hang_right) = line.hangs.physical(rtl);
        let (content_lo, width) = if lo > hi {
            (if rtl { line.bounds.body_hi } else { 0.0 }, 0.0)
        } else if rtl {
            let lo = lo + hang_left;
            (lo, hi.max(line.bounds.body_hi) - hang_right - lo)
        } else {
            (0.0, hi - hang_right)
        };
        let (slot_start, slot_end, slot_width) = (
            f64::from(slot.start()),
            f64::from(slot.end()),
            f64::from(slot.width()),
        );
        let start = if rtl && slot_end.is_finite() && width <= slot_width {
            slot_end - width
        } else {
            slot_start
        };
        let shift = start - content_lo;
        for p in line.placed.iter_mut() {
            p.x += shift;
        }
        line.bounds.body_lo += shift;
        line.bounds.body_hi += shift;
        AlignedLine {
            line,
            content: ContentBlock { start, width },
        }
    }
}

pub(super) fn visual_order_into(
    body: &[LogicalPiece<'_>],
    levels: &mut Vec<BidiLevel>,
    order: &mut VisualOrder,
) {
    levels.clear();
    levels.extend(body.iter().map(|p| p.piece.level));
    order.rebuild(levels);
}

#[cfg(test)]
pub(super) fn visual_order(body: &[LogicalPiece<'_>], levels: &mut Vec<BidiLevel>) -> Vec<usize> {
    let mut order = VisualOrder::default();
    visual_order_into(body, levels, &mut order);
    order.as_raw().to_vec()
}
