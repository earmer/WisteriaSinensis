//! Cursor motion over the caret stops (spec INT-13 to INT-15).

use super::{
    Command, CursorMove, CursorPosition, FlowMotion, Motion, ParagraphEdge, ResolvedMotion,
    stop_position,
};
use crate::middleend::ValidatedLayout;
use crate::middleend::output::{CaretAffinity, CaretStop, HangSide};
use crate::middleend::tolerance::approx_eq;

impl ValidatedLayout<'_> {
    /// Moves `position` by `motion` (spec INT-13 to INT-15). Physical arrows
    /// move over the visual caret stops; `Previous` and `Next` over caret
    /// offsets in logical order; word motions over UAX #29 word boundaries;
    /// `LineStart` and `LineEnd` to the visual line's edges; block-axis
    /// arrows keep `preferred_inline` (the caret's own inline position when
    /// `None`).
    #[must_use]
    pub fn move_cursor(
        &self,
        position: CursorPosition,
        motion: Motion,
        preferred_inline: Option<f32>,
    ) -> CursorMove {
        let p = self.paragraph();
        let rtl = p.base_direction.is_rtl();
        let (line, index) = self.locate(position);
        let command = match motion.resolve(p.policy.writing_mode) {
            ResolvedMotion::Command(command) => command,
            ResolvedMotion::Flow(flow) => {
                return match flow {
                    FlowMotion::InlinePrev => self.move_visual(line, index, false),
                    FlowMotion::InlineNext => self.move_visual(line, index, true),
                    FlowMotion::BlockPrev | FlowMotion::BlockNext => {
                        let inline =
                            preferred_inline.unwrap_or_else(|| p.lines[line].caret_stops[index].x);
                        let forward = flow == FlowMotion::BlockNext;
                        match adjacent_line(line, p.lines.len(), forward) {
                            Some(target) => {
                                CursorMove::Moved(self.hit_test_on_line(target, inline).position)
                            }
                            None => CursorMove::Exited {
                                edge: ParagraphEdge::ahead(forward),
                                inline_position: Some(inline),
                            },
                        }
                    }
                };
            }
        };
        match command {
            Command::LineStart | Command::LineEnd => {
                // The line start is on the left in LTR text, on the right in RTL.
                let side = if (command == Command::LineStart) != rtl {
                    HangSide::Left
                } else {
                    HangSide::Right
                };
                CursorMove::Moved(stop_position(self.edge_caret_stop(line, side)))
            }
            Command::Previous | Command::Next => {
                self.move_logical(position.byte_offset, command == Command::Next, None)
            }
            Command::WordPrevious | Command::WordNext => {
                let words = crate::frontend::word_boundaries(self.text());
                self.move_logical(
                    position.byte_offset,
                    command == Command::WordNext,
                    Some(&words),
                )
            }
            Command::WordLeft | Command::WordRight => {
                let words = crate::frontend::word_boundaries(self.text());
                let forward = (command == Command::WordRight) != rtl;
                self.move_logical(position.byte_offset, forward, Some(&words))
            }
        }
    }

    /// One visual step left or right (spec INT-13), onto the neighbouring
    /// line past a line edge, or out of the paragraph past its first or
    /// last line.
    fn move_visual(&self, line: usize, index: usize, rightwards: bool) -> CursorMove {
        let p = self.paragraph();
        let lines = &p.lines;
        let stops = &lines[line].caret_stops;
        let current = stops[index];
        // The nearest stop at another position: the first at it when moving
        // right, the last when moving left.
        let elsewhere = |s: &&CaretStop| !approx_eq(s.x, current.x);
        let next = if rightwards {
            stops[index + 1..].iter().find(elsewhere)
        } else {
            stops[..index].iter().rev().find(elsewhere)
        };
        if let Some(stop) = next {
            return CursorMove::Moved(stop_position(stop));
        }
        // Past the edge: rightwards is forward in LTR, backward in RTL.
        let forward = rightwards != p.base_direction.is_rtl();
        let Some(target) = adjacent_line(line, lines.len(), forward) else {
            return CursorMove::Exited {
                edge: ParagraphEdge::ahead(forward),
                inline_position: None,
            };
        };
        let side = if rightwards {
            HangSide::Left
        } else {
            HangSide::Right
        };
        let target_stops = &lines[target].caret_stops;
        let entry = self.edge_caret_stop(target, side);
        if entry.offset != current.offset {
            return CursorMove::Moved(stop_position(entry));
        }
        // The same offset on both sides of a wrap: one more step.
        let at = target_stops
            .iter()
            .position(|s| s == entry)
            .unwrap_or_default();
        let next = if rightwards {
            target_stops[at + 1..]
                .iter()
                .find(|s| !approx_eq(s.x, entry.x))
        } else {
            target_stops[..at]
                .iter()
                .rev()
                .find(|s| !approx_eq(s.x, entry.x))
        };
        CursorMove::Moved(stop_position(next.unwrap_or(entry)))
    }

    /// The next or previous caret offset in logical order (spec INT-13),
    /// restricted to `boundaries` when given (word motions, INT-14); the
    /// motion exits at offset 0 and at the text end.
    fn move_logical(
        &self,
        offset: usize,
        forward: bool,
        boundaries: Option<&[usize]>,
    ) -> CursorMove {
        let text_end = self.text().len();
        if (forward && offset >= text_end) || (!forward && offset == 0) {
            return CursorMove::Exited {
                edge: ParagraphEdge::ahead(forward),
                inline_position: None,
            };
        }
        let offsets = self
            .paragraph()
            .lines
            .iter()
            .flat_map(|l| l.caret_stops.iter().map(|s| s.offset))
            .filter(|o| boundaries.is_none_or(|b| b.binary_search(o).is_ok()));
        let target = if forward {
            offsets.filter(|&o| o > offset).min()
        } else {
            offsets.filter(|&o| o < offset).max()
        };
        let target = target.unwrap_or(if forward { text_end } else { 0 });
        CursorMove::Moved(CursorPosition {
            byte_offset: target,
            affinity: CaretAffinity::Downstream,
        })
    }
}

/// The line after (`forward`) or before `line` among `line_count` lines.
fn adjacent_line(line: usize, line_count: usize, forward: bool) -> Option<usize> {
    if forward {
        Some(line + 1).filter(|&l| l < line_count)
    } else {
        line.checked_sub(1)
    }
}

impl ParagraphEdge {
    /// The edge a motion meets going forwards or backwards in logical order.
    const fn ahead(forward: bool) -> Self {
        if forward { Self::End } else { Self::Start }
    }
}
