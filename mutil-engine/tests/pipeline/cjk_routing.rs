//! Font choice by script and locale (spec SHP-12, SHP-16, SHP-25): CJK runs
//! skip Latin faces that only share punctuation, and a fixed stack of
//! regional CJK faces serves each run from the face of its locale.

use crate::common::*;
use mutil_engine::*;
use std::sync::{Arc, LazyLock};

/// The faces of these tests, in registration order: face `i` is
/// `CJK_FONTS[i]` (`tests/assets/README.md`).
static CJK_FONTS: &[(&str, &[u8])] = &[
    (
        "Noto Serif",
        include_bytes!("../assets/NotoSerif-Subset.ttf"),
    ),
    ("Noto Sans", include_bytes!("../assets/NotoSans-Subset.ttf")),
    (
        "Noto Sans SC",
        include_bytes!("../assets/NotoSansSC-Subset.ttf"),
    ),
    (
        "Noto Sans TC",
        include_bytes!("../assets/NotoSansTC-Subset.ttf"),
    ),
    (
        "Noto Sans JP",
        include_bytes!("../assets/NotoSansJP-Subset.ttf"),
    ),
    (
        "Noto Sans CJK SC",
        include_bytes!("../assets/NotoSansCJKsc-Subset.otf"),
    ),
];

const SERIF: FontId = FontId(0);
const SANS: FontId = FontId(1);
const SC: FontId = FontId(2);
const TC: FontId = FontId(3);
const JP: FontId = FontId(4);
const PAN: FontId = FontId(5);

static CJK_SHAPER: LazyLock<HarfRustShaper> = LazyLock::new(|| {
    let mut registry = FontRegistry::new();
    for (i, (family, bytes)) in CJK_FONTS.iter().enumerate() {
        let id = registry
            .add_face(
                (*family).into(),
                FontStyle::default(),
                FontFace::new(FontBytes::from_static(bytes), 0),
            )
            .unwrap_or_else(|| panic!("{family} parses"));
        assert_eq!(id, FontId(i as u32));
    }
    HarfRustShaper::new(Arc::new(registry))
});

fn cjk_style(families: &[&str], locale: Option<&str>) -> TextStyle {
    TextStyle {
        families: families
            .iter()
            .map(|f| FontFamily::Named((*f).into()))
            .collect(),
        locale: locale.map(|l| LocaleTag::try_from_str_lenient(l).expect("a locale")),
        ..style(18.0)
    }
}

fn compile_with(
    text: &str,
    default_style: &TextStyle,
    spans: &[StyleSpan],
    options: &FrontendOptions,
) -> ParagraphAst {
    let input = ParagraphInput {
        style_spans: spans,
        ..input(text, default_style)
    };
    compile_paragraph(&input, options, &*CJK_SHAPER).expect("the frontend accepts the text")
}

fn cluster_text<'t>(text: &'t str, cluster: &ShapedCluster) -> &'t str {
    &text[cluster.byte_range.as_range()]
}

#[test]
fn faces_are_profiled_from_their_tables() {
    let registry = CJK_SHAPER.registry();
    let profile = |id| registry.profile(id).expect("registered");
    assert_eq!(profile(SERIF), FaceProfile::default());
    assert_eq!(profile(SANS), FaceProfile::default());
    for (id, region) in [
        (SC, CjkRegion::Simplified),
        (TC, CjkRegion::Traditional),
        (JP, CjkRegion::Japanese),
        // Noto Sans CJK SC lists JAN, KOR, ZHH and ZHT under `locl`.
        (PAN, CjkRegion::Pan),
    ] {
        assert_eq!(
            profile(id),
            FaceProfile {
                cjk: true,
                region: Some(region)
            },
            "{id:?}"
        );
    }
    assert_eq!(registry.profile(FontId(99)), None);
}

