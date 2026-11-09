//! Shaping runs: the unit handed to a shaping backend (spec §2.5, §3 Step 5).

use core::ops::Range;

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
    /// Alignment relative to the paragraph main baseline.
    pub baseline_align: crate::types::BaselineAlignment,
    /// Finite extra spacing between eligible atoms.
    pub letter_spacing: f32,
    /// Finite extra advance for word spaces.
    pub word_spacing: f32,
    /// Resolved glyph orientation.
    pub orientation: crate::types::GlyphOrientation,
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
    /// This run's atoms: `ast.atoms[atom_range]` for a body run (the slice
    /// handed to `TextShaper::shape_run` and `reshape_window`), or
    /// `ann.atoms[atom_range]` for an annotation run, where it is always
    /// `0..ann.atoms.len()`. Shapers ignore it. Paragraph-level index space.
    pub atom_range: Range<usize>,
    /// This run's clusters: `ast.clusters[cluster_range]` for a body run, or
    /// `ann.clusters[cluster_range]` (always `0..ann.clusters.len()`) for an
    /// annotation run. Shapers ignore it. Paragraph-level index space.
    pub cluster_range: Range<usize>,
}

impl ShapingRun {
    /// Buffer direction for shaping: RTL iff the level is odd (§4.2 req. 2).
    #[inline]
    #[must_use]
    pub const fn direction(&self) -> Direction {
        Direction::from_level(self.bidi_level)
    }
}
