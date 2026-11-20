//! Placement checks for a line's body: glyphs, inline boxes, tabs and
//! annotations (spec SLT-4, SLT-5, HNG-1, HNG-3).

use super::SourcedGlyphs;
use crate::frontend::{AnnotationSide, BoxOverflowPolicy};
use crate::middleend::line_geometry::BodyExtent;
use crate::middleend::{
    AnnotationErrorReason, GlyphRole, HangingWhitespaceMode, LineContext, LineHeight,
    LineInvariantError, PositionedGlyph, PositionedLine,
};

impl PositionedLine {
    /// Placement of glyphs (SLT-4, HNG-1, HNG-3): trailing whitespace on the
    /// far edge, hanging glyphs hanging from an outer slot. A body glyph is
    /// held by a slot by definition of its role.
    pub(super) fn check_glyph_placement(
        &self,
        ctx: &LineContext,
        body: &BodyExtent,
    ) -> Result<bool, LineInvariantError> {
        use LineInvariantError as E;
        let (first, last) = (&self.slots[body.first], &self.slots[body.last]);
        let collapsible = ctx.whitespace == HangingWhitespaceMode::Collapsible;
        let mut has_hanging = false;
        for (run, r) in self.visual_runs.iter().enumerate() {
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let (lo, hi) = g.pen_interval();
                match self.role_at(g, body.eps) {
                    GlyphRole::Trailing if g.advance != 0.0 => {
                        if collapsible {
                            return Err(E::TrailingWhitespaceNotCollapsed { run, glyph });
                        }
                        if !body.is_on_far_edge(ctx.base_direction, lo, hi) {
                            return Err(E::TrailingWhitespaceOnWrongSide { run, glyph });
                        }
                    }
                    GlyphRole::Trailing | GlyphRole::ZeroAdvance | GlyphRole::Body => {}
                    GlyphRole::Hanging => {
                        has_hanging = true;
                        // A mark hangs from the outer slot on its side:
                        // whatever of it is not overflow lies in that slot's
                        // content.
                        let over = body.overflow(lo, hi);
                        let hangs_from_first =
                            over.left > 0.0 && hi <= first.content_end() + body.eps;
                        let hangs_from_last =
                            over.right > 0.0 && lo >= last.content_start - body.eps;
                        if !body.is_outside_clear(lo, hi) || !(hangs_from_first || hangs_from_last)
                        {
                            return Err(E::BodyGlyphOutsideBlock { run, glyph });
                        }
                    }
                }
            }
        }
        Ok(has_hanging)
    }

    /// Placement of inline boxes, tabs and fragments (SLT-4, SLT-5, HNG-3).
    pub(super) fn check_object_placement(
        &self,
        ctx: &LineContext,
        body: &BodyExtent,
    ) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let collapsible = ctx.whitespace == HangingWhitespaceMode::Collapsible;
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if !self.is_held(b.x, b.x + b.width, body.eps) {
                return Err(E::InlineBoxOutsideBlock { index });
            }
            if b.overflow_policy != BoxOverflowPolicy::AllowOverflow
                && b.x + b.width > ctx.wrap_width + body.eps
            {
                return Err(E::InlineBoxWiderThanWrapWidth { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            if self.is_trailing(t.byte_range) {
                if collapsible && t.width != 0.0 {
                    return Err(E::TrailingTabNotCollapsed { index });
                }
                if t.width != 0.0 && !body.is_on_far_edge(ctx.base_direction, t.x, t.x + t.width) {
                    return Err(E::TrailingTabOnWrongSide { index });
                }
            } else if !self.is_held(t.x, t.x + t.width, body.eps) {
                return Err(E::TabOutsideBlock { index });
            }
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            if !self.is_held(f.x, f.x + f.width, body.eps) {
                return Err(E::SpanFragmentOutsideBlock { index });
            }
        }
        for (index, f) in self.atomic_fragments.iter().enumerate() {
            if !self.is_held(f.x, f.x + f.width, body.eps) {
                return Err(E::InvalidAtomicFragment { index });
            }
        }
        Ok(())
    }

    /// Each annotation sits over its base, in the base's slot.
    ///
    /// The base is the envelope of the non-synthesized body glyphs that
    /// intersect its source range. Glue and other glyph-less nodes may sit
    /// inside the base, so contiguous byte coverage is deliberately not
    /// required. Every intersecting glyph must belong wholly to the base and
    /// to one slot; a glyph straddling either boundary is a source-ownership
    /// error, not an invitation to clip it.
    pub(super) fn check_annotation_placement(
        &self,
        ctx: &LineContext,
        body: &BodyExtent,
        glyphs: Option<&SourcedGlyphs<'_>>,
    ) -> Result<(), LineInvariantError> {
        use AnnotationErrorReason as R;
        let Some(glyphs) = glyphs else {
            return Ok(());
        };
        let eps = body.eps;
        for (index, ann) in self.annotations.iter().enumerate() {
            let (mut base_lo, mut base_hi) = (f32::INFINITY, f32::NEG_INFINITY);
            for (range, (_, glyph)) in glyphs.overlapping(ann.base_byte_range) {
                if !ann.base_byte_range.contains_range(*range) {
                    return Err(R::GlyphStraddlesBase.at(index));
                }
                let (lo, hi) = glyph.pen_interval();
                (base_lo, base_hi) = (base_lo.min(lo), base_hi.max(hi));
            }
            if base_lo > base_hi {
                return Err(R::NoBaseGlyphs.at(index));
            }
            let Some(slot) = self.slots.iter().find(|slot| {
                base_lo >= slot.content_start - eps && base_hi <= slot.content_end() + eps
            }) else {
                return Err(R::BaseSpansSlots.at(index));
            };
            let lo = slot.content_start.max(base_lo - ann.max_overhang);
            let hi = slot.content_end().min(base_hi + ann.max_overhang);
            if ann
                .glyphs
                .iter()
                .map(PositionedGlyph::pen_interval)
                .any(|(glyph_lo, glyph_hi)| glyph_lo < lo - eps || glyph_hi > hi + eps)
            {
                return Err(R::OutsideOverhang.at(index));
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
                return Err(R::InsideBase.at(index));
            }
            // A fixed line box does not grow; the annotation overflows it.
            if matches!(ctx.line_height, LineHeight::Normal)
                && line_extent + eps < (base_extent + ann.block_extent).max(strut_extent)
            {
                return Err(R::DoesNotFitLineBox.at(index));
            }
        }
        Ok(())
    }
}
