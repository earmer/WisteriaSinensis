//! The abstract shaping bridge (spec §4.1).

use crate::frontend::ShapingRun;
use crate::types::{
    Direction, FontFamily, FontFeatures, FontId, FontMetrics, FontRef, FontStyle, GraphemeAtom,
    LocaleTag, ScriptTag, ShapedCluster, TextRange,
};
use smallvec::SmallVec;

/// Edit applied at a line boundary before re-shaping (spec §4.1, §5 Stage 1.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BoundaryEdit {
    /// Re-shape unchanged.
    None,
    /// Append a (hyphen) character to the prefix; the glyph is synthesised.
    AppendHyphen {
        /// U+002D or the locale's hyphenation character (CSS `hyphenate-character`).
        hyphen: char,
    },
    /// Spelling-change hyphenation: drop `cut_bytes` from the end of the
    /// prefix, append `replacement`, then the hyphen. Mirrors
    /// `HyphenOpportunity` (libhyphen `rep`/`cut`).
    ReplacePrefix {
        /// Replacement text.
        replacement: Box<str>,
        /// Original bytes removed.
        cut_bytes: u32,
        /// Hyphen appended after the replacement.
        hyphen: char,
    },
}

/// Contract between the engine and a shaping backend (HarfRust, CoreText,
/// DirectWrite, a test double). Object-safe, so a backend can be chosen at run
/// time as `&dyn TextShaper`.
///
/// Output vectors are **appended to, never cleared**: callers own the buffer
/// and may reuse its capacity.
pub trait TextShaper {
    /// Resolves the primary font for a style and returns its identifier and
    /// complete metrics (§4.5). Used for the paragraph strut.
    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
    ) -> (FontId, FontMetrics);

    /// Shapes one complete run. Output is in **logical** order (§4.2 req. 2);
    /// `atoms` is the run's atom slice, and `ShapedCluster::atom_range` indexes it.
    #[allow(clippy::too_many_arguments)] // signature fixed by spec §4.1
    fn shape_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        families: &[FontFamily],
        font_size: f32,
        features: &FontFeatures,
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// Re-shapes a window around a break or hyphenation point (§5 Stage 1.4).
    fn reshape_window(
        &self,
        text: &str,
        run: &ShapingRun,
        window: TextRange,
        edit: BoundaryEdit,
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// GDEF ligature caret positions: `n - 1` for `n` components, empty if the
    /// font has none (§6.2). Positions are unshaped and may need kerning
    /// adjustment.
    fn ligature_carets(
        &self,
        font: FontRef,
        glyph_id: u32,
        direction: Direction,
    ) -> SmallVec<[f32; 4]>;
}
