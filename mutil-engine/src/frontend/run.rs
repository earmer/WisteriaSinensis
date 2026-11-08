//! Shaping runs: the unit handed to a shaping backend (spec §2.5, §3 Step 5).

use crate::types::{BidiLevel, Direction, FamilyStack, FontStyle, LocaleTag, ScriptTag, TextRange};

/// Text decoration requested for a run. Colour is deliberately absent: it is
/// applied at paint time over a single shaping context (§3 Step 5).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextDecoration {
    /// Underline requested.
    pub underline: bool,
    /// Strike-through requested.
    pub strikethrough: bool,
}

/// A maximal span with one family stack, style, script, locale, bidi level and
/// decoration (spec §2.5). Atoms never straddle runs (§3 Step 5).
#[derive(Clone, Debug, PartialEq)]
pub struct ShapingRun {
    /// Complete shaping settings; the run is their single source of truth.
    pub features: crate::types::FontFeatures,
    /// Bytes of the paragraph covered by the run.
    pub byte_range: TextRange,
    /// Ordered family stack; shared between runs by reference count.
    pub families: FamilyStack,
    /// Requested font size; shaping and reshaping both read it from this run.
    pub font_size: f32,
    /// Weight and slant.
    pub style: FontStyle,
    /// Resolved script (never `Zyyy`/`Zinh` after the frontend, §2.1).
    pub script: ScriptTag,
    /// Explicit or derived locale (§1.2, Step 4).
    pub locale: LocaleTag,
    /// UAX #9 level shared by every atom of the run.
    pub bidi_level: BidiLevel,
    /// Requested decoration.
    pub decoration: TextDecoration,
}

impl ShapingRun {
    /// Buffer direction for shaping: RTL iff the level is odd (§4.2 req. 2).
    #[inline]
    #[must_use]
    pub const fn direction(&self) -> Direction {
        Direction::from_level(self.bidi_level)
    }
}
