//! Paragraph-level validation for [`PositionedParagraph`].

use super::super::tolerance::tolerance;
use super::super::{LineInvariantError, ParagraphErrorReason, PositionedParagraph, validated};
use crate::types::TextRange;
use std::collections::{HashMap, HashSet};

impl PositionedParagraph {
    /// Validates the geometry of the paragraph against its own recorded
    /// policy: the policy is one layout could have resolved; there is at least
    /// one line; every line is valid under its own `line_index` (so
    /// `HangStartMode::FirstLine` applies to line 0 only); under
    /// [`SlotSource::Standard`](super::super::SlotSource::Standard) every line has
    /// exactly its standard slot; and the line boxes follow one another inside
    /// `total_block_size`.
    ///
    /// Serves geometry-only fixtures; it does not certify source coverage.
    ///
    /// # Errors
    /// The index of the first invalid line and its violation.
    pub fn validate_geometry(&self) -> Result<(), (usize, LineInvariantError)> {
        use ParagraphErrorReason as R;
        let invalid = |line, reason| Err((line, LineInvariantError::InvalidParagraph { reason }));
        if !self.policy.is_valid() {
            return invalid(0, R::InvalidPolicy);
        }
        if self.lines.is_empty() {
            return invalid(0, R::NoLines);
        }
        if !self.total_block_size.is_finite() || self.total_block_size < 0.0 {
            return invalid(0, R::InvalidBlockSize);
        }
        let mut previous_bottom = 0.0;
        for (i, line) in self.lines.iter().enumerate() {
            let ctx = self.policy.line_context(self.base_direction, i);
            line.validate(&ctx).map_err(|e| (i, e))?;
            validated::check_drawing_origins(line, self.policy.writing_mode, self.total_block_size)
                .map_err(|e| (i, e))?;
            if let Some(standard) = self.policy.slot_source.standard_slot(i, ctx.wrap_width)
                && !matches!(line.slots.as_slice(), [slot] if slot.constraint == standard)
            {
                return invalid(i, R::NotStandardSlots);
            }
            let (top, bottom) =
                self.policy
                    .writing_mode
                    .line_bounds(line.baseline_y, line.ascent, line.descent);
            let eps = tolerance(line.line_height, 0.0);
            if !top.is_finite()
                || !bottom.is_finite()
                || top < previous_bottom - eps
                || bottom < top
            {
                return invalid(i, R::LineBoxesOverlap);
            }
            if line.ascent + line.descent > line.line_height + eps {
                return invalid(i, R::LineBoxExceedsLineHeight);
            }
            previous_bottom = bottom;
        }
        if self.total_block_size < previous_bottom - tolerance(previous_bottom, 0.0) {
            return invalid(self.lines.len() - 1, R::InvalidBlockSize);
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
        let invalid = |line, reason| Err((line, LineInvariantError::InvalidParagraph { reason }));
        self.validate_geometry()?;
        let last = self.lines.len() - 1;
        let mut rendered_end = 0;
        let mut annotation_ids = HashSet::new();
        for (i, line) in self.lines.iter().enumerate() {
            if line.byte_range.start() != rendered_end || line.byte_range.slice(text).is_none() {
                return invalid(i, R::SourceNotCovered);
            }
            if line.is_ellipsized && i != last {
                return invalid(i, R::InvalidTruncation);
            }
            if !line
                .annotations
                .iter()
                .all(|a| annotation_ids.insert(a.annotation_id))
            {
                return invalid(i, R::DuplicateAnnotation);
            }
            rendered_end = line.byte_range.end();
        }
        match self.truncated_range {
            None if rendered_end != text.len() => return invalid(last, R::SourceNotCovered),
            None if self.lines[last].is_ellipsized => return invalid(last, R::InvalidTruncation),
            None => {}
            Some(tail) => {
                let is_consistent = self.policy.clamp.is_some()
                    && tail.start() == rendered_end
                    && tail.end() == text.len()
                    && tail.start() < tail.end()
                    && self.lines[last].is_ellipsized;
                if !is_consistent {
                    return invalid(last, R::InvalidTruncation);
                }
            }
        }
        self.check_span_fragment_identity(rendered_end)?;
        self.check_atomic_fragment_identity(rendered_end)
    }

    /// Across the lines, the fragments of one decoration span carry one
    /// `span_range`, exactly one of them is the first and exactly one the
    /// last. Each line already ties the flags to the fragment's bytes, so the
    /// fragments are in logical order. A span whose end lies in the truncated
    /// tail (beyond `rendered_end`) has no last fragment.
    fn check_span_fragment_identity(
        &self,
        rendered_end: usize,
    ) -> Result<(), (usize, LineInvariantError)> {
        struct Seen {
            span_range: TextRange,
            line: usize,
            has_first: bool,
            has_last: bool,
        }
        let invalid = |line| {
            Err((
                line,
                LineInvariantError::InvalidParagraph {
                    reason: ParagraphErrorReason::InconsistentSpanFragments,
                },
            ))
        };
        let mut spans: HashMap<u64, Seen> = HashMap::new();
        for (line, fragment) in self
            .lines
            .iter()
            .enumerate()
            .flat_map(|(i, line)| line.span_fragments.iter().map(move |f| (i, f)))
        {
            let seen = spans.entry(fragment.span_id).or_insert(Seen {
                span_range: fragment.span_range,
                line,
                has_first: false,
                has_last: false,
            });
            if seen.span_range != fragment.span_range
                || (seen.has_first && fragment.is_first_fragment())
                || (seen.has_last && fragment.is_last_fragment())
            {
                return invalid(line);
            }
            seen.line = line;
            seen.has_first |= fragment.is_first_fragment();
            seen.has_last |= fragment.is_last_fragment();
        }
        // Report the earliest offending line, whatever the map's order.
        match spans
            .values()
            .filter(|seen| {
                let ends_in_tail = seen.span_range.end() > rendered_end;
                !seen.has_first || (!seen.has_last && !ends_in_tail)
            })
            .map(|seen| seen.line)
            .min()
        {
            Some(line) => invalid(line),
            None => Ok(()),
        }
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
        let invalid = |line| {
            Err((
                line,
                LineInvariantError::InvalidParagraph {
                    reason: ParagraphErrorReason::InconsistentAtomicFragments,
                },
            ))
        };
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
        match spans
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
        {
            Some(line) => invalid(line),
            None => Ok(()),
        }
    }
}
