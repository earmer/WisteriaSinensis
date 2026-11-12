//! Rich inline span inputs (spec §2.2, §2.5): decoration and atomic-edit spans.
//!
//! The two streams are orthogonal. An [`InlineDecorationSpan`] controls
//! geometry (background, border, padding and how they fragment across lines);
//! an [`AtomicEditSpan`] controls editing (caret stops, hit testing, deletion).
//! A Notion-style mention is usually both; an inline `code` span is often only
//! the first, a hidden-metadata range only the second.

use crate::types::{ByteIndex, TextRange};

/// Deepest proper nesting `check_layout_inputs` accepts for decoration spans
/// (a span inside a span inside ...). **[Design decision]**
/// (spec §2.5): 8 is generous for rich text and cheap to check.
pub const MAX_DECORATION_NESTING_DEPTH: usize = 8;

/// How a decoration is drawn when its span breaks across lines (CSS
/// Fragmentation 3 `box-decoration-break`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoxDecorationBreak {
    /// Padding applies only at the true logical start and end of the span.
    #[default]
    Slice,
    /// Every line fragment receives the full start and end padding.
    Clone,
}

/// Background, border and padding of an inline range (spec §2.2).
///
/// Paddings are the horizontal advance of border plus padding; the vertical
/// extent of the box comes from the line.
///
/// A paragraph holds its spans in `ParagraphAst::decoration_spans`, ordered
/// ascending by `byte_range.start` and, for equal starts, descending by
/// `byte_range.end`, so an outer span precedes the spans it contains.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineDecorationSpan {
    /// Caller-assigned identifier, unique among decoration spans.
    pub span_id: u64,
    /// Bytes the span covers. Must be non-empty.
    pub byte_range: TextRange,
    /// Advance of the left border plus padding. Finite and non-negative.
    pub start_padding: f32,
    /// Advance of the right border plus padding. Finite and non-negative.
    pub end_padding: f32,
    /// Fragmentation behaviour.
    pub break_mode: BoxDecorationBreak,
}

impl InlineDecorationSpan {
    /// `true` when both paddings are finite and non-negative and the range is
    /// not empty.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        [self.start_padding, self.end_padding]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
            && !self.byte_range.is_empty()
    }

    /// Whether the frontend must not insert `CjkAutospace` at `offset` because
    /// a padded border already separates the neighbours (§3 Step 6.6): at
    /// `byte_range.start` when `start_padding > 0`, at `byte_range.end` when
    /// `end_padding > 0`; both sides of the boundary, so the answer depends on
    /// the offset alone.
    #[must_use]
    pub fn forbids_cjk_autospace_at(&self, offset: ByteIndex) -> bool {
        (self.start_padding > 0.0 && offset == self.byte_range.start())
            || (self.end_padding > 0.0 && offset == self.byte_range.end())
    }
}

/// A range the caret treats as a single unit (spec §2.2, §6): a mention, a
/// page-reference chip, an inline code span.
///
/// It may still wrap across lines (spec §6.1); atomicity concerns editing
/// only.
///
/// A paragraph holds its spans in `ParagraphAst::atomic_spans`, ascending by
/// `byte_range.start` and pairwise disjoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AtomicEditSpan {
    /// Caller-assigned identifier, unique among atomic spans.
    pub span_id: u64,
    /// Bytes the span covers. Must be non-empty.
    pub byte_range: TextRange,
}

impl AtomicEditSpan {
    /// `true` when `offset` lies strictly inside the span
    /// (`start < offset < end`): the offsets that get no caret stop. The two
    /// boundaries are not interior.
    #[must_use]
    pub const fn is_interior(&self, offset: ByteIndex) -> bool {
        self.byte_range.start() < offset && offset < self.byte_range.end()
    }
}

/// IME preedit styling over source bytes; never changes EGC segmentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompositionSpan {
    /// UTF-8 code-point-aligned source range, which may lie inside an EGC.
    pub byte_range: TextRange,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deco(start_padding: f32, end_padding: f32) -> InlineDecorationSpan {
        InlineDecorationSpan {
            span_id: 1,
            byte_range: TextRange::new(10, 20),
            start_padding,
            end_padding,
            break_mode: BoxDecorationBreak::Clone,
        }
    }

    #[test]
    fn autospace_is_forbidden_only_beside_a_padded_edge() {
        let s = deco(4.0, 0.0);
        assert!(s.forbids_cjk_autospace_at(10));
        assert!(!s.forbids_cjk_autospace_at(20)); // end_padding is zero
        assert!(!s.forbids_cjk_autospace_at(15));
        let s = deco(0.0, 4.0);
        assert!(!s.forbids_cjk_autospace_at(10));
        assert!(s.forbids_cjk_autospace_at(20));
        assert!(!deco(0.0, 0.0).forbids_cjk_autospace_at(10));
    }

    #[test]
    fn decoration_validity() {
        assert!(deco(0.0, 3.0).is_valid());
        for bad in [-1.0, f32::NAN, f32::INFINITY] {
            assert!(!deco(bad, 0.0).is_valid());
            assert!(!deco(0.0, bad).is_valid());
        }
        let mut empty = deco(1.0, 1.0);
        empty.byte_range = TextRange::empty_at(5);
        assert!(!empty.is_valid());
        assert_eq!(BoxDecorationBreak::default(), BoxDecorationBreak::Slice);
    }

    #[test]
    fn atomic_interior_excludes_both_boundaries() {
        let a = AtomicEditSpan {
            span_id: 7,
            byte_range: TextRange::new(3, 9),
        };
        assert!(!a.is_interior(3));
        assert!(a.is_interior(4));
        assert!(a.is_interior(8));
        assert!(!a.is_interior(9));
        assert!(!a.is_interior(2));
    }
}
