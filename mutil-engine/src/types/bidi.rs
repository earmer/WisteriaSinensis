//! Bidi levels and directions (spec §2.1, UAX #9 `[U3]`).

/// UAX #9 embedding level: even = LTR, odd = RTL (spec §2.1).
pub type BidiLevel = u8;

/// UAX #9 `max_depth` (BD2). Explicit levels never exceed this; implicit
/// resolution can reach `BIDI_MAX_DEPTH + 1` `[U3]`.
pub const BIDI_MAX_DEPTH: BidiLevel = 125;

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
        if level & 1 == 1 { Self::Rtl } else { Self::Ltr }
    }

    /// The lowest embedding level of this direction (0 for LTR, 1 for RTL),
    /// i.e. the paragraph embedding level (UAX #9 P2/P3).
    #[inline]
    #[must_use]
    pub const fn base_level(self) -> BidiLevel {
        match self {
            Self::Ltr => 0,
            Self::Rtl => 1,
        }
    }

    /// `true` for [`Direction::Rtl`].
    #[inline]
    #[must_use]
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parity_gives_direction() {
        assert_eq!(Direction::from_level(0), Direction::Ltr);
        assert_eq!(Direction::from_level(1), Direction::Rtl);
        assert_eq!(Direction::from_level(2), Direction::Ltr);
        assert_eq!(Direction::from_level(BIDI_MAX_DEPTH), Direction::Rtl);
        assert_eq!(Direction::Rtl.base_level(), 1);
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
