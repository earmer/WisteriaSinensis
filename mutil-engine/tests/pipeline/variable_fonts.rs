//! The HarfRust backend over a multi-axis variable font,
//! `WisteriaVarTest.ttf` (`tests/assets/README.md`): line metrics with
//! `MVAR` deltas and `avar` (SHP-23, FNT-4), advances, a feature-variation
//! glyph swap, a variable `GDEF` ligature caret (SHP-7, INT-5, CRT-1), and
//! the whole pipeline at several axis locations (FNT-2, AST-6, MET-1, ME-2).

use crate::common::{column, input};
use mutil_engine::*;
use std::sync::{Arc, LazyLock};

static VAR_BYTES: &[u8] = include_bytes!("../assets/WisteriaVarTest.ttf");

const VAR_FAMILY: &str = "Wisteria Var Test";

const WGHT: [u8; 4] = *b"wght";
const WDTH: [u8; 4] = *b"wdth";
const SERF: [u8; 4] = *b"SERF";

/// Glyph ids of the README.
const SPACE: u32 = 1;
const H: u32 = 2;
const A: u32 = 3;
const A_SERIF: u32 = 4;
const F: u32 = 5;
const I: u32 = 6;
const F_I: u32 = 7;

/// Axis settings, in the order they are pushed.
type Settings = &'static [([u8; 4], f32)];

/// Glyphs as (id, advance in font units at 1000 per em).
type Glyphs<const N: usize> = [(u32, f32); N];

/// A shaper over a registry holding only the variable font, as face 0.
static VAR_SHAPER: LazyLock<HarfRustShaper> = LazyLock::new(|| {
    let mut registry = FontRegistry::new();
    let id = registry
        .add_face(
            VAR_FAMILY.into(),
            FontStyle::default(),
            FontFace::new(FontBytes::from_static(VAR_BYTES), 0),
        )
        .expect("the variable test font parses");
    assert_eq!(id, FontId(0));
    HarfRustShaper::new(Arc::new(registry))
});

fn var(tag: [u8; 4], value: f32) -> FontVariation {
    FontVariation::try_new(tag, value).expect("a finite coordinate")
}

/// The canonical variation list of `settings`, pushed in the order given.
fn features(settings: &[([u8; 4], f32)]) -> FontFeatures {
    let mut features = FontFeatures::default();
    for &(tag, value) in settings {
        features.push_variation(var(tag, value));
    }
    features
}

fn instance(font_size: f32, variations: &[FontVariation]) -> FontInstanceRef<'_> {
    FontInstanceRef {
        font: FontRef::unscaled(FontId(0)),
        font_size,
        variations,
    }
}

fn metrics_at(font_size: f32, settings: &[([u8; 4], f32)]) -> FontMetrics {
    let features = features(settings);
    VAR_SHAPER.font_metrics(instance(font_size, features.variations()))
}

/// Asserts `actual` is within `tolerance` of `expected`.
#[track_caller]
fn assert_near(actual: f32, expected: f32, tolerance: f32, what: &str) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{what}: {actual} != {expected} (±{tolerance})"
    );
}

/// The `MVAR`-varied metrics of the README, in font units.
#[derive(Clone, Copy, Debug)]
struct LineMetrics {
    ascender: f32,
    descender: f32,
    line_gap: f32,
    underline_position: f32,
    underline_thickness: f32,
    x_height: f32,
}

impl LineMetrics {
    /// The default location.
    const DEFAULT: Self = Self {
        ascender: 800.0,
        descender: 200.0,
        line_gap: 0.0,
        underline_position: -100.0,
        underline_thickness: 50.0,
        x_height: 500.0,
    };

    /// The default plus the README's deltas (the descender delta is given
    /// with its table sign, −40 deepening it by 40).
    const fn plus(asc: f32, desc: f32, gap: f32, upos: f32, uthick: f32, xh: f32) -> Self {
        Self {
            ascender: 800.0 + asc,
            descender: 200.0 - desc,
            line_gap: gap,
            underline_position: -100.0 + upos,
            underline_thickness: 50.0 + uthick,
            x_height: 500.0 + xh,
        }
    }
}

