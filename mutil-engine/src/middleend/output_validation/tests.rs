use super::super::*;
use super::*;
use crate::frontend::*;
use crate::middleend::{
    CaretStop, DecorationKind, DecorationLine, HangMode, HangStartMode, InlineSlotConstraint,
    LayoutPolicySnapshot, PositionedInlineBox, PositionedSpanFragment, PositionedTab, StrutMetrics,
    VisualGlyphRun,
};
use crate::types::*;
use crate::types::{BaselineAlignment, ByteIndex, FontId, FontRef, GlyphOrientation, WritingMode};
use smallvec::{SmallVec, smallvec};

const STRUT: StrutMetrics = StrutMetrics {
    ascent: 12.0,
    descent: 4.0,
    line_gap: 0.0,
};

/// An LTR horizontal context with no wrap limit. Permissive on hanging so
/// the geometry tests stay about geometry.
fn ctx() -> LineContext {
    LineContext {
        writing_mode: WritingMode::HorizontalTb,
        base_direction: BidiDirection::Ltr,
        strut: STRUT,
        line_height: LineHeight::Normal,
        whitespace: HangingWhitespaceMode::Collapsible,
        hang_mode: HangMode {
            start: HangStartMode::AllLines,
            end: HangEndMode::Force,
        },
        wrap_width: f32::INFINITY,
        line_index: 0,
    }
}

fn preserved() -> LineContext {
    LineContext {
        whitespace: HangingWhitespaceMode::Preserved,
        ..ctx()
    }
}

/// The policy of [`ctx`], for paragraph fixtures.
fn policy(ctx: &LineContext) -> LayoutPolicySnapshot {
    LayoutPolicySnapshot {
        slot_source: crate::middleend::SlotSource::Standard {
            first_line_indent: 0.0,
            hanging_indent: 0.0,
        },
        writing_mode: ctx.writing_mode,
        strut: ctx.strut,
        line_height: ctx.line_height,
        whitespace: ctx.whitespace,
        hang_mode: ctx.hang_mode,
        wrap_width: ctx.wrap_width,
        clamp: None,
    }
}

/// A slot `capacity` wide from `start`, with content from its start.
fn slot(start: f32, visual_width: f32, capacity: f32) -> PositionedSlot {
    PositionedSlot {
        constraint: InlineSlotConstraint::new(start, start + capacity).unwrap(),
        content_start: start,
        visual_width,
    }
}

fn glyph(x: f32, advance: f32, is_hanging: bool) -> PositionedGlyph {
    PositionedGlyph {
        glyph_id: 1,
        x,
        x_offset: 0.0,
        y_offset: 0.0,
        advance,
        byte_range: Some(TextRange::new(0, 1)),
        is_hanging,
    }
}

fn one_slot(visual_width: f32, wrap_width: f32) -> SmallVec<[PositionedSlot; 1]> {
    smallvec![slot(0.0, visual_width, wrap_width)]
}

fn line(glyphs: Vec<PositionedGlyph>, width: f32) -> PositionedLine {
    let byte_end = glyphs
        .iter()
        .filter_map(|g| g.byte_range.map(|r| r.end()))
        .max()
        .unwrap_or(4)
        .max(4);
    PositionedLine {
        slots: one_slot(width, f32::INFINITY),
        annotations: Vec::new(),
        is_ellipsized: false,
        atomic_fragments: vec![],

        byte_range: TextRange::new(0, byte_end),
        visible_byte_end: byte_end,
        baseline_y: 12.0,
        ascent: 12.0,
        descent: 4.0,
        line_height: 16.0,
        is_justified: false,
        visual_runs: vec![VisualGlyphRun {
            orientation: GlyphOrientation::Horizontal,
            font_size: 16.0,
            variations: Default::default(),

            font: FontRef::unscaled(FontId(0)),
            bidi_level: BidiLevel::LTR,
            byte_range: TextRange::new(0, byte_end),
            glyphs,
        }],
        visual_boxes: vec![],
        tabs: vec![],
        span_fragments: vec![],
        caret_stops: vec![],
        decorations: vec![],
    }
}

/// Sets the content width of the line's one slot.
fn set_width(l: &mut PositionedLine, width: f32) {
    l.slots[0].visual_width = width;
}

fn empty_line(height: f32) -> PositionedLine {
    PositionedLine {
        byte_range: TextRange::empty_at(7),
        visible_byte_end: 7,
        line_height: height,
        visual_runs: vec![],
        ..line(vec![], 0.0)
    }
}

fn inline_box(x: f32, width: f32) -> PositionedInlineBox {
    PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        box_id: 9,
        byte_range: TextRange::new(1, 4),
        x,
        y: 0.0,
        width,
        ascent: 10.0,
        descent: 2.0,
        overflow_policy: BoxOverflowPolicy::Clip,
    }
}

#[test]
fn empty_line_contract() {
    assert!(empty_line(16.0).is_valid(&ctx()));
    assert_eq!(
        empty_line(15.0).validate(&ctx()),
        Err(LineInvariantError::EmptyLineHeightMismatch {
            expected: 16.0,
            actual: 15.0
        })
    );
    let mut with_runs = line(vec![glyph(0.0, 5.0, false)], 5.0);
    with_runs.byte_range = TextRange::empty_at(0);
    with_runs.visible_byte_end = 0;
    assert_eq!(
        with_runs.validate(&ctx()),
        Err(LineInvariantError::EmptyLineHasVisualRuns)
    );
    let mut boxed = empty_line(16.0);
    boxed.visual_boxes.push(inline_box(0.0, 0.0));
    assert_eq!(
        boxed.validate(&ctx()),
        Err(LineInvariantError::EmptyLineHasBoxes)
    );
    let mut wide = empty_line(16.0);
    wide.slots[0].visual_width = 1.0;
    assert_eq!(
        wide.validate(&ctx()),
        Err(LineInvariantError::EmptyLineHasExtent)
    );
}

