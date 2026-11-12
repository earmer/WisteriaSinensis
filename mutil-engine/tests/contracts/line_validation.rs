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
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::NonPositiveLineHeight {
            expected: -10.0,
            actual: -10.0
        })
    );
}

#[test]
fn tab_stop_advances_at_large_coordinates() {
    let x = 1e10_f32;
    let next = TabStops::DEFAULT.next_stop(x);
    assert!(
        next.is_none_or(|next| next > x),
        "{x} -> {next:?}; unchanged input cannot signal termination"
    );
    assert!(next.is_none_or(f32::is_finite));
}

#[test]
fn partial_hanging_counts_only_overflow() {
    let mut l = line();
    // The body glyph and the 4 units of the hanging mark inside the body span
    // the slot; the mark's last 6 units hang.
    l.visual_runs = vec![run(glyph(0., 96., false)), run(glyph(96., 10., true))];
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert_eq!(l.validate(&c), Ok(()));
    assert_eq!(l.hanging_right(), 6.0);
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
    l.visual_runs = vec![run(glyph(0., 100., false))];
    l.atomic_fragments = vec![AtomicFragment {
        atomic_id: 1,
        full_byte_range: TextRange::new(0, 1),
        fragment_byte_range: TextRange::new(0, 1),
        x: 0.,
        width: 10.,
        bidi_level: BidiLevel::LTR,
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
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 0 })
    );
}

#[test]
fn negative_advance_partial_hang_uses_normalized_envelope() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 96., false)), run(glyph(106., -10., true))];
    let mut c = ctx();
    c.hang_mode.end = HangEndMode::Allow;
    assert_eq!(l.validate(&c), Ok(()));
}

#[test]
fn left_partial_hanging_counts_only_overflow() {
    let mut l = line();
    l.visual_runs = vec![run(glyph(-6., 10., true)), run(glyph(4., 96., false))];
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
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 0 })
    );
    // Zero-advance marks are exempt from containment and hanging.
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 100., false)), run(glyph(50., 0., true))];
    assert_eq!(l.validate(&c), Ok(()));
    // Nor do they add a hanging extent.
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 100., false)), run(glyph(99., 0., true))];
    assert_eq!(l.validate(&c), Ok(()));
    assert_eq!(l.hanging_right(), 0.0);
}

#[test]
fn multi_slot_containment_and_conserved_width() {
    let (valid, c) = two_slots();
    assert_eq!(valid.validate(&c), Ok(()));
    let mut bad = valid.clone();
    bad.visual_runs[0].glyphs[0].advance = 151.0;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    let mut bad = valid.clone();
    // A slot's content may not run on into the obstacle after it.
    bad.slots[0].visual_width = 151.0;
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 251.0,
            wrap_width: 250.0
        })
    );
    let mut bad = valid.clone();
    bad.visual_runs.clear();
    bad.visual_boxes.push(PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
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
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::InlineBoxOutsideBlock { index: 0 })
    );
    let mut bad = valid;
    bad.span_fragments.push(PositionedSpanFragment {
        span_id: 1,
        span_range: TextRange::new(0, 2),
        byte_range: TextRange::new(0, 2),
        x: 90.0,
        width: 70.0,
        ascent: 8.0,
        descent: 2.0,
        bidi_level: BidiLevel::LTR,
    });
    assert_eq!(
        bad.validate(&c),
        Err(LineInvariantError::SpanFragmentOutsideBlock { index: 0 })
    );
}

