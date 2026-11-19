//! Glyph roles and the hanging rules on hand-built lines (spec HNG-1 to
//! HNG-4, OUT-4): the role of a glyph is derived from its position against
//! the line's slots, never stored.

use super::common::*;
use mutil_engine::*;
use smallvec::smallvec;

/// Within the geometry tolerance (OUT-2) of a slot edge at about 100.
const WITHIN_TOLERANCE: f32 = 0.0005;

/// A glyph from source bytes `start..end`, pen at `x`, `advance` wide.
fn at(start: usize, end: usize, x: f32, advance: f32) -> PositionedGlyph {
    PositionedGlyph {
        byte_range: Some(TextRange::new(start, end)),
        ..glyph(x, advance)
    }
}

/// A line over bytes `0..end` whose glyphs are all in one run, in a slot
/// `[0, 100]` holding 100 units of content, with the first `visible` bytes
/// visible.
fn line_of(glyphs: Vec<PositionedGlyph>, end: usize, visible: usize) -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::new(0, end);
    l.visible_byte_end = visible;
    let mut r = run(glyphs[0]);
    r.byte_range = l.byte_range;
    r.glyphs = glyphs;
    l.visual_runs = vec![r];
    l
}

fn mode(start: HangStartMode, end: HangEndMode) -> LineContext {
    LineContext {
        hang_mode: HangMode { start, end },
        ..ctx()
    }
}

fn force_end() -> LineContext {
    mode(HangStartMode::None, HangEndMode::Force)
}

fn allow_end() -> LineContext {
    mode(HangStartMode::None, HangEndMode::Allow)
}

fn all_lines_start() -> LineContext {
    mode(HangStartMode::AllLines, HangEndMode::None)
}

fn both_sides() -> LineContext {
    mode(HangStartMode::AllLines, HangEndMode::Force)
}

/// A body glyph over `[0, 97]` (bytes `0..1`) and a mark over `[97, 103]`
/// (bytes `1..2`), 3 units of which hang past the right edge at 100.
fn right_mark() -> PositionedLine {
    line_of(vec![at(0, 1, 0., 97.), at(1, 2, 97., 6.)], 2, 2)
}

/// A mark over `[-3, 3]` (bytes `0..1`) hanging 3 units past the left edge
/// at 0, then a body glyph over `[3, 100]` (bytes `1..2`).
fn left_mark() -> PositionedLine {
    line_of(vec![at(0, 1, -3., 6.), at(1, 2, 3., 97.)], 2, 2)
}

/// `l` with one more glyph appended to its run, over a range one byte
/// longer.
fn with_glyph(mut l: PositionedLine, g: PositionedGlyph) -> PositionedLine {
    l.byte_range = TextRange::new(0, l.byte_range.end() + 1);
    l.visible_byte_end = l.byte_range.end();
    l.visual_runs[0].byte_range = l.byte_range;
    l.visual_runs[0].glyphs.push(g);
    l
}

#[test]
fn hng2_a_glyph_in_the_hang_zone_with_hanging_disabled_is_not_allowed() {
    let right = right_mark();
    assert_eq!(
        right.validate(&ctx()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    // A start mode does not allow hanging at the end.
    assert_eq!(
        right.validate(&all_lines_start()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    let left = left_mark();
    assert_eq!(
        left.validate(&ctx()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Left
        })
    );
    // An end mode does not allow hanging at the start.
    assert_eq!(
        left.validate(&force_end()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Left
        })
    );
    // The same lines are valid where the mode permits.
    assert_eq!(right.validate(&force_end()), Ok(()));
    assert_eq!(left.validate(&all_lines_start()), Ok(()));
}

#[test]
fn hng2_first_line_start_mode_does_not_allow_a_later_line() {
    let left = left_mark();
    let first_line = mode(HangStartMode::FirstLine, HangEndMode::None);
    assert_eq!(left.validate(&first_line), Ok(()));
    let later = LineContext {
        line_index: 1,
        ..first_line
    };
    assert_eq!(
        left.validate(&later),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Left
        })
    );
}

#[test]
fn hng2_under_allow_a_mark_that_would_have_fitted_is_without_overflow() {
    // The slot is 200 wide but holds 100 of content: the mark would have
    // fitted without hanging.
    let mut l = right_mark();
    l.slots = smallvec![slot(0., 100., 200.)];
    let wide = LineContext {
        wrap_width: 200.,
        ..allow_end()
    };
    assert_eq!(
        l.validate(&wide),
        Err(LineInvariantError::HangingWithoutOverflow {
            side: HangSide::Right
        })
    );
    // `Force` hangs regardless.
    let forced = LineContext {
        wrap_width: 200.,
        ..force_end()
    };
    assert_eq!(l.validate(&forced), Ok(()));
    // In a slot exactly as wide as its body, the mark needed to hang.
    assert_eq!(right_mark().validate(&allow_end()), Ok(()));
}