#[test]
fn line_height_follows_the_policy() {
    let mut tall = line(vec![glyph(0.0, 5.0, false)], 5.0);
    tall.line_height = 20.0;
    // Normal: taller is fine (fallback glyphs), shorter is not.
    assert!(tall.is_valid(&ctx()));
    let mut short = tall.clone();
    short.line_height = 10.0;
    assert_eq!(
        short.validate(&ctx()),
        Err(LineInvariantError::LineHeightBelowStrut {
            minimum: 16.0,
            actual: 10.0
        })
    );
    // Fixed: must equal the resolved strut exactly (within tolerance).
    let fixed = LineContext {
        line_height: LineHeight::Fixed(1.0),
        ..ctx()
    };
    assert_eq!(
        tall.validate(&fixed),
        Err(LineInvariantError::LineHeightMismatch {
            expected: 16.0,
            actual: 20.0
        })
    );
    assert!(line(vec![glyph(0.0, 5.0, false)], 5.0).is_valid(&fixed));
}

#[test]
fn hanging_on_both_edges_is_valid_and_excluded_from_width() {
    // Body [0, 20]; an opening mark hangs left, a full stop hangs right.
    let l = line(
        vec![
            glyph(-6.0, 6.0, true),
            glyph(0.0, 20.0, false),
            glyph(20.0, 5.0, true),
        ],
        20.0,
    );
    assert!(l.is_valid(&ctx()));
}

#[test]
fn counting_hanging_punctuation_in_visual_width_is_rejected() {
    let l = line(vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)], 25.0);
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 1 })
    );
}

#[test]
fn hanging_extents_are_the_overflow_of_the_hanging_glyphs() {
    let none = line(vec![glyph(0.0, 20.0, false)], 20.0);
    assert_eq!((none.hanging_left(), none.hanging_right()), (0.0, 0.0));
    let right = line(vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)], 20.0);
    assert_eq!((right.hanging_left(), right.hanging_right()), (0.0, 5.0));
    assert!(right.is_valid(&ctx()));
    // A mark straddling the edge hangs only by its overflow.
    let left = line(vec![glyph(-6.0, 8.0, true), glyph(2.0, 18.0, false)], 20.0);
    assert_eq!((left.hanging_left(), left.hanging_right()), (6.0, 0.0));
    assert_eq!(left.visual_width(), 20.0);
}

fn trailing_space(x: f32, advance: f32) -> PositionedGlyph {
    PositionedGlyph {
        byte_range: Some(TextRange::new(3, 4)),
        ..glyph(x, advance, false)
    }
}

#[test]
fn preserved_trailing_space_may_sit_outside_the_body() {
    // Body "ab" = [0, 20]; the space (bytes 3..4) hangs beyond visual_width.
    let mut l = line(
        vec![glyph(0.0, 20.0, false), trailing_space(20.0, 5.0)],
        20.0,
    );
    l.visible_byte_end = 3;
    assert!(l.is_valid(&preserved()));
    // It is not punctuation: it never counts as hanging.
    assert_eq!(l.hanging_right(), 0.0);
    // Collapsible: the same glyph must have collapsed to zero width.
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::TrailingWhitespaceNotCollapsed { run: 0, glyph: 1 })
    );
    let mut collapsed = l;
    collapsed.visual_runs[0].glyphs[1] = trailing_space(20.0, 0.0);
    assert!(collapsed.is_valid(&ctx()));
}

#[test]
fn mixed_tolerance_absorbs_f32_noise_at_large_coordinates() {
    // At 1e6 an f32 ulp is 0.0625, far above the 1e-3 absolute floor.
    let l = line(vec![glyph(0.0, 1_000_000.1, false)], 1_000_000.0);
    assert!(l.is_valid(&ctx()));
    // Real overflow is still caught (relative tolerance there is 10).
    let l = line(vec![glyph(0.0, 1_000_100.0, false)], 1_000_000.0);
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    // Small coordinates keep the strict absolute floor.
    let l = line(vec![glyph(0.0, 20.01, false)], 20.0);
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
}

#[test]
fn inline_boxes_assemble_and_are_checked() {
    let mut l = line(vec![glyph(0.0, 5.0, false)], 30.0);
    l.visual_boxes = vec![inline_box(5.0, 12.0), inline_box(17.0, 13.0)];
    assert!(l.is_valid(&ctx()));
    assert_eq!(l.visual_boxes[1].box_id, 9);
    assert_eq!(l.visual_boxes[0].byte_range, TextRange::new(1, 4));

    let mut outside = l.clone();
    outside.visual_boxes[1].width = 40.0;
    assert_eq!(
        outside.validate(&ctx()),
        Err(LineInvariantError::InlineBoxOutsideBlock { index: 1 })
    );
    let mut negative = l.clone();
    negative.visual_boxes[0].width = -1.0;
    assert_eq!(
        negative.validate(&ctx()),
        Err(LineInvariantError::InvalidInlineBox { index: 0 })
    );
    let mut nan = l;
    nan.visual_boxes[0].y = f32::NAN;
    assert_eq!(
        nan.validate(&ctx()),
        Err(LineInvariantError::InvalidInlineBox { index: 0 })
    );
}

