//! A shaper that produces nothing, to prove the trait contract closes over the
//! pipeline types.

use super::{BoundaryEdit, TextShaper};
use crate::frontend::ShapingRun;
use crate::types::{
    Direction, FontFamily, FontFeatures, FontId, FontMetrics, FontRef, FontStyle, GraphemeAtom,
    LocaleTag, ScriptTag, ShapedCluster, TextRange,
};
use smallvec::SmallVec;

/// Returns empty data and all-zero metrics. Performs no shaping, measurement or
/// font selection: every request reports "no glyphs, no ligature carets,
/// `FontId(0)` with zero metrics".
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopShaper;

impl TextShaper for NoopShaper {
    fn resolve_primary_font_metrics(
        &self,
        _families: &[FontFamily],
        _style: FontStyle,
        _script: ScriptTag,
        _locale: &LocaleTag,
        _font_size: f32,
    ) -> (FontId, FontMetrics) {
        (FontId::default(), FontMetrics::default())
    }

    fn shape_run(
        &self,
        _text: &str,
        _run: &ShapingRun,
        _atoms: &[GraphemeAtom],
        _families: &[FontFamily],
        _font_size: f32,
        _features: &FontFeatures,
        _output_clusters: &mut Vec<ShapedCluster>,
    ) {
    }

    fn reshape_window(
        &self,
        _text: &str,
        _run: &ShapingRun,
        _atoms: &[GraphemeAtom],
        _window: TextRange,
        _features: &FontFeatures,
        _edit: BoundaryEdit,
        _output_clusters: &mut Vec<ShapedCluster>,
    ) {
    }

    fn ligature_carets(
        &self,
        _font: FontRef,
        _glyph_id: u32,
        _direction: Direction,
        _font_size: f32,
    ) -> SmallVec<[f32; 4]> {
        SmallVec::new()
    }
}
