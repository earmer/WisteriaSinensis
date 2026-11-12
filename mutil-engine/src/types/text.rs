//! Offsets and spans into the paragraph's UTF-8 text (spec §2.1).

use core::ops::Range;

/// Byte offset into the paragraph's UTF-8 text (spec §2.1).
///
/// **[Design decision, §2.1]** Every internal offset is a UTF-8 byte offset and
/// synthetic elements (generated hyphens, glue) never displace them. UTF-16
/// conversion for platform text-input layers belongs to §6.7 and is not
/// implemented yet.
pub type ByteIndex = usize;

/// Half-open byte range `[start, end)` (spec §2.1).
///
/// The fields are public: a range is a coordinate primitive used throughout
/// the engine (spec §2.1). [`new`](Self::new) asserts `start <= end` for ranges
/// the caller computes; [`try_new`](Self::try_new) returns `None` for offsets
/// read from outside. Whether a range falls on the code point boundaries of a
/// particular text depends on that text, so it is checked where the text is
/// known, not by a constructor (spec §1.5, A3).
/// Layout: two `usize`s, 16 bytes on 64-bit targets, no padding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextRange {
    /// Inclusive start offset.
    pub start: ByteIndex,
    /// Exclusive end offset.
    pub end: ByteIndex,
}

impl TextRange {
    /// The empty range at offset 0.
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    /// The range `[start, end)`.
    ///
    /// # Panics
    /// If `start > end`; use [`try_new`](Self::try_new) for offsets that may
    /// be inverted.
    #[inline]
    #[must_use]
    pub const fn new(start: ByteIndex, end: ByteIndex) -> Self {
        assert!(start <= end, "TextRange requires start <= end");
        Self { start, end }
    }

    /// The range `[start, end)`, or `None` when `start > end`.
    #[inline]
    #[must_use]
    pub const fn try_new(start: ByteIndex, end: ByteIndex) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }

    /// An empty range anchored at `offset`.
    ///
    /// Spec §2.6 / §2.4: zero-width items such as `CjkAutospace` have
    /// `start == end`.
    #[inline]
    #[must_use]
    pub const fn empty_at(offset: ByteIndex) -> Self {
        Self {
            start: offset,
            end: offset,
        }
    }

    /// Inclusive start offset.
    #[inline]
    #[must_use]
    pub const fn start(&self) -> ByteIndex {
        self.start
    }

    /// Exclusive end offset.
    #[inline]
    #[must_use]
    pub const fn end(&self) -> ByteIndex {
        self.end
    }

    /// The smallest range covering both `self` and `other`.
    #[inline]
    #[must_use]
    pub const fn cover(&self, other: TextRange) -> Self {
        let start = if self.start < other.start {
            self.start
        } else {
            other.start
        };
        let end = if self.end > other.end {
            self.end
        } else {
            other.end
        };
        Self { start, end }
    }

    /// Length in bytes.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end - self.start
    }

    /// `true` when the range covers no bytes.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// `true` when `offset` lies in `[start, end)`.
    #[inline]
    #[must_use]
    pub const fn contains(&self, offset: ByteIndex) -> bool {
        self.start <= offset && offset < self.end
    }

    /// `true` when `other` lies entirely within `self`: a reflexive and
    /// transitive relation.
    #[inline]
    #[must_use]
    pub const fn contains_range(&self, other: TextRange) -> bool {
        self.start <= other.start && other.end <= self.end
    }

    /// Converts to a standard [`Range`].
    #[inline]
    #[must_use]
    pub const fn as_range(&self) -> Range<usize> {
        self.start..self.end
    }

    /// Slices `text`, returning `None` if the range is out of bounds or does
    /// not fall on code point boundaries.
    #[inline]
    #[must_use]
    pub fn slice<'a>(&self, text: &'a str) -> Option<&'a str> {
        text.get(self.as_range())
    }
}

/// The error of converting an inverted [`Range`] into a [`TextRange`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvertedRange;

impl core::fmt::Display for InvertedRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a text range must not end before it starts")
    }
}

impl core::error::Error for InvertedRange {}

impl TryFrom<Range<usize>> for TextRange {
    type Error = InvertedRange;

    #[inline]
    fn try_from(r: Range<usize>) -> Result<Self, InvertedRange> {
        Self::try_new(r.start, r.end).ok_or(InvertedRange)
    }
}

impl From<TextRange> for Range<usize> {
    #[inline]
    fn from(r: TextRange) -> Self {
        r.as_range()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_queries() {
        let r = TextRange::new(2, 5);
        assert_eq!(r.len(), 3);
        assert!(!r.is_empty());
        assert!(r.contains(2) && r.contains(4) && !r.contains(5));
        assert!(r.contains_range(TextRange::new(3, 5)));
        assert!(!r.contains_range(TextRange::new(1, 3)));
        assert!(TextRange::empty_at(7).is_empty());
    }

    #[test]
    fn containment_is_reflexive_transitive_and_rejects_inverted_ranges() {
        let (outer, middle, inner) = (
            TextRange::new(0, 9),
            TextRange::new(2, 7),
            TextRange::new(3, 4),
        );
        for r in [outer, middle, inner, TextRange::empty_at(5)] {
            assert!(r.contains_range(r));
        }
        assert!(outer.contains_range(middle) && middle.contains_range(inner));
        assert!(outer.contains_range(inner));
        // An inverted range cannot be built.
        assert_eq!(TextRange::try_new(5, 3), None);
        assert_eq!(TextRange::try_new(3, 5), Some(TextRange::new(3, 5)));
        assert_eq!(
            TextRange::new(2, 4).cover(TextRange::new(1, 3)),
            TextRange::new(1, 4)
        );
    }

    #[test]
    fn slice_respects_char_boundaries() {
        let text = "aé"; // 'é' is two bytes
        assert_eq!(TextRange::new(0, 1).slice(text), Some("a"));
        assert_eq!(TextRange::new(1, 3).slice(text), Some("é"));
        assert_eq!(TextRange::new(0, 2).slice(text), None); // splits 'é'
        assert_eq!(TextRange::new(0, 9).slice(text), None); // out of bounds
    }

    #[test]
    fn range_round_trip() {
        let r = TextRange::try_from(3..9).unwrap();
        let back: Range<usize> = r.into();
        assert_eq!(back, 3..9);
        #[allow(clippy::reversed_empty_ranges)]
        let inverted = 9..3;
        assert_eq!(TextRange::try_from(inverted), Err(InvertedRange));
    }
}