#[test]
fn visible_end_must_lie_inside_the_line() {
    let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0);
    l.visible_byte_end = 9;
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::VisibleEndOutsideRange)
    );
}

#[test]
fn non_finite_geometry_is_rejected() {
    for bad in [f32::NAN, f32::INFINITY] {
        let mut g = glyph(0.0, 5.0, false);
        g.x = bad;
        assert_eq!(
            line(vec![g], 5.0).validate(&ctx()),
            Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
        );
        let mut g = glyph(0.0, 5.0, false);
        g.advance = bad;
        assert_eq!(
            line(vec![g], 5.0).validate(&ctx()),
            Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
        );
        let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0);
        l.line_height = bad;
        assert_eq!(
            l.validate(&ctx()),
            Err(LineInvariantError::NonFiniteLineMetrics)
        );
    }
}

#[test]
fn zero_advance_marks_with_ink_offsets_do_not_trip_containment() {
    // The mark's pen position is 0; its ink sits 3 units to the left.
    let mut mark = glyph(0.0, 0.0, false);
    mark.x_offset = -3.0;
    let l = line(vec![mark, glyph(0.0, 20.0, false)], 20.0);
    assert!(l.is_valid(&ctx()));
    // `x_offset` plays no part in containment, even when large.
    let mut far = glyph(5.0, 10.0, false);
    far.x_offset = -50.0;
    let body = || vec![glyph(0.0, 5.0, false), far, glyph(15.0, 5.0, false)];
    assert!(line(body(), 20.0).is_valid(&ctx()));
    // A zero advance has no horizontal extent, so its pen position is
    // exempt from containment even outside the body (and it is not content
    // of the slot).
    let with_mark = |mark: PositionedGlyph| {
        // A run's glyphs are in screen order.
        let mut glyphs = vec![mark, glyph(0.0, 20.0, false)];
        glyphs.sort_by(|a, b| a.x.total_cmp(&b.x));
        line(glyphs, 20.0)
    };
    assert!(with_mark(glyph(-3.0, 0.0, false)).is_valid(&ctx()));
    assert!(with_mark(glyph(25.0, 0.0, false)).is_valid(&ctx()));
    // A zero-advance mark on a hanging base is flagged hanging and skipped.
    let l = line(
        vec![
            glyph(0.0, 20.0, false),
            glyph(20.0, 5.0, true),
            glyph(25.0, 0.0, true),
        ],
        20.0,
    );
    assert!(l.is_valid(&ctx()));
    // A non-finite ink offset is rejected in pass 1.
    let mut nan = glyph(0.0, 5.0, false);
    nan.x_offset = f32::NAN;
    assert_eq!(
        line(vec![nan], 5.0).validate(&ctx()),
        Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
    );
}

fn fragment(span_id: u64, x: f32, width: f32, bytes: (usize, usize)) -> PositionedSpanFragment {
    PositionedSpanFragment {
        span_id,
        span_range: TextRange::new(bytes.0, bytes.1),
        byte_range: TextRange::new(bytes.0, bytes.1),
        x,
        width,
        ascent: 12.0,
        descent: 4.0,
        bidi_level: BidiLevel::LTR,
    }
}

/// Two glyphs `[0,10]` (bytes 0..2) and `[10,30]` (bytes 2..4); body 30.
fn spanned_line() -> PositionedLine {
    let mut second = glyph(10.0, 20.0, false);
    second.byte_range = Some(TextRange::new(2, 4));
    let mut first = glyph(0.0, 10.0, false);
    first.byte_range = Some(TextRange::new(0, 2));
    line(vec![first, second], 30.0)
}

#[test]
fn span_fragments_must_be_sane_and_inside_the_block() {
    let mut l = spanned_line();
    l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 4))];
    assert!(l.is_valid(&ctx()));
    for (frag, expected) in [
        (
            fragment(1, -2.0, 10.0, (0, 4)),
            LineInvariantError::SpanFragmentOutsideBlock { index: 0 },
        ),
        (
            fragment(1, 0.0, 31.0, (0, 4)),
            LineInvariantError::SpanFragmentOutsideBlock { index: 0 },
        ),
        (
            fragment(1, 0.0, -1.0, (0, 4)),
            LineInvariantError::InvalidSpanFragment { index: 0 },
        ),
        (
            fragment(1, f32::NAN, 5.0, (0, 4)),
            LineInvariantError::InvalidSpanFragment { index: 0 },
        ),
    ] {
        let mut l = spanned_line();
        l.span_fragments = vec![frag];
        assert_eq!(l.validate(&ctx()), Err(expected));
    }
    let mut negative_extent = spanned_line();
    negative_extent.span_fragments = vec![PositionedSpanFragment {
        descent: -1.0,
        ..fragment(1, 0.0, 30.0, (0, 4))
    }];
    assert_eq!(
        negative_extent.validate(&ctx()),
        Err(LineInvariantError::InvalidSpanFragment { index: 0 })
    );
    let mut empty = empty_line(16.0);
    empty.span_fragments.push(fragment(1, 0.0, 0.0, (7, 7)));
    assert_eq!(
        empty.validate(&ctx()),
        Err(LineInvariantError::EmptyLineHasSpanFragments)
    );
}