#[test]
fn empty_slot_hanging_and_trailing_whitespace() {
    let (mut l, mut c) = two_slots();
    l.visual_runs[0].glyphs.remove(0);
    l.slots[0].visual_width = 0.0;
    c.hang_mode.start = HangStartMode::AllLines;
    assert_eq!(l.validate(&c), Ok(()));
    let mut hanging = glyph(145.0, 5.0, true);
    hanging.byte_range = Some(TextRange::new(0, 1));
    l.visual_runs[0].glyphs.insert(0, hanging);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    l.visual_runs[0].glyphs.remove(0);
    let trailing = PositionedGlyph {
        byte_range: Some(TextRange::new(2, 3)),
        ..glyph(250.0, 5.0, false)
    };
    l.byte_range = TextRange::new(l.byte_range.start(), 3);
    l.visual_runs[0].glyphs.push(trailing);
    c.whitespace = HangingWhitespaceMode::Preserved;
    assert_eq!(l.validate(&c), Ok(()));
    // In the obstacle before the occupied slot (and so first on screen).
    l.visual_runs[0].glyphs.last_mut().unwrap().x = 145.0;
    l.visual_runs[0].glyphs.swap(0, 1);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::TrailingWhitespaceOnWrongSide { run: 0, glyph: 0 })
    );
}

fn one_glyph_line() -> PositionedLine {
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 100., false))];
    l
}

#[test]
fn inline_box_alignment_and_bytes_are_checked_per_box() {
    let object = PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
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
    ellipsis.byte_range = None;
    let mut r = run(glyph(0., 10., false));
    r.byte_range = TextRange::empty_at(0);
    r.glyphs = vec![ellipsis];
    l.visual_runs = vec![r];
    set_visual_width(&mut l, 10.);
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
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::ContentOutsideSlots
        })
    );
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
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::ContentOutsideSlots
        })
    );
}

#[test]
fn empty_slots_are_invalid() {
    let mut l = one_glyph_line();
    l.slots.clear();
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::NoSlots
        })
    );
}

/// A one-slot line whose slot starts at `start` with capacity `capacity`, holding
/// one glyph of `width`; the context has wrap width `wrap_width`.
fn slot_line(
    start: f32,
    capacity: f32,
    width: f32,
    wrap_width: f32,
) -> (PositionedLine, LineContext) {
    let mut l = line();
    l.slots = smallvec::smallvec![slot(start, width, capacity)];
    l.visual_runs = vec![run(glyph(start, width, false))];
    let c = LineContext {
        wrap_width,
        ..ctx()
    };
    (l, c)
}

#[test]
fn context_slots_must_lie_inside_the_wrap_envelope() {
    let invalid = Err(LineInvariantError::InvalidSlots {
        reason: SlotErrorReason::InvalidConstraint,
    });
    for (start, capacity, wrap_width) in
        [(0., 100., 100.), (10., 90., 100.), (5., 50., f32::INFINITY)]
    {
        let (l, c) = slot_line(start, capacity, 50., wrap_width);
        assert_eq!(l.validate(&c), Ok(()), "{start} {capacity} {wrap_width}");
    }
    // Starts before 0, or ends beyond a finite wrap width.
    for (start, capacity, wrap_width) in [
        (-10., 100., 100.),
        (0., 150., 100.),
        (50., 100., 100.),
        (-1., 100., f32::INFINITY),
    ] {
        let (l, c) = slot_line(start, capacity, 50., wrap_width);
        assert_eq!(l.validate(&c), invalid, "{start} {capacity} {wrap_width}");
    }
    // An unbounded slot needs an unbounded container.
    let (l, mut c) = slot_line(0., f32::INFINITY, 50., 100.);
    assert_eq!(l.validate(&c), invalid);
    c.wrap_width = f32::INFINITY;
    assert_eq!(l.validate(&c), Ok(()));
}

#[test]
fn slot_overflow_names_the_slot_and_its_capacity() {
    // Two slots under a 250-wide container: the first holds 110 units of
    // content in a 100-unit slot, but the line as a whole still fits.
    let (mut l, c) = two_slots();
    l.visual_runs[0].glyphs[0].advance = 110.;
    l.slots[0].visual_width = 110.;
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::SlotOverflow {
            slot: 0,
            extent: 110.,
            capacity: 100.
        })
    );
    // The same for the second slot.
    let (mut l, c) = two_slots();
    l.visual_runs[0].glyphs[1].advance = 105.;
    l.slots[1].visual_width = 105.;
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::SlotOverflow {
            slot: 1,
            extent: 105.,
            capacity: 100.
        })
    );
}

