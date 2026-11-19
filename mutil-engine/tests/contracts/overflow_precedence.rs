//! The hanging errors come before the overflow errors of SLT-5, in the order
//! of the spec's error table (HNG-1 to HNG-4 rows, then the SLT-5 rows).
//! `LineWiderThanWrapWidth` names a body wider than the container,
//! `SlotOverflow` a block beyond its own constraint while the line still
//! fits; once hanging is allowed the overflow is what is reported.

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

fn hanging_end(end: HangEndMode, ctx: LineContext) -> LineContext {
    LineContext {
        hang_mode: HangMode {
            start: HangStartMode::None,
            end,
        },
        ..ctx
    }
}

/// A divisible one-slot line in a 100-wide column: two 60-unit glyphs in a
/// block declared 120 wide, and a 6-unit mark just beyond it.
fn wider_than_wrap_width_with_a_mark() -> PositionedLine {
    let mut l = line();
    l.slots = single_slot(120., 100.);
    with_glyphs(
        l,
        vec![at(0, 1, 0., 60.), at(1, 2, 60., 60.), at(2, 3, 120., 6.)],
    )
}

#[test]
fn slt5_a_hanging_violation_is_reported_before_a_line_wider_than_wrap_width() {
    let wider = Err(LineInvariantError::LineWiderThanWrapWidth {
        visual_width: 120.,
        wrap_width: 100.,
    });
    let l = wider_than_wrap_width_with_a_mark();
    // Hanging is off: the hanging rows of the error table come first.
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    // Where hanging is allowed the overflow stands.
    assert_eq!(l.validate(&hanging_end(HangEndMode::Force, ctx())), wider);

    // The same mark on a body that fits is what the hanging rule rejects.
    let mut fits = line();
    fits.slots = single_slot(100., 100.);
    let fits = with_glyphs(
        fits,
        vec![at(0, 1, 0., 50.), at(1, 2, 50., 50.), at(2, 3, 100., 6.)],
    );
    assert_eq!(
        fits.validate(&ctx()),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    assert_eq!(
        fits.validate(&hanging_end(HangEndMode::Force, ctx())),
        Ok(())
    );
}

/// Two slots `[0, 100]` and `[150, 250]` in a 250-wide column, the first
/// holding `first` units of content and the second 100, then a 6-unit mark
/// just beyond the line's right edge.
fn two_slots_with_a_mark(first: f32) -> (PositionedLine, LineContext) {
    let mut l = line();
    l.slots = smallvec![slot(0., first, 100.), slot(150., 100., 100.)];
    let l = with_glyphs(
        l,
        vec![
            at(0, 1, 0., first),
            at(1, 2, 150., 100.),
            at(2, 3, 250., 6.),
        ],
    );
    let c = LineContext {
        wrap_width: 250.,
        ..ctx()
    };
    (l, c)
}

#[test]
fn slt5_a_hanging_violation_is_reported_before_a_slot_overflow() {
    // The control: with the first block inside its constraint, the mark past
    // the right edge is the hanging rule's to reject.
    let (fits, c) = two_slots_with_a_mark(100.);
    assert_eq!(
        fits.validate(&c),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    assert_eq!(fits.validate(&hanging_end(HangEndMode::Force, c)), Ok(()));

    // The first block ends 10 units beyond its constraint, the line as a
    // whole still fits: with hanging off the mark's error comes first, and
    // once hanging is allowed the slot is named.
    let (l, c) = two_slots_with_a_mark(110.);
    let overflow = Err(LineInvariantError::SlotOverflow {
        slot: 0,
        extent: 110.,
        capacity: 100.,
    });
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::HangingNotAllowed {
            side: HangSide::Right
        })
    );
    assert_eq!(l.validate(&hanging_end(HangEndMode::Force, c)), overflow);
}

#[test]
fn slt5_the_container_is_reported_when_both_it_and_a_slot_overflow() {
    // The first block is 160 wide in a 100-wide slot, the second 100: the
    // body is 260 wide in a 250-wide container. `SlotOverflow` is for a line
    // that fits `wrap_width`, so the container is what is named.
    let mut l = line();
    l.slots = smallvec![slot(0., 160., 100.), slot(150., 100., 100.)];
    let l = with_glyphs(
        l,
        vec![at(0, 1, 0., 80.), at(1, 2, 80., 80.), at(2, 3, 150., 100.)],
    );
    let c = LineContext {
        wrap_width: 250.,
        ..ctx()
    };
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 260.,
            wrap_width: 250.
        })
    );
}