#[test]
fn span_members_must_lie_inside_a_fragment_of_their_span() {
    // Span 1 covers bytes 2..4 and a fragment [10, 30]: the second glyph fits.
    let mut l = spanned_line();
    l.span_fragments = vec![fragment(1, 10.0, 20.0, (2, 4))];
    assert!(l.is_valid(&ctx()));
    // A fragment too narrow for the glyph it owns.
    l.span_fragments = vec![fragment(1, 10.0, 15.0, (2, 4))];
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::GlyphOutsideSpanFragment {
            span_id: 1,
            run: 0,
            glyph: 1
        })
    );
    // A glyph outside every byte range is not a member and is not checked.
    l.span_fragments = vec![fragment(1, 10.0, 15.0, (3, 4))];
    assert!(l.is_valid(&ctx()));
    // Bidi can split one span into two visual groups; membership needs only
    // one fragment of the span to contain the glyph.
    l.span_fragments = vec![
        fragment(1, 0.0, 10.0, (0, 2)),
        fragment(1, 10.0, 20.0, (2, 4)),
    ];
    assert!(l.is_valid(&ctx()));
    // Padding makes a fragment larger than its content: still contained.
    let mut padded = spanned_line();
    set_width(&mut padded, 34.0);
    padded.span_fragments = vec![fragment(2, 0.0, 34.0, (0, 4))];
    assert!(padded.is_valid(&ctx()));
}

#[test]
fn span_membership_covers_inline_boxes_and_exempts_hanging_and_trailing() {
    let mut l = line(vec![glyph(0.0, 5.0, false)], 30.0);
    l.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(0, 1));
    l.visual_boxes = vec![inline_box(10.0, 12.0)]; // bytes 1..4
    l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 4))];
    assert!(l.is_valid(&ctx()));
    l.span_fragments = vec![fragment(1, 0.0, 18.0, (0, 4))];
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InlineBoxOutsideSpanFragment {
            span_id: 1,
            index: 0
        })
    );
    // A hanging glyph lies outside the body and so outside the fragment.
    let mut h = line(vec![glyph(0.0, 20.0, false), glyph(20.0, 5.0, true)], 20.0);
    h.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(0, 3));
    h.visual_runs[0].glyphs[1].byte_range = Some(TextRange::new(3, 4));
    h.span_fragments = vec![fragment(1, 0.0, 20.0, (0, 4))];
    assert!(h.is_valid(&ctx()));
}

#[test]
fn a_lone_overflowing_object_is_valid_when_the_width_equals_the_box() {
    // `AllowOverflow`: the line's visual_width is set to the box width, so
    // containment [0, visual_width] holds for a box wider than wrap_width.
    let mut l = line(vec![], 500.0);
    l.visual_runs.clear();
    l.visual_boxes = vec![inline_box(0.0, 500.0)];
    assert!(l.is_valid(&ctx()));
}

