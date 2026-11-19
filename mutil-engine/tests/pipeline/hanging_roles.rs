//! End to end: CJK hanging punctuation is exactly the line's hanging
//! glyphs (spec ST1-3, HNG-1, HNG-4, OUT-4) and the layout passes the
//! output gate (PAR-5).

use crate::acceptance_phase2::glyphs;
use crate::common::*;
use mutil_engine::*;

#[test]
fn out4_hanging_glyphs_are_exactly_the_hanging_cjk_marks() {
    // The opener and the final stop hang; the comma in the middle and the
    // ideographs do not.
    let text = "「漢字、漢字。";
    let options = LayoutOptions {
        hang_mode: HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::Force,
        },
        ..Default::default()
    };
    let p = lay_out_bidi(text, &[JAPANESE], &column(100.0), &options);
    ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    assert_eq!(p.lines.len(), 1);
    let line = &p.lines[0];
    let hanging: Vec<usize> = line
        .hanging_glyphs()
        .map(|g| g.byte_range.unwrap().start())
        .collect();
    assert_eq!(hanging, [0, text.rfind('。').unwrap()]);
    for g in glyphs(line) {
        let expected = if hanging.contains(&g.byte_range.unwrap().start()) {
            GlyphRole::Hanging
        } else {
            GlyphRole::Body
        };
        assert_eq!(line.role(g), expected, "{:?}", g.byte_range);
    }
    assert!(line.hanging_left() > 0.0);
    assert!(line.hanging_right() > 0.0);
}

#[test]
fn out4_without_a_hang_mode_nothing_has_the_hanging_role() {
    let text = "「漢字、漢字。";
    let p = lay_out_bidi(text, &[JAPANESE], &column(100.0), &LayoutOptions::default());
    ValidatedLayout::try_from_paragraph(&p, text).unwrap();
    let line = &p.lines[0];
    assert_eq!(line.hanging_glyphs().count(), 0);
    assert!(glyphs(line).all(|g| line.role(g) == GlyphRole::Body));
    assert_eq!((line.hanging_left(), line.hanging_right()), (0.0, 0.0));
}