const WGHT_900: LineMetrics = LineMetrics::plus(80.0, -40.0, 20.0, 0.0, 40.0, 20.0);
const WGHT_100: LineMetrics = LineMetrics::plus(-20.0, 0.0, 0.0, 0.0, -20.0, 0.0);
const SERF_100: LineMetrics = LineMetrics::plus(30.0, -30.0, 0.0, -30.0, 0.0, 0.0);
const CORNER: LineMetrics = LineMetrics::plus(150.0, -70.0, 20.0, -30.0, 40.0, 20.0);
/// `wght` 650 is normalised to 0.25 by `avar`: a quarter of the 900 deltas.
const WGHT_650: LineMetrics = LineMetrics::plus(20.0, -10.0, 5.0, 0.0, 10.0, 5.0);
/// `wght` 650 + `SERF` 100, normalised (0.25, 1): the `wght` region at
/// 0.25, the `SERF` region at 1 and the corner region (its own delta the
/// corner less both axes) at 0.25.
const WGHT_650_SERF_100: LineMetrics = LineMetrics::plus(
    20.0 + 30.0 + 0.25 * 40.0,
    -10.0 - 30.0,
    5.0,
    -30.0,
    10.0,
    5.0,
);

/// Asserts `m` is `expected` in font units scaled to `font_size` (1000 per
/// em), and that the unvaried fields keep their defaults.
#[track_caller]
fn assert_metrics(m: &FontMetrics, expected: LineMetrics, font_size: f32, at: &str) {
    let scale = font_size / 1000.0;
    let tolerance = 0.01 * scale;
    let check = |actual: f32, units: f32, field: &str| {
        assert_near(actual, units * scale, tolerance, &format!("{at}: {field}"));
    };
    check(m.ascent, expected.ascender, "ascent");
    check(m.descent, expected.descender, "descent");
    check(m.line_gap, expected.line_gap, "line_gap");
    check(
        m.underline_position,
        expected.underline_position,
        "underline_position",
    );
    check(
        m.underline_thickness,
        expected.underline_thickness,
        "underline_thickness",
    );
    check(m.x_height, expected.x_height, "x_height");
    check(m.strikeout_position, 300.0, "strikeout_position");
    check(m.cap_height, 700.0, "cap_height");
    assert!(m.use_typo_metrics, "{at}: USE_TYPO_METRICS is set");
    assert!(m.is_valid(), "{at}: {m:?}");
    assert_near(
        m.natural_line_height(),
        (expected.ascender + expected.descender + expected.line_gap) * scale,
        tolerance * 3.0,
        &format!("{at}: natural_line_height"),
    );
}

/// Every location the metrics tests visit, with its expected metrics.
fn locations() -> Vec<(&'static str, Settings, LineMetrics)> {
    vec![
        ("default", &[], LineMetrics::DEFAULT),
        ("wght 900", &[(WGHT, 900.0)], WGHT_900),
        ("wght 100", &[(WGHT, 100.0)], WGHT_100),
        ("wght 650", &[(WGHT, 650.0)], WGHT_650),
        ("SERF 100", &[(SERF, 100.0)], SERF_100),
        ("wght 900 SERF 100", &[(WGHT, 900.0), (SERF, 100.0)], CORNER),
        (
            "wght 650 SERF 100",
            &[(WGHT, 650.0), (SERF, 100.0)],
            WGHT_650_SERF_100,
        ),
        ("wdth 125", &[(WDTH, 125.0)], LineMetrics::DEFAULT),
        (
            "wght 900 wdth 125 SERF 100",
            &[(WGHT, 900.0), (WDTH, 125.0), (SERF, 100.0)],
            CORNER,
        ),
    ]
}

// --- Line metrics (SHP-23, FNT-4) ---

#[test]
fn shp23_default_location_metrics_scale_with_the_size() {
    for font_size in [1000.0, 10.0, 37.5] {
        let m = metrics_at(font_size, &[]);
        assert_metrics(&m, LineMetrics::DEFAULT, font_size, "default");
    }
    // No variation and an explicit default coordinate on every axis agree.
    assert_eq!(
        metrics_at(10.0, &[]),
        metrics_at(10.0, &[(WGHT, 400.0), (WDTH, 100.0), (SERF, 0.0)])
    );
}

