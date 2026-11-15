//! The HarfRust backend against the test font (spec §4, SHP-20, FNT-4).

use crate::common::*;
use mutil_engine::*;

#[test]
fn metrics_follow_the_font_tables_scaled_to_the_instance() {
    let m = SHAPER.font_metrics(instance(10.0));
    assert!(m.is_valid());
    // USE_TYPO_METRICS: typo ascender 1020, descender -300, gap 0.
    assert!(m.use_typo_metrics);
    for (actual, expected) in [
        (m.ascent, 10.2),
        (m.descent, 3.0),
        (m.line_gap, 0.0),
        // `post` and OS/2 are already positive towards line-over (FNT-4).
        (m.underline_position, -1.55),
        (m.strikeout_position, 3.2),
        // No vhea: half an em each side; no BASE: the fallbacks.
        (m.vertical_ascent, 5.0),
        (m.vertical_descent, 5.0),
        (m.ideographic_baseline_offset, -3.0),
    ] {
        assert_close(actual, expected);
    }
    let (id, primary) = SHAPER.resolve_primary_font_metrics(
        &[FontFamily::Generic(GenericFamily::Monospace)],
        FontStyle::default(),
        ScriptTag::LATIN,
        &LocaleTag::und(),
        10.0,
        &[],
    );
    assert_eq!((id, primary), (FontId(0), m));
}

#[test]
fn the_space_advance_is_the_space_glyph_scaled() {
    assert_eq!(
        SHAPER.space_advance(instance(10.0), GlyphOrientation::Horizontal),
        6.0
    );
    let scaled = FontInstanceRef {
        font: FontRef {
            font_id: FontId(0),
            scale: 2.0,
        },
        ..instance(10.0)
    };
    assert_eq!(
        SHAPER.space_advance(scaled, GlyphOrientation::Horizontal),
        12.0
    );
}
