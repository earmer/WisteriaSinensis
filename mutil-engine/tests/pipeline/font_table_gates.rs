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

#[test]
fn shp18_fnt4_the_unpatched_bad_family_lays_out() {
    let shaper = shaper_with_bad_face(FONT_BYTES.to_vec());
    assert!(lay_out_in_bad_face(&shaper, "abc").is_ok());
}

/// Offsets of the line metrics SHP-23 reads: `hhea` ascender and
/// descender, OS/2 `sTypoAscender` and `sTypoDescender`. The test font sets
/// `USE_TYPO_METRICS`, so the typo pair is the one used; both are patched.
const HHEA_ASCENDER: usize = 4;
const HHEA_DESCENDER: usize = 6;
const TYPO_ASCENDER: usize = 68;
const TYPO_DESCENDER: usize = 70;

/// The test font with `ascender` and `descender` stored in both `hhea` and
/// the OS/2 typo metrics, signs as given.
fn with_line_metrics(ascender: i16, descender: i16) -> Vec<u8> {
    let bytes = patched(FONT_BYTES, b"hhea", HHEA_ASCENDER, ascender.to_be_bytes());
    let bytes = patched(&bytes, b"hhea", HHEA_DESCENDER, descender.to_be_bytes());
    let bytes = patched(&bytes, b"OS/2", TYPO_ASCENDER, ascender.to_be_bytes());
    patched(&bytes, b"OS/2", TYPO_DESCENDER, descender.to_be_bytes())
}

/// The metrics of the `Bad` face (id 1) at 10 units per em.
fn bad_face_metrics(shaper: &HarfRustShaper) -> FontMetrics {
    shaper.font_metrics(FontInstanceRef {
        font: FontRef::unscaled(FontId(1)),
        font_size: 10.0,
        variations: &[],
    })
}

#[test]
fn shp23_a_descender_stored_positive_is_read_by_its_size() {
    // 1000 units per em at size 10: 2000 units are 20, 500 units are 5.
    let usual = shaper_with_bad_face(with_line_metrics(2000, -500));
    let positive = shaper_with_bad_face(with_line_metrics(2000, 500));
    for (name, shaper) in [("usual", &usual), ("positive", &positive)] {
        let metrics = bad_face_metrics(shaper);
        assert_close(metrics.ascent, 20.0);
        assert_close(metrics.descent, 5.0);
        let p = lay_out_in_bad_face(shaper, "abc").unwrap_or_else(|e| panic!("{name}: {e}"));
        if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&p, "abc") {
            panic!("{name}: line {line}: {error}");
        }
        // The face is taller than the strut, so it sets the line box.
        assert_close(p.lines[0].ascent, 20.0);
        assert_close(p.lines[0].descent, 5.0);
    }
    assert_eq!(bad_face_metrics(&usual), bad_face_metrics(&positive));
}

#[test]
fn shp23_a_small_positive_descender_lays_out() {
    let shaper = shaper_with_bad_face(with_line_metrics(1000, 100));
    let metrics = bad_face_metrics(&shaper);
    assert_close(metrics.ascent, 10.0);
    assert_close(metrics.descent, 1.0);
    assert!(lay_out_in_bad_face(&shaper, "abc").is_ok());
}

#[test]
fn shp23_a_negative_ascender_is_read_by_its_size() {
    let shaper = shaper_with_bad_face(with_line_metrics(-2000, -500));
    let metrics = bad_face_metrics(&shaper);
    assert_close(metrics.ascent, 20.0);
    assert_close(metrics.descent, 5.0);
    let p = lay_out_in_bad_face(&shaper, "abc").unwrap_or_else(|e| panic!("{e}"));
    assert!(ValidatedLayout::try_from_paragraph(&p, "abc").is_ok());
    assert_close(p.lines[0].ascent, 20.0);
}