#[test]
fn hng4_two_characters_overhanging_one_side_are_too_many() {
    // Two different byte ranges over the same overhanging pen interval.
    let right = with_glyph(right_mark(), at(2, 3, 97., 6.));
    assert_eq!(
        right.validate(&force_end()),
        Err(LineInvariantError::TooManyHangingGlyphs {
            side: HangSide::Right
        })
    );
    // Two marks of different extent on the left.
    let left = with_glyph(left_mark(), at(2, 3, -2., 5.));
    let left = {
        let mut l = left;
        l.visual_runs[0].glyphs.swap(1, 2);
        l
    };
    assert_eq!(
        left.validate(&all_lines_start()),
        Err(LineInvariantError::TooManyHangingGlyphs {
            side: HangSide::Left
        })
    );
}

#[test]
fn hng4_one_hanging_mark_on_each_side_is_valid() {
    let l = line_of(
        vec![at(0, 1, -3., 6.), at(1, 2, 3., 94.), at(2, 3, 97., 6.)],
        3,
        3,
    );
    assert_eq!(l.validate(&both_sides()), Ok(()));
    assert_eq!(l.hanging_left(), 3.0);
    assert_eq!(l.hanging_right(), 3.0);
}

#[test]
fn hng4_a_glyph_reaching_beyond_the_hanging_mark_is_not_outermost() {
    // A zero-advance glyph (exempt from containment, OUT-1) whose pen lies
    // beyond the mark's far edge at 103.
    let l = with_glyph(right_mark(), at(2, 3, 104., 0.));
    assert!(
        matches!(
            l.validate(&force_end()),
            Err(LineInvariantError::HangingGlyphNotOutermost { .. })
        ),
        "{:?}",
        l.validate(&force_end())
    );
    // The same on the left, beyond the mark's far edge at -3.
    let mut l = with_glyph(left_mark(), at(2, 3, -4., 0.));
    l.visual_runs[0].glyphs.rotate_right(1);
    assert!(
        matches!(
            l.validate(&all_lines_start()),
            Err(LineInvariantError::HangingGlyphNotOutermost { .. })
        ),
        "{:?}",
        l.validate(&all_lines_start())
    );
}

#[test]
fn hng1_a_glyph_wholly_outside_every_slot_is_rejected_where_hanging_is_off() {
    // The spec does not say whether a glyph with no part inside the slot
    // still hangs (the part that is not overflow is empty), so only the
    // disabled case is asserted: some error, never `Ok`.
    for glyphs in [
        vec![at(0, 1, -20., 10.), at(1, 2, 0., 100.)],
        vec![at(0, 1, 0., 100.), at(1, 2, 105., 10.)],
    ] {
        let l = line_of(glyphs, 2, 2);
        assert!(l.validate(&ctx()).is_err());
    }
}

#[test]
fn hng1_a_glyph_in_an_internal_obstacle_is_a_body_glyph_outside_the_block() {
    let (l, c) = two_slots();
    assert_eq!(l.validate(&c), Ok(()));
    let both = LineContext {
        hang_mode: HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::Force,
        },
        ..c
    };
    // Between the slots, wholly in the obstacle.
    let mut between = l.clone();
    between.visual_runs[0].glyphs[1] = at(1, 2, 110., 30.);
    assert_eq!(
        between.validate(&both),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 1 })
    );
    // Straddling the first slot's right edge: the slot is not the outer one
    // on that side, so the glyph does not hang from it.
    let mut straddle = l;
    straddle.visual_runs[0].glyphs[0] = at(0, 1, 0., 105.);
    assert_eq!(
        straddle.validate(&both),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
}

#[test]
fn hng1_a_glyph_in_an_unused_slot_is_a_body_glyph_outside_the_block() {
    // The first slot has no content: it holds nothing, and a glyph at its
    // position is neither in the used slot nor hanging from it.
    let mut l = line();
    l.byte_range = TextRange::new(0, 2);
    l.visible_byte_end = 2;
    l.slots = smallvec![slot(0., 0., 50.), slot(60., 100., 100.)];
    let mut r = run(at(0, 1, 10., 10.));
    r.byte_range = l.byte_range;
    r.glyphs.push(at(1, 2, 60., 100.));
    l.visual_runs = vec![r];
    let c = LineContext {
        wrap_width: 160.,
        ..both_sides()
    };
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
}