#[test]
fn shp23_single_axis_mvar_deltas_apply() {
    for (at, settings, expected) in [
        ("wght 900", (WGHT, 900.0), WGHT_900),
        ("wght 100", (WGHT, 100.0), WGHT_100),
        ("SERF 100", (SERF, 100.0), SERF_100),
    ] {
        for font_size in [1000.0, 10.0] {
            let m = metrics_at(font_size, &[settings]);
            assert_metrics(&m, expected, font_size, at);
        }
    }
}

#[test]
fn shp23_mvar_deltas_interpolate_unrounded() {
    // SERF 49 is normalised 0.49: ascender 800 + 0.49 · 30.
    let m = metrics_at(1000.0, &[(SERF, 49.0)]);
    assert_near(m.ascent, 814.7, 0.01, "SERF 49 ascent");
    assert_near(m.descent, 214.7, 0.01, "SERF 49 descent");
    assert_near(
        m.underline_position,
        -114.7,
        0.01,
        "SERF 49 underline_position",
    );
}

#[test]
fn shp23_avar_maps_wght_650_to_a_quarter_of_the_900_delta() {
    let m = metrics_at(1000.0, &[(WGHT, 650.0)]);
    assert_metrics(&m, WGHT_650, 1000.0, "wght 650");
    // Not the linear half way (ascender 840).
    assert!((m.ascent - 840.0).abs() > 1.0, "{m:?}");
}

#[test]
fn shp23_two_axes_reach_the_corner_not_the_sum() {
    let m = metrics_at(1000.0, &[(WGHT, 900.0), (SERF, 100.0)]);
    assert_metrics(&m, CORNER, 1000.0, "wght 900 SERF 100");
    assert_near(m.ascent, 950.0, 0.01, "corner ascent");
    // The sum of the single-axis deltas would give 910.
    assert!((m.ascent - 910.0).abs() > 1.0, "{m:?}");

    let mid = metrics_at(1000.0, &[(WGHT, 650.0), (SERF, 100.0)]);
    assert_metrics(&mid, WGHT_650_SERF_100, 1000.0, "wght 650 SERF 100");
    assert_near(mid.ascent, 860.0, 0.01, "mid ascent");
}

#[test]
fn shp23_wdth_moves_no_line_metric() {
    for wdth in [75.0, 125.0, 110.0] {
        assert_eq!(
            metrics_at(1000.0, &[(WDTH, wdth)]),
            metrics_at(1000.0, &[]),
            "wdth {wdth}"
        );
    }
    let m = metrics_at(1000.0, &[(WGHT, 900.0), (WDTH, 125.0), (SERF, 100.0)]);
    assert_metrics(&m, CORNER, 1000.0, "three axes");
}

#[test]
fn shp23_coordinates_outside_an_axis_clamp_to_its_end() {
    for (outside, end, expected) in [
        ((WGHT, 1000.0), (WGHT, 900.0), WGHT_900),
        ((WGHT, 1.0), (WGHT, 100.0), WGHT_100),
        ((SERF, 250.0), (SERF, 100.0), SERF_100),
        ((SERF, -40.0), (SERF, 0.0), LineMetrics::DEFAULT),
    ] {
        let m = metrics_at(1000.0, &[outside]);
        assert_metrics(&m, expected, 1000.0, &format!("{outside:?}"));
        assert_eq!(m, metrics_at(1000.0, &[end]), "{outside:?}");
    }
}

#[test]
fn shp23_an_unknown_axis_is_ignored() {
    assert_eq!(
        metrics_at(1000.0, &[(*b"ZZZZ", 500.0)]),
        metrics_at(1000.0, &[])
    );
    let m = metrics_at(1000.0, &[(*b"ZZZZ", 500.0), (WGHT, 900.0)]);
    assert_metrics(&m, WGHT_900, 1000.0, "ZZZZ + wght 900");
}

#[test]
fn fnt2_push_order_does_not_change_the_instance() {
    let forward = features(&[(WGHT, 900.0), (WDTH, 125.0), (SERF, 100.0)]);
    let backward = features(&[(SERF, 100.0), (WDTH, 125.0), (WGHT, 900.0)]);
    assert_eq!(forward, backward);
    assert_eq!(forward.variations(), backward.variations());
    // The last setting of a tag wins.
    let overridden = features(&[(WGHT, 100.0), (SERF, 100.0), (WGHT, 900.0), (WDTH, 125.0)]);
    assert_eq!(overridden, forward);
    let tags: Vec<[u8; 4]> = forward.variations().iter().map(|v| v.tag()).collect();
    assert_eq!(tags, [SERF, WDTH, WGHT], "ascending by tag");
    assert_eq!(
        VAR_SHAPER.font_metrics(instance(1000.0, forward.variations())),
        VAR_SHAPER.font_metrics(instance(1000.0, backward.variations()))
    );
    // And the pipeline shapes them alike.
    assert_eq!(
        glyphs(GLYPH_SAMPLE, &forward),
        glyphs(GLYPH_SAMPLE, &backward)
    );
}

