//! Paragraph-level validation for [`PositionedParagraph`].

use crate::middleend::tolerance::{approx_eq, tolerance};
use crate::middleend::{
    LineInvariantError, ParagraphErrorReason, PositionedParagraph, PositionedSpanFragment,
    validated,
};
use crate::types::TextRange;
use std::collections::{HashMap, HashSet};

impl ParagraphErrorReason {
    /// The paragraph-level violation, reported at `line`.
    #[cold]
    #[inline(never)]
    fn at(self, line: usize) -> (usize, LineInvariantError) {
        (line, LineInvariantError::InvalidParagraph { reason: self })
    }
}

impl PositionedParagraph {
    /// Validates the geometry of the paragraph against its own recorded
    /// policy: the policy is one layout could have resolved; there is at least
    /// one line; every line is valid under its own `line_index` (so
    /// `HangStartMode::FirstLine` applies to line 0 only); under
    /// [`SlotSource::Standard`](crate::middleend::SlotSource::Standard) every line has
    /// exactly its standard slot; and the line boxes follow one another inside
    /// `total_block_size`.
    ///
    /// Serves geometry-only fixtures; it does not certify source coverage.
    ///
    /// # Errors
    /// The index of the first invalid line and its violation.
    pub fn validate_geometry(&self) -> Result<(), (usize, LineInvariantError)> {
        use ParagraphErrorReason as R;
        if !self.policy.is_valid() {
            return Err(R::InvalidPolicy.at(0));
        }
        if self.lines.is_empty() {
            return Err(R::NoLines.at(0));
        }
        if !self.total_block_size.is_finite() || self.total_block_size < 0.0 {
            return Err(R::InvalidBlockSize.at(0));
        }
        let mut previous_bottom = 0.0;
        for (i, line) in self.lines.iter().enumerate() {
            let ctx = self.policy.line_context(self.base_direction, i);
            line.validate(&ctx).map_err(|e| (i, e))?;
            validated::check_drawing_origins(
                line,
                self.policy.writing_mode,
                self.policy.strut,
                self.total_block_size,
            )
            .map_err(|e| (i, e))?;
            if let Some(standard) = self.policy.slot_source.standard_slot(i, ctx.wrap_width)
                && !matches!(line.slots.as_slice(), [slot] if slot.constraint == standard)
            {
                return Err(R::NotStandardSlots.at(i));
            }
            let (top, bottom) =
                self.policy
                    .writing_mode
                    .line_bounds(line.baseline_y, line.ascent, line.descent);
            let overlap_eps = tolerance(top, previous_bottom);
            // Bottoms ascend exactly: hit testing bisects them (spec INT-20).
            if !top.is_finite()
                || !bottom.is_finite()
                || top < previous_bottom - overlap_eps
                || bottom < top
                || bottom < previous_bottom
            {
                return Err(R::LineBoxesOverlap.at(i));
            }
            if line.ascent + line.descent > line.line_height + tolerance(line.line_height, 0.0) {
                return Err(R::LineBoxExceedsLineHeight.at(i));
            }
            previous_bottom = bottom;
        }
        if self.total_block_size < previous_bottom - tolerance(previous_bottom, 0.0) {
            return Err(R::InvalidBlockSize.at(self.lines.len() - 1));
        }
        Ok(())
    }

