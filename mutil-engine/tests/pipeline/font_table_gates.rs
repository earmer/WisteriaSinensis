//! Fonts whose tables hold unusable numbers: a zero `unitsPerEm` is refused
//! at registration (SHP-21), and a negative ascent or descent is judged by
//! the shaper gate, never repaired (SHP-18, FNT-4, SHP-23).

use crate::common::*;
use mutil_engine::*;
use std::sync::Arc;

/// The (offset, length) of sfnt table `tag` in `font`.
fn table(font: &[u8], tag: &[u8; 4]) -> Option<(usize, usize)> {
    let count = u16::from_be_bytes([font[4], font[5]]) as usize;
    (0..count).find_map(|i| {
        let record = &font[12 + 16 * i..28 + 16 * i];
        (&record[..4] == tag).then(|| {
            let be = |at: usize| u32::from_be_bytes(record[at..at + 4].try_into().unwrap());
            (be(8) as usize, be(12) as usize)
        })
    })
}

/// A copy of `font` with the big-endian 16-bit `value` written at `offset`
/// of table `tag` (checksums are left stale), if the table is long enough.
fn patched(font: &[u8], tag: &[u8; 4], offset: usize, value: [u8; 2]) -> Vec<u8> {
    let (start, length) = table(font, tag).unwrap_or_else(|| panic!("no {tag:?} table"));
    assert!(offset + 2 <= length, "{tag:?} is too short");
    let mut bytes = font.to_vec();
    bytes[start + offset..start + offset + 2].copy_from_slice(&value);
    bytes
}

fn add(bytes: Vec<u8>) -> Option<FontId> {
    FontRegistry::new().add_face(
        FAMILY.into(),
        FontStyle::default(),
        FontFace::new(FontBytes::new(bytes), 0),
    )
}

const UNITS_PER_EM: usize = 18;
const HHEA_ASCENDER: usize = 4;
const HHEA_DESCENDER: usize = 6;
const OS2_TYPO_ASCENDER: usize = 68;
const OS2_TYPO_DESCENDER: usize = 70;

#[test]
fn shp21_the_unpatched_font_registers() {
    assert_eq!(add(FONT_BYTES.to_vec()), Some(FontId(0)));
    // The parser does not insist on the stale checksum.
    assert_eq!(
        add(patched(
            FONT_BYTES,
            b"head",
            UNITS_PER_EM,
            2048u16.to_be_bytes()
        )),
        Some(FontId(0))
    );
}

#[test]
fn shp21_a_font_with_zero_units_per_em_is_not_registered() {
    let bytes = patched(FONT_BYTES, b"head", UNITS_PER_EM, [0, 0]);
    let mut registry = FontRegistry::new();
    let face = FontFace::new(FontBytes::new(bytes), 0);
    assert_eq!(
        registry.add_face(FAMILY.into(), FontStyle::default(), face),
        None
    );
    // Nothing was registered: the next face still gets the first id.
    let good = FontFace::new(FontBytes::from_static(FONT_BYTES), 0);
    assert_eq!(
        registry.add_face(FAMILY.into(), FontStyle::default(), good),
        Some(FontId(0))
    );
}

/// A shaper whose primary face is the test font, with a second family,
/// `Bad`, made of `bad_bytes`.
fn shaper_with_bad_face(bad_bytes: Vec<u8>) -> HarfRustShaper {
    let mut registry = FontRegistry::new();
    registry
        .add_face(
            FAMILY.into(),
            FontStyle::default(),
            FontFace::new(FontBytes::from_static(FONT_BYTES), 0),
        )
        .expect("the test font parses");
    registry
        .add_face(
            "Bad".into(),
            FontStyle::default(),
            FontFace::new(FontBytes::new(bad_bytes), 0),
        )
        .expect("the patched font still registers");
    registry.set_generic(GenericFamily::Monospace, FAMILY.into());
    HarfRustShaper::new(Arc::new(registry))
}

/// Lays out `text` with the strut from the good face and the whole text in
/// the `Bad` family.
fn lay_out_in_bad_face(
    shaper: &HarfRustShaper,
    text: &str,
) -> Result<PositionedParagraph, LayoutInputError> {
    let base = style(10.0);
    let bad = TextStyle {
        families: Arc::from([FontFamily::Named("Bad".into())]),
        ..style(10.0)
    };
    let spans = [StyleSpan {
        byte_range: TextRange::new(0, text.len()),
        style: bad,
    }];
    let input = ParagraphInput {
        style_spans: &spans,
        ..input(text, &base)
    };
    let ast = compile_paragraph(&input, &FrontendOptions::default(), shaper)
        .expect("the frontend accepts the text");
    layout_paragraph(
        &ast,
        text,
        shaper,
        &column(200.0),
        &LayoutOptions::default(),
    )
}

/// Both tables the line metrics may come from (SHP-23: OS/2 typo metrics
/// under `USE_TYPO_METRICS`, else `hhea`) get the same number, so the test
/// does not depend on which one the face selects.
fn with_metrics(ascender: i16, descender: i16) -> Vec<u8> {
    let mut bytes = FONT_BYTES.to_vec();
    for (tag, offset, value) in [
        (b"hhea", HHEA_ASCENDER, ascender),
        (b"hhea", HHEA_DESCENDER, descender),
        (b"OS/2", OS2_TYPO_ASCENDER, ascender),
        (b"OS/2", OS2_TYPO_DESCENDER, descender),
    ] {
        bytes = patched(&bytes, tag, offset, value.to_be_bytes());
    }
    bytes
}

#[test]
fn shp18_fnt4_the_unpatched_bad_family_lays_out() {
    let shaper = shaper_with_bad_face(FONT_BYTES.to_vec());
    assert!(lay_out_in_bad_face(&shaper, "abc").is_ok());
}

#[test]
fn shp18_fnt4_a_negative_ascent_is_rejected_at_the_shaper_gate() {
    // The font's descender is about -300 units, so the ascent plus descent
    // stays positive: only the sign of the ascent is wrong.
    let shaper = shaper_with_bad_face(with_metrics(-100, -300));
    assert_eq!(
        lay_out_in_bad_face(&shaper, "abc").map(drop),
        Err(LayoutInputError::InvalidShaperMetrics)
    );
}

#[test]
fn shp18_fnt4_a_negative_descent_is_rejected_at_the_shaper_gate() {
    // `hhea` stores the descender positive towards line-over: a positive
    // value of 100 is a descent of -100 below the baseline.
    let shaper = shaper_with_bad_face(with_metrics(1000, 100));
    assert_eq!(
        lay_out_in_bad_face(&shaper, "abc").map(drop),
        Err(LayoutInputError::InvalidShaperMetrics)
    );
}
