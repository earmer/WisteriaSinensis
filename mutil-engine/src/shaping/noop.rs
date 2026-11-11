//! A shaper that produces nothing, to prove the trait contract closes over the
//! pipeline types.

use super::{LeadingEdit, TextShaper, TrailingEdit};
use crate::frontend::ShapingRun;
use crate::types::{
    Direction, FontFamily, FontId, FontInstanceRef, FontMetrics, FontStyle, FontVariation,
    GraphemeAtom, LocaleTag, RawShapedGlyph, ScriptTag, ShapedCluster, TextRange,
};
use smallvec::SmallVec;

/// Returns empty data and all-zero metrics. Performs no shaping, measurement or
/// font selection: every request reports "no glyphs, no ligature carets,
/// `FontId(0)` with zero metrics". As a test double it is exempt from the
/// one-cluster contract for tate-chu-yoko runs: it returns no clusters at all.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopShaper;

impl TextShaper for NoopShaper {
    fn font_metrics(&self, _instance: FontInstanceRef<'_>) -> FontMetrics {
        FontMetrics::default()
    }
    fn resolve_primary_font_metrics(
        &self,
        _families: &[FontFamily],
        _style: FontStyle,
        _script: ScriptTag,
        _locale: &LocaleTag,
        _font_size: f32,
        _variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        (FontId::default(), FontMetrics::default())
    }

    fn shape_run(
        &self,
        _text: &str,
        _run: &ShapingRun,
        _atoms: &[GraphemeAtom],
        _output_clusters: &mut Vec<ShapedCluster>,
        _output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
    }

    fn reshape_window(
        &self,
        _text: &str,
        _run: &ShapingRun,
        _atoms: &[GraphemeAtom],
        _window: TextRange,
        _leading_edit: LeadingEdit,
        _trailing_edit: TrailingEdit,
        _output_clusters: &mut Vec<ShapedCluster>,
        _output_glyphs: &mut Vec<RawShapedGlyph>,
    ) {
    }

    fn ligature_carets(
        &self,
        _instance: FontInstanceRef<'_>,
        _glyph_id: u32,
        _direction: Direction,
    ) -> SmallVec<[f32; 4]> {
        SmallVec::new()
    }
}
