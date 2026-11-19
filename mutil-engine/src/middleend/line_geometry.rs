//! Geometry derived from a [`PositionedLine`]: its body width, the slots that
//! hold its body, the role of each glyph and the hanging extents (spec OUT-4).
//!
//! Nothing here is stored on the line (AX-2). The output gate and the
//! renderer-facing queries read the same definitions.

use crate::middleend::tolerance::tolerance;
use crate::middleend::{PositionedGlyph, PositionedLine, PositionedSlot};
use crate::types::{BidiDirection, TextRange};

/// What a glyph is to the body of its line, decided by [`PositionedLine::role`]
/// in this precedence (spec OUT-4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphRole {
    /// Trailing whitespace: source text at or after `visible_byte_end`.
    Trailing,
    /// A pen position, not an extent.
    ZeroAdvance,
    /// Hanging punctuation: no slot's content block holds its pen interval.
    Hanging,
    /// Content held by a slot's content block.
    Body,
}

impl GlyphRole {
    /// `true` for a glyph that is neither hanging nor trailing whitespace:
    /// what the body of the line is made of, zero-advance glyphs included.
    pub(crate) const fn is_inside_body(self) -> bool {
        matches!(self, Self::ZeroAdvance | Self::Body)
    }
}

/// The overflow of hanging glyphs beyond the body's edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct HangExtents {
    pub(crate) left: f32,
    pub(crate) right: f32,
}

/// The body of a line: from the left edge of the first occupied slot's
/// content to the right edge of the last one's, with the tolerance of the
/// line's width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BodyExtent {
    /// Index of the first occupied slot.
    pub(crate) first: usize,
    /// Index of the last occupied slot.
    pub(crate) last: usize,
    /// Left edge of the first occupied slot's content.
    pub(crate) left: f32,
    /// Right edge of the last occupied slot's content.
    pub(crate) right: f32,
    /// [`tolerance`] at the scale of the line's body width.
    pub(crate) eps: f32,
    /// Number of slots of the line.
    slot_count: usize,
}

impl PositionedSlot {
    /// Whether `[lo, hi]` lies in this slot's content block, within `eps`. The
    /// one rule that decides both whether content belongs to a slot and whether
    /// the slot counts as occupied. `eps` is the line's
    /// ([`PositionedLine::eps`]), so that a glyph's role and its overflow
    /// beyond the body are judged at the same scale.
    pub(crate) fn holds(&self, lo: f32, hi: f32, eps: f32) -> bool {
        lo >= self.content_start - eps && hi <= self.content_end() + eps
    }
}

impl BodyExtent {
    /// The body of `line`, or `None` when no slot holds content: a slot is
    /// occupied when it has a non-zero block, or holds a visible glyph with
    /// source bytes, an inline box or a visible tab.
    pub(crate) fn of(line: &PositionedLine) -> Option<Self> {
        let eps = line.eps();
        let mut occupied: Vec<bool> = line.slots.iter().map(|s| s.visual_width > 0.0).collect();
        let mut occupy = |lo: f32, hi: f32| {
            for (index, _) in line.slots_holding(lo, hi, eps) {
                occupied[index] = true;
            }
        };
        for g in line.glyphs() {
            if g.byte_range.is_some_and(|r| !line.is_trailing(r)) {
                let (lo, hi) = g.pen_interval();
                occupy(lo, hi);
            }
        }
        for b in &line.visual_boxes {
            occupy(b.x, b.x + b.width);
        }
        for t in &line.tabs {
            if !line.is_trailing(t.byte_range) {
                occupy(t.x, t.x + t.width);
            }
        }
        let first = occupied.iter().position(|&o| o)?;
        let last = occupied.iter().rposition(|&o| o)?;
        Some(Self {
            first,
            last,
            left: line.slots[first].content_start,
            right: line.slots[last].content_end(),
            eps,
            slot_count: line.slots.len(),
        })
    }

    /// `[lo, hi]` lies outside the body without entering an internal obstacle
    /// or an unused slot: it may pass an edge only on a side where the line
    /// has no further slot.
    pub(crate) fn is_outside_clear(&self, lo: f32, hi: f32) -> bool {
        (lo >= self.left - self.eps || self.first == 0)
            && (hi <= self.right + self.eps || self.last + 1 == self.slot_count)
    }

    /// `[lo, hi]` is on the far edge of the base direction (UAX #9 L1), where
    /// trailing whitespace lies.
    pub(crate) fn is_on_far_edge(&self, direction: BidiDirection, lo: f32, hi: f32) -> bool {
        self.is_outside_clear(lo, hi)
            && match direction {
                BidiDirection::Ltr => lo >= self.right - self.eps,
                BidiDirection::Rtl => hi <= self.left + self.eps,
            }
    }