#[test]
fn line_wider_than_wrap_width_reports_the_container_not_the_slot() {
    // An indented slot has capacity 92 in a 100-wide container.
    let (l, c) = slot_line(8., 92., 95., 100.);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::SlotOverflow {
            slot: 0,
            extent: 95.,
            capacity: 92.
        })
    );
    // Wider than the container itself: the report is the container's width.
    let (l, c) = slot_line(8., 92., 105., 100.);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 105.,
            wrap_width: 100.
        })
    );
}

#[test]
fn rigid_boxes_are_compared_with_the_wrap_width_not_the_slot_capacity() {
    let object = |x: f32, width: f32| PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(0, 1),
        x,
        y: 0.,
        width,
        ascent: 5.,
        descent: 1.,
        overflow_policy: BoxOverflowPolicy::Clip,
    };
    // An indented 50-unit slot in a 200-wide container.
    let boxed = |x: f32, width: f32| {
        let (mut l, c) = slot_line(50., 50., width, 200.);
        l.visual_runs.clear();
        l.visual_boxes = vec![object(x, width)];
        (l, c)
    };
    // 60 units overrun the slot's own capacity but not the container: the slot
    // is what overflows.
    let (l, c) = boxed(50., 60.);
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::SlotOverflow {
            slot: 0,
            extent: 60.,
            capacity: 50.
        })
    );
    // Past the container's edge the box itself is too wide.
    let (l, c) = slot_line(50., 150., 160., 200.);
    let mut l = l;
    l.visual_runs.clear();
    l.visual_boxes = vec![object(50., 160.)];
    assert_eq!(
        l.validate(&c),
        Err(LineInvariantError::InlineBoxWiderThanWrapWidth { index: 0 })
    );
    // Within the slot and the container it is fine.
    let (l, c) = boxed(50., 50.);
    assert_eq!(l.validate(&c), Ok(()));
}

fn empty_line_with_metrics(ascent: f32, descent: f32) -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::empty_at(0);
    l.visible_byte_end = 0;
    set_visual_width(&mut l, 0.);
    l.ascent = ascent;
    l.descent = descent;
    l.line_height = 10.;
    l
}

#[test]
fn an_empty_line_keeps_the_fixed_strut_metrics() {
    // Strut 8 / 2, a fixed line box of 10.
    let fixed = LineContext {
        line_height: LineHeight::Fixed(1.),
        ..ctx()
    };
    assert_eq!(empty_line_with_metrics(8., 2.).validate(&fixed), Ok(()));
    assert_eq!(
        empty_line_with_metrics(9., 1.).validate(&fixed),
        Err(LineInvariantError::LineMetricsMismatch {
            expected_ascent: 8.,
            expected_descent: 2.,
            actual_ascent: 9.,
            actual_descent: 1.,
        })
    );
    // Under `Normal` only the line height is pinned, as before.
    assert_eq!(empty_line_with_metrics(9., 1.).validate(&ctx()), Ok(()));
}

#[test]
fn the_visible_end_sits_on_a_cluster_boundary() {
    // One glyph (one cluster) over bytes 0..3.
    let mut g = glyph(0., 100., false);
    g.byte_range = Some(TextRange::new(0, 3));
    let mut l = line();
    l.byte_range = TextRange::new(0, 3);
    l.visual_runs = vec![run(g)];
    l.visible_byte_end = 3;
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.visible_byte_end = 1;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::VisibleEndNotOnBoundary)
    );
    // Outside the line it is still the older error.
    l.visible_byte_end = 4;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::VisibleEndOutsideRange)
    );
    // The edges of a box or a tab on the line are boundaries too.
    let mut l = line();
    l.byte_range = TextRange::new(0, 5);
    let mut first = glyph(0., 20., false);
    first.byte_range = Some(TextRange::new(0, 2));
    l.visual_runs = vec![run(first)];
    l.visual_boxes = vec![PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(2, 4),
        x: 20.,
        y: 0.,
        width: 30.,
        ascent: 5.,
        descent: 1.,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    l.tabs = vec![PositionedTab {
        byte_range: TextRange::new(4, 5),
        x: 50.,
        width: 50.,
    }];
    for visible in [2, 4, 5] {
        l.visible_byte_end = visible;
        // A tab at or after the visible end is trailing whitespace: collapsed.
        let mut accepted = l.clone();
        if visible <= 4 {
            accepted.tabs[0].width = 0.;
            set_visual_width(&mut accepted, 50.);
        } else {
            set_visual_width(&mut accepted, 100.);
        }
        assert_eq!(accepted.validate(&ctx()), Ok(()), "visible end {visible}");
    }
    for visible in [1, 3] {
        l.visible_byte_end = visible;
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::VisibleEndNotOnBoundary),
            "visible end {visible}"
        );
    }
}

