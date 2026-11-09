//! `PositionedLine::validate`: glyph containment, hanging, tabs, slots.

use crate::common::*;
use mutil_engine::*;

#[test]
fn reject_negative_line_height() {
    let mut l = line();
    l.ascent = -8.;
    l.descent = -2.;
    l.line_height = -10.;
    let mut c = ctx();
    c.strut.ascent = -8.;
    c.strut.descent = -2.;
    assert!(l.validate(&c).is_err());
}

#[test]
fn tab_stop_advances_at_large_coordinates() {
    let x = 1e10_f32;
    let next = TabStops { step: 8. }.next_stop(x);
    assert!(
        next.is_none_or(|next| next > x),
        "{x} -> {next:?}; unchanged input cannot signal termination"
    );
    assert!(next.is_none_or(f32::is_finite));
}

#[test]
fn partial_hanging_counts_only_overflow() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(96., 10., true))];
    l.hanging_right = 6.;
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert_eq!(l.validate(&c), Ok(()));
    l.hanging_right = 10.;
    assert!(l.validate(&c).is_err());
}

#[test]
fn nbsp_remains_visible_in_collapsible_output() {
    // Output-contract control only: no layout algorithm exists to test actual folding yet.
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visible_byte_end = 3;
    l.visual_width = 14.;
    let mut nbsp = glyph(10., 4., false);
    nbsp.byte_range = Some(TextRange::new(1, 3));
    l.visual_runs = vec![run(glyph(0., 10., false)), run(nbsp)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    assert_eq!(l.visual_runs[1].glyphs[0].advance, 4.);
}

#[test]
fn malformed_atomic_fragments_are_rejected() {
    let mut l = line();
    l.atomic_fragments = vec![AtomicFragment {
        atomic_id: 1,
        full_byte_range: TextRange::new(0, 1),
        fragment_byte_range: TextRange::new(0, 1),
        x_start: 0.,
        x_end: 10.,
        bidi_level: 0,
    }];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.atomic_fragments[0].x_end = f32::NAN;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidAtomicFragment { index: 0 })
    );
}

#[test]
fn invalid_visual_run_font_instance_is_rejected() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 10., false))];
    l.visual_runs[0].font_size = 0.;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidVisualRun { run: 0 })
    );
    l.visual_runs[0].font_size = 16.;
    l.visual_runs[0]
        .variations
        .push(FontVariation::new(*b"wght", f32::INFINITY));
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidVisualRun { run: 0 })
    );
}

#[test]
fn non_positive_line_height_is_named() {
    let mut l = line();
    l.line_height = 0.;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::NonPositiveLineHeight {
            expected: 10.,
            actual: 0.
        })
    );
}

#[test]
fn false_hanging_flag_inside_body_is_rejected() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(90., 5., true))];
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert!(l.validate(&c).is_err());
}

#[test]
fn negative_advance_partial_hang_uses_normalized_envelope() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(106., -10., true))];
    l.hanging_right = 6.;
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert_eq!(l.validate(&c), Ok(()));
}

#[test]
fn left_partial_hanging_counts_only_overflow() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(-6., 10., true))];
    l.hanging_left = 6.;
    let mut c = ctx();
    c.hang_mode.start = HangStartMode::AllLines;
    assert_eq!(l.validate(&c), Ok(()));
}

#[test]
fn hanging_requires_overflow_beyond_tolerance() {
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    // A non-zero-advance hanging glyph inside the block is still rejected.
    let mut l = line();
    l.visual_runs = vec![run(glyph(99., 1.00001, true))];
    assert!(l.validate(&c).is_err());
    // Zero-advance marks are exempt from containment and hanging.
    let mut l = line();
    l.visual_runs = vec![run(glyph(50., 0., true))];
    assert_eq!(l.validate(&c), Ok(()));
    // But they cannot fake a declared hanging extent.
    let mut l = line();
    l.hanging_right = 5.;
    l.visual_runs = vec![run(glyph(99., 0., true))];
    assert!(l.validate(&c).is_err());
}

#[test]
fn multi_slot_containment_and_conserved_width() {
    let (valid, c) = two_slots();
    assert_eq!(valid.validate(&c), Ok(()));
    let mut bad = valid.clone();
    bad.visual_runs[0].glyphs[0].advance = 151.0;
    assert!(bad.validate(&c).is_err());
    let mut bad = valid.clone();
    bad.slots[0].visual_width = 151.0;
    bad.visual_width = 251.0;
    assert!(bad.validate(&c).is_err());
    let mut bad = valid.clone();
    bad.visual_width += 1.0;
    assert_eq!(bad.validate(&c), Err(LineInvariantError::InvalidSlots));
    let mut bad = valid.clone();
    bad.visual_runs.clear();
    bad.visual_boxes.push(PositionedInlineBox {
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(0, 1),
        x: 90.0,
        y: 0.0,
        width: 70.0,
        ascent: 8.0,
        descent: 2.0,
        overflow_policy: BoxOverflowPolicy::Clip,
    });
    assert!(bad.validate(&c).is_err());
    let mut bad = valid;
    bad.span_fragments.push(PositionedSpanFragment {
        span_id: 1,
        byte_range: TextRange::new(0, 2),
        x: 90.0,
        width: 70.0,
        ascent: 8.0,
        descent: 2.0,
        bidi_level: 0,
        is_first_fragment: true,
        is_last_fragment: true,
    });
    assert!(bad.validate(&c).is_err());
}

#[test]
fn empty_slot_hanging_and_trailing_whitespace() {
    let (mut l, mut c) = two_slots();
    l.visual_runs[0].glyphs.remove(0);
    l.slots[0].visual_width = 0.0;
    l.visual_width = 100.0;
    c.hang_mode.start = HangStartMode::AllLines;
    assert_eq!(l.validate(&c), Ok(()));
    let mut hanging = glyph(145.0, 5.0, true);
    hanging.byte_range = Some(TextRange::new(0, 1));
    l.visual_runs[0].glyphs.push(hanging);
    l.hanging_left = 5.0;
    assert!(l.validate(&c).is_err());
    l.visual_runs[0].glyphs.pop();
    l.hanging_left = 0.0;
    let trailing = PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(250.0, 5.0, false)
    };
    l.byte_range.end = 3;
    l.visual_runs[0].glyphs.push(trailing);
    c.whitespace = HangingWhitespaceMode::Preserved;
    assert_eq!(l.validate(&c), Ok(()));
    l.visual_runs[0].glyphs.last_mut().unwrap().x = 145.0;
    assert!(l.validate(&c).is_err());
}