#[test]
fn a_host_can_state_a_faces_locale() {
    let mut registry = FontRegistry::new();
    let face = |bytes: &'static [u8]| FontFace::new(FontBytes::from_static(bytes), 0);
    let tag = |t: &str| LocaleTag::try_from_str_lenient(t).expect("a locale");
    let sans = CJK_FONTS[1].1;
    let jp = CJK_FONTS[4].1;
    let as_hk = registry
        .add_face_with_locale("A".into(), FontStyle::default(), face(jp), &tag("zh-HK"))
        .expect("parses");
    let as_latin = registry
        .add_face_with_locale("B".into(), FontStyle::default(), face(jp), &tag("en"))
        .expect("parses");
    let as_pan = registry
        .add_face_with_locale(
            "C".into(),
            FontStyle::default(),
            face(sans),
            &tag("und-Hani"),
        )
        .expect("parses");
    let detected = registry
        .add_face_with_locale("D".into(), FontStyle::default(), face(jp), &tag("und"))
        .expect("parses");
    let region = |id| registry.profile(id).expect("registered");
    assert_eq!(
        region(as_hk),
        FaceProfile {
            cjk: true,
            region: Some(CjkRegion::HongKong)
        }
    );
    assert_eq!(region(as_latin), FaceProfile::default());
    assert_eq!(region(as_pan).region, Some(CjkRegion::Pan));
    assert_eq!(region(detected).region, Some(CjkRegion::Japanese));
}

/// SHP-12: `Hani` skips Latin-only faces. Noto Serif maps “ ” … — ·, and
/// comes first in the stack, yet every cluster of the Han run is served by
/// Noto Sans JP.
#[test]
fn a_han_run_skips_a_latin_face_that_shares_its_punctuation() {
    let text = "「東京」“東京”……——·";
    let style = cjk_style(&["Noto Serif", "Noto Sans JP"], Some("ja-JP"));
    let options = FrontendOptions {
        quote_policy: QuotePolicy::CjkAware,
        default_locale: LocaleTag::try_from_str_lenient("ja-JP").unwrap(),
        ..FrontendOptions::default()
    };
    let ast = compile_with(text, &style, &[], &options);
    assert!(
        ast.runs.iter().all(|run| run.script == ScriptTag::HAN),
        "{:?}",
        ast.runs.iter().map(|r| r.script).collect::<Vec<_>>()
    );
    for cluster in &ast.clusters {
        assert_eq!(
            cluster.font.font_id,
            JP,
            "{:?}",
            cluster_text(text, cluster)
        );
        assert!(
            ast.glyphs[cluster.glyph_range.clone()]
                .iter()
                .all(|g| g.glyph_id != 0),
            ".notdef for {:?}",
            cluster_text(text, cluster)
        );
    }
    // The ideographs, corner brackets and ellipses are full width at 18.
    // Noto Sans JP's own “ ” and · are proportional (about 474 and 561 units,
    // the Japanese convention); they are its glyphs, not Noto Serif's 450 and 250.
    let width = |s: &str| {
        ast.clusters
            .iter()
            .find(|c| cluster_text(text, c) == s)
            .map(|c| c.natural_width)
            .unwrap_or_else(|| panic!("{s} has a cluster"))
    };
    for full in ["「", "東", "」", "…"] {
        assert_close(width(full), 18.0);
    }
    for (proportional, serif) in [("“", 0.450), ("·", 0.250)] {
        let w = width(proportional);
        assert!(
            w > 18.0 * 0.46 && w < 18.0 * 0.6 && (w - 18.0 * serif).abs() > 0.1,
            "{proportional}: {w}"
        );
    }
}

/// SHP-25: ASCII atoms keep stack order inside a Han run. FE-13 folds the
/// digits of 2026年 into the `Hani` run, yet they come from Noto Serif first
/// in the stack, while 年 and the shared quotes skip it for Noto Sans SC.
#[test]
fn ascii_in_a_han_run_keeps_the_leading_latin_face() {
    let text = "2026年“東京”";
    let style = cjk_style(&["Noto Serif", "Noto Sans SC"], Some("zh-CN"));
    let options = FrontendOptions {
        quote_policy: QuotePolicy::CjkAware,
        ..FrontendOptions::default()
    };
    let ast = compile_with(text, &style, &[], &options);
    assert!(
        ast.runs.iter().all(|run| run.script == ScriptTag::HAN),
        "{:?}",
        ast.runs.iter().map(|r| r.script).collect::<Vec<_>>()
    );
    for cluster in &ast.clusters {
        let piece = cluster_text(text, cluster);
        let want = if piece.is_ascii() { SERIF } else { SC };
        assert_eq!(cluster.font.font_id, want, "{piece:?}");
    }
    let digits = ast
        .clusters
        .iter()
        .filter(|c| cluster_text(text, c).is_ascii())
        .count();
    assert_eq!(digits, 4);
}

