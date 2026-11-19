//! `ShapeOutput` reuse (SHP-4) and a line backend's numbers on a window with
//! a hyphen edit (CLU-6, SHP-18, SHP-24).

use crate::common::*;
use mutil_engine::*;

fn shape(ast: &ParagraphAst, text: &str, out: &mut ShapeOutput) {
    SHAPER.shape_run(text, &ast.runs[0], ast.run_atoms(0), out);
}

/// What `out` holds, in a comparable form.
fn held(out: &ShapeOutput) -> String {
    format!("{:?}\n{:?}", out.clusters, out.glyphs)
}

#[test]
fn shp4_shape_run_appends_and_clear_makes_reuse_equal_a_fresh_output() {
    let (first, second) = ("abc", "xy z");
    let (first_ast, second_ast) = (compile(first), compile(second));
    let mut fresh_first = ShapeOutput::default();
    shape(&first_ast, first, &mut fresh_first);
    let mut fresh_second = ShapeOutput::default();
    shape(&second_ast, second, &mut fresh_second);

    let mut reused = ShapeOutput::default();
    shape(&first_ast, first, &mut reused);
    assert_eq!(held(&reused), held(&fresh_first));
    // Without `clear` the second call appends.
    let (clusters, glyphs) = (reused.clusters.len(), reused.glyphs.len());
    shape(&second_ast, second, &mut reused);
    assert_eq!(
        reused.clusters.len(),
        clusters + fresh_second.clusters.len()
    );
    assert_eq!(reused.glyphs.len(), glyphs + fresh_second.glyphs.len());
    // With it, the buffer starts again, glyph ranges included.
    reused.clear();
    assert!(reused.clusters.is_empty() && reused.glyphs.is_empty());
    shape(&second_ast, second, &mut reused);
    assert_eq!(held(&reused), held(&fresh_second));
}

/// A line backend of one 6-wide glyph per character in face 0, whose hyphen
/// has `hyphen_advance`; metrics from the test shaper.
struct HyphenNumbers {
    hyphen_advance: f32,
}

impl LineShaper for HyphenNumbers {
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
                x_advance: if c == '-' { self.hyphen_advance } else { 6.0 },
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

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        SHAPER.space_advance(instance, orientation)
    }
}

#[test]
fn clu6_shp18_a_non_finite_hyphen_advance_is_forwarded_and_rejected_by_the_gate() {
    let text = "abcdef";
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let shaper = LineOnlyShaper::new(HyphenNumbers {
            hyphen_advance: bad,
        });
        let style = style(10.0);
        let ast = compile_paragraph(&input(text, &style), &FrontendOptions::default(), &shaper)
            .expect("the frontend accepts honest numbers");
        let mut out = ShapeOutput::default();
        shaper.reshape_window(
            &ReshapeRequest {
                text,
                run: &ast.runs[0],
                atoms: ast.run_atoms(0),
                window: TextRange::new(0, 3),
                leading_edit: &LeadingEdit::None,
                trailing_edit: &TrailingEdit::AppendHyphen { hyphen: '-' },
                context: TextRange::new(0, text.len()),
            },
            &mut out,
        );
        // Not zeroed or clamped on the way: the number is still there.
        let hyphen = out
            .glyphs
            .iter()
            .find(|g| g.is_synthesized)
            .expect("the hyphen glyph");
        assert!(
            !hyphen.x_advance.is_finite(),
            "{bad}: repaired to {}",
            hyphen.x_advance
        );
        // A paragraph carrying the glyph is stopped by the input gate.
        let mut bad_ast = ast.clone();
        bad_ast.glyphs[0] = *hyphen;
        let checked =
            check_layout_inputs(&bad_ast, text, &column(200.0), &LayoutOptions::default());
        assert!(
            matches!(checked, Err(LayoutInputError::InvalidTextWidth { .. })),
            "{bad}: {checked:?}"
        );
    }
}