    /// Validates the whole paragraph against `text`, the paragraph text it was
    /// laid out from. Beyond [`validate_geometry`](Self::validate_geometry):
    ///
    /// * **coverage**: the lines' byte ranges tile the rendered text from
    ///   offset 0, on character boundaries;
    /// * **truncation**: an unclamped paragraph renders all of `text`; a
    ///   clamped one has a clamp policy, a non-empty `truncated_range` from the
    ///   last line's end to the end of `text`, and exactly its last line
    ///   ellipsized;
    /// * **identity across lines**: annotation ids are unique; the fragments
    ///   of one decoration span agree on its range and hold exactly one first
    ///   and one last fragment (the last one may fall in the truncated tail);
    ///   the fragments of one atomic span agree on its range and cover every
    ///   rendered byte of it.
    ///
    /// # Errors
    /// The first invalid line and its violation; a paragraph-level violation
    /// is reported with the line where it was detected.
    pub fn validate(&self, text: &str) -> Result<(), (usize, LineInvariantError)> {
        use ParagraphErrorReason as R;
        self.validate_geometry()?;
        let last = self.lines.len() - 1;
        let mut rendered_end = 0;
        let mut annotation_ids = HashSet::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.byte_range.start() != rendered_end || line.byte_range.slice(text).is_none() {
                return Err(R::SourceNotCovered.at(i));
            }
            if let Some(index) = line
                .caret_stops
                .iter()
                .position(|s| !text.is_char_boundary(s.offset))
            {
                return Err((i, LineInvariantError::InvalidCaretStop { index }));
            }
            // A hit inside an atomic fragment snaps to its span's ends
            // (INT-10), which must be caret positions of the text.
            if let Some(index) = line.atomic_fragments.iter().position(|f| {
                f.full_byte_range.slice(text).is_none()
                    || f.fragment_byte_range.slice(text).is_none()
            }) {
                return Err((i, LineInvariantError::InvalidAtomicFragment { index }));
            }
            if line.is_ellipsized && i != last {
                return Err(R::InvalidTruncation.at(i));
            }
            if !line
                .annotations
                .iter()
                .all(|a| annotation_ids.insert(a.annotation_id))
            {
                return Err(R::DuplicateAnnotation.at(i));
            }
            rendered_end = line.byte_range.end();
        }
        match self.truncated_range {
            None if rendered_end != text.len() => return Err(R::SourceNotCovered.at(last)),
            None if self.lines[last].is_ellipsized => return Err(R::InvalidTruncation.at(last)),
            None => {}
            Some(tail) => {
                let is_consistent = self.policy.clamp.is_some()
                    && tail.start() == rendered_end
                    && tail.end() == text.len()
                    && tail.start() < tail.end()
                    && self.lines[last].is_ellipsized;
                if !is_consistent {
                    return Err(R::InvalidTruncation.at(last));
                }
            }
        }
        self.check_span_fragment_identity(rendered_end)?;
        self.check_atomic_fragment_identity(rendered_end)
    }

    /// Across the lines, the fragments of one decoration span carry one
    /// `span_range`, exactly one of them is the first and exactly one the
    /// last, and their `ribbon_offset` values start at `0.0`, advance by each
    /// fragment's `width` in logical order, and sum to `total_span_advance`.
    /// A span whose end lies in the truncated tail (beyond `rendered_end`)
    /// has no last fragment.
    fn check_span_fragment_identity(
        &self,
        rendered_end: usize,
    ) -> Result<(), (usize, LineInvariantError)> {
        struct Seen {
            span_range: TextRange,
            line: usize,
            has_first: bool,
            has_last: bool,
            ribbon_acc: f32,
            total_span_advance: f32,
        }
        use ParagraphErrorReason as R;
        let invalid = |line| Err(R::InconsistentSpanFragments.at(line));
        let mut spans: HashMap<u64, Seen> = HashMap::new();
        for (line_idx, line) in self.lines.iter().enumerate() {
            if line.span_fragments.is_empty() {
                continue;
            }
            let mut ordered: Vec<&PositionedSpanFragment> = line.span_fragments.iter().collect();
            ordered.sort_by_key(|f| (f.byte_range.start(), f.byte_range.end()));
            for fragment in ordered {
                let seen = spans.entry(fragment.span_id).or_insert(Seen {
                    span_range: fragment.span_range,
                    line: line_idx,
                    has_first: false,
                    has_last: false,
                    ribbon_acc: 0.0,
                    total_span_advance: fragment.total_span_advance,
                });
                if seen.span_range != fragment.span_range
                    || (seen.has_first && fragment.is_first_fragment())
                    || (seen.has_last && fragment.is_last_fragment())
                    || !approx_eq(fragment.ribbon_offset, seen.ribbon_acc)
                    || !approx_eq(fragment.total_span_advance, seen.total_span_advance)
                {
                    return invalid(line_idx);
                }
                seen.line = line_idx;
                seen.has_first |= fragment.is_first_fragment();
                seen.has_last |= fragment.is_last_fragment();
                seen.ribbon_acc += fragment.width;
            }
        }
        // Report the earliest offending line, whatever the map's order.
        spans
            .values()
            .filter(|seen| {
                let ends_in_tail = seen.span_range.end() > rendered_end;
                !seen.has_first
                    || (!seen.has_last && !ends_in_tail)
                    || !approx_eq(seen.ribbon_acc, seen.total_span_advance)
            })
            .map(|seen| seen.line)
            .min()
            .map_or(Ok(()), invalid)
    }

    /// Across the lines, the fragments of one atomic span carry one
    /// `full_byte_range` and cover every rendered byte of it. Each line
    /// already checks that its fragments cover the part of the span on that
    /// line without overlap, so the byte counts decide: a line that crosses
    /// the span without a fragment leaves the total short.
    fn check_atomic_fragment_identity(
        &self,
        rendered_end: usize,
    ) -> Result<(), (usize, LineInvariantError)> {
        struct Seen {
            full: TextRange,
            line: usize,
            bytes: usize,
        }
        use ParagraphErrorReason as R;
        let invalid = |line| Err(R::InconsistentAtomicFragments.at(line));
        let mut spans: HashMap<u64, Seen> = HashMap::new();
        for (line, fragment) in self
            .lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| line.atomic_fragments.iter().map(move |f| (i, f)))
        {
            let seen = spans.entry(fragment.atomic_id).or_insert(Seen {
                full: fragment.full_byte_range,
                line,
                bytes: 0,
            });
            if seen.full != fragment.full_byte_range {
                return invalid(line);
            }
            seen.line = line;
            seen.bytes += fragment.fragment_byte_range.len();
        }
        spans
            .values()
            .filter(|seen| {
                seen.bytes
                    != seen
                        .full
                        .end()
                        .min(rendered_end)
                        .saturating_sub(seen.full.start())
            })
            .map(|seen| seen.line)
            .min()
            .map_or(Ok(()), invalid)
    }
}