#[test]
fn a_slot_is_as_wide_as_the_content_placed_in_it() {
    let (valid, c) = two_slots();
    assert_eq!(valid.validate(&c), Ok(()));
    // The second slot's content is gone but its width is not.
    let mut empty_second = valid.clone();
    empty_second.visual_runs[0].glyphs.pop();
    assert_eq!(
        empty_second.validate(&c),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::WidthNotContentSpan
        })
    );
    // Content narrower than its slot's declared width.
    let mut narrow = valid.clone();
    narrow.visual_runs[0].glyphs[1].advance = 60.;
    assert_eq!(
        narrow.validate(&c),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::WidthNotContentSpan
        })
    );
    // The slot may be narrower than its capacity when its content is.
    narrow.slots[1].visual_width = 60.;
    assert_eq!(narrow.validate(&c), Ok(()));
}

/// A one-slot line `[0, 100]` holding one 60-wide glyph at `x`, with the
/// content block `[content_start, content_start + visual_width]`.
fn aligned_line(x: f32, content_start: f32, visual_width: f32) -> PositionedLine {
    let mut l = line();
    l.slots[0].content_start = content_start;
    l.slots[0].visual_width = visual_width;
    l.visual_runs = vec![run(glyph(x, 60., false))];
    l
}

#[test]
fn the_content_block_separates_alignment_offset_from_leading_glue() {
    let rtl = LineContext {
        base_direction: Direction::Rtl,
        ..ctx()
    };
    // End-aligned: the 40 units before the content are alignment offset.
    assert_eq!(aligned_line(40., 40., 60.).validate(&ctx()), Ok(()));
    // Centred.
    assert_eq!(aligned_line(20., 20., 60.).validate(&ctx()), Ok(()));
    // Preserved leading glue: the block starts 10 units before the first
    // glyph, and those 10 units are part of the line's width.
    assert_eq!(aligned_line(10., 0., 70.).validate(&ctx()), Ok(()));
    // The line-end edge is exact: the block ends where the last item does.
    let span = Err(LineInvariantError::InvalidSlots {
        reason: SlotErrorReason::WidthNotContentSpan,
    });
    assert_eq!(aligned_line(0., 0., 70.).validate(&ctx()), span);
    // In an RTL paragraph the line starts on the right: leading glue follows
    // the last item, and the block's left edge is the exact one.
    assert_eq!(aligned_line(0., 0., 70.).validate(&rtl), Ok(()));
    assert_eq!(aligned_line(10., 0., 70.).validate(&rtl), span);
    // An item outside the block is outside the slot's content.
    assert_eq!(
        aligned_line(10., 20., 60.).validate(&ctx()),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    // The block starts inside its constraint ...
    let mut before = aligned_line(-5., -5., 60.);
    before.slots[0].constraint =
        InlineSlotConstraint::new(0., before.slots[0].constraint.end()).unwrap();
    assert_eq!(
        before.validate(&ctx()),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::InvalidContent
        })
    );
    // ... and ends inside it: an offset can push fitting content out.
    assert_eq!(
        aligned_line(50., 50., 60.).validate(&ctx()),
        Err(LineInvariantError::SlotOverflow {
            slot: 0,
            extent: 110.,
            capacity: 100.
        })
    );
}

