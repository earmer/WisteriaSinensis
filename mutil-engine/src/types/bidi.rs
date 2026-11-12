//! Bidi levels and directions (spec §2.1, UAX #9 `[U3]`).

/// UAX #9 `max_depth` (BD2). Explicit levels never exceed this; implicit
/// resolution can reach `BIDI_MAX_DEPTH + 1` `[U3]`.
pub const BIDI_MAX_DEPTH: u8 = 125;

/// A UAX #9 embedding level: even = LTR, odd = RTL (spec §2.1).
///
/// Parsed, not validated (spec §1.5, A3): [`new`](Self::new) refuses a level
/// above [`BidiLevel::MAX`] (`BIDI_MAX_DEPTH + 1`, the highest level implicit
/// resolution reaches), so a level that exists is one UAX #9 can produce.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BidiLevel(u8);

impl BidiLevel {
    /// Level 0: the paragraph level of an LTR paragraph.
    pub const LTR: Self = Self(0);
    /// Level 1: the paragraph level of an RTL paragraph.
    pub const RTL: Self = Self(1);
    /// The highest level: `BIDI_MAX_DEPTH + 1`.
    pub const MAX: Self = Self(BIDI_MAX_DEPTH + 1);

    /// The level `level`, or `None` above [`MAX`](Self::MAX).
    #[must_use]
    pub const fn new(level: u8) -> Option<Self> {
        if level <= Self::MAX.0 {
            Some(Self(level))
        } else {
            None
        }
    }

    /// The level as a number.
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }

    /// The direction of the level: odd levels are RTL.
    #[must_use]
    pub const fn direction(self) -> Direction {
        if self.0 & 1 == 1 {
            Direction::Rtl
        } else {
            Direction::Ltr
        }
    }
}

/// Text direction of a level or paragraph.
///
/// Spec §2.5 calls the paragraph-level value `BidiDirection` and §4.1 uses
/// `Direction` for the shaper; both are one two-valued concept, so they are one
/// type with an alias ([`BidiDirection`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Left to right (even level).
    #[default]
    Ltr,
    /// Right to left (odd level).
    Rtl,
}

/// The paragraph base direction type named by spec §2.5.
pub type BidiDirection = Direction;

impl Direction {
    /// Direction implied by an embedding level (odd = RTL).
    #[inline]
    #[must_use]
    pub const fn from_level(level: BidiLevel) -> Self {
        level.direction()
    }

    /// The lowest embedding level of this direction (0 for LTR, 1 for RTL),
    /// i.e. the paragraph embedding level (UAX #9 P2/P3).
    #[inline]
    #[must_use]
    pub const fn base_level(self) -> BidiLevel {
        match self {
            Self::Ltr => BidiLevel::LTR,
            Self::Rtl => BidiLevel::RTL,
        }
    }

    /// `true` for [`Direction::Rtl`].
    #[inline]
    #[must_use]
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// Original UAX #9 bidi class (`Bidi_Class`), retained for line-local
/// L1 processing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BidiClass {
    /// Left-to-Right (L).
    L,
    /// Right-to-Left (R).
    R,
    /// Arabic Letter (AL).
    Al,
    /// European Number (EN).
    En,
    /// European Separator (ES).
    Es,
    /// European Terminator (ET).
    Et,
    /// Arabic Number (AN).
    An,
    /// Common Separator (CS).
    Cs,
    /// Nonspacing Mark (NSM).
    Nsm,
    /// Boundary Neutral (BN).
    Bn,
    /// Paragraph Separator (B).
    B,
    /// Segment Separator (S).
    S,
    /// White Space (WS).
    Ws,
    /// Other Neutrals (ON).
    On,
    /// Left-to-Right Embedding (LRE).
    Lre,
    /// Left-to-Right Override (LRO).
    Lro,
    /// Right-to-Left Embedding (RLE).
    Rle,
    /// Right-to-Left Override (RLO).
    Rlo,
    /// Pop Directional Format (PDF).
    Pdf,
    /// Left-to-Right Isolate (LRI).
    Lri,
    /// Right-to-Left Isolate (RLI).
    Rli,
    /// First Strong Isolate (FSI).
    Fsi,
    /// Pop Directional Isolate (PDI).
    Pdi,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parity_gives_direction() {
        assert_eq!(Direction::from_level(BidiLevel::LTR), Direction::Ltr);
        assert_eq!(Direction::from_level(BidiLevel::RTL), Direction::Rtl);
        assert_eq!(
            Direction::from_level(BidiLevel::new(2).unwrap()),
            Direction::Ltr
        );
        assert_eq!(
            Direction::from_level(BidiLevel::new(BIDI_MAX_DEPTH).unwrap()),
            Direction::Rtl
        );
        assert_eq!(BidiLevel::MAX.get(), BIDI_MAX_DEPTH + 1);
        assert_eq!(BidiLevel::new(BIDI_MAX_DEPTH + 2), None);
        assert_eq!(Direction::Rtl.base_level(), BidiLevel::RTL);
    }
}
