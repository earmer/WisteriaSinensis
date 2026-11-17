//! Slot constraint, containment, hanging, overflow, and span-fragment checks.

use super::super::options::slots_are_valid;
use super::super::tolerance::{approx_eq, tolerance};
use super::super::{
    AnnotationErrorReason, HangEndMode, HangSide, HangingWhitespaceMode, LineContext, LineHeight,
    LineInvariantError, PositionedGlyph, PositionedLine, SlotErrorReason,
};
use super::{SourcedGlyphs, slot_holds};
use crate::frontend::{AnnotationSide, BoxOverflowPolicy};
use crate::types::{BidiDirection, TextRange};

impl PositionedLine {
    /// Pass 4: the slots. Each constraint lies inside the wrap envelope and
    /// right of the previous one; each content block is finite and starts
    /// inside its constraint.
    /// Whether a block also *ends* inside its constraint is the overflow rule
    /// of pass 5.
    pub(super) fn check_slots(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use SlotErrorReason as R;
        let invalid = |reason| Err(LineInvariantError::InvalidSlots { reason });
        if ctx.wrap_width.is_nan() || ctx.wrap_width <= 0.0 {
            return invalid(R::InvalidWrapWidth);
        }
        if self.slots.is_empty() {
            return invalid(R::NoSlots);
        }
        let constraints: smallvec::SmallVec<[_; 2]> =
            self.slots.iter().map(|slot| slot.constraint).collect();
        if !slots_are_valid(&constraints, ctx.wrap_width) {
            return invalid(R::InvalidConstraint);
        }
        for slot in &self.slots {
            let eps = tolerance(slot.content_start, slot.constraint.start());
            if !(slot.content_start.is_finite()
                && slot.visual_width.is_finite()
                && slot.visual_width >= 0.0
                && slot.content_end().is_finite()
                && slot.content_start >= slot.constraint.start() - eps
                && slot.content_start <= slot.constraint.end() + eps)
            {
                return invalid(R::InvalidContent);
            }
        }
        Ok(())
    }

