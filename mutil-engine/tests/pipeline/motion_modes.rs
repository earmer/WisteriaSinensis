//! Cursor motions in every writing mode (INT-13 to INT-15, INT-19): the four
//! physical arrows resolve through the mode, while the logical and word
//! motions do not depend on it.

use crate::common::*;
use mutil_engine::*;

const MODES: [WritingMode; 3] = [
    WritingMode::HorizontalTb,
    WritingMode::VerticalRl,
    WritingMode::VerticalLr,
];

fn at(offset: usize) -> CursorPosition {
    CursorPosition {
        byte_offset: offset,
        affinity: CaretAffinity::Downstream,
    }
}

fn upstream(offset: usize) -> CursorPosition {
    CursorPosition {
        byte_offset: offset,
        affinity: CaretAffinity::Upstream,
    }
}

/// `p` re-labelled as laid out in `mode`: a horizontal single-direction
/// layout is a valid flow-space layout in a vertical mode once its runs are
/// set sideways and the block size holds the mode's line bounds.
fn in_mode(p: &PositionedParagraph, mode: WritingMode) -> PositionedParagraph {
    let mut p = p.clone();
    if mode != WritingMode::HorizontalTb {
        p.policy.writing_mode = mode;
        for line in &mut p.lines {
            for run in &mut line.visual_runs {
                run.orientation = GlyphOrientation::VerticalSideways;
            }
        }
        let bottom = p
            .lines
            .iter()
            .map(|l| mode.line_bounds(l.baseline_y, l.ascent, l.descent).1)
            .fold(p.total_block_size, f32::max);
        p.total_block_size = bottom;
    }
    p
}

/// "ab cd" in a column that wraps it into "ab" and "cd" (glyphs 6 wide),
/// relabelled as laid out in `mode`.
fn two_lines(mode: WritingMode) -> (&'static str, PositionedParagraph) {
    let text = "ab cd";
    let p = lay_out(text, &column(20.0), &LayoutOptions::default());
    assert_eq!(p.lines.len(), 2);
    (text, in_mode(&p, mode))
}

fn validated<'a>(
    text: &'a str,
    p: &'a PositionedParagraph,
    mode: WritingMode,
) -> ValidatedLayout<'a> {
    ValidatedLayout::try_from_paragraph(p, text)
        .unwrap_or_else(|(line, e)| panic!("{mode:?}: line {line}: {e}"))
}

fn exit(edge: ParagraphEdge, inline_position: Option<f32>) -> CursorMove {
    CursorMove::Exited {
        edge,
        inline_position,
    }
}

/// The physical arrow that is the flow motion `flow` in `mode`.
fn arrow(mode: WritingMode, flow: FlowMotion) -> Motion {
    let all = [Motion::Left, Motion::Right, Motion::Up, Motion::Down];
    *all.iter()
        .find(|m| m.flow_direction(mode) == Some(flow))
        .unwrap_or_else(|| panic!("{mode:?}: no arrow resolves to {flow:?}"))
}

#[test]
fn int19_the_arrows_resolve_to_flow_motions_by_the_writing_mode() {
    use FlowMotion::*;
    let expected = [
        // (mode, Left, Right, Up, Down)
        (
            WritingMode::HorizontalTb,
            InlinePrev,
            InlineNext,
            BlockPrev,
            BlockNext,
        ),
        (
            WritingMode::VerticalRl,
            BlockNext,
            BlockPrev,
            InlinePrev,
            InlineNext,
        ),
        (
            WritingMode::VerticalLr,
            BlockPrev,
            BlockNext,
            InlinePrev,
            InlineNext,
        ),
    ];
    for (mode, left, right, up, down) in expected {
        assert_eq!(Motion::Left.flow_direction(mode), Some(left), "{mode:?}");
        assert_eq!(Motion::Right.flow_direction(mode), Some(right), "{mode:?}");
        assert_eq!(Motion::Up.flow_direction(mode), Some(up), "{mode:?}");
        assert_eq!(Motion::Down.flow_direction(mode), Some(down), "{mode:?}");
        for logical in [
            Motion::Previous,
            Motion::Next,
            Motion::WordLeft,
            Motion::WordRight,
            Motion::WordPrevious,
            Motion::WordNext,
            Motion::LineStart,
            Motion::LineEnd,
        ] {
            assert_eq!(
                logical.flow_direction(mode),
                None,
                "{logical:?} in {mode:?}"
            );
        }
    }
}

#[test]
fn int13_the_inline_arrows_step_one_stop_and_exit_at_the_paragraph_edges_in_every_mode() {
    for mode in MODES {
        let (text, p) = two_lines(mode);
        let l = validated(text, &p, mode);
        let next = arrow(mode, FlowMotion::InlineNext);
        let prev = arrow(mode, FlowMotion::InlinePrev);
        let what = format!("{mode:?}");
        assert_eq!(
            l.move_cursor(at(0), next, None),
            CursorMove::Moved(at(1)),
            "{what}"
        );
        assert_eq!(
            l.move_cursor(at(3), prev, None),
            CursorMove::Moved(upstream(2)),
            "{what}"
        );
        // Past a line end, over the collapsed space, to the next line.
        assert_eq!(
            l.move_cursor(upstream(2), next, None),
            CursorMove::Moved(at(3)),
            "{what}"
        );
        // The exits carry no inline position.
        assert_eq!(
            l.move_cursor(at(0), prev, None),
            exit(ParagraphEdge::Start, None),
            "{what}"
        );
        assert_eq!(
            l.move_cursor(at(5), next, None),
            exit(ParagraphEdge::End, None),
            "{what}"
        );
    }
}

