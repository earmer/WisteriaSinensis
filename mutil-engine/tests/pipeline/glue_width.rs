//! The width of a word-space glue is `max(0, a + word_spacing)` with `a` the
//! backend's space advance (FE-27); the clamp absorbs a negative
//! `word_spacing` only, and the input gate (G1) judges the glue it produced
//! (WN-10, SHP-18).

use crate::common::*;
use mutil_engine::*;

/// A line backend of 6-unit glyphs whose `space_advance` is `space`.
struct SpaceAdvance {
    space: f32,
}

impl LineShaper for SpaceAdvance {
    fn shape_line(
        &self,
        text: &str,
        _families: &[FontFamily],
        _style: FontStyle,
        _font_size: f32,
        _features: &FontFeatures,
    ) -> Vec<LineGlyph> {
        text.char_indices()
            .map(|(byte_index, c)| LineGlyph {
                font: FontId(0),
                glyph_id: glyph_for(FONT_BYTES, c).unwrap_or(0),
                byte_index,
                x_advance: 6.0,
                x_offset: 0.0,
                y_offset: 0.0,
            })
            .collect()
    }

    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        SHAPER.font_metrics(instance)
    }

    fn resolve_primary_font_metrics(
        &self,
        families: &[FontFamily],
        style: FontStyle,
        script: ScriptTag,
        locale: &LocaleTag,
        font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics) {
        SHAPER.resolve_primary_font_metrics(families, style, script, locale, font_size, variations)
    }

    fn space_advance(&self, _instance: FontInstanceRef<'_>, _orientation: GlyphOrientation) -> f32 {
        self.space
    }
}

/// Compiles "a b" with the given space advance and word spacing, and runs the
/// input gate on the result.
fn compile_and_gate(space: f32, word_spacing: f32) -> (ParagraphAst, Result<(), LayoutInputError>) {
    let text = "a b";
    let mut spaced = style(10.0);
    spaced.word_spacing = word_spacing;
    let shaper = LineOnlyShaper::new(SpaceAdvance { space });
    let ast = compile_paragraph(&input(text, &spaced), &FrontendOptions::default(), &shaper)
        .unwrap_or_else(|e| panic!("a = {space}, spacing = {word_spacing}: {e}"));
    let checked = check_layout_inputs(&ast, text, &column(200.0), &LayoutOptions::default());
    (ast, checked)
}

fn glue_of(ast: &ParagraphAst) -> &GlueNode {
    ast.items
        .iter()
        .find_map(|item| match item {
            WordNode::Glue(glue) => Some(glue),
            _ => None,
        })
        .expect("the paragraph has a word space")
}

#[test]
fn fe27_wn10_a_negative_word_spacing_that_overshoots_gives_a_zero_width_glue_the_gate_accepts() {
    for (space, word_spacing) in [(6.0, -10.0), (6.0, -6.0), (0.0, -1.0), (6.0, -1e6)] {
        let (ast, checked) = compile_and_gate(space, word_spacing);
        let glue = glue_of(&ast);
        assert_eq!(
            glue.natural_width, 0.0,
            "a = {space}, spacing = {word_spacing}"
        );
        assert_eq!(glue.max_stretch, 0.0);
        assert_eq!(glue.max_shrink, 0.0);
        assert_eq!(checked, Ok(()), "a = {space}, spacing = {word_spacing}");
    }
}

#[test]
fn fe27_a_valid_advance_and_a_word_spacing_whose_sum_is_no_measurement_is_an_invalid_glue() {
    // Each is a measurement (MSR-1, at the bound), the sum is not. A style
    // spacing is itself a measurement, so the sum cannot reach infinity
    // through `word_spacing` (see the report on this gap); the gate's verdict
    // on a glue that is not a measurement is what is pinned here.
    for (space, word_spacing) in [
        (MAX_MEASUREMENT, MAX_MEASUREMENT),
        (MAX_MEASUREMENT, MAX_MEASUREMENT / 2.0),
    ] {
        let (_, checked) = compile_and_gate(space, word_spacing);
        assert!(
            matches!(checked, Err(LayoutInputError::InvalidGlue { .. })),
            "a = {space}, spacing = {word_spacing}: {checked:?}"
        );
    }
    // The sum at the bound is still a measurement.
    let (_, checked) = compile_and_gate(MAX_MEASUREMENT, 0.0);
    assert!(
        !matches!(checked, Err(LayoutInputError::InvalidGlue { .. })),
        "{checked:?}"
    );
}

#[test]
fn fe27_shp18_a_negative_advance_is_an_invalid_glue_whatever_the_word_spacing() {
    for (space, word_spacing) in [(-3.0, 100.0), (-0.5, 1e6), (-3.0, 3.0), (-3.0, 0.0)] {
        let (ast, checked) = compile_and_gate(space, word_spacing);
        // Not clamped to zero: the glue keeps a width the gate then rejects
        // (the sum itself would have been valid for the first two).
        assert!(
            matches!(checked, Err(LayoutInputError::InvalidGlue { .. })),
            "a = {space}, spacing = {word_spacing}: {checked:?} (glue {})",
            glue_of(&ast).natural_width
        );
    }
}