#[test]
fn body_glyphs_must_stay_inside_the_block_and_widths_must_be_finite() {
    let outside = line(vec![glyph(0.0, 30.0, false)], 20.0);
    assert_eq!(
        outside.validate(&ctx()),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    let nan = line(vec![glyph(0.0, 5.0, false)], f32::NAN);
    assert_eq!(
        nan.validate(&ctx()),
        Err(LineInvariantError::InvalidSlots {
            reason: SlotErrorReason::InvalidContent
        })
    );
}

#[test]
fn errors_display_and_implement_std_error() {
    let e = LineInvariantError::BodyGlyphOutsideBlock { run: 1, glyph: 2 };
    let dynamic: &dyn std::error::Error = &e;
    assert_eq!(
        dynamic.to_string(),
        "body glyph 2 of run 1 lies outside the content of every slot"
    );
    let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(e);
    assert!(!boxed.to_string().is_empty());
}

fn no_hang() -> LineContext {
    LineContext {
        hang_mode: HangMode::NONE,
        ..ctx()
    }
}

/// A paragraph of `lines` laid out under the policy of `ctx`.
fn paragraph(ctx: &LineContext, lines: Vec<PositionedLine>) -> PositionedParagraph {
    PositionedParagraph {
        policy: policy(ctx),
        base_direction: ctx.base_direction,
        truncated_range: None,
        total_block_size: 16.0,
        lines,
    }
}

fn caret(x: f32, offset: ByteIndex) -> CaretStop {
    CaretStop {
        x,
        offset,
        affinity: crate::middleend::CaretAffinity::Downstream,
        level: BidiLevel::LTR,
        line: 0,
    }
}

fn underline(x: f32, width: f32) -> DecorationLine {
    DecorationLine {
        kind: DecorationKind::Underline,
        byte_range: TextRange::new(0, 1),
        x,
        width,
        y_offset: 1.0,
        thickness: 1.0,
    }
}

#[test]
fn caret_stops_must_be_finite_ordered_and_inside_the_line() {
    // One glyph rendering the whole line, bytes 0..4.
    let whole = PositionedGlyph {
        byte_range: Some(TextRange::new(0, 4)),
        ..glyph(0.0, 5.0, false)
    };
    let base = || line(vec![whole], 5.0);
    let invalid = Err(LineInvariantError::InvalidCaretStop { index: 1 });
    let mut l = base();
    l.caret_stops = vec![caret(0.0, 0), caret(5.0, 4)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    // Equal x (a bidi boundary has two stops) is in order.
    l.caret_stops = vec![caret(5.0, 4), caret(5.0, 2)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.caret_stops = vec![caret(0.0, 0), caret(f32::NAN, 1)];
    assert_eq!(l.validate(&ctx()), invalid);
    l.caret_stops = vec![caret(5.0, 4), caret(1.0, 1)];
    assert_eq!(l.validate(&ctx()), invalid);
    l.caret_stops = vec![caret(0.0, 0), caret(5.0, 5)];
    assert_eq!(l.validate(&ctx()), invalid);
    l.caret_stops = vec![
        caret(0.0, 0),
        CaretStop {
            line: 1,
            ..caret(5.0, 4)
        },
    ];
    assert_eq!(l.validate(&ctx()), invalid);
    assert_eq!(
        l.validate(&LineContext {
            line_index: 1,
            ..ctx()
        }),
        Err(LineInvariantError::InvalidCaretStop { index: 0 })
    );
}

#[test]
fn caret_stop_levels_follow_their_owners() {
    let invalid = |index| Err(LineInvariantError::InvalidCaretStop { index });
    let leveled = |offset, level| CaretStop {
        level,
        ..caret(0.0, offset)
    };
    // A glyph at level 0 owns 0..1; nothing owns 2..4.
    let mut l = line(vec![glyph(0.0, 5.0, false)], 5.0);
    l.caret_stops = vec![leveled(1, BidiLevel::LTR)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.caret_stops = vec![leveled(1, BidiLevel::RTL)];
    assert_eq!(l.validate(&ctx()), invalid(0));
    // A stop no glyph, box or tab touches lies in glyph-less glue bytes
    // (CRT-2): the paragraph level or a neighbour's level, nothing else.
    l.caret_stops = vec![leveled(3, BidiLevel::LTR)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.caret_stops = vec![leveled(3, BidiLevel::RTL)];
    assert_eq!(l.validate(&ctx()), invalid(0));
    l.caret_stops = vec![leveled(3, BidiLevel::LTR)];

    // An inline object and a tab own their bytes and lend their levels.
    let object = PositionedInlineBox {
        baseline_align: BaselineAlignment::Alphabetic,
        box_id: 1,
        byte_range: TextRange::new(1, 2),
        x: 5.0,
        y: 0.0,
        width: 0.0,
        ascent: 0.0,
        descent: 0.0,
        overflow_policy: BoxOverflowPolicy::AllowOverflow,
        bidi_level: BidiLevel::new(2).unwrap(),
    };
    let tab = PositionedTab {
        byte_range: TextRange::new(2, 4),
        x: 5.0,
        width: 0.0,
    };
    l.visual_boxes = vec![object];
    l.tabs = vec![tab];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.caret_stops = vec![leveled(3, BidiLevel::RTL)];
    assert_eq!(l.validate(&ctx()), invalid(0));
    // Between the glyph and the object either level qualifies.
    l.caret_stops = vec![leveled(1, BidiLevel::new(2).unwrap())];
    assert_eq!(l.validate(&ctx()), Ok(()));
    l.caret_stops = vec![leveled(1, BidiLevel::RTL)];
    assert_eq!(l.validate(&ctx()), invalid(0));
    // A tab lends the paragraph level (UAX #9 L1): level 1 in an RTL
    // paragraph, where the glyph and the object keep their own levels.
    let rtl = LineContext {
        base_direction: BidiDirection::Rtl,
        ..ctx()
    };
    l.caret_stops = vec![leveled(3, BidiLevel::RTL)];
    assert_eq!(l.validate(&rtl), Ok(()));
    l.caret_stops = vec![leveled(3, BidiLevel::LTR)];
    assert_eq!(l.validate(&rtl), invalid(0));
}

#[test]
fn an_empty_line_stop_carries_the_paragraph_level() {
    let mut l = line(vec![], 0.0);
    l.visual_runs.clear();
    l.byte_range = TextRange::empty_at(0);
    l.visible_byte_end = 0;
    l.caret_stops = vec![caret(0.0, 0)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    let rtl = LineContext {
        base_direction: BidiDirection::Rtl,
        ..ctx()
    };
    assert_eq!(
        l.validate(&rtl),
        Err(LineInvariantError::InvalidCaretStop { index: 0 })
    );
    l.caret_stops[0].level = BidiLevel::RTL;
    assert_eq!(l.validate(&rtl), Ok(()));
}

#[test]
fn decorations_must_be_finite_nonnegative_and_inside_the_line() {
    let base = || line(vec![glyph(0.0, 5.0, false)], 5.0);
    let invalid = Err(LineInvariantError::InvalidDecoration { index: 0 });
    let mut l = base();
    l.decorations = vec![underline(0.0, 5.0)];
    assert_eq!(l.validate(&ctx()), Ok(()));
    for bad in [
        underline(0.0, f32::INFINITY),
        underline(f32::NAN, 1.0),
        underline(0.0, -1.0),
        DecorationLine {
            thickness: -1.0,
            ..underline(0.0, 5.0)
        },
        DecorationLine {
            byte_range: TextRange::new(2, 9),
            ..underline(0.0, 5.0)
        },
    ] {
        l.decorations = vec![bad];
        assert_eq!(l.validate(&ctx()), invalid);
    }
}

#[test]
fn overflowing_endpoints_are_rejected_before_comparison() {
    // Each coordinate is finite, but x + advance overflows to infinity.
    let l = line(vec![glyph(3.0e38, 3.0e38, false)], 5.0);
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::NonFiniteGlyph { run: 0, glyph: 0 })
    );
    let mut b = line(vec![glyph(0.0, 5.0, false)], 5.0);
    b.visual_boxes.push(inline_box(3.0e38, 3.0e38));
    assert_eq!(
        b.validate(&ctx()),
        Err(LineInvariantError::InvalidInlineBox { index: 0 })
    );
    let mut t = line(vec![glyph(0.0, 5.0, false)], 5.0);
    t.tabs.push(PositionedTab {
        byte_range: TextRange::new(1, 2),
        x: 3.0e38,
        width: 3.0e38,
    });
    assert_eq!(
        t.validate(&ctx()),
        Err(LineInvariantError::InvalidTab { index: 0 })
    );
}

#[test]
fn negative_advance_is_judged_by_its_normalised_extent() {
    // Occupies [20, 25] although `x + advance` is 20: outside a 20-wide body.
    let l = line(vec![glyph(25.0, -5.0, false)], 20.0);
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    // Inside the body it is fine.
    let body = vec![
        glyph(0.0, 10.0, false),
        glyph(10.0, -5.0, false),
        glyph(10.0, 10.0, false),
    ];
    assert!(line(body, 20.0).is_valid(&ctx()));
}

#[test]
fn synthesized_hyphen_with_no_range_is_not_trailing_whitespace() {
    // "abc" + generated hyphen whose range is empty at visible_byte_end.
    let mut l = line(vec![glyph(0.0, 15.0, false), glyph(15.0, 5.0, false)], 20.0);
    l.byte_range = TextRange::new(0, 4);
    l.visible_byte_end = 4;
    l.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(0, 4));
    l.visual_runs[0].glyphs[1] = PositionedGlyph {
        byte_range: None,
        ..glyph(15.0, 5.0, false)
    };
    assert_eq!(l.validate(&ctx()), Ok(()));
    // Ordinary text must own at least one source byte.
    l.visual_runs[0].glyphs[1].byte_range = Some(TextRange::empty_at(4));
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::InvalidGlyphSource { run: 0, glyph: 1 })
    );
}

#[test]
fn trailing_whitespace_cannot_be_marked_hanging() {
    let mut l = line(
        vec![glyph(0.0, 20.0, false), trailing_space(20.0, 0.0)],
        20.0,
    );
    l.visible_byte_end = 3;
    l.visual_runs[0].glyphs[1].is_hanging = true;
    assert_eq!(
        l.validate(&no_hang()),
        Err(LineInvariantError::TrailingWhitespaceMarkedHanging { run: 0, glyph: 1 })
    );
}

#[test]
fn a_hanging_glyph_with_negligible_overflow_is_rejected() {
    // Extent 1e-4 sits inside the tolerance: the glyph does not hang.
    let l = line(
        vec![glyph(-1.0e-4, 1.0e-4, true), glyph(0.0, 20.0, false)],
        20.0,
    );
    let expected = Err(LineInvariantError::HangingGlyphInsideBlock { run: 0, glyph: 0 });
    assert_eq!(l.validate(&no_hang()), expected);
    assert_eq!(l.validate(&ctx()), expected);
}

#[test]
fn paragraph_validation_uses_the_paragraphs_own_direction() {
    // Preserved space left of the body is valid only in an RTL paragraph.
    let mut l = line(
        vec![glyph(5.0, 20.0, false), trailing_space(0.0, 5.0)],
        20.0,
    );
    l.visible_byte_end = 3;
    l.visual_runs[0].glyphs[0].x = 0.0;
    l.visual_runs[0].glyphs[1].x = -5.0;
    // Screen order: the space, left of the body, comes first.
    l.visual_runs[0].glyphs.swap(0, 1);
    let rtl = LineContext {
        whitespace: HangingWhitespaceMode::Preserved,
        base_direction: BidiDirection::Rtl,
        ..no_hang()
    };
    let mut para = paragraph(&rtl, vec![l]);
    assert_eq!(para.validate_geometry(), Ok(()));
    para.base_direction = BidiDirection::Ltr;
    assert_eq!(
        para.validate_geometry(),
        Err((
            0,
            LineInvariantError::TrailingWhitespaceOnWrongSide { run: 0, glyph: 0 }
        ))
    );
}

// ---- wrap width (spec §2.6, Pass 3 (h)) and fragment hit-testing (spec §6.3) ----

fn wrap(w: f32) -> LineContext {
    LineContext {
        wrap_width: w,
        ..ctx()
    }
}

/// Validation of a single-slot line against a context of wrap width `w`.
trait ValidateAt {
    fn validate_at(&self, w: f32) -> Result<(), LineInvariantError>;
}

impl ValidateAt for PositionedLine {
    fn validate_at(&self, w: f32) -> Result<(), LineInvariantError> {
        let mut l = self.clone();
        l.slots[0].constraint =
            InlineSlotConstraint::new(l.slots[0].constraint.start(), w).unwrap();
        PositionedLine::validate(&l, &wrap(w))
    }
}

fn word_glyph(x: f32, advance: f32, bytes: (usize, usize)) -> PositionedGlyph {
    let mut g = glyph(x, advance, false);
    g.byte_range = Some(TextRange::new(bytes.0, bytes.1));
    g
}

#[test]
fn a_line_must_fit_the_wrap_width() {
    // Two clusters, 30 wide: fits 30 (and within tolerance), not 29.
    let two = || {
        line(
            vec![
                word_glyph(0.0, 10.0, (0, 1)),
                word_glyph(10.0, 20.0, (1, 2)),
            ],
            30.0,
        )
    };
    assert!(two().validate_at(30.0).is_ok());
    assert!(two().validate_at(30.0 - 1e-4).is_ok());
    assert_eq!(
        two().validate_at(29.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 30.0,
            wrap_width: 29.0
        })
    );
    // An infinite wrap width imposes no limit.
    assert!(two().validate(&ctx()).is_ok());
}

