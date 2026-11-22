//! The shaper gate (spec AX-3, G2): checks every `TextShaper` reply at layout time.

use super::{LayoutInputError, LineHeight, StrutMetrics};
use crate::shaping::{ReshapeRequest, ShapeOutput, TextShaper};
use crate::types::{
    FontInstanceRef, FontMetrics, GraphemeAtom, RawShapedGlyph, ShapedCluster, ShapingRun,
    WritingMode, is_extent, is_measurement, natural_width,
};

pub(crate) fn checked_font_metrics(
    shaper: &dyn TextShaper,
    instance: FontInstanceRef<'_>,
) -> Result<FontMetrics, LayoutInputError> {
    Some(shaper.font_metrics(instance))
        .filter(FontMetrics::is_valid)
        .ok_or(LayoutInputError::InvalidShaperMetrics)
}

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

pub(crate) fn glyph_is_measured(glyph: &RawShapedGlyph) -> bool {
    [glyph.x_advance, glyph.x_offset, glyph.y_offset]
        .into_iter()
        .all(is_measurement)
}

pub(crate) fn checked_reshape_window(
    shaper: &dyn TextShaper,
    request: &ReshapeRequest<'_>,
    cluster: &ShapedCluster,
) -> Option<(Vec<RawShapedGlyph>, f32)> {
    let mut output = ShapeOutput::default();
    shaper.reshape_window(request, &mut output);
    let ShapeOutput { clusters, glyphs } = output;
    let [only] = clusters.as_slice() else {
        return None;
    };
    if only.atom_range != cluster.atom_range
        || only.font != cluster.font
        || only.glyph_range != (0..glyphs.len())
        || !is_extent(only.natural_width)
        || !glyphs.iter().all(glyph_is_measured)
    {
        return None;
    }
    let width = natural_width(&glyphs, request.run.letter_spacing);
    is_extent(width).then_some((glyphs, width))
}

pub(crate) fn checked_shape_run(
    shaper: &dyn TextShaper,
    text: &str,
    run: &ShapingRun,
    atoms: &[GraphemeAtom],
    mode: WritingMode,
    mut emit: impl FnMut(&ShapedCluster, &[RawShapedGlyph]) -> Result<(), LayoutInputError>,
) -> Result<(), LayoutInputError> {
    let mut output = ShapeOutput::default();
    shaper.shape_run(text, run, atoms, &mut output);
    for c in &output.clusters {
        let Some(glyphs) = output.glyphs.get(c.glyph_range.clone()) else {
            return Err(LayoutInputError::InvalidShaperMetrics);
        };
        if !(is_measurement(c.font.scale) && c.font.scale > 0.0)
            || !c.orientation.is_valid(mode, c.bidi_level)
            || !is_extent(c.natural_width)
            || glyphs.is_empty()
            || !glyphs.iter().all(glyph_is_measured)
        {
            return Err(LayoutInputError::InvalidShaperMetrics);
        }
        emit(c, glyphs)?;
    }
    Ok(())
}
