//! Logical atoms: the extended-grapheme-cluster unit (spec §2.2).

use super::{BidiLevel, GlyphOrientation, ScriptTag, TextRange};

/// UAX #14 `Line_Break` property values `[U7]`.
///
/// Names follow the UAX #14 two-letter codes in Pascal case. The list covers
/// the classes defined through Unicode 17.0 (including `Ak`, `Ap`, `As`, `Vf`,
/// `Vi` and `Hh`); later versions may add values, hence `#[non_exhaustive]`. The data
/// version is pinned and reported by `conformance_report()` (§1.1).
/// One byte wide. `#[repr(u8)]` fixes the size only: the numeric values are
/// not part of the contract and move when Unicode adds a class, so the enum
/// is never stored or exchanged as a number (spec ATM-2).
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
    /// Unambiguous hyphen (Unicode 17.0, split from `Ba`).
    Hh,
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

bitflags::bitflags! {
    /// Per-atom boolean properties, stored as a 16-bit mask (spec §2.2).
    ///
    /// Thirteen of sixteen bits are assigned; the high three are reserved and
    /// are masked off by `from_bits_truncate` and `!`. Layout: 2 bytes,
    /// alignment 2.
    #[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
    pub struct AtomFlags: u16 {
        /// Han ideographs and full-width Kana.
        const IS_CJK_IDEOGRAPH = 1 << 0;
        /// The boundary after this atom is an inter-character justification
        /// opportunity (replaces explicit zero-width glue nodes).
        const ALLOW_CJK_JUSTIFY_AFTER = 1 << 1;
        /// Cursive scripts that admit no gaps for justification or letter-spacing
        /// (Arabic, Hanifi Rohingya, Mandaic, Mongolian, N'Ko, Phags Pa, Syriac;
        /// CSS Text 4 Appendix D `[W2]`). Split from the draft's `IS_CURSIVE_CONNECTED`.
        const NO_LETTER_SPACING = 1 << 2;
        /// Arabic-script atoms eligible for elongation (kashida). Open question 12.
        const KASHIDA_CAPABLE = 1 << 3;
        /// A UTS #51 emoji sequence: core, ZWJ, keycap, flag, tag or modifier `[U5]`.
        const IS_EMOJI_SEQUENCE = 1 << 4;
        /// Opening punctuation.
        const IS_OPEN_PUNCT = 1 << 5;
        /// Closing punctuation.
        const IS_CLOSE_PUNCT = 1 << 6;
        /// Cluster contains U+FE0E, U+FE0F or an IVS selector (U+E0100..U+E01EF);
        /// set after segmentation (spec §3 Step 1, FE-7).
        const HAS_VARIATION_SELECTOR = 1 << 7;
        /// Resolved intra-word apostrophe (spec §3 Step 2, FE-11).
        const IS_APOSTROPHE = 1 << 8;
        /// Continues an unbreakable FE-15 dash, ellipsis or middle-dot run (spec §3 Step 3, FE-15).
        const BINDS_TO_PREVIOUS = 1 << 9;
        /// UAX #9 L1 resets this atom with whitespace at a line end or before a
        /// tab: its first code point has class WS, S, BN, an isolate initiator or
        /// PDI, or an embedding or override control (spec FE-30, ST2-1).
        const L1_RESETTABLE = 1 << 10;
        /// An opening mark that may hang at a line start: UAX #14 class OP, or
        /// General Category Ps or Pi (spec ST1-3).
        const HANGABLE_START = 1 << 11;
        /// A stop or comma that may hang at a line end: U+002C, U+002E, U+3001,
        /// U+3002, U+FF0C or U+FF0E (spec ST1-3).
        const HANGABLE_END = 1 << 12;

        /// No flags set.
        const EMPTY = 0;
        /// Every assigned bit.
        const ALL = (1 << 13) - 1;
        /// Flags a cluster takes only from its **first** atom.
        const FIRST_ATOM_ONLY = Self::IS_OPEN_PUNCT.bits()
            | Self::BINDS_TO_PREVIOUS.bits()
            | Self::HANGABLE_START.bits();
        /// Flags a cluster takes only from its **last** atom: the justification
        /// opportunity *after* a ligature is the one after its last component, and
        /// a closing mark closes at the end, and a line-end mark hangs from it.
        const LAST_ATOM_ONLY = Self::ALLOW_CJK_JUSTIFY_AFTER.bits()
            | Self::IS_CLOSE_PUNCT.bits()
            | Self::HANGABLE_END.bits();
    }
}

