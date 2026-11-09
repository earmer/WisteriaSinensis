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
    set_visual_width(&mut l, 14.);
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
        x: 0.,
        width: 10.,
        bidi_level: 0,
    }];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.atomic_fragments[0].width = f32::NAN;
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

fn one_glyph_line() -> PositionedLine {
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 10., false))];
    l
}

#[test]
fn inline_box_alignment_and_bytes_are_checked_per_box() {
    let object = PositionedInlineBox {
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(0, 1),
        x: 20.,
        y: 0.,
        width: 10.,
        ascent: 5.,
        descent: 1.,
        overflow_policy: BoxOverflowPolicy::Clip,
    };
    let mut l = one_glyph_line();
    l.visual_boxes = vec![object];
    assert_eq!(l.validate(&ctx()), Ok(()));
    let invalid = Err(LineInvariantError::InvalidInlineBox { index: 1 });
    l.visual_boxes = vec![
        object,
        PositionedInlineBox {
            baseline_align: BaselineAlignment::Offset(f32::NAN),
            ..object
        },
    ];
    assert_eq!(l.validate(&ctx()), invalid);
    l.visual_boxes[1] = PositionedInlineBox {
        byte_range: TextRange::new(0, 2),
        ..object
    };
    assert_eq!(l.validate(&ctx()), invalid);
}

#[test]
fn non_synthesized_glyphs_own_bytes_inside_the_line() {
    let mut l = one_glyph_line();
    l.visual_runs[0].glyphs.push(glyph(10., 10., false));
    assert_eq!(l.validate(&ctx()), Ok(()));
    let invalid = Err(LineInvariantError::InvalidGlyphSource { run: 0, glyph: 1 });
    l.visual_runs[0].glyphs[1].byte_range = Some(TextRange::empty_at(0));
    assert_eq!(l.validate(&ctx()), invalid);
    l.visual_runs[0].glyphs[1].byte_range = Some(TextRange::new(0, 2));
    assert_eq!(l.validate(&ctx()), invalid);
    // A synthesized glyph carries no range at all.
    l.visual_runs[0].glyphs[1].byte_range = None;
    assert_eq!(l.validate(&ctx()), invalid);
    l.visual_runs[0].glyphs[1].is_synthesized = true;
    assert_eq!(l.validate(&ctx()), Ok(()));
}

#[test]
fn visual_runs_hold_glyphs_and_stay_inside_the_line() {
    let mut l = one_glyph_line();
    l.visual_runs.push(run(glyph(10., 10., false)));
    assert_eq!(l.validate(&ctx()), Ok(()));
    let invalid = Err(LineInvariantError::InvalidVisualRun { run: 1 });
    l.visual_runs[1].glyphs.clear();
    assert_eq!(l.validate(&ctx()), invalid);
    l.visual_runs[1] = run(glyph(10., 10., false));
    l.visual_runs[1].byte_range = TextRange::new(0, 2);
    assert_eq!(l.validate(&ctx()), invalid);
}

#[test]
fn tabs_own_non_empty_bytes_inside_the_line() {
    let tab = PositionedTab {
        byte_range: TextRange::new(0, 1),
        x: 10.,
        width: 8.,
    };
    let mut l = one_glyph_line();
    l.tabs = vec![tab];
    assert_eq!(l.validate(&ctx()), Ok(()));
    for byte_range in [TextRange::empty_at(0), TextRange::new(0, 2)] {
        l.tabs = vec![PositionedTab { byte_range, ..tab }];
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::InvalidTab { index: 0 })
        );
    }
}

#[test]
fn ellipsis_only_line_rejects_every_body_entity() {
    let mut l = line();
    l.byte_range = TextRange::empty_at(0);
    l.visible_byte_end = 0;
    l.is_ellipsized = true;
    let mut ellipsis = glyph(0., 10., false);
    ellipsis.is_synthesized = true;
    ellipsis.byte_range = None;
    let mut r = run(glyph(0., 10., false));
    r.byte_range = TextRange::empty_at(0);
    r.glyphs = vec![ellipsis];
    l.visual_runs = vec![r];
    assert_eq!(l.validate(&ctx()), Ok(()));
    let mut body = l.clone();
    body.visual_runs[0].glyphs.push(glyph(10., 10., false));
    assert_eq!(
        body.validate(&ctx()),
        Err(LineInvariantError::InvalidGlyphSource { run: 0, glyph: 1 })
    );
    let mut tabbed = l;
    tabbed.tabs = vec![PositionedTab {
        byte_range: TextRange::empty_at(0),
        x: 0.,
        width: 0.,
    }];
    assert_eq!(
        tabbed.validate(&ctx()),
        Err(LineInvariantError::EmptyLineHasTabs)
    );
}

#[test]
fn a_slot_holding_only_a_tab_is_occupied() {
    let mut l = line();
    set_visual_width(&mut l, 0.);
    l.tabs = vec![PositionedTab {
        byte_range: TextRange::new(0, 1),
        x: 0.,
        width: 0.,
    }];
    assert_eq!(l.validate(&ctx()), Ok(()));
    // A trailing tab does not occupy the slot.
    l.visible_byte_end = 0;
    l.tabs[0].byte_range = TextRange::new(0, 1);
    assert_eq!(l.validate(&ctx()), Err(LineInvariantError::InvalidSlots));
}

#[test]
fn occupancy_and_containment_share_one_tolerance() {
    // The mark sits 1e-4 before the slot: inside by tolerance, hence occupying.
    let mut l = line();
    set_visual_width(&mut l, 0.);
    l.visual_runs = vec![run(glyph(-1e-4, 0., false))];
    assert_eq!(l.validate(&ctx()), Ok(()));
    // Beyond the tolerance the slot stays empty although a glyph is present.
    l.visual_runs = vec![run(glyph(-1e-2, 0., false))];
    assert_eq!(l.validate(&ctx()), Err(LineInvariantError::InvalidSlots));
}

#[test]
fn empty_slots_are_invalid() {
    let mut l = one_glyph_line();
    l.slots.clear();
    assert_eq!(l.validate(&ctx()), Err(LineInvariantError::InvalidSlots));
}
