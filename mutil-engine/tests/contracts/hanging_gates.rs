//! The output gate on hand-built hanging lines (spec HNG-4, OUT-4, LV-2):
//! one character per side, and it is the outermost glyph; and the Phase 2
//! cluster flags of ATM-4.

use super::common::*;
use mutil_engine::*;

/// A full one-slot line over bytes `0..3`: a body glyph `[0, 97]` (bytes
/// `0..1`) and a mark `[97, 103]` (bytes `1..2`) hanging 3 units beyond the
/// slot's right edge at 100, in one run.
fn right_hanging_line() -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    let mut body = run(glyph(0., 97.));
    body.byte_range = l.byte_range;
    body.glyphs.push(PositionedGlyph {
        byte_range: Some(TextRange::new(1, 2)),
        ..glyph(97., 6.)
    });
    l.visual_runs = vec![body];
    l
}

fn force_end() -> LineContext {
    LineContext {
        hang_mode: HangMode {
            start: HangStartMode::None,
            end: HangEndMode::Force,
        },
        ..ctx()
    }
}

fn all_lines_start() -> LineContext {
    LineContext {
        hang_mode: HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::None,
        },
        ..ctx()
    }
}

#[test]
fn one_hanging_mark_per_side_passes() {
    let l = right_hanging_line();
    assert_eq!(l.validate(&force_end()), Ok(()));
    assert_eq!(l.hanging_right(), 3.0);
    assert_eq!(l.hanging_left(), 0.0);
    assert_eq!(l.visual_width(), 100.0);
}

#[test]
fn two_glyphs_of_one_character_count_as_one_hanging_mark() {
    // A second glyph of the same character (bytes 1..2), drawn at the same
    // pen interval: still one source range.
    let mut l = right_hanging_line();
    let mark = l.visual_runs[0].glyphs[1];
    l.visual_runs[0].glyphs.push(mark);
    assert_eq!(l.validate(&force_end()), Ok(()));
}

#[test]
fn two_characters_hanging_on_one_side_are_too_many() {
    // A second character (bytes 2..3) with the same pen interval as the
    // first: both are outermost, but two characters hang.
    let mut l = right_hanging_line();
    l.visual_runs[0].glyphs.push(PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(97., 6.)
    });
    assert_eq!(
        l.validate(&force_end()),
        Err(LineInvariantError::TooManyHangingGlyphs {
            side: HangSide::Right
        })
    );
    // The same on the left.
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    let mut r = run(glyph(-3., 6.));
    r.byte_range = l.byte_range;
    r.glyphs.push(PositionedGlyph {
        byte_range: Some(TextRange::new(1, 2)),
        ..glyph(-3., 6.)
    });
    r.glyphs.push(PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(3., 97.)
    });
    l.visual_runs = vec![r];
    assert_eq!(
        l.validate(&all_lines_start()),
        Err(LineInvariantError::TooManyHangingGlyphs {
            side: HangSide::Left
        })
    );
}

#[test]
fn a_hanging_mark_must_be_the_outermost_glyph_on_its_side() {
    // A zero-advance body mark (bytes 2..3) whose pen position lies at 104,
    // beyond the hanging mark's right edge at 103: a zero-advance mark is
    // exempt from containment (OUT-1), but its pen interval reaches further
    // right than the hanging glyph's.
    let mut l = right_hanging_line();
    l.visual_runs[0].glyphs.push(PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(104., 0.)
    });
    assert!(
        matches!(
            l.validate(&force_end()),
            Err(LineInvariantError::HangingGlyphNotOutermost { .. })
        ),
        "{:?}",
        l.validate(&force_end())
    );
    // At 103, level with the mark's edge, it does not reach further.
    let mut l = right_hanging_line();
    l.visual_runs[0].glyphs.push(PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(103., 0.)
    });
    assert_eq!(l.validate(&force_end()), Ok(()));
}

#[test]
fn atm4_hangable_flags_come_from_the_clusters_edges() {
    let with = |offset: usize, flags: AtomFlags| GraphemeAtom {
        flags,
        ..atom(offset)
    };
    // HANGABLE_START is read from the first atom only, HANGABLE_END and
    // ALLOW_CJK_JUSTIFY_AFTER from the last only; NO_LETTER_SPACING and
    // IS_CJK_IDEOGRAPH from any atom.
    let atoms = [
        with(0, AtomFlags::HANGABLE_START | AtomFlags::IS_CJK_IDEOGRAPH),
        with(
            1,
            AtomFlags::HANGABLE_START
                | AtomFlags::HANGABLE_END
                | AtomFlags::ALLOW_CJK_JUSTIFY_AFTER
                | AtomFlags::NO_LETTER_SPACING,
        ),
    ];
    let f = AtomFlags::for_cluster(&atoms);
    assert!(f.contains(AtomFlags::HANGABLE_START));
    assert!(f.contains(AtomFlags::HANGABLE_END));
    assert!(f.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER));
    assert!(f.contains(AtomFlags::NO_LETTER_SPACING | AtomFlags::IS_CJK_IDEOGRAPH));
    let reversed = [atoms[1].clone(), atoms[0].clone()];
    let f = AtomFlags::for_cluster(&reversed);
    assert!(f.contains(AtomFlags::HANGABLE_START));
    assert!(!f.contains(AtomFlags::HANGABLE_END));
    assert!(!f.contains(AtomFlags::ALLOW_CJK_JUSTIFY_AFTER));
    assert!(f.contains(AtomFlags::NO_LETTER_SPACING | AtomFlags::IS_CJK_IDEOGRAPH));
    let atoms = [
        with(0, AtomFlags::EMPTY),
        with(1, AtomFlags::HANGABLE_START),
    ];
    assert!(!AtomFlags::for_cluster(&atoms).contains(AtomFlags::HANGABLE_START));
    // The partition constants name them.
    assert!(AtomFlags::FIRST_ATOM_ONLY.contains(AtomFlags::HANGABLE_START));
    assert!(
        AtomFlags::LAST_ATOM_ONLY
            .contains(AtomFlags::HANGABLE_END | AtomFlags::ALLOW_CJK_JUSTIFY_AFTER)
    );
    assert!(AtomFlags::ALL.contains(
        AtomFlags::NO_LETTER_SPACING
            | AtomFlags::IS_CJK_IDEOGRAPH
            | AtomFlags::HANGABLE_START
            | AtomFlags::HANGABLE_END
    ));
}