/// A full one-slot line over bytes `0..2`: glyphs `[0, 50]` and `[50, 100]`,
/// one byte each, in one run.
fn two_glyph_line() -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::new(0, 2);
    l.visible_byte_end = 2;
    let mut second = glyph(50., 50., false);
    second.byte_range = Some(TextRange::new(1, 2));
    let mut r = run(glyph(0., 50., false));
    r.byte_range = l.byte_range;
    r.glyphs.push(second);
    l.visual_runs = vec![r];
    l
}

#[test]
fn the_glyphs_of_a_run_are_in_screen_order() {
    let l = two_glyph_line();
    assert_eq!(l.validate(&ctx()), Ok(()));
    // A mark shares its base's pen position; its ink offset is not compared.
    let mut marked = l.clone();
    let mut mark = glyph(50., 0., false);
    mark.byte_range = Some(TextRange::new(1, 2));
    mark.x_offset = -30.;
    marked.visual_runs[0].glyphs.push(mark);
    assert_eq!(marked.validate(&ctx()), Ok(()));
    // A negative advance (kerning past zero) leaves the pen positions ordered.
    let mut kerned = l.clone();
    kerned.visual_runs[0].glyphs[1].x = 100.;
    kerned.visual_runs[0].glyphs[1].advance = -50.;
    assert_eq!(kerned.validate(&ctx()), Ok(()));
    let mut swapped = l;
    swapped.visual_runs[0].glyphs.swap(0, 1);
    assert_eq!(
        swapped.validate(&ctx()),
        Err(LineInvariantError::GlyphsOutOfVisualOrder { run: 0, glyph: 1 })
    );
}

fn caret(x: f32, offset: usize, level: BidiLevel) -> CaretStop {
    CaretStop {
        x,
        offset,
        affinity: CaretAffinity::Downstream,
        level,
        line: 0,
    }
}

#[test]
fn a_caret_stop_carries_the_level_of_a_run_owning_its_offset() {
    // "a" at level 0 in `[0, 50]`, "b" at level 1 in `[50, 100]`.
    let mut l = two_glyph_line();
    let mut rtl = l.visual_runs[0].clone();
    rtl.bidi_level = BidiLevel::RTL;
    rtl.byte_range = TextRange::new(1, 2);
    rtl.glyphs.remove(0);
    l.visual_runs[0].glyphs.pop();
    l.visual_runs[0].byte_range = TextRange::new(0, 1);
    l.visual_runs.push(rtl);
    let with = |stops: Vec<CaretStop>| {
        let mut l = l.clone();
        l.caret_stops = stops;
        l.validate(&ctx())
    };
    let invalid = |index| Err(LineInvariantError::InvalidCaretStop { index });
    // Offset 1 is the run boundary: either run's level, and both stops sit
    // at one x.
    assert_eq!(
        with(vec![
            caret(0., 0, BidiLevel::LTR),
            caret(50., 1, BidiLevel::LTR),
            caret(50., 1, BidiLevel::RTL),
            caret(50., 2, BidiLevel::RTL)
        ]),
        Ok(())
    );
    // Offset 0 belongs to the level-0 run only, offset 2 to the level-1 run.
    assert_eq!(with(vec![caret(0., 0, BidiLevel::RTL)]), invalid(0));
    assert_eq!(
        with(vec![
            caret(0., 0, BidiLevel::LTR),
            caret(50., 2, BidiLevel::LTR)
        ]),
        invalid(1)
    );
    // A level no run has is wrong even at the boundary.
    assert_eq!(
        with(vec![caret(50., 1, BidiLevel::new(2).unwrap())]),
        invalid(0)
    );

    // An inline box lends its own level to the stops touching it.
    let mut boxed = l.clone();
    boxed.visual_runs.pop();
    boxed.visual_boxes = vec![PositionedInlineBox {
        bidi_level: BidiLevel::RTL,
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(1, 2),
        x: 50.,
        y: 0.,
        width: 50.,
        ascent: 5.,
        descent: 1.,
        overflow_policy: BoxOverflowPolicy::Clip,
    }];
    boxed.caret_stops = vec![
        caret(50., 1, BidiLevel::RTL),
        caret(100., 2, BidiLevel::RTL),
    ];
    assert_eq!(boxed.validate(&ctx()), Ok(()));
    boxed.caret_stops = vec![caret(100., 2, BidiLevel::LTR)];
    assert_eq!(boxed.validate(&ctx()), invalid(0));
    // The stop of an empty line carries the paragraph level.
    let mut empty = line();
    empty.byte_range = TextRange::empty_at(0);
    empty.visible_byte_end = 0;
    set_visual_width(&mut empty, 0.);
    empty.caret_stops = vec![caret(0., 0, BidiLevel::LTR)];
    assert_eq!(empty.validate(&ctx()), Ok(()));
    empty.caret_stops = vec![caret(0., 0, BidiLevel::RTL)];
    assert_eq!(empty.validate(&ctx()), invalid(0));
}