#[test]
fn hng1_overflow_within_the_tolerance_is_a_body_glyph() {
    let right = line_of(vec![at(0, 1, 0., 100. + WITHIN_TOLERANCE)], 1, 1);
    assert_eq!(right.validate(&ctx()), Ok(()));
    assert_eq!(right.role(&right.visual_runs[0].glyphs[0]), GlyphRole::Body);
    assert_eq!(right.hanging_glyphs().count(), 0);
    assert_eq!(right.hanging_right(), 0.0);
    assert_eq!(right.hanging_left(), 0.0);

    let left = line_of(
        vec![at(0, 1, -WITHIN_TOLERANCE, 100. + WITHIN_TOLERANCE)],
        1,
        1,
    );
    assert_eq!(left.validate(&ctx()), Ok(()));
    assert_eq!(left.role(&left.visual_runs[0].glyphs[0]), GlyphRole::Body);
    assert_eq!(left.hanging_left(), 0.0);
    assert_eq!(left.hanging_right(), 0.0);
}

#[test]
fn hng1_overflow_beyond_the_tolerance_is_a_hanging_glyph() {
    let l = line_of(vec![at(0, 1, 0., 90.), at(1, 2, 90., 10.01)], 2, 2);
    assert_eq!(l.role(&l.visual_runs[0].glyphs[1]), GlyphRole::Hanging);
    assert_eq!(l.role(&l.visual_runs[0].glyphs[0]), GlyphRole::Body);
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    assert_eq!(l.validate(&force_end()), Ok(()));
    assert!(
        (l.hanging_right() - 0.01).abs() < 1e-3,
        "{}",
        l.hanging_right()
    );
}

fn preserved_ctx(hang_mode: HangMode) -> LineContext {
    LineContext {
        whitespace: HangingWhitespaceMode::Preserved,
        hang_mode,
        ..ctx()
    }
}

#[test]
fn hng3_a_trailing_glyph_is_never_hanging() {
    // Preserved trailing whitespace (bytes 1..2, not visible) on the far
    // edge, 8 units beyond the right edge.
    let l = line_of(vec![at(0, 1, 0., 100.), at(1, 2, 100., 8.)], 2, 1);
    for hang_mode in [
        HangMode::NONE,
        HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::Force,
        },
    ] {
        assert_eq!(
            l.validate(&preserved_ctx(hang_mode)),
            Ok(()),
            "{hang_mode:?}"
        );
    }
    assert_eq!(l.role(&l.visual_runs[0].glyphs[1]), GlyphRole::Trailing);
    assert_eq!(l.hanging_glyphs().count(), 0);
    assert_eq!(l.hanging_right(), 0.0);
    assert_eq!(l.hanging_left(), 0.0);
    assert_eq!(l.visual_width(), 100.0);
}

#[test]
fn hng3_a_trailing_glyph_does_not_add_to_a_real_hanging_extent() {
    // A hanging mark (bytes 1..2) 3 units over the edge, then trailing
    // whitespace (bytes 2..3) further out.
    let l = line_of(
        vec![at(0, 1, 0., 97.), at(1, 2, 97., 6.), at(2, 3, 103., 8.)],
        3,
        2,
    );
    let c = preserved_ctx(HangMode {
        start: HangStartMode::None,
        end: HangEndMode::Force,
    });
    assert_eq!(l.validate(&c), Ok(()));
    let roles: Vec<_> = l
        .visual_runs
        .iter()
        .flat_map(|r| &r.glyphs)
        .map(|g| l.role(g))
        .collect();
    assert_eq!(
        roles,
        [GlyphRole::Body, GlyphRole::Hanging, GlyphRole::Trailing]
    );
    assert_eq!(l.hanging_right(), 3.0);
    let hanging: Vec<_> = l.hanging_glyphs().map(|g| g.byte_range).collect();
    assert_eq!(hanging, [Some(TextRange::new(1, 2))]);
}