    /// Pass 5: containment of glyphs, boxes, tabs and fragments in the slots'
    /// content; hanging; trailing whitespace; annotations; overflow.
    pub(super) fn check_containment_and_hanging(
        &self,
        ctx: &LineContext,
        glyphs: &SourcedGlyphs<'_>,
    ) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let slots = &self.slots;
        let Some(occupied) = self.occupied_slots() else {
            let has_nothing = self.visual_runs.iter().all(|r| r.glyphs.is_empty())
                && self.visual_boxes.is_empty()
                && self.tabs.is_empty()
                && self.span_fragments.is_empty()
                && self.atomic_fragments.is_empty()
                && self.annotations.is_empty();
            return if has_nothing {
                Ok(())
            } else {
                Err(E::InvalidSlots {
                    reason: SlotErrorReason::ContentOutsideSlots,
                })
            };
        };
        let (first_index, last_index) = occupied;
        let (first, last) = (&slots[first_index], &slots[last_index]);
        // The body: from the left edge of the first occupied slot's content to
        // the right edge of the last one's.
        let (left, right) = (first.content_start, last.content_end());
        let eps = tolerance(self.visual_width(), 0.0);
        let inside = |lo: f32, hi: f32| slots.iter().any(|s| slot_holds(s, lo, hi));
        // Hanging marks and preserved whitespace lie outside the body, and
        // never in an internal obstacle or an unused slot.
        let is_outside_clear = |lo: f32, hi: f32| {
            (lo >= left - eps || first_index == 0)
                && (hi <= right + eps || last_index + 1 == slots.len())
        };
        // UAX #9 L1: trailing whitespace is on the far edge of the base direction.
        let is_on_far_edge = |lo: f32, hi: f32| {
            is_outside_clear(lo, hi)
                && match ctx.base_direction {
                    BidiDirection::Ltr => lo >= right - eps,
                    BidiDirection::Rtl => hi <= left + eps,
                }
        };
        let collapsible = ctx.whitespace == HangingWhitespaceMode::Collapsible;
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let (lo, hi) = g.pen_interval();
                if self.is_trailing_whitespace(g) {
                    if g.is_hanging {
                        return Err(E::TrailingWhitespaceMarkedHanging { run, glyph });
                    }
                    if collapsible && g.advance != 0.0 {
                        return Err(E::TrailingWhitespaceNotCollapsed { run, glyph });
                    }
                    if g.advance != 0.0 && !is_on_far_edge(lo, hi) {
                        return Err(E::TrailingWhitespaceOnWrongSide { run, glyph });
                    }
                } else if g.advance == 0.0 {
                    continue;
                } else if g.is_hanging {
                    let over_left = (left - lo).max(0.0);
                    let over_right = (hi - right).max(0.0);
                    if over_left <= eps && over_right <= eps {
                        return Err(E::HangingGlyphInsideBlock { run, glyph });
                    }
                    // A mark hangs from the outer slot on its side: whatever
                    // of it is not overflow lies in that slot's content.
                    let hangs_from_first = over_left > eps && hi <= first.content_end() + eps;
                    let hangs_from_last = over_right > eps && lo >= last.content_start - eps;
                    if !is_outside_clear(lo, hi) || !(hangs_from_first || hangs_from_last) {
                        return Err(E::BodyGlyphOutsideBlock { run, glyph });
                    }
                } else if !inside(lo, hi) {
                    return Err(E::BodyGlyphOutsideBlock { run, glyph });
                }
            }
        }
        self.check_hanging_marks((left, right), eps)?;
        self.check_hanging_sides(ctx, occupied)?;
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if !inside(b.x, b.x + b.width) {
                return Err(E::InlineBoxOutsideBlock { index });
            }
            if b.overflow_policy != BoxOverflowPolicy::AllowOverflow
                && b.x + b.width > ctx.wrap_width + eps
            {
                return Err(E::InlineBoxWiderThanWrapWidth { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            if t.byte_range.start() >= self.visible_byte_end {
                if collapsible && t.width != 0.0 {
                    return Err(E::TrailingTabNotCollapsed { index });
                }
                if t.width != 0.0 && !is_on_far_edge(t.x, t.x + t.width) {
                    return Err(E::TrailingTabOnWrongSide { index });
                }
            } else if !inside(t.x, t.x + t.width) {
                return Err(E::TabOutsideBlock { index });
            }
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            if !inside(f.x, f.x + f.width) {
                return Err(E::SpanFragmentOutsideBlock { index });
            }
        }
        for (index, f) in self.atomic_fragments.iter().enumerate() {
            if !inside(f.x, f.x + f.width) {
                return Err(E::InvalidAtomicFragment { index });
            }
        }
        self.check_annotation_placement(ctx, glyphs)?;
        self.check_span_fragments()?;
        self.check_slot_overflow(ctx)?;
        self.check_slot_spans(ctx.base_direction)
    }

    /// Each annotation sits over its base, in the base's slot.
    ///
    /// The base is the envelope of the non-synthesized body glyphs that
    /// intersect its source range. Glue and other glyph-less nodes may sit
    /// inside the base, so contiguous byte coverage is deliberately not
    /// required. Every intersecting glyph must belong wholly to the base and
    /// to one slot; a glyph straddling either boundary is a source-ownership
    /// error, not an invitation to clip it.
    fn check_annotation_placement(
        &self,
        ctx: &LineContext,
        glyphs: &SourcedGlyphs<'_>,
    ) -> Result<(), LineInvariantError> {
        use AnnotationErrorReason as R;
        let eps = tolerance(self.visual_width(), 0.0);
        for (index, ann) in self.annotations.iter().enumerate() {
            let invalid = |reason| Err(LineInvariantError::InvalidAnnotation { index, reason });
            let (mut base_lo, mut base_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for (range, (_, glyph)) in glyphs.overlapping(ann.base_byte_range) {
                if !ann.base_byte_range.contains_range(*range) {
                    return invalid(R::GlyphStraddlesBase);
                }
                let (lo, hi) = glyph.pen_interval();
                (base_lo, base_hi) = (base_lo.min(lo), base_hi.max(hi));
            }
            if base_lo > base_hi {
                return invalid(R::NoBaseGlyphs);
            }
            let Some(slot) = self.slots.iter().find(|slot| {
                base_lo >= slot.content_start - eps && base_hi <= slot.content_end() + eps
            }) else {
                return invalid(R::BaseSpansSlots);
            };
            let lo = slot.content_start.max(base_lo - ann.max_overhang);
            let hi = slot.content_end().min(base_hi + ann.max_overhang);
            if ann
                .glyphs
                .iter()
                .map(PositionedGlyph::pen_interval)
                .any(|(glyph_lo, glyph_hi)| glyph_lo < lo - eps || glyph_hi > hi + eps)
            {
                return invalid(R::OutsideOverhang);
            }
            let (base_extent, line_extent, strut_extent, clears_base) = match ann.side {
                AnnotationSide::Over => (
                    ann.base_over,
                    self.ascent,
                    ctx.strut.ascent,
                    ann.baseline_offset >= ann.base_over - eps,
                ),
                AnnotationSide::Under => (
                    ann.base_under,
                    self.descent,
                    ctx.strut.descent,
                    ann.baseline_offset <= -ann.base_under + eps,
                ),
            };
            if !clears_base {
                return invalid(R::InsideBase);
            }
            // A fixed line box does not grow; the annotation overflows it.
            if matches!(ctx.line_height, LineHeight::Normal)
                && line_extent + eps < (base_extent + ann.block_extent).max(strut_extent)
            {
                return invalid(R::DoesNotFitLineBox);
            }
        }
        Ok(())
    }

    /// A slot's content ends inside its constraint, unless the line is one
    /// slot holding, from the slot's start, a single entity that cannot be
    /// divided and is wider than the slot (`is_single_entity_overflow`). The
    /// slot need not be as wide as the wrap width: an indented line, or a
    /// provider's narrow slot, places an entity that fits nowhere the same way
    /// (spec SLT-5).
    fn check_slot_overflow(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        for (index, slot) in self.slots.iter().enumerate() {
            let eps = tolerance(slot.visual_width, 0.0);
            let capacity = slot.constraint.width();
            let overflows = slot.content_end() > slot.constraint.end() + eps;
            // The exemption is for an entity that cannot fit: wider than the
            // slot, and not pushed out of it by an alignment offset.
            let is_exempt = self.slots.len() == 1
                && slot.visual_width > capacity + eps
                && approx_eq(slot.content_start, slot.constraint.start())
                && self.is_single_entity_overflow(slot.content_start);
            if overflows && !is_exempt {
                // The whole line is wider than the container, or only this
                // slot is wider than its own capacity.
                let line_overflows =
                    self.visual_width() > ctx.wrap_width + tolerance(self.visual_width(), 0.0);
                return Err(if line_overflows {
                    E::LineWiderThanWrapWidth {
                        visual_width: self.visual_width(),
                        wrap_width: ctx.wrap_width,
                    }
                } else {
                    E::SlotOverflow {
                        slot: index,
                        extent: slot.content_end() - slot.constraint.start(),
                        capacity,
                    }
                });
            }
        }
        Ok(())
    }

    /// Each slot's content block is exactly the span of the content placed in
    /// it (see [`PositionedSlot`]): every item lies inside the block, the
    /// item nearest the **line end** ends on the block's edge, and only the
    /// line-start edge may stand off, by the width of preserved leading glue.
    /// A slot without content has `visual_width == 0`.
    ///
    /// Content is the body glyphs (pen `x` to `x + advance`), inline boxes,
    /// tabs and span fragments (which carry the padding of a decoration span,
    /// advance that no glyph covers) that fit the slot.
    ///
    /// Not content: trailing whitespace and trailing tabs (they lie outside the
    /// body under `Preserved` and have no width under `Collapsible`),
    /// zero-advance glyphs (a pen position, not an extent) and hanging
    /// punctuation. A hanging glyph counts only through the part of it that lies
    /// inside the slot's content, so a mark that straddles the edge is content of
    /// the slot it hangs from and one entirely outside is not.
    fn check_slot_spans(&self, base_direction: BidiDirection) -> Result<(), LineInvariantError> {
        let slots = &self.slots;
        let mut spans: Vec<Option<(f32, f32)>> = vec![None; slots.len()];
        let mut place = |lo: f32, hi: f32| {
            if let Some(i) = slots.iter().position(|s| slot_holds(s, lo, hi)) {
                let (a, b) = spans[i].get_or_insert((lo, hi));
                *a = a.min(lo);
                *b = b.max(hi);
            }
        };
        for g in self.visual_runs.iter().flat_map(|r| &r.glyphs) {
            if self.is_trailing_whitespace(g) || g.advance == 0.0 {
                continue;
            }
            let (lo, hi) = g.pen_interval();
            if g.is_hanging {
                for s in slots {
                    let (lo, hi) = (lo.max(s.content_start), hi.min(s.content_end()));
                    if lo < hi {
                        place(lo, hi);
                    }
                }
            } else {
                place(lo, hi);
            }
        }
        for b in &self.visual_boxes {
            place(b.x, b.x + b.width);
        }
        for t in self
            .tabs
            .iter()
            .filter(|t| t.byte_range.start() < self.visible_byte_end)
        {
            place(t.x, t.x + t.width);
        }
        // The padding of a decoration span is advance of the body that no glyph
        // or box covers: the fragment, which includes it, is part of the span.
        for f in &self.span_fragments {
            place(f.x, f.x + f.width);
        }
        for (slot, span) in slots.iter().zip(spans) {
            let is_content_span = match span {
                None => approx_eq(slot.visual_width, 0.0),
                // `slot_holds` already keeps the content inside the block.
                Some((lo, hi)) => match base_direction {
                    BidiDirection::Ltr => approx_eq(hi, slot.content_end()),
                    BidiDirection::Rtl => approx_eq(lo, slot.content_start),
                },
            };
            if !is_content_span {
                return Err(LineInvariantError::InvalidSlots {
                    reason: SlotErrorReason::WidthNotContentSpan,
                });
            }
        }
        Ok(())
    }

    /// The single-entity overflow exemptions (spec §2.6, Pass 5 (h)), for a
    /// line whose one slot's content starts at `origin`.
    ///
    /// **A:** nothing but one `AllowOverflow` box (no visual runs, no tabs).
    /// **B:** nothing but the glyphs of one indivisible cluster (a shared
    /// non-empty `byte_range`; no boxes, no tabs). The entity spans
    /// `[entity_start_x, entity_end_x]` and is `entity_width` wide.
    ///
    /// * No span fragments: the entity starts at `origin` and `visual_width`
    ///   equals `entity_width`.
    /// * Span fragments: they are the padding around the one entity. Every
    ///   fragment starts at the entity's start and ends at or after its end;
    ///   the entity ends at or after `visible_byte_end`, so the extra bytes lie
    ///   in the trailing whitespace (`check_span_fragments` keeps fragments
    ///   inside the line). The entity lies inside the body and the fragments
    ///   reach both edges of the body (`min x == origin`, `max end == origin +
    ///   visual_width`). Unpadded entities cannot inflate `visual_width` this
    ///   way.
    fn is_single_entity_overflow(&self, origin: f32) -> bool {
        let vw = self.visual_width();
        if !self.tabs.is_empty() {
            return false;
        }
        // (byte_range, start_x, end_x, entity_width)
        let (range, start, end, width) = match self.visual_boxes.as_slice() {
            [b] => {
                if !(self.visual_runs.is_empty()
                    && b.overflow_policy == BoxOverflowPolicy::AllowOverflow)
                {
                    return false;
                }
                (b.byte_range, b.x, b.x + b.width, b.width)
            }
            [] => {
                let glyphs = || {
                    self.visual_runs
                        .iter()
                        .flat_map(|r| r.glyphs.iter())
                        .filter(|g| {
                            !g.is_hanging
                                && (!g.is_synthesized() || self.is_ellipsized)
                                && g.byte_range
                                    .is_none_or(|b| b.start() < self.visible_byte_end)
                        })
                };
                let mut ranges = glyphs()
                    .filter(|g| !g.is_synthesized())
                    .map(|g| g.byte_range);
                let first = ranges.next().unwrap_or_else(|| {
                    self.is_ellipsized
                        .then_some(TextRange::empty_at(self.visible_byte_end))
                });
                let Some(r) = first else {
                    return false;
                };
                if (r.is_empty() && !self.is_ellipsized) || !ranges.all(|o| o == first) {
                    return false;
                }
                let start = glyphs().map(|g| g.x).fold(f32::INFINITY, f32::min);
                let end = glyphs()
                    .map(|g| g.x + g.advance)
                    .fold(f32::NEG_INFINITY, f32::max);
                let width = glyphs().map(|g| g.advance).sum::<f32>();
                if !approx_eq(width, end - start) {
                    return false;
                }
                (r, start, end, width)
            }
            _ => return false,
        };
        if self.span_fragments.is_empty() {
            return approx_eq(start, origin) && approx_eq(width, vw);
        }
        let left = self
            .span_fragments
            .iter()
            .map(|f| f.x)
            .fold(f32::INFINITY, f32::min);
        let right = self
            .span_fragments
            .iter()
            .map(|f| f.x + f.width)
            .fold(f32::NEG_INFINITY, f32::max);
        self.span_fragments
            .iter()
            .all(|f| f.byte_range.start() == range.start() && f.byte_range.end() >= range.end())
            && range.end() >= self.visible_byte_end
            && start >= origin - tolerance(width, vw)
            && end <= origin + vw + tolerance(width, vw)
            && approx_eq(left, origin)
            && approx_eq(right - origin, vw)
    }

    /// HNG-2: each side with a hanging extent is one the mode allows, and
    /// under `Allow` the line end hangs only where the mark would not fit.
    fn check_hanging_sides(
        &self,
        ctx: &LineContext,
        occupied: (usize, usize),
    ) -> Result<(), LineInvariantError> {
        let (hanging_left, hanging_right) = self.hanging_extents(&occupied);
        for (side, expected, slot) in [
            (HangSide::Left, hanging_left, &self.slots[occupied.0]),
            (HangSide::Right, hanging_right, &self.slots[occupied.1]),
        ] {
            if expected == 0.0 {
                continue;
            }
            if !ctx
                .hang_mode
                .allows(side, ctx.base_direction, ctx.line_index == 0)
            {
                return Err(LineInvariantError::HangingNotAllowed { side });
            }
            // `Allow` hangs a mark only if it would otherwise wrap: put back
            // into the body, it would not fit the slot it hangs from.
            let at_line_end = !side.is_line_start(ctx.base_direction);
            let capacity = slot.constraint.width();
            let would_fit = capacity == f32::INFINITY
                || slot.visual_width + expected <= capacity - tolerance(capacity, 0.0);
            if at_line_end && ctx.hang_mode.end == HangEndMode::Allow && would_fit {
                return Err(LineInvariantError::HangingWithoutOverflow { side });
            }
        }
        Ok(())
    }

    /// HNG-4: on each side at most one character hangs, and it is the
    /// outermost glyph there: no other body glyph reaches further towards
    /// that side. `(left, right)` are the body's edges.
    fn check_hanging_marks(
        &self,
        (left, right): (f32, f32),
        eps: f32,
    ) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let glyphs = || {
            self.visual_runs.iter().enumerate().flat_map(|(run, r)| {
                r.glyphs
                    .iter()
                    .enumerate()
                    .filter(|(_, g)| g.advance != 0.0 && !self.is_trailing_whitespace(g))
                    .map(move |(glyph, g)| (run, glyph, g))
            })
        };
        for side in [HangSide::Left, HangSide::Right] {
            // How far a pen interval reaches towards `side`.
            let reach = |g: &PositionedGlyph| {
                let (lo, hi) = g.pen_interval();
                if side == HangSide::Left { -lo } else { hi }
            };
            let edge = if side == HangSide::Left { -left } else { right };
            let hanging: Vec<_> = glyphs()
                .filter(|(_, _, g)| g.is_hanging && reach(g) > edge + eps)
                .collect();
            let Some(&(run, glyph, mark)) = hanging
                .iter()
                .max_by(|a, b| reach(a.2).total_cmp(&reach(b.2)))
            else {
                continue;
            };
            if hanging
                .iter()
                .any(|(_, _, g)| g.byte_range != mark.byte_range)
            {
                return Err(E::TooManyHangingGlyphs { side });
            }
            // Every other body glyph counts, zero-advance ones included; the
            // mark's own glyphs do not.
            let other_reaches_further = self.visual_runs.iter().flat_map(|r| &r.glyphs).any(|g| {
                !g.is_hanging
                    && g.byte_range != mark.byte_range
                    && !self.is_trailing_whitespace(g)
                    && reach(g) > reach(mark) + eps
            });
            if other_reaches_further {
                return Err(E::HangingGlyphNotOutermost { run, glyph });
            }
        }
        Ok(())
    }

    /// Span fragments (spec §2.6, Pass 5 (g)). Each fragment covers a part of
    /// its span inside the line (its first / last edges follow from the two
    /// ranges). Every glyph
    /// or inline box that belongs to a span lies inside one of its fragments.
    fn check_span_fragments(&self) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        if self.span_fragments.is_empty() {
            return Ok(());
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            if !self.owns_bytes(f.byte_range) {
                return Err(E::SpanFragmentOutsideLine { index });
            }
            if !f.span_range.contains_range(f.byte_range) {
                return Err(E::SpanFragmentOutsideSpan { index });
            }
        }
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let Some(range) = g.byte_range else {
                    continue;
                };
                if g.is_hanging || self.is_trailing_whitespace(g) {
                    continue;
                }
                let (lo, hi) = g.pen_interval();
                if let Some(span_id) = self.uncovered_span(range, lo, hi) {
                    return Err(E::GlyphOutsideSpanFragment {
                        span_id,
                        run,
                        glyph,
                    });
                }
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if let Some(span_id) = self.uncovered_span(b.byte_range, b.x, b.x + b.width) {
                return Err(E::InlineBoxOutsideSpanFragment { span_id, index });
            }
        }
        Ok(())
    }

    /// The first span owning `range` (some fragment of it covers `range` in
    /// bytes) none of whose fragments contains `[lo, hi]` horizontally.
    fn uncovered_span(&self, range: TextRange, lo: f32, hi: f32) -> Option<u64> {
        let vw = self.visual_width();
        self.span_fragments
            .iter()
            .filter(|f| f.byte_range.contains_range(range))
            .map(|f| f.span_id)
            .find(|&id| {
                !self.span_fragments.iter().any(|f| {
                    let tol = tolerance(f.x.abs().max((f.x + f.width).abs()), vw);
                    f.span_id == id && lo >= f.x - tol && hi <= f.x + f.width + tol
                })
            })
    }
}
