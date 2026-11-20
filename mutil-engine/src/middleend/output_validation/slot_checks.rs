//! Slot constraint, containment, hanging, overflow, and span-fragment checks.

use crate::frontend::BoxOverflowPolicy;
use crate::middleend::line_geometry::BodyExtent;
use crate::middleend::options::slots_are_valid;
use crate::middleend::tolerance::{approx_eq, tolerance};
use crate::middleend::{
    GlyphRole, HangEndMode, HangSide, LineContext, LineInvariantError, PositionedGlyph,
    PositionedLine, SlotErrorReason,
};
use crate::types::{BidiDirection, TextRange};

impl PositionedLine {
    /// The slots. Each constraint lies inside the wrap envelope and right of
    /// the previous one; each content block is finite and starts inside its
    /// constraint. Whether a block also *ends* inside its constraint is the
    /// overflow rule of `check_slot_overflow`.
    ///
    /// Returns the body, or `None` when no slot holds content.
    pub(super) fn check_slots(
        &self,
        ctx: &LineContext,
    ) -> Result<Option<BodyExtent>, LineInvariantError> {
        use SlotErrorReason as R;
        if ctx.wrap_width.is_nan() || ctx.wrap_width <= 0.0 {
            return Err(R::InvalidWrapWidth.error());
        }
        if self.slots.is_empty() {
            return Err(R::NoSlots.error());
        }
        let constraints: smallvec::SmallVec<[_; 2]> =
            self.slots.iter().map(|slot| slot.constraint).collect();
        if !slots_are_valid(&constraints, ctx.wrap_width) {
            return Err(R::InvalidConstraint.error());
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
                return Err(R::InvalidContent.error());
            }
        }
        Ok(BodyExtent::of(self))
    }

    /// A line without a body holds nothing outside its slots.
    pub(super) fn check_nothing_outside_slots(&self) -> Result<(), LineInvariantError> {
        let has_nothing = self.visual_runs.iter().all(|r| r.glyphs.is_empty())
            && self.visual_boxes.is_empty()
            && self.tabs.is_empty()
            && self.span_fragments.is_empty()
            && self.atomic_fragments.is_empty()
            && self.annotations.is_empty();
        if has_nothing {
            Ok(())
        } else {
            Err(SlotErrorReason::ContentOutsideSlots.error())
        }
    }

    /// Hanging marks: one per side, outermost, on a side the mode allows
    /// (HNG-2, HNG-4). Skipped when `check_glyph_placement` proved no glyph
    /// hangs.
    pub(super) fn check_hanging(
        &self,
        ctx: &LineContext,
        body: &BodyExtent,
        has_hanging: bool,
    ) -> Result<(), LineInvariantError> {
        if !has_hanging {
            return Ok(());
        }
        self.check_hanging_marks(body)?;
        self.check_hanging_sides(ctx, body)
    }

    /// A slot's content ends inside its constraint, unless the line is one
    /// slot holding, from the slot's start, a single entity that cannot be
    /// divided and is wider than the slot (`is_single_entity_overflow`). The
    /// slot need not be as wide as the wrap width: an indented line, or a
    /// provider's narrow slot, places an entity that fits nowhere the same way
    /// (spec SLT-5).
    pub(super) fn check_slot_overflow(
        &self,
        ctx: &LineContext,
        body: &BodyExtent,
    ) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let visual_width = self.visual_width();
        let line_overflows = visual_width > ctx.wrap_width + body.eps;
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
                return Err(if line_overflows {
                    E::LineWiderThanWrapWidth {
                        visual_width,
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
    /// it (see [`PositionedSlot`](crate::middleend::PositionedSlot)): every
    /// item lies inside the block, the item nearest the **line end** ends on
    /// the block's edge, and only the line-start edge may stand off, by the
    /// width of preserved leading glue. A slot without content has
    /// `visual_width == 0`.
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
    pub(super) fn check_slot_spans(
        &self,
        base_direction: BidiDirection,
    ) -> Result<(), LineInvariantError> {
        let slots = &self.slots;
        let eps = self.eps();
        let mut spans: smallvec::SmallVec<[Option<(f32, f32)>; 2]> = smallvec::SmallVec::new();
        spans.resize(slots.len(), None);
        let owner = |lo: f32, hi: f32| slots.iter().position(|s| s.holds(lo, hi, eps));
        let mut extend = |slot: usize, lo: f32, hi: f32| {
            let (a, b) = spans[slot].get_or_insert((lo, hi));
            *a = a.min(lo);
            *b = b.max(hi);
        };
        for g in self.glyphs() {
            let (lo, hi) = g.pen_interval();
            match self.role_at(g, eps) {
                GlyphRole::Trailing | GlyphRole::ZeroAdvance => {}
                GlyphRole::Body => {
                    if let Some(slot) = owner(lo, hi) {
                        extend(slot, lo, hi);
                    }
                }
                // Only the part inside a slot's content counts, for that slot.
                GlyphRole::Hanging => {
                    for (slot, s) in slots.iter().enumerate() {
                        let (lo, hi) = (lo.max(s.content_start), hi.min(s.content_end()));
                        if lo < hi {
                            extend(slot, lo, hi);
                        }
                    }
                }
            }
        }
        // Inline boxes, tabs that are not trailing, and the span fragments: the
        // padding of a decoration span is advance of the body that no glyph or
        // box covers, and the fragment, which includes it, is part of the span.
        let objects = self
            .visual_boxes
            .iter()
            .map(|b| (b.x, b.x + b.width))
            .chain(
                self.tabs
                    .iter()
                    .filter(|t| !self.is_trailing(t.byte_range))
                    .map(|t| (t.x, t.x + t.width)),
            )
            .chain(self.span_fragments.iter().map(|f| (f.x, f.x + f.width)));
        for (lo, hi) in objects {
            if let Some(slot) = owner(lo, hi) {
                extend(slot, lo, hi);
            }
        }
        for (slot, span) in slots.iter().zip(spans) {
            let is_content_span = match span {
                None => approx_eq(slot.visual_width, 0.0),
                // `holds` already keeps the content inside the block.
                Some((lo, hi)) => match base_direction {
                    BidiDirection::Ltr => approx_eq(hi, slot.content_end()),
                    BidiDirection::Rtl => approx_eq(lo, slot.content_start),
                },
            };
            if !is_content_span {
                return Err(SlotErrorReason::WidthNotContentSpan.error());
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
                let eps = self.eps();
                let glyphs = || {
                    self.glyphs().filter(|g| {
                        self.role_at(g, eps).is_inside_body()
                            && (!g.is_synthesized() || self.is_ellipsized)
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
        body: &BodyExtent,
    ) -> Result<(), LineInvariantError> {
        let extents = self.hang_extents(body);
        for (side, expected, slot) in [
            (HangSide::Left, extents.left, &self.slots[body.first]),
            (HangSide::Right, extents.right, &self.slots[body.last]),
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
    /// that side.
    fn check_hanging_marks(&self, body: &BodyExtent) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let roled = || {
            self.visual_runs.iter().enumerate().flat_map(|(run, r)| {
                r.glyphs
                    .iter()
                    .enumerate()
                    .map(move |(glyph, g)| (run, glyph, g, self.role_at(g, body.eps)))
            })
        };
        for side in [HangSide::Left, HangSide::Right] {
            // How far a pen interval reaches towards `side`.
            let reach = |g: &PositionedGlyph| {
                let (lo, hi) = g.pen_interval();
                if side == HangSide::Left { -lo } else { hi }
            };
            let edge = if side == HangSide::Left {
                -body.left
            } else {
                body.right
            };
            let hanging: Vec<_> = roled()
                .filter(|(_, _, g, role)| *role == GlyphRole::Hanging && reach(g) > edge + body.eps)
                .collect();
            let Some(&(run, glyph, mark, _)) = hanging
                .iter()
                .max_by(|a, b| reach(a.2).total_cmp(&reach(b.2)))
            else {
                continue;
            };
            if hanging
                .iter()
                .any(|(_, _, g, _)| g.byte_range != mark.byte_range)
            {
                return Err(E::TooManyHangingGlyphs { side });
            }
            // Every other glyph that is not itself hanging counts,
            // zero-advance ones included; the mark's own glyphs do not.
            let other_reaches_further = roled().any(|(_, _, g, role)| {
                role.is_inside_body()
                    && g.byte_range != mark.byte_range
                    && reach(g) > reach(mark) + body.eps
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
    pub(super) fn check_span_fragments(&self) -> Result<(), LineInvariantError> {
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
        let visual_width = self.visual_width();
        let eps = tolerance(visual_width, 0.0);
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let Some(range) = g.byte_range else {
                    continue;
                };
                if !self.role_at(g, eps).is_inside_body() {
                    continue;
                }
                let (lo, hi) = g.pen_interval();
                if let Some(span_id) = self.uncovered_span(range, lo, hi, visual_width) {
                    return Err(E::GlyphOutsideSpanFragment {
                        span_id,
                        run,
                        glyph,
                    });
                }
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if let Some(span_id) =
                self.uncovered_span(b.byte_range, b.x, b.x + b.width, visual_width)
            {
                return Err(E::InlineBoxOutsideSpanFragment { span_id, index });
            }
        }
        Ok(())
    }

    /// The first span owning `range` (some fragment of it covers `range` in
    /// bytes) none of whose fragments contains `[lo, hi]` horizontally.
    fn uncovered_span(&self, range: TextRange, lo: f32, hi: f32, vw: f32) -> Option<u64> {
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
