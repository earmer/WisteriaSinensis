//! A backend never repairs the numbers it is given or returns (SHP-18, AX-3):
//! the frontend rejects an unusable size before the shaper's first call, the
//! input gate (G1) judges the numbers a shaper answered, and the `LineOnly`
//! adapter forwards a line backend's numbers unchanged (SHP-24).

use crate::common::*;
use mutil_engine::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A line backend that answers one glyph per character, face 0, 6 units
/// wide, with one number of the second character replaced, and metrics from
/// the test shaper. `space` replaces the answer of `space_advance` when given.
struct Numbers {
    corrupt: Corruption,
    space: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
enum Corruption {
    None,
    XAdvance(f32),
    XOffset(f32),
    YOffset(f32),
}

impl LineShaper for Numbers {
    fn shape_line(
        &self,
        text: &str,
        _families: &[FontFamily],
        _style: FontStyle,
        _font_size: f32,
        _features: &FontFeatures,
    ) -> Vec<LineGlyph> {
        text.char_indices()
            .map(|(byte_index, c)| {
                let mut glyph = LineGlyph {
                    font: FontId(0),
                    glyph_id: glyph_for(FONT_BYTES, c).unwrap_or(0),
                    byte_index,
                    x_advance: 6.0,
                    x_offset: 0.0,
                    y_offset: 0.0,
                };
                if byte_index == 1 {
                    match self.corrupt {
                        Corruption::None => {}
                        Corruption::XAdvance(v) => glyph.x_advance = v,
                        Corruption::XOffset(v) => glyph.x_offset = v,
                        Corruption::YOffset(v) => glyph.y_offset = v,
                    }
                }
                glyph
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
        self.space
            .unwrap_or_else(|| SHAPER.space_advance(instance, orientation))
    }
}

fn numbers(corrupt: Corruption, space: Option<f32>) -> LineOnlyShaper<Numbers> {
    LineOnlyShaper::new(Numbers { corrupt, space })
}

fn compile_with(shaper: &dyn TextShaper, text: &str) -> ParagraphAst {
    let style = style(10.0);
    compile_paragraph(&input(text, &style), &FrontendOptions::default(), shaper)
        .unwrap_or_else(|e| panic!("{text:?}: the frontend accepts the shaper's answers: {e}"))
}

fn gate(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    check_layout_inputs(ast, text, &column(200.0), &LayoutOptions::default())
}

const NON_FINITE: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

#[test]
fn shp18_shp24_a_non_finite_glyph_number_reaches_the_input_gate_unchanged() {
    let text = "abc";
    // Control: the same backend with honest numbers passes.
    let ok = compile_with(&numbers(Corruption::None, None), text);
    assert_eq!(gate(&ok, text), Ok(()));

    for bad in NON_FINITE {
        for corrupt in [
            Corruption::XAdvance(bad),
            Corruption::XOffset(bad),
            Corruption::YOffset(bad),
        ] {
            let shaper = numbers(corrupt, None);
            let ast = compile_with(&shaper, text);
            // Not replaced by zero (or anything finite) on the way.
            assert!(
                ast.glyphs.iter().any(|g| !(g.x_advance.is_finite()
                    && g.x_offset.is_finite()
                    && g.y_offset.is_finite())),
                "{corrupt:?}: the number was repaired: {:?}",
                ast.glyphs
            );
            let checked = gate(&ast, text);
            assert!(
                matches!(checked, Err(LayoutInputError::InvalidTextWidth { .. })),
                "{corrupt:?}: {checked:?}"
            );
            // layout_paragraph runs the same gate first and propagates it.
            let laid_out = layout_paragraph(
                &ast,
                text,
                &shaper,
                &column(200.0),
                &LayoutOptions::default(),
            );
            assert!(
                matches!(laid_out, Err(LayoutInputError::InvalidTextWidth { .. })),
                "{corrupt:?}: {:?}",
                laid_out.map(drop)
            );
        }
    }
}

#[test]
fn shp20_wn10_shp24_a_bad_space_advance_is_an_invalid_glue_at_the_input_gate() {
    let text = "a b";
    let ok = compile_with(&numbers(Corruption::None, None), text);
    assert_eq!(gate(&ok, text), Ok(()));
    // A zero advance is finite and non-negative: valid.
    let zero = compile_with(&numbers(Corruption::None, Some(0.0)), text);
    assert_eq!(gate(&zero, text), Ok(()));

    for bad in NON_FINITE {
        let shaper = numbers(Corruption::None, Some(bad));
        let ast = compile_with(&shaper, text);
        let checked = gate(&ast, text);
        assert!(
            matches!(checked, Err(LayoutInputError::InvalidGlue { .. })),
            "space_advance {bad}: {checked:?}"
        );
    }
}

/// SHP-18 names a negative `space_advance` as an `InvalidGlue` at the gate,
/// but FE-27 sizes the glue as `max(0, a + word_spacing)`, which would turn a
/// negative `a` into a valid zero-width glue. The spec is silent on which
/// wins, so this stays ignored until it says.
#[test]
#[ignore = "SHP-18 (negative space_advance fails as InvalidGlue) conflicts with FE-27's max(0, ..)"]
fn shp18_a_negative_space_advance_is_an_invalid_glue_at_the_input_gate() {
    let text = "a b";
    for bad in [-3.0, -0.5] {
        let shaper = numbers(Corruption::None, Some(bad));
        let ast = compile_with(&shaper, text);
        let checked = gate(&ast, text);
        assert!(
            matches!(checked, Err(LayoutInputError::InvalidGlue { .. })),
            "space_advance {bad}: {checked:?}"
        );
    }
}

/// Forwards to the test shaper and counts every call.
struct Counting {
    calls: AtomicUsize,
}

impl Counting {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn hit(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
    }
}

impl TextShaper for Counting {
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics {
        self.hit();
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
        self.hit();
        SHAPER.resolve_primary_font_metrics(families, style, script, locale, font_size, variations)
    }

    fn shape_run(
        &self,
        text: &str,
        run: &ShapingRun,
        atoms: &[GraphemeAtom],
        output: &mut ShapeOutput,
    ) {
        self.hit();
        SHAPER.shape_run(text, run, atoms, output);
    }

    fn reshape_window(&self, request: &ReshapeRequest<'_>, output: &mut ShapeOutput) {
        self.hit();
        SHAPER.reshape_window(request, output);
    }

    fn ligature_carets(
        &self,
        instance: FontInstanceRef<'_>,
        glyph_id: u32,
        direction: Direction,
    ) -> smallvec::SmallVec<[f32; 4]> {
        self.hit();
        SHAPER.ligature_carets(instance, glyph_id, direction)
    }

    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32 {
        self.hit();
        SHAPER.space_advance(instance, orientation)
    }
}

#[test]
fn shp18_a_bad_style_font_size_is_rejected_before_the_first_shaper_call() {
    let text = "a b";
    // Control: a good size reaches the shaper.
    let counting = Counting::new();
    let good = style(10.0);
    compile_paragraph(&input(text, &good), &FrontendOptions::default(), &counting)
        .expect("a valid style compiles");
    assert!(counting.count() > 0);

    for bad in [0.0, -0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let counting = Counting::new();
        let bad_style = style(bad);
        let result = compile_paragraph(
            &input(text, &bad_style),
            &FrontendOptions::default(),
            &counting,
        );
        assert!(
            matches!(result, Err(FrontendError::InvalidStyle { span: None })),
            "font_size {bad}: {:?}",
            result.map(drop)
        );
        assert_eq!(
            counting.count(),
            0,
            "font_size {bad}: the shaper was called"
        );
    }
}

#[test]
fn shp18_a_bad_shaping_run_font_size_is_rejected_by_the_gate_before_any_shaper_call() {
    let text = "abc";
    let compiled = compile(text);
    assert_eq!(gate(&compiled, text), Ok(()));

    for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        let mut ast = compiled.clone();
        ast.runs[0].font_size = bad;
        assert_eq!(
            gate(&ast, text),
            Err(LayoutInputError::InvalidShapingRun { run: 0 }),
            "font_size {bad}"
        );
        let counting = Counting::new();
        let laid_out = layout_paragraph(
            &ast,
            text,
            &counting,
            &column(200.0),
            &LayoutOptions::default(),
        );
        assert!(
            matches!(
                laid_out,
                Err(LayoutInputError::InvalidShapingRun { run: 0 })
            ),
            "font_size {bad}: {:?}",
            laid_out.map(drop)
        );
        assert_eq!(
            counting.count(),
            0,
            "font_size {bad}: the shaper was called"
        );
    }
}

/// A line backend returning fixed glyphs, `(byte, face, glyph id)`, each 5
/// units wide, whatever string it is given.
struct Fixed(Vec<(usize, u32, u32)>);

impl LineShaper for Fixed {
    fn shape_line(
        &self,
        _text: &str,
        _families: &[FontFamily],
        _style: FontStyle,
        _font_size: f32,
        _features: &FontFeatures,
    ) -> Vec<LineGlyph> {
        self.0
            .iter()
            .map(|&(byte_index, font, glyph_id)| LineGlyph {
                font: FontId(font),
                glyph_id,
                byte_index,
                x_advance: 5.0,
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

/// Reshapes the window `1..3` ("12") of the tate-chu-yoko run "a12b" with
/// `trailing_edit`, the backend answering `glyphs`.
fn reshape_tate_chu_yoko(
    glyphs: Vec<(usize, u32, u32)>,
    trailing_edit: &TrailingEdit,
) -> (Vec<ShapedCluster>, Vec<RawShapedGlyph>) {
    let text = "a12b";
    let (atoms, run) = crate::shaper::tate_chu_yoko(text, &[FAMILY]);
    let shaper = LineOnlyShaper::new(Fixed(glyphs));
    let mut out = ShapeOutput::default();
    shaper.reshape_window(
        &ReshapeRequest {
            trailing_edit,
            ..ReshapeRequest::plain(text, &run, &atoms, TextRange::new(1, 3))
        },
        &mut out,
    );
    (out.clusters, out.glyphs)
}

#[test]
fn shp6_plain_tate_chu_yoko_window_is_one_cluster_in_the_first_glyphs_face() {
    // "1" in face 1, "2" in face 2.
    let (clusters, glyphs) =
        reshape_tate_chu_yoko(vec![(0, 1, 10), (1, 2, 20)], &TrailingEdit::None);
    assert_eq!(clusters.len(), 1, "{clusters:?}");
    assert_eq!(clusters[0].atom_range, 1..3, "{clusters:?}");
    assert_eq!(clusters[0].font.font_id, FontId(1), "{clusters:?}");
    assert_eq!(clusters[0].glyph_range, 0..2, "{clusters:?}");
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [10, 0],
        "{glyphs:?}"
    );
    assert_eq!(glyphs[1].x_advance, 5.0, "{glyphs:?}");
    assert!(glyphs.iter().all(|g| !g.is_synthesized), "{glyphs:?}");
}

#[test]
fn shp6_trailing_hyphen_on_a_tate_chu_yoko_window_lands_on_its_one_cluster() {
    // "1" in face 1, "2" in face 2, then the hyphen "-" in face 1.
    let (clusters, glyphs) = reshape_tate_chu_yoko(
        vec![(0, 1, 10), (1, 2, 20), (2, 1, 40)],
        &TrailingEdit::AppendHyphen { hyphen: '-' },
    );
    // The kept text is still one cluster; the hyphen's glyphs are appended
    // to the window's last cluster, which is that one.
    assert_eq!(clusters.len(), 1, "{clusters:?}");
    assert_eq!(clusters[0].atom_range, 1..3, "{clusters:?}");
    assert_eq!(clusters[0].font.font_id, FontId(1), "{clusters:?}");
    assert_eq!(clusters[0].glyph_range, 0..3, "{clusters:?}");
    assert_eq!(
        glyphs.iter().map(|g| g.glyph_id).collect::<Vec<_>>(),
        [10, 0, 40],
        "{glyphs:?}"
    );
    assert_eq!(glyphs[1].x_advance, 5.0, "{glyphs:?}");
    assert_eq!(
        glyphs.iter().map(|g| g.is_synthesized).collect::<Vec<_>>(),
        [false, false, true],
        "{glyphs:?}"
    );
}

#[test]
fn shp4_reshape_window_appends_after_what_the_buffer_already_holds() {
    let text = "abcd";
    let ast = compile(text);
    let (atoms, run) = (&ast.atoms, &ast.runs[0]);

    // Seed the buffer with the whole run shaped, then reshape "bc" into it.
    let mut out = ShapeOutput::default();
    SHAPER.shape_run(text, run, atoms, &mut out);
    let seeded = out.clone();
    assert!(!seeded.clusters.is_empty() && !seeded.glyphs.is_empty());

    SHAPER.reshape_window(
        &ReshapeRequest::plain(text, run, atoms, TextRange::new(1, 3)),
        &mut out,
    );

    // Nothing was cleared or rewritten.
    assert!(out.clusters.len() > seeded.clusters.len(), "{out:?}");
    assert!(out.glyphs.len() > seeded.glyphs.len(), "{out:?}");
    assert_eq!(out.clusters[..seeded.clusters.len()], seeded.clusters[..]);
    assert_eq!(out.glyphs[..seeded.glyphs.len()], seeded.glyphs[..]);

    // The new clusters partition exactly the window's atoms, and their glyph
    // ranges are absolute: they tile what the call appended.
    let new = &out.clusters[seeded.clusters.len()..];
    let mut next_atom = 1;
    let mut next_glyph = seeded.glyphs.len();
    for cluster in new {
        assert_eq!(cluster.atom_range.start, next_atom, "{new:?}");
        assert_eq!(cluster.glyph_range.start, next_glyph, "{new:?}");
        next_atom = cluster.atom_range.end;
        next_glyph = cluster.glyph_range.end;
    }
    assert_eq!(next_atom, 3, "{new:?}");
    assert_eq!(next_glyph, out.glyphs.len(), "{new:?}");
}