#[test]
fn a_single_wide_cluster_may_overflow() {
    // One cluster of two glyphs (base + mark share the byte range).
    let l = line(
        vec![
            word_glyph(0.0, 25.0, (0, 3)),
            word_glyph(25.0, 15.0, (0, 3)),
        ],
        40.0,
    );
    assert!(l.validate_at(30.0).is_ok());
    // Two different clusters are divisible, so they must wrap.
    let l = line(
        vec![
            word_glyph(0.0, 25.0, (0, 3)),
            word_glyph(25.0, 15.0, (3, 6)),
        ],
        40.0,
    );
    assert_eq!(
        l.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 40.0,
            wrap_width: 30.0
        })
    );
    // A lone cluster whose extent is not the line width is not the overflow
    // state either (the line is wider than what it holds).
    let l = line(vec![word_glyph(0.0, 25.0, (0, 3))], 40.0);
    assert_eq!(
        l.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 40.0,
            wrap_width: 30.0
        })
    );
}

#[test]
fn a_single_wide_inline_box_may_overflow() {
    let object = |x: f32, width: f32, policy| PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(0, 1),
        x,
        y: 0.0,
        width,
        ascent: 5.0,
        descent: 1.0,
        overflow_policy: policy,
    };
    let allow = BoxOverflowPolicy::AllowOverflow;
    // Exemption A: no visual runs, no tabs, one AllowOverflow box at x = 0
    // that is as wide as the line.
    let mut l = line(vec![], 50.0);
    l.byte_range = TextRange::new(0, 1);
    l.visible_byte_end = 1;
    l.visual_runs.clear();
    l.visual_boxes = vec![object(0.0, 50.0, allow)];
    assert!(l.validate_at(30.0).is_ok());
    // A rigid policy must fit: Clip and ScaleToFit boxes wider than the line
    // are rejected, by the box rule and then by the width rule.
    for policy in [BoxOverflowPolicy::Clip, BoxOverflowPolicy::ScaleToFit] {
        let mut rigid = l.clone();
        rigid.visual_boxes = vec![object(0.0, 50.0, policy)];
        assert_eq!(
            rigid.validate_at(30.0),
            Err(LineInvariantError::InlineBoxWiderThanWrapWidth { index: 0 })
        );
        // A rigid box that fits is fine.
        rigid.visual_boxes = vec![object(0.0, 30.0, policy)];
        set_width(&mut rigid, 30.0);
        assert!(rigid.validate_at(30.0).is_ok());
    }
    // Not when it shares the line with a glyph run, a tab or a second box.
    let mut with_run = l.clone();
    with_run.visual_runs = line(vec![word_glyph(0.0, 50.0, (0, 1))], 50.0).visual_runs;
    with_run.visual_runs[0].byte_range = with_run.byte_range;
    assert_eq!(
        with_run.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 50.0,
            wrap_width: 30.0
        })
    );
    let mut with_tab = l.clone();
    with_tab.tabs = vec![PositionedTab {
        byte_range: TextRange::new(0, 1),
        x: 50.0,
        width: 0.0,
    }];
    assert_eq!(
        with_tab.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 50.0,
            wrap_width: 30.0
        })
    );
    let mut two = l.clone();
    two.visual_boxes = vec![object(0.0, 25.0, allow), object(25.0, 25.0, allow)];
    assert_eq!(
        two.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 50.0,
            wrap_width: 30.0
        })
    );
    // The box must start at 0 and be as wide as the line.
    let mut narrow = l.clone();
    narrow.visual_boxes = vec![object(0.0, 20.0, allow)];
    assert_eq!(
        narrow.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 50.0,
            wrap_width: 30.0
        })
    );
    let mut shifted = l;
    shifted.visual_boxes = vec![object(5.0, 45.0, allow)];
    assert_eq!(
        shifted.validate_at(30.0),
        Err(LineInvariantError::LineWiderThanWrapWidth {
            visual_width: 50.0,
            wrap_width: 30.0
        })
    );
}