// --- Shaping through the pipeline ---

/// The variable font at `font_size` with `features`.
fn var_style(font_size: f32, features: &FontFeatures) -> TextStyle {
    TextStyle {
        families: Arc::from([FontFamily::Named(VAR_FAMILY.into())]),
        font_size,
        style: FontStyle::default(),
        features: features.clone(),
        decoration: TextDecoration::default(),
        baseline_align: BaselineAlignment::Auto,
        letter_spacing: 0.0,
        word_spacing: 0.0,
        script: None,
        locale: None,
    }
}

#[track_caller]
fn compile_var(input: &ParagraphInput<'_>) -> ParagraphAst {
    compile_paragraph(input, &FrontendOptions::default(), &*VAR_SHAPER)
        .unwrap_or_else(|e| panic!("{:?}: {e}", input.text))
}

/// Every glyph of `text` at size 1000 in `features`, as (id, advance), in
/// logical order, spaces left out.
fn glyphs(text: &str, features: &FontFeatures) -> Vec<(u32, f32)> {
    let style = var_style(1000.0, features);
    let ast = compile_var(&input(text, &style));
    for run in &ast.runs {
        assert_eq!(run.features.variations(), features.variations(), "FNT-2");
    }
    for cluster in &ast.clusters {
        assert_eq!(cluster.font.font_id, FontId(0));
    }
    ast.glyphs
        .iter()
        .filter(|g| g.glyph_id != SPACE)
        .map(|g| (g.glyph_id, g.x_advance))
        .collect()
}

/// `H`, `a`, a lone `f`, a lone `i` and the ligature `f_i`.
const GLYPH_SAMPLE: &str = "Ha f i fi";

/// Asserts `found` are the glyphs `expected` (id, advance in font units at
/// 1000 per em), an advance within one unit when it is not whole.
#[track_caller]
fn assert_glyphs(found: &[(u32, f32)], expected: &[(u32, f32)], at: &str) {
    let ids: Vec<u32> = found.iter().map(|g| g.0).collect();
    let expected_ids: Vec<u32> = expected.iter().map(|g| g.0).collect();
    assert_eq!(ids, expected_ids, "{at}: glyph ids");
    for (&(id, advance), &(_, want)) in found.iter().zip(expected) {
        let tolerance = if want.fract() == 0.0 { 1e-3 } else { 1.0 };
        assert_near(advance, want, tolerance, &format!("{at}: glyph {id}"));
    }
}

#[test]
fn shp23_advances_follow_each_axis() {
    // (location, H, a or a.serif, f, i, f_i)
    let cases: [(&str, Settings, Glyphs<5>); 7] = [
        (
            "default",
            &[],
            [(H, 700.0), (A, 500.0), (F, 300.0), (I, 250.0), (F_I, 500.0)],
        ),
        (
            "wght 900",
            &[(WGHT, 900.0)],
            [(H, 800.0), (A, 600.0), (F, 400.0), (I, 350.0), (F_I, 600.0)],
        ),
        (
            "wght 100",
            &[(WGHT, 100.0)],
            [(H, 650.0), (A, 450.0), (F, 250.0), (I, 200.0), (F_I, 450.0)],
        ),
        (
            "wdth 125",
            &[(WDTH, 125.0)],
            [(H, 875.0), (A, 625.0), (F, 375.0), (I, 312.5), (F_I, 625.0)],
        ),
        (
            "wdth 75",
            &[(WDTH, 75.0)],
            [(H, 525.0), (A, 375.0), (F, 225.0), (I, 187.5), (F_I, 375.0)],
        ),
        (
            "SERF 100",
            &[(SERF, 100.0)],
            [
                (H, 740.0),
                (A_SERIF, 600.0),
                (F, 340.0),
                (I, 290.0),
                (F_I, 540.0),
            ],
        ),
        (
            "wght 900 wdth 125 SERF 100",
            &[(WGHT, 900.0), (WDTH, 125.0), (SERF, 100.0)],
            [
                (H, 1015.0),
                (A_SERIF, 840.0),
                (F, 515.0),
                (I, 452.5),
                (F_I, 765.0),
            ],
        ),
    ];
    for (at, settings, expected) in cases {
        assert_glyphs(&glyphs(GLYPH_SAMPLE, &features(settings)), &expected, at);
    }
}

