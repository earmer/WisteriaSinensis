//! Logical atoms: the extended-grapheme-cluster unit (spec §2.2).

use super::{BidiLevel, ScriptTag, TextRange};
use core::fmt;
use core::ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, Not, Sub};

/// UAX #14 `Line_Break` property values `[U7]`.
///
/// Names follow the UAX #14 two-letter codes in Pascal case. The list covers
/// the classes defined through Unicode 15.1 (including `Ak`, `Ap`, `As`, `Vf`,
/// `Vi`); Unicode 18 may add values, hence `#[non_exhaustive]`. The data
/// version is pinned and reported by `conformance_report()` (§1.1, Phase 1).
/// One byte wide.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[repr(u8)]
pub enum LineBreakClass {
    /// Mandatory break.
    Bk,
    /// Carriage return.
    Cr,
    /// Line feed.
    Lf,
    /// Combining mark.
    Cm,
    /// Next line.
    Nl,
    /// Surrogate.
    Sg,
    /// Word joiner.
    Wj,
    /// Zero-width space.
    Zw,
    /// Non-breaking (glue).
    Gl,
    /// Space.
    Sp,
    /// Zero-width joiner.
    Zwj,
    /// Break opportunity before and after.
    B2,
    /// Break after.
    Ba,
    /// Break before.
    Bb,
    /// Hyphen.
    Hy,
    /// Contingent break.
    Cb,
    /// Close punctuation.
    Cl,
    /// Close parenthesis.
    Cp,
    /// Exclamation / interrogation.
    Ex,
    /// Inseparable.
    In,
    /// Non-starter.
    Ns,
    /// Open punctuation.
    Op,
    /// Quotation.
    Qu,
    /// Infix numeric separator.
    Is,
    /// Numeric.
    Nu,
    /// Postfix numeric.
    Po,
    /// Prefix numeric.
    Pr,
    /// Symbols allowing break after.
    Sy,
    /// Ambiguous (alphabetic or ideographic).
    Ai,
    /// Aksara.
    Ak,
    /// Alphabetic.
    Al,
    /// Aksara prebase.
    Ap,
    /// Aksara start.
    As,
    /// Conditional Japanese starter.
    Cj,
    /// Emoji base.
    Eb,
    /// Emoji modifier.
    Em,
    /// Hangul LV syllable.
    H2,
    /// Hangul LVT syllable.
    H3,
    /// Hebrew letter.
    Hl,
    /// Ideographic.
    Id,
    /// Hangul L jamo.
    Jl,
    /// Hangul T jamo.
    Jt,
    /// Hangul V jamo.
    Jv,
    /// Regional indicator.
    Ri,
    /// Complex-context dependent (South East Asian).
    Sa,
    /// Virama final.
    Vf,
    /// Virama.
    Vi,
    /// Unknown. The default, as UAX #14 LB1 resolves it to `Al`.
    #[default]
    Xx,
}

/// Per-atom boolean properties, stored as a 16-bit mask (spec §2.2).
///
/// Hand-rolled instead of depending on `bitflags`, to keep the dependency
/// surface minimal. Nine of sixteen bits are assigned; the high seven are
/// reserved and are masked off by [`AtomFlags::from_bits_truncate`] and `!`.
/// Layout: 2 bytes, alignment 2.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct AtomFlags(u16);

impl AtomFlags {
    /// Han ideographs and full-width Kana.
    pub const IS_CJK_IDEOGRAPH: Self = Self(1 << 0);
    /// The boundary after this atom is an inter-character justification
    /// opportunity (replaces explicit zero-width glue nodes).
    pub const ALLOW_CJK_JUSTIFY_AFTER: Self = Self(1 << 1);
    /// Cursive scripts that admit no gaps for justification or letter-spacing
    /// (Arabic, Hanifi Rohingya, Mandaic, Mongolian, N'Ko, Phags Pa, Syriac;
    /// CSS Text 4 Appendix D `[W2]`). Split from the draft's `IS_CURSIVE_CONNECTED`.
    pub const NO_LETTER_SPACING: Self = Self(1 << 2);
    /// Arabic-script atoms eligible for elongation (kashida). Open question 12.
    pub const KASHIDA_CAPABLE: Self = Self(1 << 3);
    /// A UTS #51 emoji sequence: core, ZWJ, keycap, flag, tag or modifier `[U5]`.
    pub const IS_EMOJI_SEQUENCE: Self = Self(1 << 4);
    /// Opening punctuation.
    pub const IS_OPEN_PUNCT: Self = Self(1 << 5);
    /// Closing punctuation.
    pub const IS_CLOSE_PUNCT: Self = Self(1 << 6);
    /// Cluster contains U+FE0E, U+FE0F or an IVS selector (U+E0100..U+E01EF);
    /// set after segmentation (§3 Step 1.4).
    pub const HAS_VARIATION_SELECTOR: Self = Self(1 << 7);
    /// Resolved intra-word apostrophe (§3 Step 2.4).
    pub const IS_APOSTROPHE: Self = Self(1 << 8);