#[test]
fn padded_single_entities_may_overflow_inside_their_fragments() {
    let wide = |paddings: (f32, f32)| {
        // One 26-wide cluster between the paddings, in a chip over its bytes.
        let (left, right) = paddings;
        let mut l = line(vec![word_glyph(left, 26.0, (0, 3))], left + 26.0 + right);
        l.span_fragments = vec![fragment(1, 0.0, left + 26.0 + right, (0, 3))];
        // Byte 3 is the line's trailing whitespace.
        l.visible_byte_end = 3;
        l
    };
    // 3 + 26 + 3 = 32 > 30: legal, the padding rides with the cluster.
    assert!(wide((3.0, 3.0)).validate_at(30.0).is_ok());
    assert!(wide((3.0, 0.0)).validate_at(30.0).is_ok());
    assert!(wide((0.0, 3.0)).validate_at(30.0).is_ok());
    // Nested fragments of the same bytes: all must wrap the entity.
    let mut nested = wide((3.0, 3.0));
    let inner = fragment(2, 3.0, 26.0, (0, 3));
    nested.span_fragments.push(inner);
    assert!(nested.validate_at(30.0).is_ok());
    // Every padded fixture here is 3 + 26 + 3 = 32 wide.
    let too_wide = |l: &PositionedLine| {
        assert_eq!(
            l.validate_at(30.0),
            Err(LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 32.0,
                wrap_width: 30.0
            })
        );
    };
    // A fragment may run on over the trailing whitespace ...
    let mut trailing = wide((3.0, 3.0));
    trailing.span_fragments[0].byte_range = TextRange::new(0, 4);
    trailing.span_fragments[0].span_range = TextRange::new(0, 4);
    assert!(trailing.validate_at(30.0).is_ok());
    // ... but not over a visible byte the entity does not own,
    let mut visible = trailing.clone();
    visible.visible_byte_end = 4;
    too_wide(&visible);
    // not start elsewhere or stop short of the entity,
    for bytes in [(1, 3), (0, 2)] {
        let mut other = wide((3.0, 3.0));
        other.span_fragments[0].byte_range = TextRange::new(bytes.0, bytes.1);
        other.span_fragments[0].span_range = TextRange::new(bytes.0, bytes.1);
        too_wide(&other);
    }
    // and not leave the line (Pass 3(g)).
    let mut beyond = wide((3.0, 3.0));
    beyond.span_fragments[0].byte_range = TextRange::new(0, 5);
    beyond.span_fragments[0].span_range = TextRange::new(0, 5);
    assert_eq!(
        beyond.validate_at(30.0),
        Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
    );
    // Fragments that stop short of an edge of the body.
    let mut short = wide((3.0, 3.0));
    short.span_fragments = vec![fragment(1, 0.0, 29.0, (0, 3))];
    too_wide(&short);
    let mut late = wide((3.0, 3.0));
    late.span_fragments = vec![fragment(1, 3.0, 29.0, (0, 3))];
    too_wide(&late);
    // The padding is only a claim: the entity cannot be outside the body.
    // (Pass 3 rejects it earlier, as containment.)
    let mut outside = wide((3.0, 3.0));
    outside.visual_runs[0].glyphs[0].x = 10.0;
    assert_eq!(
        outside.validate_at(30.0),
        Err(LineInvariantError::BodyGlyphOutsideBlock { run: 0, glyph: 0 })
    );
    // Unpadded, the width must still be exactly the entity's.
    let mut bare = line(vec![word_glyph(0.0, 26.0, (0, 3))], 32.0);
    too_wide(&bare);
    set_width(&mut bare, 26.0);
    assert!(bare.validate_at(30.0).is_ok());

    // Exemption A, padded: one AllowOverflow box in a chip over its bytes.
    let object = |x: f32, width: f32| PositionedInlineBox {
        bidi_level: BidiLevel::LTR,
        baseline_align: BaselineAlignment::Auto,
        box_id: 1,
        byte_range: TextRange::new(0, 1),
        x,
        y: 0.0,
        width,
        ascent: 5.0,
        descent: 1.0,
        overflow_policy: BoxOverflowPolicy::AllowOverflow,
    };
    let mut l = line(vec![], 32.0);
    l.visual_runs.clear();
    l.byte_range = TextRange::new(0, 2);
    l.visible_byte_end = 1;
    l.visual_runs.clear();
    l.visual_boxes = vec![object(3.0, 26.0)];
    l.span_fragments = vec![fragment(1, 0.0, 32.0, (0, 1))];
    assert!(l.validate_at(30.0).is_ok());
    // Trailing whitespace inside the fragment is fine; stopping short is not.
    let mut trailing = l.clone();
    trailing.span_fragments[0].byte_range = TextRange::new(0, 2);
    trailing.span_fragments[0].span_range = TextRange::new(0, 2);
    assert!(trailing.validate_at(30.0).is_ok());
    let mut other = l.clone();
    other.span_fragments[0].byte_range = TextRange::new(0, 0);
    other.span_fragments[0].span_range = TextRange::new(0, 0);
    too_wide(&other);
    let mut short = l.clone();
    short.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 1))];
    too_wide(&short);
    // Without fragments the box must still start at 0.
    let mut bare = l;
    bare.span_fragments.clear();
    too_wide(&bare);
}