#[test]
fn shp23_the_space_advance_scales_with_wdth_only() {
    for (settings, want) in [
        (&[][..], 250.0_f32),
        (&[(WGHT, 900.0)][..], 250.0),
        (&[(SERF, 100.0)][..], 250.0),
        (&[(WDTH, 125.0)][..], 312.5),
        (&[(WGHT, 900.0), (WDTH, 125.0), (SERF, 100.0)][..], 312.5),
        (&[(WDTH, 75.0)][..], 187.5),
    ] {
        let features = features(settings);
        let advance = VAR_SHAPER.space_advance(
            instance(1000.0, features.variations()),
            GlyphOrientation::Horizontal,
        );
        let tolerance = if want.fract() == 0.0 { 1e-3 } else { 1.0 };
        assert_near(advance, want, tolerance, &format!("{settings:?}"));
    }
}

#[test]
fn shp23_out_of_range_and_unknown_axes_shape_like_their_clamped_instance() {
    for (outside, inside) in [
        (&[(WGHT, 1000.0)][..], &[(WGHT, 900.0)][..]),
        (&[(WGHT, 1.0)][..], &[(WGHT, 100.0)][..]),
        (&[(WDTH, 200.0)][..], &[(WDTH, 125.0)][..]),
        (&[(WDTH, 10.0)][..], &[(WDTH, 75.0)][..]),
        (&[(SERF, 400.0)][..], &[(SERF, 100.0)][..]),
        (&[(*b"ZZZZ", 7.0)][..], &[][..]),
        (&[(*b"ZZZZ", 7.0), (WGHT, 900.0)][..], &[(WGHT, 900.0)][..]),
    ] {
        assert_eq!(
            glyphs(GLYPH_SAMPLE, &features(outside)),
            glyphs(GLYPH_SAMPLE, &features(inside)),
            "{outside:?}"
        );
    }
}

#[test]
fn shp23_fnt2_the_feature_variation_swaps_a_for_a_serif_from_serf_50() {
    for (serf, id, advance) in [
        (0.0, A, 500.0),
        (49.0, A, 519.6),
        (50.0, A_SERIF, 580.0),
        (100.0, A_SERIF, 600.0),
    ] {
        let found = glyphs("a", &features(&[(SERF, serf)]));
        assert_glyphs(&found, &[(id, advance)], &format!("SERF {serf}"));
    }
}

// --- Ligature and caret (SHP-7, INT-5, CRT-1) ---

/// Locations with the expected `f_i` advance and caret, in font units.
const LIGATURE_CASES: [(&str, Settings, f32, f32); 7] = [
    ("default", &[], 500.0, 250.0),
    ("wght 900", &[(WGHT, 900.0)], 600.0, 300.0),
    ("wght 650", &[(WGHT, 650.0)], 525.0, 262.5),
    ("SERF 100", &[(SERF, 100.0)], 540.0, 250.0),
    ("wdth 125", &[(WDTH, 125.0)], 625.0, 250.0),
    (
        "wght 900 SERF 100",
        &[(WGHT, 900.0), (SERF, 100.0)],
        640.0,
        300.0,
    ),
    (
        "wght 900 wdth 125 SERF 100",
        &[(WGHT, 900.0), (WDTH, 125.0), (SERF, 100.0)],
        765.0,
        300.0,
    ),
];