#[test]
fn out4_a_zero_advance_glyph_has_its_own_role_and_adds_no_extent() {
    // A zero-advance glyph (bytes 1..2) with its pen anywhere is exempt
    // from containment and never hangs.
    for x in [-40., 50., 140.] {
        let mut glyphs = vec![at(0, 1, 0., 100.), at(1, 2, x, 0.)];
        let zero = if x < 0. { 0 } else { 1 };
        if x < 0. {
            glyphs.reverse();
        }
        let l = line_of(glyphs, 2, 2);
        for c in [ctx(), both_sides()] {
            assert_eq!(l.validate(&c), Ok(()), "x = {x}");
        }
        assert_eq!(
            l.role(&l.visual_runs[0].glyphs[zero]),
            GlyphRole::ZeroAdvance
        );
        assert_eq!(l.role(&l.visual_runs[0].glyphs[1 - zero]), GlyphRole::Body);
        assert_eq!(l.hanging_glyphs().count(), 0, "x = {x}");
        assert_eq!(l.hanging_left(), 0.0, "x = {x}");
        assert_eq!(l.hanging_right(), 0.0, "x = {x}");
        assert_eq!(l.visual_width(), 100.0);
    }
}

#[test]
fn out4_a_trailing_zero_advance_glyph_is_trailing() {
    // Roles apply in order: trailing before zero-advance.
    let l = line_of(vec![at(0, 1, 0., 100.), at(1, 2, 100., 0.)], 2, 1);
    assert_eq!(l.role(&l.visual_runs[0].glyphs[1]), GlyphRole::Trailing);
}

#[test]
fn out4_hanging_glyphs_are_exactly_the_glyphs_with_the_hanging_role() {
    // Left mark, body, zero-advance, right mark, trailing whitespace.
    let l = line_of(
        vec![
            at(0, 1, -3., 6.),
            at(1, 2, 3., 94.),
            at(2, 3, 97., 0.),
            at(3, 4, 97., 6.),
            at(4, 5, 103., 8.),
        ],
        5,
        4,
    );
    let c = LineContext {
        whitespace: HangingWhitespaceMode::Preserved,
        ..both_sides()
    };
    assert_eq!(l.validate(&c), Ok(()));
    let all: Vec<&PositionedGlyph> = l.visual_runs.iter().flat_map(|r| &r.glyphs).collect();
    let roles: Vec<_> = all.iter().map(|g| l.role(g)).collect();
    assert_eq!(
        roles,
        [
            GlyphRole::Hanging,
            GlyphRole::Body,
            GlyphRole::ZeroAdvance,
            GlyphRole::Hanging,
            GlyphRole::Trailing,
        ]
    );
    let by_role: Vec<_> = all
        .iter()
        .filter(|g| l.role(g) == GlyphRole::Hanging)
        .map(|g| g.byte_range)
        .collect();
    let reported: Vec<_> = l.hanging_glyphs().map(|g| g.byte_range).collect();
    assert_eq!(reported, by_role);
    assert_eq!(reported.len(), 2);
    assert_eq!(l.hanging_left(), 3.0);
    assert_eq!(l.hanging_right(), 3.0);
}

#[test]
fn out4_the_hanging_extents_ignore_overflow_within_the_tolerance() {
    // A real mark on the right and a sub-tolerance overflow on the left.
    let l = line_of(
        vec![
            at(0, 1, -WITHIN_TOLERANCE, 97. + WITHIN_TOLERANCE),
            at(1, 2, 97., 6.),
        ],
        2,
        2,
    );
    assert_eq!(l.validate(&force_end()), Ok(()));
    assert_eq!(l.hanging_left(), 0.0);
    assert_eq!(l.hanging_right(), 3.0);
    assert_eq!(l.hanging_glyphs().count(), 1);
}

#[test]
fn out4_tolerance_is_one_per_line_at_the_scale_of_its_body_width() {
    // A narrow outer slot [0, 50] and a wide one [100, 2100]: the body is
    // about 2050 wide, so the tolerance is about 0.02 for the whole line. A
    // glyph overflowing the narrow slot's outer edge by 0.01 is within it.
    let mut l = line();
    l.byte_range = TextRange::new(0, 2);
    l.visible_byte_end = 2;
    l.slots = smallvec![slot(0., 50., 50.), slot(100., 2000., 2000.)];
    let mut r = run(at(0, 1, -0.01, 50.01));
    r.byte_range = l.byte_range;
    r.glyphs.push(at(1, 2, 100., 2000.));
    l.visual_runs = vec![r];
    let c = LineContext {
        wrap_width: 2100.,
        ..ctx()
    };
    assert_eq!(l.validate(&c), Ok(()));
    assert_eq!(l.role(&l.visual_runs[0].glyphs[0]), GlyphRole::Body);
    assert_eq!(l.hanging_glyphs().count(), 0);
    assert_eq!(l.hanging_left(), 0.0);
    assert_eq!(l.hanging_right(), 0.0);
}
