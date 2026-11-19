//! A slot whose content block overflows past the slots after it (OUT-4): a
//! glyph is held by any slot whose content block holds its pen interval,
//! whatever the order of the blocks' edges.

use super::common::*;
use mutil_engine::*;
use smallvec::smallvec;

/// A glyph from source bytes `start..end`, pen at `x`, `advance` wide.
fn at(start: usize, end: usize, x: f32, advance: f32) -> PositionedGlyph {
    PositionedGlyph {
        byte_range: Some(TextRange::new(start, end)),
        ..glyph(x, advance)
    }
}

/// Four slots with content blocks `[0, 5]`, `[20, 100]`, `[40, 45]` and
/// `[60, 65]`; the second one's block runs past the right edges of the
/// third and fourth slots' constraints and blocks.
fn overflowing_line(glyphs: Vec<PositionedGlyph>) -> PositionedLine {
    let mut l = line();
    l.slots = smallvec![
        slot(0., 5., 10.),
        slot(20., 80., 10.),
        slot(40., 5., 10.),
        slot(60., 5., 10.),
    ];
    l.byte_range = TextRange::new(0, glyphs.len());
    l.visible_byte_end = glyphs.len();
    let mut r = run(glyphs[0]);
    r.byte_range = l.byte_range;
    r.glyphs = glyphs;
    l.visual_runs = vec![r];
    l
}

#[test]
fn out4_a_glyph_inside_a_later_slots_block_is_body_though_an_earlier_block_spans_past_it() {
    let inside_third = at(0, 1, 40., 5.);
    let inside_fourth = at(1, 2, 60., 5.);
    let l = overflowing_line(vec![inside_third, inside_fourth]);
    assert_eq!(l.role(&inside_third), GlyphRole::Body);
    assert_eq!(l.role(&inside_fourth), GlyphRole::Body);
    assert_eq!(l.hanging_glyphs().count(), 0);
    assert_eq!(l.hanging_left(), 0.0);
    assert_eq!(l.hanging_right(), 0.0);
}

#[test]
fn out4_a_glyph_in_the_first_block_and_one_in_the_spanning_block_are_body_too() {
    let first = at(0, 1, 0., 5.);
    let spanning = at(1, 2, 20., 15.);
    let l = overflowing_line(vec![first, spanning]);
    assert_eq!(l.role(&first), GlyphRole::Body);
    assert_eq!(l.role(&spanning), GlyphRole::Body);
}

#[test]
fn out4_the_glyph_inside_the_third_block_is_not_hanging_under_validation() {
    let c = LineContext {
        wrap_width: 100.,
        ..ctx()
    };
    let l = overflowing_line(vec![at(0, 1, 40., 5.)]);
    assert!(
        !matches!(
            l.validate(&c),
            Err(LineInvariantError::HangingNotAllowed { .. }
                | LineInvariantError::BodyGlyphOutsideBlock { .. })
        ),
        "{:?}",
        l.validate(&c)
    );
}

/// Blocks `[0, 5]`, `[20, 100]` (past the next slot's `[40, 45]`) and
/// `[110, 115]` and `[130, 135]` after it.
fn far_line(glyphs: Vec<PositionedGlyph>) -> PositionedLine {
    let mut l = overflowing_line(glyphs);
    l.slots = smallvec![
        slot(0., 5., 10.),
        slot(20., 80., 10.),
        slot(40., 5., 10.),
        slot(110., 5., 10.),
        slot(130., 5., 10.),
    ];
    l
}

#[test]
fn out4_every_glyph_held_by_a_block_is_body_wherever_the_overflow_ends() {
    let held = [
        at(0, 1, 0., 5.),
        at(1, 2, 20., 5.),
        at(2, 3, 40., 5.),
        at(3, 4, 110., 5.),
        at(4, 5, 130., 5.),
    ];
    let l = far_line(held.to_vec());
    for g in &held {
        assert_eq!(l.role(g), GlyphRole::Body, "{g:?}");
    }
    assert_eq!(l.hanging_glyphs().count(), 0);
}

#[test]
fn out4_validation_reports_the_overflowing_slot_not_a_role_of_a_later_glyph() {
    // Blocks `[0, 5]`, `[20, 100]`, `[40, 45]` and `[60, 65]` over slots
    // whose capacity is 10: the second slot is the one that overflows, and
    // the glyphs inside the third and fourth blocks are held, not hanging.
    let l = overflowing_line(vec![
        at(0, 1, 0., 5.),
        at(1, 2, 20., 80.),
        at(2, 3, 40., 5.),
        at(3, 4, 60., 5.),
    ]);
    let c = LineContext {
        wrap_width: 100.,
        ..ctx()
    };
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::SlotOverflow {
            slot: 1,
            extent: 80.,
            capacity: 10.
        })
    );
    assert_eq!(l.hanging_glyphs().count(), 0);
}