/// SHP-12 with the regional faces: in `zh-CN`, Noto Sans first in the stack
/// no longer serves the shared punctuation, which takes Noto Sans SC's full
/// width forms.
#[test]
fn simplified_chinese_punctuation_takes_the_full_width_face() {
    let text = "“东京”……——·";
    let style = cjk_style(&["Noto Sans", "Noto Sans SC"], Some("zh-CN"));
    let options = FrontendOptions {
        quote_policy: QuotePolicy::CjkAware,
        ..FrontendOptions::default()
    };
    let text = text.replace('东', "東");
    let ast = compile_with(&text, &style, &[], &options);
    for cluster in &ast.clusters {
        assert_eq!(
            cluster.font.font_id,
            SC,
            "{:?}",
            cluster_text(&text, cluster)
        );
    }
    for cluster in ast
        .clusters
        .iter()
        .filter(|c| matches!(cluster_text(&text, c), "“" | "”" | "…" | "·"))
    {
        assert_close(cluster.natural_width, 18.0);
    }
}

/// SHP-25: one fixed stack, three locales by style span, three faces.
#[test]
fn style_span_locales_route_han_to_their_regional_faces() {
    let families = ["Noto Sans", "Noto Sans SC", "Noto Sans TC", "Noto Sans JP"];
    let default_style = cjk_style(&families, None);
    let parts = [
        ("令你甘心。骨肉分离。", "zh-CN", SC),
        ("令你甘心。骨肉分離。", "zh-Hant", TC),
        ("令你甘心。骨肉分離。", "zh-TW", TC),
        ("令。骨肉分離。", "ja-JP", JP),
        ("令。骨肉分離。", "jp", JP),
    ];
    let mut text = String::new();
    let mut spans = Vec::new();
    let mut expected = Vec::new();
    for (i, (part, locale, face)) in parts.iter().enumerate() {
        if i > 0 {
            text.push(' ');
        }
        let start = text.len();
        text.push_str(part);
        let range = TextRange::new(start, text.len());
        spans.push(StyleSpan {
            byte_range: range,
            style: cjk_style(&families, Some(locale)),
        });
        expected.push((range, *face));
    }
    let ast = compile_with(&text, &default_style, &spans, &FrontendOptions::default());
    for cluster in &ast.clusters {
        let Some(&(_, face)) = expected
            .iter()
            .find(|(range, _)| range.contains_range(cluster.byte_range))
        else {
            continue; // a separating space, shaped in the stack's Latin face
        };
        assert_eq!(
            cluster.font.font_id,
            face,
            "{:?}",
            cluster_text(&text, cluster)
        );
    }
    // The stack is the same for every run: the engine chose, not the host.
    assert!(
        ast.runs
            .iter()
            .all(|run| run.families == default_style.families)
    );
}

/// SHP-25 pass 2: a Simplified ideograph the Japanese face lacks (们, which
/// only the SC subset maps) comes from the next CJK face that has it, past
/// Noto Sans TC, never from the Latin face.
#[test]
fn an_ideograph_the_regional_face_lacks_falls_to_another_cjk_face() {
    let text = "人们東京";
    let style = cjk_style(
        &["Noto Sans", "Noto Sans JP", "Noto Sans TC", "Noto Sans SC"],
        Some("ja-JP"),
    );
    let ast = compile_with(text, &style, &[], &FrontendOptions::default());
    for cluster in &ast.clusters {
        let want = if cluster_text(text, cluster) == "们" {
            SC
        } else {
            JP
        };
        assert_eq!(
            cluster.font.font_id,
            want,
            "{:?}",
            cluster_text(text, cluster)
        );
    }
}

/// SHP-16: the run's locale reaches HarfRust as its language, so the
/// pan-CJK face's `locl` picks the regional glyph of 令 and 骨.
#[test]
fn the_run_locale_selects_localised_glyphs() {
    let glyphs = |locale: &str| -> Vec<u32> {
        let style = cjk_style(&["Noto Sans CJK SC"], Some(locale));
        let ast = compile_with("令骨", &style, &[], &FrontendOptions::default());
        assert!(ast.clusters.iter().all(|c| c.font.font_id == PAN));
        ast.glyphs.iter().map(|g| g.glyph_id).collect()
    };
    let simplified = glyphs("zh-CN");
    let traditional = glyphs("zh-TW");
    let japanese = glyphs("ja-JP");
    assert_ne!(simplified, traditional, "ZHT changes 令 or 骨");
    assert_ne!(simplified, japanese, "JAN changes 令 or 骨");
    assert_eq!(glyphs("zh-Hant"), traditional);
    assert_eq!(glyphs("zh-tc"), traditional);
    assert_eq!(glyphs("cn"), simplified);
}