#[test]
fn int15_the_block_arrows_keep_the_preferred_position_and_exit_with_it_in_every_mode() {
    for mode in MODES {
        let (text, p) = two_lines(mode);
        let l = validated(text, &p, mode);
        let next = arrow(mode, FlowMotion::BlockNext);
        let prev = arrow(mode, FlowMotion::BlockPrev);
        let what = format!("{mode:?}");
        // x = 6 on the first line is x = 6 on the second: offsets 1 and 4.
        assert_eq!(
            l.move_cursor(at(1), next, None),
            CursorMove::Moved(at(4)),
            "{what}"
        );
        assert_eq!(
            l.move_cursor(at(4), prev, None),
            CursorMove::Moved(at(1)),
            "{what}"
        );
        // A preferred position held by the caller overrides the caret's own.
        assert_eq!(
            l.move_cursor(at(4), prev, Some(0.5)),
            CursorMove::Moved(at(0)),
            "{what}"
        );
        // Off the first or last line the paragraph is left, with the position.
        assert_eq!(
            l.move_cursor(at(1), prev, Some(3.0)),
            exit(ParagraphEdge::Start, Some(3.0)),
            "{what}"
        );
        assert_eq!(
            l.move_cursor(at(4), next, Some(3.0)),
            exit(ParagraphEdge::End, Some(3.0)),
            "{what}"
        );
    }
}

#[test]
fn int19_a_vertical_mode_moves_as_the_horizontal_one_with_the_arrows_remapped() {
    let (text, h) = two_lines(WritingMode::HorizontalTb);
    let horizontal = validated(text, &h, WritingMode::HorizontalTb);
    for (mode, block_next, block_prev) in [
        (WritingMode::VerticalRl, Motion::Left, Motion::Right),
        (WritingMode::VerticalLr, Motion::Right, Motion::Left),
    ] {
        let (_, p) = two_lines(mode);
        let vertical = validated(text, &p, mode);
        for offset in [0, 1, 3, 4] {
            let cases = [
                (block_next, Motion::Down),
                (block_prev, Motion::Up),
                // Up and Down are the inline arrows in a vertical mode.
                (Motion::Down, Motion::Right),
                (Motion::Up, Motion::Left),
            ];
            for (physical, as_horizontal) in cases {
                assert_eq!(
                    vertical.move_cursor(at(offset), physical, None),
                    horizontal.move_cursor(at(offset), as_horizontal, None),
                    "{mode:?}: {physical:?} at {offset}"
                );
            }
        }
    }
}

#[test]
fn int13_int19_the_logical_and_word_motions_do_not_depend_on_the_writing_mode() {
    let text = "ab cd";
    let wrapped = lay_out(text, &column(20.0), &LayoutOptions::default());
    let words_text = "one two three";
    let one_line = lay_out(words_text, &column(1000.0), &LayoutOptions::default());
    for mode in MODES {
        let what = format!("{mode:?}");
        let p = in_mode(&wrapped, mode);
        let l = validated(text, &p, mode);
        let moves = [
            (1, Motion::Next, CursorMove::Moved(at(2))),
            (2, Motion::Next, CursorMove::Moved(at(3))),
            (3, Motion::Previous, CursorMove::Moved(at(2))),
            (0, Motion::Previous, exit(ParagraphEdge::Start, None)),
            (5, Motion::Next, exit(ParagraphEdge::End, None)),
            (1, Motion::LineStart, CursorMove::Moved(at(0))),
            (0, Motion::LineEnd, CursorMove::Moved(upstream(2))),
            (4, Motion::LineStart, CursorMove::Moved(at(3))),
        ];
        for (offset, motion, expected) in moves {
            assert_eq!(
                l.move_cursor(at(offset), motion, None),
                expected,
                "{what}: {motion:?} at {offset}"
            );
        }

        let p = in_mode(&one_line, mode);
        let w = validated(words_text, &p, mode);
        let words = [
            (0, Motion::WordNext, CursorMove::Moved(at(3))),
            (3, Motion::WordNext, CursorMove::Moved(at(4))),
            (5, Motion::WordPrevious, CursorMove::Moved(at(4))),
            // In left-to-right text the physical word motions are the
            // logical ones.
            (5, Motion::WordRight, CursorMove::Moved(at(7))),
            (5, Motion::WordLeft, CursorMove::Moved(at(4))),
            (0, Motion::WordPrevious, exit(ParagraphEdge::Start, None)),
            (13, Motion::WordNext, exit(ParagraphEdge::End, None)),
        ];
        for (offset, motion, expected) in words {
            assert_eq!(
                w.move_cursor(at(offset), motion, None),
                expected,
                "{what}: {motion:?} at {offset}"
            );
        }
    }
}
