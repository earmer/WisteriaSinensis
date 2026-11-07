//! Offsets and spans into the paragraph's UTF-8 text (spec §2.1).

use core::ops::Range;

/// Byte offset into the paragraph's UTF-8 text (spec §2.1).
///
/// **[Design decision, §2.1]** Every internal offset is a UTF-8 byte offset and
/// synthetic elements (generated hyphens, glue) never displace them. UTF-16
/// conversion for platform text-input layers belongs to §6.7 and is not part of
/// Phase 0.
pub type ByteIndex = usize;

/// Half-open byte range `[start, end)`, aligned to code point boundaries (spec §2.1).
///
/// The fields are public because the range is a plain value type; the
/// invariant `start <= end` is checked by [`TextRange::new`] in debug builds.
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

    /// Builds a range. Debug builds assert `start <= end`.
    #[inline]
    #[must_use]
    pub const fn new(start: ByteIndex, end: ByteIndex) -> Self {
        debug_assert!(start <= end, "TextRange requires start <= end");
        Self { start, end }
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

    /// Length in bytes (0 if the range is inverted, which `new` rejects).
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// `true` when the range covers no bytes.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.start >= self.end
    }

    /// `true` when `offset` lies in `[start, end)`.
    #[inline]
    #[must_use]
    pub const fn contains(&self, offset: ByteIndex) -> bool {
        self.start <= offset && offset < self.end
    }

    /// `true` when `other` lies entirely within `self`.
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

impl From<Range<usize>> for TextRange {
    #[inline]
    fn from(r: Range<usize>) -> Self {
        Self::new(r.start, r.end)
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
    fn slice_respects_char_boundaries() {
        let text = "aé"; // 'é' is two bytes
        assert_eq!(TextRange::new(0, 1).slice(text), Some("a"));
        assert_eq!(TextRange::new(1, 3).slice(text), Some("é"));
        assert_eq!(TextRange::new(0, 2).slice(text), None); // splits 'é'
        assert_eq!(TextRange::new(0, 9).slice(text), None); // out of bounds
    }

    #[test]
    fn range_round_trip() {
        let r: TextRange = (3..9).into();
        let back: Range<usize> = r.into();
        assert_eq!(back, 3..9);
    }
}
