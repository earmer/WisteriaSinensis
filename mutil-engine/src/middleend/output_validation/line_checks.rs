//! Line-level finite, envelope, source, caret, and annotation record checks.

use super::{RangeIndex, SourcedGlyphs};
use crate::frontend::AnnotationSide;
use crate::middleend::tolerance::{approx_eq, approx_lt};
use crate::middleend::{
    AnnotationErrorReason, LineContext, LineHeight, LineInvariantError, PositionedGlyph,
    PositionedLine,
};
use crate::types::{BidiLevel, GlyphOrientation, TextRange};
use std::collections::HashSet;

impl PositionedLine {
    /// Pass 1: all numbers finite, no negative extent or size.
    pub(super) fn check_finite(&self) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let metrics = [
            self.baseline_y,
            self.ascent,
            self.descent,
            self.line_height,
            self.baseline_y - self.ascent,
            self.baseline_y + self.ascent,
            self.baseline_y - self.descent,
            self.baseline_y + self.descent,
            self.ascent + self.descent,
        ];
        if !metrics.into_iter().all(f32::is_finite) {
            return Err(E::NonFiniteLineMetrics);
        }
        for (run, r) in self.visual_runs.iter().enumerate() {
            if !(r.font_size.is_finite()
                && r.font_size > 0.0
                && r.font.scale.is_finite()
                && r.font.scale > 0.0)
            {
                return Err(E::InvalidVisualRun { run });
            }
            for (glyph, g) in r.glyphs.iter().enumerate() {
                let finite = g.x.is_finite()
                    && g.x_offset.is_finite()
                    && g.advance.is_finite()
                    && g.y_offset.is_finite();
                if !(finite && (g.x + g.advance).is_finite() && (g.x + g.x_offset).is_finite()) {
                    return Err(E::NonFiniteGlyph { run, glyph });
                }
            }
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            // The extents about the line's baseline too: selection geometry
            // takes `ascent - y` and `descent + y` (INT-16).
            let finite = [
                b.x,
                b.y,
                b.width,
                b.ascent,
                b.descent,
                b.x + b.width,
                b.ascent - b.y,
                b.descent + b.y,
            ]
            .into_iter()
            .all(f32::is_finite);
            if !(finite && b.width >= 0.0 && b.ascent >= 0.0 && b.descent >= 0.0) {
                return Err(E::InvalidInlineBox { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            let finite = t.x.is_finite() && t.width.is_finite() && (t.x + t.width).is_finite();
            if !(finite && t.width >= 0.0) {
                return Err(E::InvalidTab { index });
            }
        }
        for (index, f) in self.span_fragments.iter().enumerate() {
            let finite = [f.x, f.width, f.ascent, f.descent, f.x + f.width]
                .into_iter()
                .all(f32::is_finite);
            if !(finite && f.width >= 0.0 && f.ascent >= 0.0 && f.descent >= 0.0) {
                return Err(E::InvalidSpanFragment { index });
            }
        }
        for (index, f) in self.atomic_fragments.iter().enumerate() {
            let finite = f.x.is_finite() && f.width.is_finite() && (f.x + f.width).is_finite();
            if !(finite && f.width >= 0.0) {
                return Err(E::InvalidAtomicFragment { index });
            }
        }
        for (index, c) in self.caret_stops.iter().enumerate() {
            if !c.x.is_finite() {
                return Err(E::InvalidCaretStop { index });
            }
        }
        for (index, d) in self.decorations.iter().enumerate() {
            let finite = [d.x, d.width, d.y_offset, d.thickness, d.x + d.width]
                .into_iter()
                .all(f32::is_finite);
            if !(finite && d.width >= 0.0 && d.thickness >= 0.0) {
                return Err(E::InvalidDecoration { index });
            }
        }
        Ok(())
    }

    /// Pass 2: byte range, atomic fragments, empty-line state, line box.
    pub(super) fn check_envelope(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let range = self.byte_range;
        if !(range.start() <= self.visible_byte_end && self.visible_byte_end <= range.end()) {
            return Err(E::VisibleEndOutsideRange);
        }
        self.check_atomic_fragment_ranges()?;
        let expected = ctx.strut.line_height();
        if !(self.line_height.is_finite()
            && self.line_height > 0.0
            && expected.is_finite()
            && expected > 0.0)
        {
            return Err(E::NonPositiveLineHeight {
                expected,
                actual: self.line_height,
            });
        }
        if range.is_empty() {
            self.check_empty_line(expected)?;
            if !self.is_ellipsized {
                // A fixed line box is the strut's even when the line is empty.
                return match ctx.line_height {
                    LineHeight::Fixed(_) => self.check_fixed_metrics(ctx),
                    LineHeight::Normal => Ok(()),
                };
            }
        }
        match ctx.line_height {
            LineHeight::Fixed(_) => {
                if !approx_eq(self.line_height, expected) {
                    return Err(E::LineHeightMismatch {
                        expected,
                        actual: self.line_height,
                    });
                }
                self.check_fixed_metrics(ctx)
            }
            LineHeight::Normal => {
                if approx_lt(self.line_height, expected) {
                    return Err(E::LineHeightBelowStrut {
                        minimum: expected,
                        actual: self.line_height,
                    });
                }
                Ok(())
            }
        }
    }

    /// A line without source bytes holds nothing that stands for source text.
    /// An ordinary empty line holds nothing at all and is exactly the strut
    /// high; a clamp may remove every base cluster of the last line, which
    /// then keeps only its synthesized suffix.
    fn check_empty_line(&self, strut_height: f32) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        if !self.is_ellipsized && !self.visual_runs.is_empty() {
            return Err(E::EmptyLineHasVisualRuns);
        }
        if !self.visual_boxes.is_empty() {
            return Err(E::EmptyLineHasBoxes);
        }
        if !self.tabs.is_empty() {
            return Err(E::EmptyLineHasTabs);
        }
        if !self.span_fragments.is_empty() {
            return Err(E::EmptyLineHasSpanFragments);
        }
        if !self.atomic_fragments.is_empty() {
            return Err(E::EmptyLineHasAtomicFragments);
        }
        if !self.annotations.is_empty() {
            return Err(E::InvalidAnnotation {
                index: 0,
                reason: AnnotationErrorReason::OnEmptyLine,
            });
        }
        if self.is_ellipsized {
            for (run, r) in self.visual_runs.iter().enumerate() {
                if let Some(glyph) = r.glyphs.iter().position(|g| !g.is_synthesized()) {
                    return Err(E::InvalidGlyphSource { run, glyph });
                }
            }
            return Ok(());
        }
        if !approx_eq(self.line_height, strut_height) {
            return Err(E::EmptyLineHeightMismatch {
                expected: strut_height,
                actual: self.line_height,
            });
        }
        if self.slots.iter().any(|slot| slot.visual_width != 0.0) {
            return Err(E::EmptyLineHasExtent);
        }
        Ok(())
    }

    /// Under `LineHeight::Fixed` the line box's `ascent` and `descent` are the
    /// strut's, whether or not the line has content.
    fn check_fixed_metrics(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        if approx_eq(self.ascent, ctx.strut.ascent) && approx_eq(self.descent, ctx.strut.descent) {
            return Ok(());
        }
        Err(LineInvariantError::LineMetricsMismatch {
            expected_ascent: ctx.strut.ascent,
            expected_descent: ctx.strut.descent,
            actual_ascent: self.ascent,
            actual_descent: self.descent,
        })
    }

    /// The bytes of the atomic fragments: each fragment owns a non-empty range
    /// inside the line and inside its span, and the fragments of one span
    /// agree on the span's range, are disjoint and together cover exactly the
    /// part of the span that lies on this line. (Bidi reordering can split
    /// one span into several visual fragments.)
    fn check_atomic_fragment_ranges(&self) -> Result<(), LineInvariantError> {
        let invalid = |index| Err(LineInvariantError::InvalidAtomicFragment { index });
        let fragments = &self.atomic_fragments;
        for (index, f) in fragments.iter().enumerate() {
            if f.fragment_byte_range.is_empty()
                || !f.full_byte_range.contains_range(f.fragment_byte_range)
                || !self.byte_range.contains_range(f.fragment_byte_range)
            {
                return invalid(index);
            }
        }
        let mut order: Vec<usize> = (0..fragments.len()).collect();
        order.sort_by_key(|&i| {
            (
                fragments[i].atomic_id,
                fragments[i].fragment_byte_range.start(),
            )
        });
        let same_span = |a: &usize, b: &usize| fragments[*a].atomic_id == fragments[*b].atomic_id;
        for span in order.chunk_by(same_span) {
            let full = fragments[span[0]].full_byte_range;
            let mut covered = full.start().max(self.byte_range.start());
            for &index in span {
                let f = &fragments[index];
                if f.full_byte_range != full || f.fragment_byte_range.start() != covered {
                    return invalid(index);
                }
                covered = f.fragment_byte_range.end();
            }
            if covered != full.end().min(self.byte_range.end()) {
                return invalid(span[span.len() - 1]);
            }
        }
        Ok(())
    }

    /// Pass 3: source ownership. Every run, glyph, box, tab and decoration
    /// refers to bytes of this line, the runs and the glyphs of each run are
    /// in screen order (OUT-3), and `visible_byte_end` is a boundary of the line's content.
    pub(super) fn check_sources(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
        use LineInvariantError as E;
        let valid_glyph = |g: &PositionedGlyph| {
            g.byte_range
                .is_none_or(|r| r.start() < r.end() && self.byte_range.contains_range(r))
        };
        for (run, r) in self.visual_runs.iter().enumerate() {
            if !r.orientation.is_valid(ctx.writing_mode, r.bidi_level) {
                return Err(E::OrientationMismatch);
            }
            if r.orientation == GlyphOrientation::HorizontalInVertical {
                return Err(E::UnplacedTateChuYoko { run });
            }
            if let Some(glyph) = r.glyphs.iter().position(|g| !valid_glyph(g)) {
                return Err(E::InvalidGlyphSource { run, glyph });
            }
            if r.glyphs.is_empty() || !self.owns_bytes(r.byte_range) {
                return Err(E::InvalidVisualRun { run });
            }
            // Screen order: a pen position never lies left of the previous one.
            // Marks share their base's position, so only `x` is compared.
            if let Some(step) = r.glyphs.windows(2).position(|w| approx_lt(w[1].x, w[0].x)) {
                return Err(E::GlyphsOutOfVisualOrder {
                    run,
                    glyph: step + 1,
                });
            }
        }
        // And across runs, from the last glyph of one to the first of the next.
        if let Some(step) = self
            .visual_runs
            .windows(2)
            .position(|w| approx_lt(w[1].glyphs[0].x, w[0].glyphs[w[0].glyphs.len() - 1].x))
        {
            return Err(E::VisualRunsOutOfOrder { run: step + 1 });
        }
        for (index, b) in self.visual_boxes.iter().enumerate() {
            if !b.baseline_align.is_valid()
                || b.byte_range.is_empty()
                || !self.owns_bytes(b.byte_range)
            {
                return Err(E::InvalidInlineBox { index });
            }
        }
        for (index, t) in self.tabs.iter().enumerate() {
            if t.byte_range.is_empty() || !self.owns_bytes(t.byte_range) {
                return Err(E::InvalidTab { index });
            }
        }
        for (index, d) in self.decorations.iter().enumerate() {
            if !self.owns_bytes(d.byte_range) {
                return Err(E::InvalidDecoration { index });
            }
        }
        self.check_visible_end_boundary()
    }

    /// `visible_byte_end` is a position the line's own content ends or starts
    /// at: the line's start or end, or the start or end of a non-synthesized
    /// glyph, an inline box or a tab. Glyph byte ranges are whole clusters and
    /// clusters are whole graphemes, so such an offset is a grapheme boundary;
    /// an offset inside a cluster could split one.
    fn check_visible_end_boundary(&self) -> Result<(), LineInvariantError> {
        let end = self.visible_byte_end;
        let on_edge = |r: TextRange| r.start() == end || r.end() == end;
        let ok = end == self.byte_range.start()
            || end == self.byte_range.end()
            || self
                .visual_runs
                .iter()
                .flat_map(|r| &r.glyphs)
                .any(|g| g.byte_range.is_some_and(on_edge))
            || self.visual_boxes.iter().any(|b| on_edge(b.byte_range))
            || self.tabs.iter().any(|t| on_edge(t.byte_range));
        if ok {
            Ok(())
        } else {
            Err(LineInvariantError::VisibleEndNotOnBoundary)
        }
    }

    /// Caret stops are in visual order, lie on this line, carry the bidi
    /// level of the content they belong to, and no two share an offset at
    /// one position.
    ///
    /// A stop's owners are the glyphs, inline boxes and tabs whose bytes
    /// contain or touch its `offset`; at a boundary either neighbour qualifies,
    /// and the stop's level must be the level of one of them. A glyph lends
    /// its run's level, a box its own, and a tab the paragraph level (UAX #9
    /// L1). A stop with no
    /// owner is legal only on a line with no glyphs, boxes or tabs (an empty
    /// line), and there it carries the paragraph level.
    pub(super) fn check_caret_stops(
        &self,
        ctx: &LineContext,
        glyphs: &SourcedGlyphs<'_>,
    ) -> Result<(), LineInvariantError> {
        let objects = RangeIndex::new(
            self.visual_boxes
                .iter()
                .map(|b| (b.byte_range, b.bidi_level))
                .chain(
                    self.tabs
                        .iter()
                        .map(|t| (t.byte_range, ctx.base_direction.base_level())),
                )
                .collect(),
        );
        let repeats = self.repeated_caret_stops();
        for (index, stop) in self.caret_stops.iter().enumerate() {
            let in_order = index == 0 || stop.x >= self.caret_stops[index - 1].x;
            let level_is_owned = || {
                let mut owners = glyphs
                    .touching(stop.offset)
                    .map(|(_, (level, _))| *level)
                    .chain(objects.touching(stop.offset).map(|(_, level)| *level))
                    .peekable();
                if owners.peek().is_some() {
                    return owners.any(|level| level == stop.level);
                }
                // Inside bytes no entity renders (glyph-less `WordSpace` glue,
                // spec CRT-2): the base level, or the level of the nearest
                // entity on either side.
                if stop.level == ctx.base_direction.base_level() {
                    return true;
                }
                let [glyph_before, glyph_after] = glyphs.neighbours(stop.offset);
                let [object_before, object_after] = objects.neighbours(stop.offset);
                let glyph_before = glyph_before.map(|(r, (level, _))| (r.end(), *level));
                let glyph_after = glyph_after.map(|(r, (level, _))| (r.start(), *level));
                let object_before = object_before.map(|(r, level)| (r.end(), *level));
                let object_after = object_after.map(|(r, level)| (r.start(), *level));
                let before = glyph_before
                    .into_iter()
                    .chain(object_before)
                    .max_by_key(|(end, _)| *end);
                let after = glyph_after
                    .into_iter()
                    .chain(object_after)
                    .min_by_key(|(start, _)| *start);
                before
                    .into_iter()
                    .chain(after)
                    .any(|(_, level)| level == stop.level)
            };
            if !in_order
                || !(self.byte_range.start()..=self.byte_range.end()).contains(&stop.offset)
                || !level_is_owned()
            {
                return Err(LineInvariantError::InvalidCaretStop { index });
            }
            if repeats[index] {
                return Err(LineInvariantError::DuplicateCaretStop { index });
            }
        }
        Ok(())
    }

    /// For each caret stop, whether it repeats the offset of an earlier stop
    /// at the same position, within the tolerance the layout merges at
    /// (INT-20). Sorting by offset keeps each offset's stops in `x` order, so
    /// a stop repeats one when it repeats its predecessor there: O(n log n),
    /// however many stops share a position (PAR-3).
    fn repeated_caret_stops(&self) -> Vec<bool> {
        let stops = &self.caret_stops;
        let mut by_offset: Vec<usize> = (0..stops.len()).collect();
        // Stable: within an offset, the stops keep their visual order.
        by_offset.sort_by_key(|&i| stops[i].offset);
        let mut repeats = vec![false; stops.len()];
        for pair in by_offset.windows(2) {
            let (earlier, later) = (&stops[pair[0]], &stops[pair[1]]);
            repeats[pair[1]] = earlier.offset == later.offset && approx_eq(earlier.x, later.x);
        }
        repeats
    }

    /// The annotations' own records: metrics, base range, glyphs, unique ids
    /// and disjoint bases per side. Their placement is checked with the
    /// geometry, in `check_annotation_placement`.
    pub(super) fn check_annotation_records(
        &self,
        ctx: &LineContext,
    ) -> Result<(), LineInvariantError> {
        use AnnotationErrorReason as R;
        let finite_nonnegative = |v: f32| v.is_finite() && v >= 0.0;
        let finite_positive = |v: f32| v.is_finite() && v > 0.0;
        for (index, ann) in self.annotations.iter().enumerate() {
            let reason = if !(finite_positive(ann.font_size)
                && finite_positive(ann.font.scale)
                && ann.baseline_offset.is_finite()
                && finite_nonnegative(ann.max_overhang)
                && finite_nonnegative(ann.block_extent)
                && finite_nonnegative(ann.base_over)
                && finite_nonnegative(ann.base_under))
            {
                R::InvalidMetrics
            } else if !ann.orientation.is_valid(ctx.writing_mode, BidiLevel::LTR) {
                R::OrientationMismatch
            } else if ann.base_byte_range.is_empty()
                || ann.base_byte_range.start() < self.byte_range.start()
                || ann.base_byte_range.end() > self.visible_byte_end
            {
                R::BaseOutsideLine
            } else if ann.glyphs.is_empty() {
                R::NoGlyphs
            } else if ann.glyphs.iter().any(|g| {
                let numbers = [
                    g.x,
                    g.x_offset,
                    g.y_offset,
                    g.advance,
                    g.x + g.advance,
                    g.x + g.x_offset,
                ];
                g.byte_range.is_some() || !numbers.into_iter().all(f32::is_finite)
            }) {
                R::InvalidGlyph
            } else {
                continue;
            };
            return Err(LineInvariantError::InvalidAnnotation { index, reason });
        }
        let mut ids = HashSet::new();
        if let Some(index) = self
            .annotations
            .iter()
            .position(|a| !ids.insert(a.annotation_id))
        {
            return Err(LineInvariantError::InvalidAnnotation {
                index,
                reason: R::DuplicateId,
            });
        }
        for side in [AnnotationSide::Over, AnnotationSide::Under] {
            let mut on_side: Vec<usize> = (0..self.annotations.len())
                .filter(|&i| self.annotations[i].side == side)
                .collect();
            on_side.sort_by_key(|&i| self.annotations[i].base_byte_range.start());
            let mut reach = 0;
            for &index in &on_side {
                let base = self.annotations[index].base_byte_range;
                if base.start() < reach {
                    return Err(LineInvariantError::InvalidAnnotation {
                        index,
                        reason: R::OverlapsOnSide,
                    });
                }
                reach = base.end();
            }
        }
        Ok(())
    }
}