impl AtomFlags {
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
}

/// One extended grapheme cluster (EGC) per UAX #29 including GB9c `[U2]`
/// (spec §2.2). The logical unit for carets, script, bidi level and break
/// classes.
///
/// Layout on 64-bit targets is 32 bytes, aligned to 8, including orientation
/// and padding; asserted at compile time below.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphemeAtom {
    /// Resolved glyph orientation.
    pub orientation: GlyphOrientation,
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
    assert!(core::mem::size_of::<GraphemeAtom>() == 32);
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
            orientation: GlyphOrientation::Horizontal,
            byte_range: TextRange::new(0, 1),
            script: ScriptTag::LATIN,
            bidi_level: crate::types::BidiLevel::LTR,
            linebreak_class: LineBreakClass::Al,
            flags,
        }
    }

    #[test]
    fn cluster_flags_inherit_per_flag_from_first_last_or_all() {
        let all = AtomFlags::ALL;
        // A three-atom ligature: only the outer atoms carry position-dependent flags.
        let lig = [
            atom_with(
                AtomFlags::IS_OPEN_PUNCT
                    | AtomFlags::BINDS_TO_PREVIOUS
                    | AtomFlags::ALLOW_CJK_JUSTIFY_AFTER,
            ),
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
        // IS_OPEN_PUNCT and BINDS_TO_PREVIOUS: from the first atom only.
        assert!(f.contains(AtomFlags::IS_OPEN_PUNCT));
        assert!(f.contains(AtomFlags::BINDS_TO_PREVIOUS));
        // IS_CLOSE_PUNCT: from the last atom only.
        assert!(f.contains(AtomFlags::IS_CLOSE_PUNCT));
        // Everything else is ORed.
        assert!(f.contains(AtomFlags::KASHIDA_CAPABLE | AtomFlags::IS_EMOJI_SEQUENCE));

        // Reverse the roles: first has no open mark or continuation bind, middle
        // atoms do; last has the justification opportunity but no close mark.
        let lig = [
            atom_with(AtomFlags::IS_CLOSE_PUNCT),
            atom_with(
                AtomFlags::IS_OPEN_PUNCT | AtomFlags::IS_CLOSE_PUNCT | AtomFlags::BINDS_TO_PREVIOUS,
            ),
            atom_with(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER | AtomFlags::BINDS_TO_PREVIOUS),
        ];
        let f = AtomFlags::for_cluster(&lig);
        assert!(f.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER));
        assert!(!f.contains(AtomFlags::IS_OPEN_PUNCT)); // first atom lacks it
        assert!(!f.contains(AtomFlags::BINDS_TO_PREVIOUS)); // first atom lacks it
        assert!(!f.contains(AtomFlags::IS_CLOSE_PUNCT)); // last atom lacks it

        // One atom: identity. No atoms: empty.
        assert_eq!(AtomFlags::for_cluster(&[atom_with(all)]), all);
        assert_eq!(AtomFlags::for_cluster(&[]), AtomFlags::EMPTY);
    }

    #[test]
    fn complement_and_truncate_stay_within_assigned_bits() {
        assert_eq!(!AtomFlags::EMPTY, AtomFlags::ALL);
        assert_eq!(!AtomFlags::ALL, AtomFlags::EMPTY);
        assert_eq!(AtomFlags::from_bits_truncate(0xFFFF), AtomFlags::ALL);
    }
}