    /// No flags set.
    pub const EMPTY: Self = Self(0);

    /// Every assigned bit.
    pub const ALL: Self = Self(
        Self::IS_CJK_IDEOGRAPH.0
            | Self::ALLOW_CJK_JUSTIFY_AFTER.0
            | Self::NO_LETTER_SPACING.0
            | Self::KASHIDA_CAPABLE.0
            | Self::IS_EMOJI_SEQUENCE.0
            | Self::IS_OPEN_PUNCT.0
            | Self::IS_CLOSE_PUNCT.0
            | Self::HAS_VARIATION_SELECTOR.0
            | Self::IS_APOSTROPHE.0,
    );

    const NAMES: [(Self, &'static str); 9] = [
        (Self::IS_CJK_IDEOGRAPH, "IS_CJK_IDEOGRAPH"),
        (Self::ALLOW_CJK_JUSTIFY_AFTER, "ALLOW_CJK_JUSTIFY_AFTER"),
        (Self::NO_LETTER_SPACING, "NO_LETTER_SPACING"),
        (Self::KASHIDA_CAPABLE, "KASHIDA_CAPABLE"),
        (Self::IS_EMOJI_SEQUENCE, "IS_EMOJI_SEQUENCE"),
        (Self::IS_OPEN_PUNCT, "IS_OPEN_PUNCT"),
        (Self::IS_CLOSE_PUNCT, "IS_CLOSE_PUNCT"),
        (Self::HAS_VARIATION_SELECTOR, "HAS_VARIATION_SELECTOR"),
        (Self::IS_APOSTROPHE, "IS_APOSTROPHE"),
    ];

    /// The raw bits.
    #[inline]
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Builds from raw bits, discarding unassigned ones.
    #[inline]
    #[must_use]
    pub const fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// `true` when no flag is set.
    #[inline]
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// `true` when every flag in `other` is set in `self`.
    #[inline]
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `true` when at least one flag in `other` is set in `self`.
    #[inline]
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Union (usable in `const` context, unlike `|`).
    #[inline]
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Sets the flags in `other`.
    #[inline]
    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    /// Clears the flags in `other`.
    #[inline]
    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    /// Sets or clears the flags in `other` depending on `value`.
    #[inline]
    pub fn set(&mut self, other: Self, value: bool) {
        if value {
            self.insert(other);
        } else {
            self.remove(other);
        }
    }
}

impl BitOr for AtomFlags {
    type Output = Self;
    #[inline]
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}
impl BitOrAssign for AtomFlags {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}
impl BitAnd for AtomFlags {
    type Output = Self;
    #[inline]
    fn bitand(self, rhs: Self) -> Self {
        Self(self.0 & rhs.0)
    }
}
impl BitAndAssign for AtomFlags {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0;
    }
}
impl Sub for AtomFlags {
    type Output = Self;
    /// Set difference: flags in `self` that are not in `rhs`.
    #[inline]
    fn sub(self, rhs: Self) -> Self {
        Self(self.0 & !rhs.0)
    }
}
impl Not for AtomFlags {
    type Output = Self;
    /// Complement within the assigned bits.
    #[inline]
    fn not(self) -> Self {
        Self(!self.0 & Self::ALL.0)
    }
}