/// A line whose body is `body` wide in a 100-unit slot, with a 5-unit mark
/// hanging off its right edge.
fn end_hanging_line(body: f32) -> PositionedLine {
    let mut l = two_glyph_line();
    set_visual_width(&mut l, body);
    l.visual_runs[0].glyphs[0].advance = body;
    l.visual_runs[0].glyphs[1] = PositionedGlyph {
        byte_range: Some(TextRange::new(1, 2)),
        ..glyph(body, 5., true)
    };
    l
}

#[test]
fn allow_end_hangs_a_mark_only_when_it_would_otherwise_wrap() {
    let hang = |end| LineContext {
        hang_mode: HangMode {
            start: HangStartMode::None,
            end,
        },
        ..ctx()
    };
    let unneeded = Err(LineInvariantError::HangingWithoutOverflow {
        side: HangSide::Right,
    });
    // 100 + 5 and 96 + 5 do not fit 100 units: the mark had to hang.
    assert_eq!(
        end_hanging_line(100.).validate(&hang(HangEndMode::Allow)),
        Ok(())
    );
    assert_eq!(
        end_hanging_line(96.).validate(&hang(HangEndMode::Allow)),
        Ok(())
    );
    // 94 + 5 and 60 + 5 fit: under `Allow` the mark belongs in the body.
    assert_eq!(
        end_hanging_line(94.).validate(&hang(HangEndMode::Allow)),
        unneeded
    );
    assert_eq!(
        end_hanging_line(60.).validate(&hang(HangEndMode::Allow)),
        unneeded
    );
    // `Force` hangs it regardless.
    assert_eq!(
        end_hanging_line(60.).validate(&hang(HangEndMode::Force)),
        Ok(())
    );
    // An unbounded line never needs to hang anything.
    let mut unbounded = end_hanging_line(100.);
    unbounded.slots[0].constraint =
        InlineSlotConstraint::new(unbounded.slots[0].constraint.start(), f32::INFINITY).unwrap();
    let no_wrap = LineContext {
        wrap_width: f32::INFINITY,
        ..hang(HangEndMode::Allow)
    };
    assert_eq!(unbounded.validate(&no_wrap), unneeded);
    // The rule is about the line end: a start-edge hang is not subject to it.
    let mut start = end_hanging_line(60.);
    start.visual_runs[0].glyphs[0].x = 5.;
    start.visual_runs[0].glyphs[1].x = 0.;
    start.visual_runs[0].glyphs.swap(0, 1);
    start.slots[0].content_start = 5.;
    assert_eq!((start.hanging_left(), start.hanging_right()), (5., 0.));
    let first_line = LineContext {
        hang_mode: HangMode {
            start: HangStartMode::FirstLine,
            end: HangEndMode::Allow,
        },
        ..ctx()
    };
    assert_eq!(start.validate(&first_line), Ok(()));
}

fn span_fragment(span: (usize, usize), bytes: (usize, usize)) -> PositionedSpanFragment {
    PositionedSpanFragment {
        span_id: 1,
        span_range: TextRange::new(span.0, span.1),
        byte_range: TextRange::new(bytes.0, bytes.1),
        x: 0.,
        width: 100.,
        ascent: 8.,
        descent: 2.,
        bidi_level: BidiLevel::LTR,
    }
}