#[test]
fn shp7_ligature_carets_follow_wght() {
    for (at, settings, _, caret) in LIGATURE_CASES {
        let features = features(settings);
        for font_size in [1000.0, 10.0] {
            let carets = VAR_SHAPER.ligature_carets(
                instance(font_size, features.variations()),
                F_I,
                Direction::Ltr,
            );
            assert_eq!(carets.len(), 1, "{at} at {font_size}: {carets:?}");
            assert_near(
                carets[0],
                caret * font_size / 1000.0,
                0.01 * font_size / 1000.0,
                &format!("{at} at {font_size}"),
            );
        }
    }
    // The fallback scale enters the effective size.
    let carets = VAR_SHAPER.ligature_carets(
        FontInstanceRef {
            font: FontRef {
                font_id: FontId(0),
                scale: 2.0,
            },
            font_size: 10.0,
            variations: &[var(WGHT, 900.0)],
        },
        F_I,
        Direction::Ltr,
    );
    assert_eq!(carets.len(), 1, "{carets:?}");
    assert_near(carets[0], 6.0, 1e-3, "scaled caret");
    // A glyph that is no ligature has none.
    let none = VAR_SHAPER.ligature_carets(instance(1000.0, &[]), A, Direction::Ltr);
    assert!(none.is_empty(), "{none:?}");
}

/// Lays out `input` at a wide column and passes it through the output gate.
#[track_caller]
fn lay_out_var(input: &ParagraphInput<'_>) -> (ParagraphAst, PositionedParagraph) {
    let ast = compile_var(input);
    let paragraph = layout_paragraph(
        &ast,
        input.text,
        &*VAR_SHAPER,
        &column(100_000.0),
        &LayoutOptions::default(),
    )
    .unwrap_or_else(|e| panic!("{:?}: {e}", input.text));
    if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&paragraph, input.text) {
        panic!("{:?}: line {line}: {error}\n{paragraph:#?}", input.text);
    }
    (ast, paragraph)
}

#[test]
fn int5_crt1_the_ligature_caret_stop_sits_at_the_gdef_caret() {
    for (at, settings, advance, caret) in LIGATURE_CASES {
        let features = features(settings);
        let style = var_style(1000.0, &features);
        let (ast, paragraph) = lay_out_var(&input("fi", &style));
        // `liga` forms the ligature, one cluster of two atoms.
        assert_eq!(ast.clusters.len(), 1, "{at}");
        assert_eq!(ast.clusters[0].atom_count(), 2, "{at}");
        let ids: Vec<u32> = ast.glyphs.iter().map(|g| g.glyph_id).collect();
        assert_eq!(ids, [F_I], "{at}");
        assert_near(ast.glyphs[0].x_advance, advance, 1e-3, at);

        assert_eq!(paragraph.lines.len(), 1, "{at}");
        let stops: Vec<(usize, f32)> = paragraph.lines[0]
            .caret_stops
            .iter()
            .map(|s| (s.offset, s.x))
            .collect();
        assert!(
            stops.windows(2).all(|w| w[0].1 <= w[1].1),
            "{at}: visual order {stops:?}"
        );
        let x_at = |offset: usize| {
            stops
                .iter()
                .find(|s| s.0 == offset)
                .unwrap_or_else(|| panic!("{at}: no stop at {offset} in {stops:?}"))
                .1
        };
        assert_near(x_at(0), 0.0, 1e-3, &format!("{at}: start"));
        assert_near(x_at(1), caret, 0.01, &format!("{at}: interior stop"));
        assert_near(x_at(2), advance, 1e-3, &format!("{at}: end"));
    }
}

// --- Line boxes (AST-6, MET-1, ME-2, BAS-1) ---

/// A sample with every glyph of the README's metrics tests.
const LINE_SAMPLE: &str = "Hafi xo Ha";

#[test]
fn ast6_met1_line_boxes_follow_each_instance() {
    for (at, settings, expected) in locations() {
        for font_size in [1000.0, 10.0] {
            let features = features(settings);
            let style = var_style(font_size, &features);
            let (ast, paragraph) = lay_out_var(&input(LINE_SAMPLE, &style));
            let at = format!("{at} at {font_size}");
            // AST-6: the strut is the raw metrics of that instance.
            assert_eq!(
                ast.strut_metrics,
                VAR_SHAPER.font_metrics(instance(font_size, features.variations())),
                "{at}"
            );
            assert_metrics(&ast.strut_metrics, expected, font_size, &at);
            assert_eq!(ast.strut_font_size, font_size, "{at}");

            // ME-2 under `LineHeight::Normal`: ascent and descent each take
            // half the line gap.
            let scale = font_size / 1000.0;
            let half_gap = expected.line_gap / 2.0;
            let line = &paragraph.lines[0];
            let tolerance = 0.01 * scale;
            assert_near(
                line.ascent,
                (expected.ascender + half_gap) * scale,
                tolerance,
                &format!("{at}: ascent"),
            );
            assert_near(
                line.descent,
                (expected.descender + half_gap) * scale,
                tolerance,
                &format!("{at}: descent"),
            );
            assert_near(
                line.line_height,
                (expected.ascender + expected.descender + expected.line_gap) * scale,
                tolerance * 2.0,
                &format!("{at}: line_height"),
            );
            for run in &line.visual_runs {
                assert_eq!(&run.variations[..], features.variations(), "{at}");
                assert_eq!(run.font.font_id, FontId(0), "{at}");
            }
        }
    }
}

