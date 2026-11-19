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
        let Some((line, index)) = self.locate(position) else {
            // No line has a stop, so there is nowhere to move.
            return CursorMove::Moved(position);
        };
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
                        // A block step never stalls: a line whose hit snaps
                        // back to the caret (an atomic span over several
                        // lines) is crossed too.
                        let lines = std::iter::successors(
                            self.adjacent_line_with_stops(line, forward),
                            |&l| self.adjacent_line_with_stops(l, forward),
                        );
                        let moved = lines
                            .map(|target| self.hit_test_on_line(target, inline).position)
                            .find(|hit| hit.byte_offset != position.byte_offset);
                        match moved {
                            Some(hit) => CursorMove::Moved(hit),
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
                CursorMove::Moved(
                    self.edge_caret_stop(line, side)
                        .map_or(position, stop_position),
                )
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
        let entered = self
            .adjacent_line_with_stops(line, forward)
            .and_then(|l| Some((l, self.edge_caret_stop(l, entry_side(rightwards))?)));
        let Some((target, entry)) = entered else {
            return CursorMove::Exited {
                edge: ParagraphEdge::ahead(forward),
                inline_position: None,
            };
        };
        let target_stops = &lines[target].caret_stops;
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

    /// The nearest line after (`forward`) or before `line` that has caret
    /// stops: a line inside an atomic span has none and is crossed whole
    /// (CRT-1, INT-13).
    fn adjacent_line_with_stops(&self, line: usize, forward: bool) -> Option<usize> {
        let lines = &self.paragraph().lines;
        std::iter::successors(adjacent_line(line, lines.len(), forward), |&l| {
            adjacent_line(l, lines.len(), forward)
        })
        .find(|&l| lines[l].has_caret_stops())
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
        // A clamped paragraph renders up to its truncated tail (OUT-1).
        let text_end = self
            .paragraph()
            .truncated_range
            .map_or(self.text().len(), |tail| tail.start());
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

/// The edge a visual step enters a line by: its left one moving right.
fn entry_side(rightwards: bool) -> HangSide {
    if rightwards {
        HangSide::Left
    } else {
        HangSide::Right
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
