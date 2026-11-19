//! What a slot's content block holds when glyphs hang (OUT-4, SLT-3, HNG-1):
//! the inside part of a hanging glyph counts for the slot it hangs from, and
//! occupancy is decided by the content blocks and the tolerance alone.
//! Everything is judged through `PositionedLine::validate`.

use super::common::*;
use mutil_engine::*;
use smallvec::smallvec;

const WIDTH_NOT_CONTENT_SPAN: Result<(), LineInvariantError> =
    Err(LineInvariantError::InvalidSlots {
        reason: SlotErrorReason::WidthNotContentSpan,
    });

/// A glyph from source bytes `start..end`, pen at `x`, `advance` wide.
fn at(start: usize, end: usize, x: f32, advance: f32) -> PositionedGlyph {
    PositionedGlyph {
        byte_range: Some(TextRange::new(start, end)),
        ..glyph(x, advance)
    }
}

/// `l` over bytes `0..glyphs.len()`, all glyphs in one run.
fn with_glyphs(mut l: PositionedLine, glyphs: Vec<PositionedGlyph>) -> PositionedLine {
    l.byte_range = TextRange::new(0, glyphs.len());
    l.visible_byte_end = glyphs.len();
    let mut r = run(glyphs[0]);
    r.byte_range = l.byte_range;
    r.glyphs = glyphs;
    l.visual_runs = vec![r];
    l
}

fn hanging(start: HangStartMode, end: HangEndMode, wrap_width: f32) -> LineContext {
    LineContext {
        hang_mode: HangMode { start, end },
        wrap_width,
        ..ctx()
    }
}

#[test]
fn out4_a_straddling_hanging_glyph_counts_only_through_its_inside_part() {
    // A body glyph over [0, 97] and a mark over [97, 106]. The slot is wider
    // than its content (200), so the block is exactly the content: whatever
    // part of the mark lies inside the block is content, the rest hangs.
    let build = |visual_width: f32| {
        let mut l = line();
        l.slots = smallvec![slot(0., visual_width, 200.)];
        with_glyphs(l, vec![at(0, 1, 0., 97.), at(1, 2, 97., 9.)])
    };
    let c = hanging(HangStartMode::None, HangEndMode::Force, 200.);
    // 100 leaves 3 units of the mark inside and 6 hanging; at 97 the mark is
    // wholly beyond (HNG-1); at 106 nothing overflows.
    for visual_width in [97., 100., 103., 106.] {
        assert_eq!(build(visual_width).validate(&c), Ok(()), "{visual_width}");
    }
    // A block cannot reach past the mark's pen end, where the content ends.
    for visual_width in [106.5, 110.] {
        assert_eq!(
            build(visual_width).validate(&c),
            WIDTH_NOT_CONTENT_SPAN,
            "{visual_width}"
        );
    }
    // Nor can it stop short of the body glyph.
    assert!(build(96.).validate(&c).is_err());
}

#[test]
fn out4_a_straddling_glyph_on_the_left_counts_only_through_its_inside_part() {
    // A mark over [-6, 3] straddling the left edge at 0, then a body glyph
    // over [3, 100].
    let build = |visual_width: f32| {
        let mut l = line();
        l.slots = smallvec![slot(0., visual_width, 200.)];
        with_glyphs(l, vec![at(0, 1, -6., 9.), at(1, 2, 3., 97.)])
    };
    let c = hanging(HangStartMode::AllLines, HangEndMode::None, 200.);
    assert_eq!(build(100.).validate(&c), Ok(()));
    // The line-end edge is exact: the block ends at the body glyph's end.
    assert_eq!(build(106.).validate(&c), WIDTH_NOT_CONTENT_SPAN);
}

#[test]
fn out4_a_straddling_glyph_counts_for_the_outer_slot_it_hangs_from() {
    // Two slots [0, 100] and [150, 300] in a 300-wide column: a mark over
    // [247, 256] straddles the end of the second slot's block [150, 250] and
    // hangs from it. Its inside part is that slot's content, the first slot
    // is unaffected.
    let build = |second_width: f32| {
        let mut l = line();
        l.slots = smallvec![slot(0., 100., 100.), slot(150., second_width, 150.)];
        with_glyphs(
            l,
            vec![at(0, 1, 0., 100.), at(1, 2, 150., 97.), at(2, 3, 247., 9.)],
        )
    };
    let c = hanging(HangStartMode::None, HangEndMode::Force, 300.);
    assert_eq!(build(100.).validate(&c), Ok(()));
    // A block reaching past the mark's pen end holds no content there.
    assert_eq!(build(120.).validate(&c), WIDTH_NOT_CONTENT_SPAN);
}