    /// How far `[lo, hi]` overflows the body on each side, beyond tolerance.
    pub(crate) fn overflow(&self, lo: f32, hi: f32) -> HangExtents {
        let beyond = |over: f32| if over > self.eps { over } else { 0.0 };
        HangExtents {
            left: beyond(self.left - lo),
            right: beyond(hi - self.right),
        }
    }
}

impl PositionedLine {
    /// The body width: the sum of the slots' content advances, hanging
    /// punctuation, preserved trailing whitespace and obstacles excluded.
    #[must_use]
    pub fn visual_width(&self) -> f32 {
        self.slots.iter().map(|slot| slot.visual_width).sum()
    }

    /// Overflow of hanging punctuation beyond the left edge of the first
    /// occupied slot's content.
    #[must_use]
    pub fn hanging_left(&self) -> f32 {
        BodyExtent::of(self).map_or(0.0, |body| self.hang_extents(&body).left)
    }

    /// Overflow of hanging punctuation beyond the right edge of the last
    /// occupied slot's content.
    #[must_use]
    pub fn hanging_right(&self) -> f32 {
        BodyExtent::of(self).map_or(0.0, |body| self.hang_extents(&body).right)
    }

    /// The glyphs of the line that hang ([`GlyphRole::Hanging`]).
    pub fn hanging_glyphs(&self) -> impl Iterator<Item = &PositionedGlyph> {
        let eps = self.eps();
        self.glyphs()
            .filter(move |glyph| self.role_at(glyph, eps) == GlyphRole::Hanging)
    }

    /// The role of `glyph` on this line (spec OUT-4).
    #[must_use]
    pub fn role(&self, glyph: &PositionedGlyph) -> GlyphRole {
        self.role_at(glyph, self.eps())
    }

    /// [`PositionedLine::role`] with the line's [`PositionedLine::eps`]
    /// computed once by the caller, so a pass over the glyphs sums the
    /// slots once.
    pub(crate) fn role_at(&self, glyph: &PositionedGlyph, eps: f32) -> GlyphRole {
        let (lo, hi) = glyph.pen_interval();
        if self.is_trailing_whitespace(glyph) {
            GlyphRole::Trailing
        } else if glyph.advance == 0.0 {
            GlyphRole::ZeroAdvance
        } else if self.is_held(lo, hi, eps) {
            GlyphRole::Body
        } else {
            GlyphRole::Hanging
        }
    }

    /// The largest overflow of a hanging glyph beyond each edge of `body`.
    pub(crate) fn hang_extents(&self, body: &BodyExtent) -> HangExtents {
        self.hanging_glyphs()
            .map(|glyph| {
                let (lo, hi) = glyph.pen_interval();
                body.overflow(lo, hi)
            })
            .fold(
                HangExtents {
                    left: 0.0,
                    right: 0.0,
                },
                |most, over| HangExtents {
                    left: most.left.max(over.left),
                    right: most.right.max(over.right),
                },
            )
    }

    /// Every glyph of every visual run, in order.
    pub(crate) fn glyphs(&self) -> impl Iterator<Item = &PositionedGlyph> {
        self.visual_runs.iter().flat_map(|run| &run.glyphs)
    }

    /// The line's one geometry [`tolerance`], at the scale of its body width.
    pub(crate) fn eps(&self) -> f32 {
        tolerance(self.visual_width(), 0.0)
    }

    /// Whether some slot's content block holds `[lo, hi]`, within `eps`
    /// (the line's [`PositionedLine::eps`]).
    pub(crate) fn is_held(&self, lo: f32, hi: f32, eps: f32) -> bool {
        self.slots_holding(lo, hi, eps).next().is_some()
    }

    /// The slots whose content block holds `[lo, hi]` ([`PositionedSlot::holds`]),
    /// with their indexes. The slots ascend and are disjoint (spec GEO-3), so a
    /// binary search finds the first slot ending late enough and the scan stops
    /// at the first one that starts too late.
    pub(crate) fn slots_holding(
        &self,
        lo: f32,
        hi: f32,
        eps: f32,
    ) -> impl Iterator<Item = (usize, &PositionedSlot)> {
        let from = self.slots.partition_point(|s| s.content_end() + eps < hi);
        self.slots
            .iter()
            .enumerate()
            .skip(from)
            .take_while(move |(_, s)| s.content_start - eps <= lo)
            .filter(move |(_, s)| s.holds(lo, hi, eps))
    }

    /// `true` for source bytes at or after `visible_byte_end`.
    pub(crate) fn is_trailing(&self, range: TextRange) -> bool {
        range.start() >= self.visible_byte_end
    }

    /// `true` for a glyph of the trailing whitespace.
    pub(crate) fn is_trailing_whitespace(&self, glyph: &PositionedGlyph) -> bool {
        glyph
            .byte_range
            .is_some_and(|range| self.is_trailing(range))
    }
}
