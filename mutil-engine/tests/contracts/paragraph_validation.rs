//! Paragraph-level output invariants: the recorded policy, and the identity of
//! spans across lines.

use crate::common::*;
use mutil_engine::*;

/// Line `index` of a paragraph of 10-unit lines: bytes `start..end`, one
/// glyph filling the 100-unit slot.
fn text_line(index: usize, start: usize, end: usize) -> PositionedLine {
    let mut l = line();
    l.byte_range = TextRange::new(start, end);
    l.visible_byte_end = end;
    l.baseline_y = 8. + 10. * index as f32;
    let mut g = glyph(0., 100., false);
    g.byte_range = Some(l.byte_range);
    l.visual_runs = vec![run(g)];
    l
}

/// "abcdef" on three lines of two bytes each.
fn three_lines() -> PositionedParagraph {
    let lines = (0..3).map(|i| text_line(i, 2 * i, 2 * i + 2)).collect();
    let mut p = paragraph(&ctx(), lines);
    p.total_block_size = 30.;
    p
}

const TEXT: &str = "abcdef";

fn invalid(line: usize, reason: ParagraphErrorReason) -> Result<(), (usize, LineInvariantError)> {
    Err((line, LineInvariantError::InvalidParagraph { reason }))
}

#[test]
fn a_paragraph_is_validated_against_the_policy_it_records() {
    let p = three_lines();
    assert_eq!(p.validate(TEXT), Ok(()));
    // The lines are 100 wide: a narrower recorded wrap width rejects them,
    // whatever the caller would have liked to check them against.
    let mut narrow = p.clone();
    narrow.policy.wrap_width = 80.;
    assert!(matches!(
        narrow.validate(TEXT),
        Err((0, LineInvariantError::InvalidSlots { .. }))
    ));
    // The line boxes are the strut's under a fixed line height.
    let mut fixed = p.clone();
    fixed.policy.line_height = LineHeight::Fixed(1.0);
    assert_eq!(fixed.validate(TEXT), Ok(()));
    fixed.policy.strut.ascent = 7.;
    fixed.policy.strut.descent = 3.;
    assert!(matches!(
        fixed.validate(TEXT),
        Err((0, LineInvariantError::LineMetricsMismatch { .. }))
    ));
    // A policy layout could not have resolved is rejected as such.
    for break_policy in [
        (|p: &mut LayoutPolicySnapshot| p.wrap_width = 0.) as fn(&mut LayoutPolicySnapshot),
        |p| p.strut.ascent = f32::NAN,
        |p| p.strut.descent = -8.,
        |p| p.line_height = LineHeight::Fixed(0.),
        |p| p.clamp = Some(LineClampPolicy::max_lines(0)),
    ] {
        let mut bad = p.clone();
        break_policy(&mut bad.policy);
        assert_eq!(
            bad.validate_geometry(),
            invalid(0, ParagraphErrorReason::InvalidPolicy)
        );
    }
}

#[test]
fn standard_slots_follow_the_recorded_indents() {
    let indents = |first_line_indent, hanging_indent| SlotSource::Standard {
        first_line_indent,
        hanging_indent,
    };
    let mut p = three_lines();
    // Line 0 starts at the first-line indent, later lines at the hanging one.
    p.policy.slot_source = indents(10., 20.);
    for (i, l) in p.lines.iter_mut().enumerate() {
        let start = if i == 0 { 10. } else { 20. };
        l.slots = smallvec::smallvec![slot(start, 0., 100. - start)];
        l.visual_runs.clear();
        l.byte_range = TextRange::new(2 * i, 2 * i + 2);
    }
    // Without glyphs the lines only need consistent slots and coverage, so
    // the check under test is the only one that can fail.
    let slots_only = p.validate_geometry();
    assert_eq!(slots_only, Ok(()));
    // Swapped indents put line 0 at the hanging indent.
    p.policy.slot_source = indents(20., 10.);
    assert_eq!(
        p.validate_geometry(),
        invalid(0, ParagraphErrorReason::NotStandardSlots)
    );
    // A second slot is never standard; a provider may answer with any slots.
    p.policy.slot_source = indents(10., 20.);
    p.lines[1].slots = smallvec::smallvec![slot(20., 0., 30.), slot(60., 0., 40.)];
    assert_eq!(
        p.validate_geometry(),
        invalid(1, ParagraphErrorReason::NotStandardSlots)
    );
    p.policy.slot_source = SlotSource::Provider;
    assert_eq!(p.validate_geometry(), Ok(()));
    // An indent outside `[0, wrap_width)` is a policy layout never records.
    p.policy.slot_source = indents(100., 0.);
    assert_eq!(
        p.validate_geometry(),
        invalid(0, ParagraphErrorReason::InvalidPolicy)
    );
}

#[test]
fn the_policy_records_where_the_slots_came_from() {
    let a = ast("a");
    let constraints = LayoutConstraints {
        first_line_indent: 5.,
        hanging_indent: 7.,
        ..constraints()
    };
    let policy =
        LayoutPolicySnapshot::resolve(&a, &constraints, &LayoutOptions::default()).unwrap();
    assert_eq!(
        policy.slot_source,
        SlotSource::Standard {
            first_line_indent: 5.,
            hanging_indent: 7.
        }
    );
    assert_eq!(
        policy.slot_source.standard_slot(1, policy.wrap_width),
        Some(InlineSlotConstraint::new(7., 100.).unwrap())
    );
    assert_eq!(SlotSource::Provider.standard_slot(0, 100.), None);
}

