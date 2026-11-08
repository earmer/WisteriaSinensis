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
/// surface minimal. Ten of sixteen bits are assigned; the high six are
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
    /// The atom lies inside an `AtomicEditSpan` (mention, chip, code span; spec
    /// §2.2, §6): caret stops strictly inside the span are suppressed and
    /// Backspace / Delete remove the whole span. Combined by bitwise OR across
    /// the atoms of a cluster, so it is neither a first-only nor a last-only flag.
    /// `check_layout_inputs` requires it on exactly the atoms inside an atomic span.
    pub const IN_ATOMIC_EDIT_SPAN: Self = Self(1 << 9);

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
            | Self::IS_APOSTROPHE.0
            | Self::IN_ATOMIC_EDIT_SPAN.0,
    );

    const NAMES: [(Self, &'static str); 10] = [
        (Self::IS_CJK_IDEOGRAPH, "IS_CJK_IDEOGRAPH"),
        (Self::ALLOW_CJK_JUSTIFY_AFTER, "ALLOW_CJK_JUSTIFY_AFTER"),
        (Self::NO_LETTER_SPACING, "NO_LETTER_SPACING"),
        (Self::KASHIDA_CAPABLE, "KASHIDA_CAPABLE"),
        (Self::IS_EMOJI_SEQUENCE, "IS_EMOJI_SEQUENCE"),
        (Self::IS_OPEN_PUNCT, "IS_OPEN_PUNCT"),
        (Self::IS_CLOSE_PUNCT, "IS_CLOSE_PUNCT"),
        (Self::HAS_VARIATION_SELECTOR, "HAS_VARIATION_SELECTOR"),
        (Self::IS_APOSTROPHE, "IS_APOSTROPHE"),
        (Self::IN_ATOMIC_EDIT_SPAN, "IN_ATOMIC_EDIT_SPAN"),
    ];

    /// Flags a cluster takes only from its **first** atom.
    pub const FIRST_ATOM_ONLY: Self = Self::IS_OPEN_PUNCT;
    /// Flags a cluster takes only from its **last** atom: the justification
    /// opportunity *after* a ligature is the one after its last component, and
    /// a closing mark closes at the end.
    pub const LAST_ATOM_ONLY: Self = Self(Self::ALLOW_CJK_JUSTIFY_AFTER.0 | Self::IS_CLOSE_PUNCT.0);

    /// Aggregates the flags of the atoms a cluster covers (spec §2.3):
    /// [`FIRST_ATOM_ONLY`](Self::FIRST_ATOM_ONLY) from the first atom,
    /// [`LAST_ATOM_ONLY`](Self::LAST_ATOM_ONLY) from the last, and every other
    /// flag as the bitwise OR of all atoms. An empty slice gives
    /// [`EMPTY`](Self::EMPTY).
    #[must_use]
    pub fn for_cluster(atoms: &[GraphemeAtom]) -> Self {
        let (Some(first), Some(last)) = (atoms.first(), atoms.last()) else {
            return Self::EMPTY;
        };
        let ored = atoms.iter().fold(Self::EMPTY, |acc, a| acc | a.flags);
        (ored - Self::FIRST_ATOM_ONLY - Self::LAST_ATOM_ONLY)
            | (first.flags & Self::FIRST_ATOM_ONLY)
            | (last.flags & Self::LAST_ATOM_ONLY)
    }

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

    fn atom_with(flags: AtomFlags) -> GraphemeAtom {
        GraphemeAtom {
            byte_range: TextRange::new(0, 1),
            script: ScriptTag::LATIN,
            bidi_level: 0,
            linebreak_class: LineBreakClass::Al,
            flags,
        }
    }

    #[test]
    fn cluster_flags_inherit_per_flag_from_first_last_or_all() {
        let all = AtomFlags::ALL;
        // A three-atom ligature: only the outer atoms carry position-dependent flags.
        let lig = [
            atom_with(AtomFlags::IS_OPEN_PUNCT | AtomFlags::ALLOW_CJK_JUSTIFY_AFTER),
            atom_with(
                AtomFlags::IS_OPEN_PUNCT
                    | AtomFlags::IS_CLOSE_PUNCT
                    | AtomFlags::ALLOW_CJK_JUSTIFY_AFTER
                    | AtomFlags::KASHIDA_CAPABLE,
            ),
            atom_with(AtomFlags::IS_CLOSE_PUNCT | AtomFlags::IS_EMOJI_SEQUENCE),
        ];
        let f = AtomFlags::for_cluster(&lig);
        // ALLOW_CJK_JUSTIFY_AFTER: last atom has it not, so the cluster does not,
        // although the first and middle atoms do.
        assert!(!f.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER));
        // IS_OPEN_PUNCT: from the first atom only.
        assert!(f.contains(AtomFlags::IS_OPEN_PUNCT));
        // IS_CLOSE_PUNCT: from the last atom only.
        assert!(f.contains(AtomFlags::IS_CLOSE_PUNCT));
        // Everything else is ORed.
        assert!(f.contains(AtomFlags::KASHIDA_CAPABLE | AtomFlags::IS_EMOJI_SEQUENCE));

        // Reverse the roles: first has no open mark, middle atoms do; last has
        // the justification opportunity but no close mark.
        let lig = [
            atom_with(AtomFlags::IS_CLOSE_PUNCT),
            atom_with(AtomFlags::IS_OPEN_PUNCT | AtomFlags::IS_CLOSE_PUNCT),
            atom_with(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER),
        ];
        let f = AtomFlags::for_cluster(&lig);
        assert!(f.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER));
        assert!(!f.contains(AtomFlags::IS_OPEN_PUNCT)); // first atom lacks it
        assert!(!f.contains(AtomFlags::IS_CLOSE_PUNCT)); // last atom lacks it

        // One atom: identity. No atoms: empty.
        assert_eq!(AtomFlags::for_cluster(&[atom_with(all)]), all);
        assert_eq!(AtomFlags::for_cluster(&[]), AtomFlags::EMPTY);
    }

    #[test]
    fn atomic_edit_span_flag_is_ored_over_every_atom_of_a_cluster() {
        // Present on a middle atom only: the cluster still carries it.
        let lig = [
            atom_with(AtomFlags::EMPTY),
            atom_with(AtomFlags::IN_ATOMIC_EDIT_SPAN),
            atom_with(AtomFlags::EMPTY),
        ];
        assert!(AtomFlags::for_cluster(&lig).contains(AtomFlags::IN_ATOMIC_EDIT_SPAN));
        // It is neither a first-only nor a last-only flag.
        assert!(!AtomFlags::FIRST_ATOM_ONLY.intersects(AtomFlags::IN_ATOMIC_EDIT_SPAN));
        assert!(!AtomFlags::LAST_ATOM_ONLY.intersects(AtomFlags::IN_ATOMIC_EDIT_SPAN));
        // Absent everywhere: absent.
        assert!(
            !AtomFlags::for_cluster(&[atom_with(AtomFlags::EMPTY)])
                .contains(AtomFlags::IN_ATOMIC_EDIT_SPAN)
        );
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