impl fmt::Debug for AtomFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("AtomFlags(EMPTY)");
        }
        f.write_str("AtomFlags(")?;
        let mut first = true;
        for (flag, name) in Self::NAMES {
            if self.contains(flag) {
                if !first {
                    f.write_str(" | ")?;
                }
                f.write_str(name)?;
                first = false;
            }
        }
        f.write_str(")")
    }
}

/// One extended grapheme cluster (EGC) per UAX #29 including GB9c `[U2]`
/// (spec §2.2). The logical unit for carets, script, bidi level and break
/// classes.
///
/// Layout (64-bit): `TextRange` 16 + `ScriptTag` 4 + `BidiLevel` 1 +
/// `LineBreakClass` 1 + `AtomFlags` 2 = 24 bytes, alignment 8, no padding;
/// asserted at compile time below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphemeAtom {
    /// Bytes of the cluster in the paragraph text.
    pub byte_range: TextRange,
    /// Resolved script. After frontend resolution never `Zyyy` or `Zinh` (§2.1).
    pub script: ScriptTag,
    /// UAX #9 level of the atom's first code point; non-spacing marks follow
    /// their base (§3 Step 2.3).
    pub bidi_level: BidiLevel,
    /// `Line_Break` class of the atom's **base (first) code point**
    /// **[Design decision, §2.2]**. The segmenter itself runs over the original
    /// text so that LB9/LB10 see the real sequence.
    pub linebreak_class: LineBreakClass,
    /// Per-atom boolean properties.
    pub flags: AtomFlags,
}

#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<GraphemeAtom>() == 24);
    assert!(core::mem::align_of::<GraphemeAtom>() == 8);
    assert!(core::mem::size_of::<AtomFlags>() == 2);
    assert!(core::mem::size_of::<LineBreakClass>() == 1);
    assert!(core::mem::size_of::<ScriptTag>() == 4);
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_algebra() {
        let mut f = AtomFlags::IS_CJK_IDEOGRAPH | AtomFlags::ALLOW_CJK_JUSTIFY_AFTER;
        assert!(f.contains(AtomFlags::IS_CJK_IDEOGRAPH));
        assert!(!f.contains(AtomFlags::NO_LETTER_SPACING));
        assert!(f.intersects(AtomFlags::IS_CJK_IDEOGRAPH | AtomFlags::KASHIDA_CAPABLE));

        f.insert(AtomFlags::IS_EMOJI_SEQUENCE);
        f.remove(AtomFlags::IS_CJK_IDEOGRAPH);
        assert!(!f.contains(AtomFlags::IS_CJK_IDEOGRAPH));
        assert!(f.contains(AtomFlags::IS_EMOJI_SEQUENCE));

        f.set(AtomFlags::IS_APOSTROPHE, true);
        f.set(AtomFlags::IS_APOSTROPHE, false);
        assert!(!f.contains(AtomFlags::IS_APOSTROPHE));

        assert_eq!(
            f - AtomFlags::IS_EMOJI_SEQUENCE,
            AtomFlags::ALLOW_CJK_JUSTIFY_AFTER
        );
        assert!(AtomFlags::EMPTY.is_empty());
    }

    #[test]
    fn assigned_bits_are_distinct() {
        let mut seen = 0u16;
        for (flag, _) in AtomFlags::NAMES {
            assert_eq!(flag.bits().count_ones(), 1);
            assert_eq!(seen & flag.bits(), 0, "duplicate bit");
            seen |= flag.bits();
        }
        assert_eq!(seen, AtomFlags::ALL.bits());
    }

    #[test]
    fn complement_and_truncate_stay_within_assigned_bits() {
        assert_eq!(!AtomFlags::EMPTY, AtomFlags::ALL);
        assert_eq!(!AtomFlags::ALL, AtomFlags::EMPTY);
        assert_eq!(AtomFlags::from_bits_truncate(0xFFFF), AtomFlags::ALL);
    }

    #[test]
    fn debug_lists_names() {
        let f = AtomFlags::NO_LETTER_SPACING | AtomFlags::KASHIDA_CAPABLE;
        assert_eq!(
            format!("{f:?}"),
            "AtomFlags(NO_LETTER_SPACING | KASHIDA_CAPABLE)"
        );
        assert_eq!(format!("{:?}", AtomFlags::EMPTY), "AtomFlags(EMPTY)");
    }
}