/// `LINE_SAMPLE` in `base`, with a span in `span` over its last word.
fn two_instances(
    base: &[([u8; 4], f32)],
    span: &[([u8; 4], f32)],
) -> (
    FontFeatures,
    FontFeatures,
    ParagraphAst,
    PositionedParagraph,
) {
    let base = features(base);
    let span = features(span);
    let base_style = var_style(1000.0, &base);
    let spans = [StyleSpan {
        byte_range: TextRange::new(8, LINE_SAMPLE.len()),
        style: var_style(1000.0, &span),
    }];
    let input = ParagraphInput {
        style_spans: &spans,
        ..input(LINE_SAMPLE, &base_style)
    };
    let (ast, paragraph) = lay_out_var(&input);
    (base, span, ast, paragraph)
}

#[test]
fn fnt2_me2_two_instances_shape_apart_and_the_taller_sets_the_line() {
    let (base, span, ast, paragraph) = two_instances(&[], &[(WGHT, 900.0)]);
    // Each run carries its own instance.
    for run in &ast.runs {
        let in_span = run.byte_range.start() >= 8;
        let want = if in_span { &span } else { &base };
        assert_eq!(run.features.variations(), want.variations(), "{run:?}");
    }
    assert!(ast.runs.iter().any(|r| r.byte_range.start() >= 8));
    // The `H` before the span is 700 wide, the one inside it 800.
    let h_advances: Vec<f32> = ast
        .glyphs
        .iter()
        .filter(|g| g.glyph_id == H)
        .map(|g| g.x_advance)
        .collect();
    assert_eq!(h_advances.len(), 2, "{h_advances:?}");
    assert_near(h_advances[0], 700.0, 1e-3, "default H");
    assert_near(h_advances[1], 800.0, 1e-3, "wght 900 H");
    // The strut is the default instance's.
    assert_metrics(&ast.strut_metrics, LineMetrics::DEFAULT, 1000.0, "strut");

    let line = &paragraph.lines[0];
    let variations: Vec<&[FontVariation]> =
        line.visual_runs.iter().map(|r| &r.variations[..]).collect();
    assert!(variations.contains(&base.variations()), "{variations:?}");
    assert!(variations.contains(&span.variations()), "{variations:?}");
    // The wght 900 run is taller: 880 + 10 above, 240 + 10 below.
    assert_near(line.ascent, 890.0, 0.01, "ascent");
    assert_near(line.descent, 250.0, 0.01, "descent");
    assert_near(line.line_height, 1140.0, 0.02, "line_height");
}

#[test]
fn me2_a_shorter_span_leaves_the_strut_box() {
    // Base wght 900 + SERF 100 (950 / 270, gap 20), span wght 100 (780 /
    // 200, no gap): the base sets the line.
    let (_, _, ast, paragraph) = two_instances(&[(WGHT, 900.0), (SERF, 100.0)], &[(WGHT, 100.0)]);
    assert_metrics(&ast.strut_metrics, CORNER, 1000.0, "strut");
    let line = &paragraph.lines[0];
    assert_near(line.ascent, 960.0, 0.01, "ascent");
    assert_near(line.descent, 280.0, 0.01, "descent");
    // The span's `a` is the sans form: SERF is not inherited by the span's
    // own style.
    let ids: Vec<u32> = ast.glyphs.iter().map(|g| g.glyph_id).collect();
    assert!(ids.contains(&A_SERIF) && ids.contains(&A), "{ids:?}");
}