#[test]
fn span_fragment_flags_say_where_the_fragment_lies_in_its_span() {
    let with = |fragment| {
        let mut l = two_glyph_line();
        l.span_fragments = vec![fragment];
        l.validate(&ctx())
    };
    let mismatch = Err(LineInvariantError::SpanFragmentOutsideSpan { index: 0 });
    // The whole span, its first part, a middle part and its last part.
    for span in [(0, 2), (0, 9), (0, 9), (0, 2)] {
        assert_eq!(with(span_fragment(span, (0, 2))), Ok(()), "{span:?}");
    }
    let mut l = two_glyph_line();
    l.byte_range = TextRange::new(4, 6);
    l.visible_byte_end = 6;
    l.visual_runs[0].byte_range = l.byte_range;
    l.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(4, 5));
    l.visual_runs[0].glyphs[1].byte_range = Some(TextRange::new(5, 6));
    for span in [(0, 9), (0, 6), (4, 9)] {
        l.span_fragments = vec![span_fragment(span, (4, 6))];
        assert_eq!(l.validate(&ctx()), Ok(()), "{span:?}");
    }
    // Each edge is the fragment's exactly when it reaches that end.
    let fragment = span_fragment((0, 9), (0, 2));
    assert!(fragment.is_first_fragment() && !fragment.is_last_fragment());
    let fragment = span_fragment((0, 9), (4, 9));
    assert!(!fragment.is_first_fragment() && fragment.is_last_fragment());
    // The fragment lies inside its span.
    assert_eq!(with(span_fragment((1, 2), (0, 2))), mismatch);
}

fn atomic_fragment(full: (usize, usize), bytes: (usize, usize), x: f32) -> AtomicFragment {
    AtomicFragment {
        atomic_id: 7,
        full_byte_range: TextRange::new(full.0, full.1),
        fragment_byte_range: TextRange::new(bytes.0, bytes.1),
        x,
        width: 50.,
        bidi_level: BidiLevel::LTR,
    }
}

#[test]
fn the_atomic_fragments_of_a_line_cover_the_span_on_that_line() {
    let with = |fragments: Vec<AtomicFragment>| {
        let mut l = two_glyph_line();
        l.atomic_fragments = fragments;
        l.validate(&ctx())
    };
    let invalid = |index| Err(LineInvariantError::InvalidAtomicFragment { index });
    // One fragment, or two that bidi reordering split.
    assert_eq!(with(vec![atomic_fragment((0, 2), (0, 2), 0.)]), Ok(()));
    assert_eq!(
        with(vec![
            atomic_fragment((0, 2), (1, 2), 0.),
            atomic_fragment((0, 2), (0, 1), 50.)
        ]),
        Ok(())
    );
    // A span that wraps: the line holds the part of it that is on the line.
    assert_eq!(with(vec![atomic_fragment((0, 9), (0, 2), 0.)]), Ok(()));
    assert_eq!(with(vec![atomic_fragment((1, 9), (1, 2), 50.)]), Ok(()));
    // A fragment owns bytes, inside its span.
    assert_eq!(with(vec![atomic_fragment((0, 2), (1, 1), 0.)]), invalid(0));
    assert_eq!(with(vec![atomic_fragment((1, 2), (0, 2), 0.)]), invalid(0));
    // Part of the span's bytes on the line is missing.
    assert_eq!(with(vec![atomic_fragment((0, 2), (0, 1), 0.)]), invalid(0));
    assert_eq!(with(vec![atomic_fragment((0, 2), (1, 2), 50.)]), invalid(0));
    // Two fragments of one span overlap, or disagree about the span.
    assert_eq!(
        with(vec![
            atomic_fragment((0, 2), (0, 2), 0.),
            atomic_fragment((0, 2), (1, 2), 50.)
        ]),
        invalid(1)
    );
    assert_eq!(
        with(vec![
            atomic_fragment((0, 2), (0, 1), 0.),
            atomic_fragment((0, 3), (1, 2), 50.)
        ]),
        invalid(1)
    );
}
