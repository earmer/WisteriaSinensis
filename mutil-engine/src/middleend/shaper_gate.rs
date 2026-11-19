//! The shaper gate (spec AX-3, G2): every answer a [`TextShaper`] gives at
//! layout time is checked here, once, before layout uses it.
//!
//! Shaping output that the frontend stores in the AST is checked by the input
//! gate instead. Ligature carets are checked by
//! [`resolve_ligature_carets`](super::resolve_ligature_carets) (spec INT-5).

use super::{LayoutInputError, LineHeight, StrutMetrics};
use crate::shaping::{ReshapeRequest, ShapeOutput, TextShaper};
use crate::types::{
    FontInstanceRef, FontMetrics, GraphemeAtom, RawShapedGlyph, ShapingRun, TextRange, WritingMode,
    is_extent, is_measurement,
};

/// The shaper's metrics for one font instance, checked against MSR-1 and
/// FNT-4 before layout uses them.
///
/// # Errors
/// `InvalidShaperMetrics` when any metric or line-gap relation fails
/// [`FontMetrics::is_valid`].
pub(crate) fn checked_font_metrics(
    shaper: &dyn TextShaper,
    instance: FontInstanceRef<'_>,
) -> Result<FontMetrics, LayoutInputError> {
    Some(shaper.font_metrics(instance))
        .filter(FontMetrics::is_valid)
        .ok_or(LayoutInputError::InvalidShaperMetrics)
}

/// The line box of one font instance under `line_height`: the shaper's
/// metrics for `instance`, checked (spec FNT-4, MSR-1), resolved like the
/// paragraph strut and checked again as a strut.
///
/// # Errors
/// `InvalidShaperMetrics` when the shaper's metrics are invalid or resolve to an
/// invalid line box.
pub(crate) fn checked_line_box(
    shaper: &dyn TextShaper,
    instance: FontInstanceRef<'_>,
    line_height: LineHeight,
    mode: WritingMode,
) -> Result<StrutMetrics, LayoutInputError> {
    let font_size = instance.font_size;
    let metrics = checked_font_metrics(shaper, instance)?;
    resolved_line_box(&metrics, font_size, line_height, mode)
}

/// The line box `line_height` resolves from already checked `metrics`, like
/// the paragraph strut, checked again as a strut.
///
/// # Errors
/// `InvalidShaperMetrics` when the metrics resolve to an invalid line box.
pub(crate) fn resolved_line_box(
    metrics: &FontMetrics,
    font_size: f32,
    line_height: LineHeight,
    mode: WritingMode,
) -> Result<StrutMetrics, LayoutInputError> {
    Some(line_height.resolve_strut(metrics, font_size, mode))
        .filter(StrutMetrics::is_valid)
        .ok_or(LayoutInputError::InvalidShaperMetrics)
}

/// Whether every measurement of `glyph` is a measurement (MSR-1).
pub(crate) fn glyph_is_measured(glyph: &RawShapedGlyph) -> bool {
    [glyph.x_advance, glyph.x_offset, glyph.y_offset]
        .into_iter()
        .all(is_measurement)
}

/// Re-shape one cluster window with no edits and verify the shaper's reply
/// (spec ST1-5, G2, MSR-1).
///
/// Returns the replacement glyphs and adjusted cluster width when the shaper
/// produces a single cluster matching `expected_atom_range` with valid glyph
/// bounds and measurements; otherwise returns `None` so callers keep the
/// original shaping.
pub(crate) fn checked_reshape_window(
    shaper: &dyn TextShaper,
    text: &str,
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    window: TextRange,
    expected_atom_range: core::ops::Range<usize>,
) -> Option<(Vec<RawShapedGlyph>, f32)> {
    let mut output = ShapeOutput::default();
    shaper.reshape_window(
        &ReshapeRequest::plain(text, run, atoms, window),
        &mut output,
    );
    let ShapeOutput { clusters, glyphs } = output;
    let [only] = clusters.as_slice() else {
        return None;
    };
    if only.atom_range != expected_atom_range
        || only.glyph_range != (0..glyphs.len())
        || !is_extent(only.natural_width)
        || !glyphs.iter().all(glyph_is_measured)
    {
        return None;
    }
    let advance: f32 = glyphs.iter().map(|g| g.x_advance).sum();
    let width = (advance + run.letter_spacing).max(0.0);
    is_extent(width).then_some((glyphs, width))
}