#[test]
fn block_geometry_reasons_are_distinct() {
    let p = three_lines();
    let mut overlapping = p.clone();
    overlapping.lines[1].baseline_y -= 5.;
    assert_eq!(
        overlapping.validate_geometry(),
        invalid(1, ParagraphErrorReason::LineBoxesOverlap)
    );
    let mut tall = p.clone();
    tall.lines[2].descent = 5.;
    tall.total_block_size = 40.;
    assert_eq!(
        tall.validate_geometry(),
        invalid(2, ParagraphErrorReason::LineBoxExceedsLineHeight)
    );
    let mut short = p.clone();
    short.total_block_size = 29.;
    assert_eq!(
        short.validate_geometry(),
        invalid(2, ParagraphErrorReason::InvalidBlockSize)
    );
    let mut skipping = p;
    skipping.lines[1].byte_range = TextRange::new(3, skipping.lines[1].byte_range.end());
    skipping.lines[1].visual_runs[0].byte_range =
        TextRange::new(3, skipping.lines[1].visual_runs[0].byte_range.end());
    skipping.lines[1].visual_runs[0].glyphs[0].byte_range = Some(TextRange::new(3, 4));
    assert_eq!(
        skipping.validate(TEXT),
        invalid(1, ParagraphErrorReason::SourceNotCovered)
    );
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
fn a_span_has_one_first_and_one_last_fragment_across_the_lines() {
    let inconsistent = |line| invalid(line, ParagraphErrorReason::InconsistentSpanFragments);
    // A span over all three lines: first, middle and last fragment.
    let mut p = three_lines();
    for (i, l) in p.lines.iter_mut().enumerate() {
        l.span_fragments = vec![span_fragment((0, 6), (2 * i, 2 * i + 2))];
    }
    assert_eq!(p.validate(TEXT), Ok(()));
    // The fragments of one span agree about its range. (A line alone cannot
    // tell: each of these is a valid middle fragment of its own span.)
    let mut disagreeing = p.clone();
    disagreeing.lines[1].span_fragments[0].span_range = TextRange::new(0, 7);
    assert_eq!(disagreeing.validate(TEXT), inconsistent(1));
    // The line holding the span's start has no fragment: no first fragment.
    let mut headless = p.clone();
    headless.lines[0].span_fragments.clear();
    assert_eq!(headless.validate(TEXT), inconsistent(2));
    // Nor may the last one be missing ...
    let mut tailless = p.clone();
    tailless.lines[2].span_fragments.clear();
    assert_eq!(tailless.validate(TEXT), inconsistent(1));
    // ... unless a clamp cut the paragraph before the span's end.
    let mut clamped = tailless;
    clamped.lines.pop();
    clamped.total_block_size = 20.;
    clamped.truncated_range = Some(TextRange::new(4, 6));
    clamped.policy.clamp = Some(LineClampPolicy::max_lines(2));
    clamped.lines[1].is_ellipsized = true;
    assert_eq!(clamped.validate(TEXT), Ok(()));
    // Two fragments cannot both hold the span's start.
    let mut doubled = p;
    let first = doubled.lines[0].span_fragments[0];
    doubled.lines[0].span_fragments.push(first);
    assert_eq!(doubled.validate(TEXT), inconsistent(0));
}

fn atomic_fragment(full: (usize, usize), bytes: (usize, usize)) -> AtomicFragment {
    AtomicFragment {
        atomic_id: 7,
        full_byte_range: TextRange::new(full.0, full.1),
        fragment_byte_range: TextRange::new(bytes.0, bytes.1),
        x: 0.,
        width: 100.,
        bidi_level: BidiLevel::LTR,
    }
}

#[test]
fn the_fragments_of_an_atomic_span_cover_it_across_the_lines() {
    let inconsistent = |line| invalid(line, ParagraphErrorReason::InconsistentAtomicFragments);
    // An atomic span over the last two lines.
    let mut p = three_lines();
    p.lines[1].atomic_fragments = vec![atomic_fragment((2, 6), (2, 4))];
    p.lines[2].atomic_fragments = vec![atomic_fragment((2, 6), (4, 6))];
    assert_eq!(p.validate(TEXT), Ok(()));
    // A line the span crosses has no fragment of it.
    let mut gap = p.clone();
    gap.lines[2].atomic_fragments.clear();
    assert_eq!(gap.validate(TEXT), inconsistent(1));
    // The lines disagree about the span's range.
    let mut disagreeing = p.clone();
    disagreeing.lines[2].atomic_fragments[0].full_byte_range = TextRange::new(4, 6);
    assert_eq!(disagreeing.validate(TEXT), inconsistent(2));
    // A clamp may cut the span: its rendered part is still covered.
    let mut clamped = p;
    clamped.lines.pop();
    clamped.total_block_size = 20.;
    clamped.truncated_range = Some(TextRange::new(4, 6));
    clamped.policy.clamp = Some(LineClampPolicy::max_lines(2));
    clamped.lines[1].is_ellipsized = true;
    assert_eq!(clamped.validate(TEXT), Ok(()));
}