#[test]
fn out4_a_sliver_at_an_earlier_slots_edge_is_credited_to_the_slot_it_lies_in() {
    // Slot A is [0, 100] and full. Slot B is zero-width and starts 0.0005
    // after A's edge, within the tolerance (OUT-2). A sliver of a glyph lies
    // in B's block: B holds it, so B is occupied and is the last slot of the
    // body. A mark beyond B's block, in B's constraint, hangs from B.
    let build = |with_sliver: bool| {
        let mut l = line();
        l.slots = smallvec![slot(0., 100., 100.), slot(100.0005, 0., 49.9995)];
        let mut glyphs = vec![at(0, 1, 0., 100.)];
        if with_sliver {
            glyphs.push(at(glyphs.len(), glyphs.len() + 1, 100.0005, 0.0003));
        }
        let next = glyphs.len();
        glyphs.push(at(next, next + 1, 101., 6.));
        let l = with_glyphs(l, glyphs);
        let c = hanging(HangStartMode::None, HangEndMode::Force, 150.);
        (l, c)
    };
    let (l, c) = build(true);
    assert_eq!(l.validate(&c), Ok(()));
    // Without the sliver B holds nothing: the mark then lies in an unused
    // slot's constraint, and no unused slot holds a hanging glyph.
    let (l, c) = build(false);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 1 })
    );
}

#[test]
fn out4_a_glyph_within_tolerance_of_a_zero_width_slot_is_held_and_occupies_it() {
    // Slot A is zero-width at 10 inside [0, 50]; slot B is [60, 160] and
    // full. A mark over [4, 10] lies left of A's block: it hangs from A when
    // A is occupied, which the 0.0005-wide glyph at A's block makes it.
    let build = |with_sliver: bool| {
        let mut l = line();
        l.slots = smallvec![slot(10., 0., 40.), slot(60., 100., 100.)];
        let mut glyphs = vec![at(0, 1, 4., 6.)];
        if with_sliver {
            glyphs.push(at(1, 2, 10., 0.0005));
        }
        let next = glyphs.len();
        glyphs.push(at(next, next + 1, 60., 100.));
        let l = with_glyphs(l, glyphs);
        let c = hanging(HangStartMode::AllLines, HangEndMode::None, 160.);
        (l, c)
    };
    let (l, c) = build(true);
    assert_eq!(l.validate(&c), Ok(()));
    // Without it A is unused: the mark lies in an unused slot.
    let (l, c) = build(false);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
}

#[test]
fn hng1_a_glyph_wholly_beyond_the_outer_edge_hangs_under_force_subject_to_hng2() {
    // The mark has no part inside the block: the part that is not overflow
    // is empty, so the condition holds vacuously (HNG-1).
    let right = || {
        let mut l = line();
        l.slots = smallvec![slot(0., 100., 100.)];
        with_glyphs(l, vec![at(0, 1, 0., 100.), at(1, 2, 105., 10.)])
    };
    let left = || {
        let mut l = line();
        l.slots = smallvec![slot(0., 100., 100.)];
        with_glyphs(l, vec![at(0, 1, -20., 10.), at(1, 2, 0., 100.)])
    };
    let force = hanging(HangStartMode::AllLines, HangEndMode::Force, 100.);
    assert_eq!(right().validate(&force), Ok(()));
    assert_eq!(left().validate(&force), Ok(()));
    // The hang mode still governs (HNG-2): the side must allow it.
    let off = hanging(HangStartMode::None, HangEndMode::None, 100.);
    assert_eq!(
        right().validate(&off),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    assert_eq!(
        left().validate(&off),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Left
        })
    );
    // Under `Allow` a mark hangs only if it would otherwise wrap: this slot
    // is exactly as wide as its body, so it needed to.
    assert_eq!(
        right().validate(&hanging(HangStartMode::None, HangEndMode::Allow, 100.)),
        Ok(())
    );
    // And it is the roles that say so.
    assert_eq!(right().hanging_glyphs().count(), 1);
    assert_eq!(left().hanging_glyphs().count(), 1);
}