#[test]
fn paragraph_validation_applies_the_wrap_width_to_every_line() {
    let two = line(
        vec![
            word_glyph(0.0, 10.0, (0, 1)),
            word_glyph(10.0, 20.0, (1, 2)),
        ],
        30.0,
    );
    let mut p = paragraph(&wrap(30.0), vec![two]);
    p.lines[0].slots[0].constraint =
        InlineSlotConstraint::new(p.lines[0].slots[0].constraint.start(), 30.0).unwrap();
    assert!(p.validate_geometry().is_ok());
    p.policy.wrap_width = 10.0;
    p.lines[0].slots[0].constraint =
        InlineSlotConstraint::new(p.lines[0].slots[0].constraint.start(), 10.0).unwrap();
    assert_eq!(
        p.validate_geometry(),
        Err((
            0,
            LineInvariantError::LineWiderThanWrapWidth {
                visual_width: 30.0,
                wrap_width: 10.0
            }
        ))
    );
}

#[test]
fn a_fragment_may_carry_the_highest_level() {
    let mut l = spanned_line();
    let mut f = fragment(1, 0.0, 30.0, (0, 4));
    f.bidi_level = BidiLevel::MAX;
    l.span_fragments = vec![f];
    assert!(l.validate(&ctx()).is_ok());
}

#[test]
fn span_fragment_bytes_stay_inside_the_line() {
    let mut l = spanned_line();
    l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 4))];
    assert!(l.validate(&ctx()).is_ok());
    // Beyond the line's bytes, or inverted.
    l.span_fragments = vec![fragment(1, 0.0, 30.0, (0, 9))];
    assert_eq!(
        l.validate(&ctx()),
        Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
    );
    // Starting before the line (a nonzero-start line).
    let mut later = spanned_line();
    later.byte_range = TextRange::new(1, 4);
    later.visual_runs[0].byte_range = TextRange::new(1, 4);
    later.visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(1, 2));
    later.span_fragments = vec![fragment(1, 0.0, 30.0, (1, 4))];
    assert!(later.validate(&ctx()).is_ok());
    later.span_fragments[0].byte_range = TextRange::new(0, 4);
    later.span_fragments[0].span_range = TextRange::new(0, 4);
    assert_eq!(
        later.validate(&ctx()),
        Err(LineInvariantError::SpanFragmentOutsideLine { index: 0 })
    );
}
