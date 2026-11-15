//! Interactive types: caret, hit-test, movement and selection data (spec §6).
//!
//! The data types, and the queries on validated output: `hit_test`,
//! `hit_test_on_line`, `caret_geometry`, `rect_for_offset` and `move_cursor`.
//! Selection geometry, range rectangles and deletion ranges are not
//! implemented yet.

use super::ValidatedLayout;
use super::output::{AtomicFragment, CaretAffinity, CaretStop, HangSide, PositionedParagraph};
use super::tolerance::approx_eq;
use crate::types::{BidiLevel, ByteIndex, Point, Rect, WritingMode};

/// A caret location: offset plus affinity (spec §6.1).
///
/// Invariant: `byte_offset` is a grapheme atom boundary. Deletion ranges are
/// not so constrained (§6.5), but they are not carets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorPosition {
    /// Atom-aligned byte offset.
    pub byte_offset: ByteIndex,
    /// Affinity at wraps and bidi boundaries.
    pub affinity: CaretAffinity,
}

/// One visual piece of a selection (spec §6.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRect {
    /// Rectangle in paragraph coordinates.
    pub rect: Rect,
    /// Index into `PositionedParagraph::lines`.
    pub line: usize,
    /// Bidi level of the covered slots.
    pub level: BidiLevel,
    /// `true` for the optional extension drawn for a selected newline.
    pub is_line_end_extension: bool,
}

/// Result of `hit_test` (spec §6.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HitResult {
    /// Nearest caret position.
    pub position: CursorPosition,
    /// `false` when the point was clamped to the first or last line, or to a
    /// line edge (DirectWrite's `isInside` convention `[C1]`).
    pub inside: bool,
}

/// Caret rectangles for one position (spec §6.4).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretVisual {
    /// Base-direction insertion point (Pango/Android "strong" caret).
    pub primary_rect: Rect,
    /// Opposite-direction insertion point; `Some` only if its x differs from the
    /// primary's.
    pub secondary_rect: Option<Rect>,
}

/// A cursor movement request (spec §6.5). `Left`/`Right` are physical and
/// visual; `Previous`/`Next` are logical, because Firefox's word commands mapped
/// to the wrong direction in RTL text `[C2]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Motion {
    /// One visual stop to the left.
    Left,
    /// One visual stop to the right.
    Right,
    /// One atom backwards in logical order.
    Previous,
    /// One atom forwards in logical order.
    Next,
    /// Previous UAX #29 word boundary, physically to the left.
    WordLeft,
    /// Next UAX #29 word boundary, physically to the right.
    WordRight,
    /// Previous UAX #29 word boundary in logical order.
    WordPrevious,
    /// Next UAX #29 word boundary in logical order.
    WordNext,
    /// Start of the visual line.
    LineStart,
    /// End of the visual line.
    LineEnd,
    /// One physical step up: a block-axis move in horizontal modes, an
    /// inline-axis one in vertical modes ([`Motion::flow_direction`]).
    Up,
    /// One physical step down: a block-axis move in horizontal modes, an
    /// inline-axis one in vertical modes ([`Motion::flow_direction`]).
    Down,
}

/// The edge through which a motion leaves a paragraph, in logical (document)
/// order (spec INT-13, INT-15).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParagraphEdge {
    /// Towards the previous paragraph.
    Start,
    /// Towards the next paragraph.
    End,
}

/// The result of a cursor motion (spec §6.5).
///
/// A paragraph does not know its neighbours, so a motion that would leave it
/// reports the exit instead of clamping; a caller with no neighbour on that
/// side clamps to the text start or end itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CursorMove {
    /// The caret stays in this paragraph.
    Moved(CursorPosition),
    /// The motion leaves the paragraph through `edge`.
    Exited {
        /// Which neighbour the motion continues into.
        edge: ParagraphEdge,
        /// For a block-axis motion ([`FlowMotion::BlockPrev`],
        /// [`FlowMotion::BlockNext`]): the preferred position along the inline
        /// axis, in the paragraph's flow space, at which the caller re-enters
        /// the neighbour on its nearest line (spec INT-15). `None` for any
        /// other motion.
        inline_position: Option<f32>,
    },
}

/// A selection stored logically as anchor and focus (spec §6.5). Shift+arrow
/// moves `focus` visually; the anchor stays put. `anchor == focus` is a caret.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Selection {
    /// The fixed end.
    pub anchor: CursorPosition,
    /// The moving end.
    pub focus: CursorPosition,
}

/// Height policy for selection rectangles, after Flutter's `BoxHeightStyle`
/// `[C1]` (spec §6.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoxHeightStyle {
    /// Union of selected font-instance ascent/descent, excluding gap and leading.
    ///
    /// **Forward contract.** Selection geometry is not computed yet; when it
    /// is, it must query `TextShaper::font_metrics` for every
    /// selected instance and union their ascent/descent.
    #[default]
    Tight,
    /// Includes line gap and maximum inline-box height.
    Max,
    /// The paragraph's resolved strut (`ParagraphAst::strut_metrics` after `LineHeight::resolve_strut`).
    Strut,
}

impl AtomicFragment {
    /// Snaps to the entire span's outer endpoints by midpoint rule.
    /// Even level: left half → start/Downstream, right half → end/Upstream.
    /// Odd level: halves swap. Midpoint itself counts as right half.
    pub fn hit_test(&self, x: f32) -> CursorPosition {
        let midpoint = self.x + 0.5 * self.width;
        let is_in_left_half = x < midpoint;
        let is_even_level = !self.bidi_level.direction().is_rtl();
        // Even levels map the left half to the start; odd levels swap halves.
        let snaps_to_start = is_in_left_half == is_even_level;
        if snaps_to_start {
            CursorPosition {
                byte_offset: self.full_byte_range.start(),
                affinity: CaretAffinity::Downstream,
            }
        } else {
            CursorPosition {
                byte_offset: self.full_byte_range.end(),
                affinity: CaretAffinity::Upstream,
            }
        }
    }
}

/// The shaper gate for ligature carets (spec INT-5, AX-3): accepts the
/// shaper's GDEF subdivisions when usable and otherwise falls back to K equal
/// atom slots. A cluster of one atom, or of zero width, has no sub-slots.
///
/// # Precondition
/// `width` is the width of a validated cluster: a non-negative measurement
/// (spec MSR-1). It is not checked again.
pub fn resolve_ligature_carets(width: f32, atom_count: usize, carets: &[f32]) -> Vec<f32> {
    debug_assert!(
        crate::types::is_extent(width),
        "resolve_ligature_carets precondition violated (spec MSR-1)"
    );
    if atom_count <= 1 || width == 0.0 {
        return Vec::new();
    }

    if carets_are_usable(width, atom_count, carets) {
        carets.to_vec()
    } else {
        equal_caret_slots(width, atom_count)
    }
}

fn carets_are_usable(width: f32, atom_count: usize, carets: &[f32]) -> bool {
    if carets.len() != atom_count - 1 {
        return false;
    }
    let within_width = carets
        .iter()
        .all(|position| position.is_finite() && *position > 0.0 && *position < width);
    let strictly_increasing = carets.windows(2).all(|pair| pair[0] < pair[1]);
    within_width && strictly_increasing
}

/// K equal slots. The index is scaled before multiplying so a large finite
/// width cannot overflow the intermediate.
fn equal_caret_slots(width: f32, atom_count: usize) -> Vec<f32> {
    (1..atom_count)
        .map(|index| width * (index as f32 / atom_count as f32))
        .collect()
}

/// A physical direction resolved once at the interaction boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FlowMotion {
    /// Previous visual inline stop.
    InlinePrev,
    /// Next visual inline stop.
    InlineNext,
    /// Previous block-axis line.
    BlockPrev,
    /// Next block-axis line.
    BlockNext,
}

impl Motion {
    /// Resolves the four physical arrows; logical and word commands return `None`.
    #[must_use]
    pub const fn flow_direction(self, mode: WritingMode) -> Option<FlowMotion> {
        use FlowMotion::*;
        use WritingMode::*;
        match (mode, self) {
            (HorizontalTb, Self::Left) | (VerticalRl | VerticalLr, Self::Up) => Some(InlinePrev),
            (HorizontalTb, Self::Right) | (VerticalRl | VerticalLr, Self::Down) => Some(InlineNext),
            (HorizontalTb, Self::Up) | (VerticalRl, Self::Right) | (VerticalLr, Self::Left) => {
                Some(BlockPrev)
            }
            (HorizontalTb, Self::Down) | (VerticalRl, Self::Left) | (VerticalLr, Self::Right) => {
                Some(BlockNext)
            }
            _ => None,
        }
    }
}

/// The queries of §6 for laid-out, validated output (spec INT-1 to INT-15,
/// INT-18 `rect_for_offset`). Points and rectangles are paragraph-local and
/// physical (HST-8); inline positions are in flow space.
impl ValidatedLayout<'_> {
    /// The caret position nearest `point` (spec INT-7, INT-8): the point is
    /// clamped to the first or last line, then placed on that line.
    /// `inside` is `false` when the point was clamped to a line or to a line
    /// edge.
    #[must_use]
    pub fn hit_test(&self, point: Point) -> HitResult {
        let p = self.paragraph();
        let flow = p
            .policy
            .writing_mode
            .physical_to_flow_point(point, p.total_block_size);
        let lines = &p.lines;
        let (top, _) = line_block(p, 0);
        let (_, bottom) = line_block(p, lines.len() - 1);
        let line = lines
            .iter()
            .enumerate()
            .position(|(i, _)| flow.y < line_block(p, i).1)
            .unwrap_or(lines.len() - 1);
        let hit = self.hit_test_on_line(line, flow.x);
        HitResult {
            position: hit.position,
            inside: hit.inside && flow.y >= top && flow.y < bottom,
        }
    }

    /// The caret position on line `line` nearest the flow-space inline
    /// position `inline_position` (spec INT-7 to INT-10, INT-15). A line
    /// index past the last line is clamped to it.
    #[must_use]
    pub fn hit_test_on_line(&self, line: usize, inline_position: f32) -> HitResult {
        let lines = &self.paragraph().lines;
        let line = &lines[line.min(lines.len() - 1)];
        let x = inline_position;
        // Atomic spans snap to an end (INT-10).
        if let Some(fragment) = line
            .atomic_fragments
            .iter()
            .find(|f| f.x <= x && x <= f.x + f.width)
        {
            return HitResult {
                position: fragment.hit_test(x),
                inside: true,
            };
        }
        let stops = &line.caret_stops;
        let (Some(first), Some(last)) = (stops.first(), stops.last()) else {
            return HitResult {
                position: CursorPosition {
                    byte_offset: line.byte_range.start(),
                    affinity: CaretAffinity::Downstream,
                },
                inside: false,
            };
        };
        let to_position = |s: &CaretStop| CursorPosition {
            byte_offset: s.offset,
            affinity: s.affinity,
        };
        // Beyond an edge: the extreme stop on that side (INT-8).
        if x < first.x {
            let stop = line.extreme_caret_stop(HangSide::Left).unwrap_or(first);
            return HitResult {
                position: to_position(stop),
                inside: false,
            };
        }
        if x > last.x {
            let stop = line.extreme_caret_stop(HangSide::Right).unwrap_or(last);
            return HitResult {
                position: to_position(stop),
                inside: false,
            };
        }
        // The nearest stop; the midpoint belongs to the right half, and of
        // two stops at one x the one on the side of the point wins.
        let right = stops.partition_point(|s| s.x <= x).min(stops.len() - 1);
        let left = right.saturating_sub(1);
        let (l, r) = (&stops[left], &stops[right]);
        let stop = if r.x - x <= x - l.x || r.x <= x {
            // Approaching from the left: the first stop at that x.
            let at = stops.partition_point(|s| s.x < r.x);
            &stops[at.min(right)]
        } else {
            // From the right: the last stop at the left one's x.
            let at = stops.partition_point(|s| s.x <= l.x);
            &stops[at.saturating_sub(1)]
        };
        HitResult {
            position: to_position(stop),
            inside: true,
        }
    }

    /// The caret rectangles of `position` (spec INT-11): zero-width
    /// rectangles spanning the line box, physical and paragraph-local.
    #[must_use]
    pub fn caret_geometry(&self, position: CursorPosition) -> CaretVisual {
        let p = self.paragraph();
        let (line, index) = self.locate(position);
        let stops = &p.lines[line].caret_stops;
        let (top, bottom) = line_block(p, line);
        let rect = |x: f32| {
            p.policy
                .writing_mode
                .flow_to_physical_rect(Rect::new(x, top, 0.0, bottom - top), p.total_block_size)
        };
        let primary = stops[index];
        let secondary = stops
            .iter()
            .find(|s| s.offset == primary.offset && !approx_eq(s.x, primary.x))
            .map(|s| rect(s.x));
        CaretVisual {
            primary_rect: rect(primary.x),
            secondary_rect: secondary,
        }
    }

    /// The caret rectangle of `offset`, downstream (spec INT-18).
    #[must_use]
    pub fn rect_for_offset(&self, offset: ByteIndex) -> Rect {
        self.caret_geometry(CursorPosition {
            byte_offset: offset,
            affinity: CaretAffinity::Downstream,
        })
        .primary_rect
    }

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
        if let Some(flow) = motion.flow_direction(p.policy.writing_mode) {
            return match flow {
                FlowMotion::InlinePrev => self.move_visual(line, index, false),
                FlowMotion::InlineNext => self.move_visual(line, index, true),
                FlowMotion::BlockPrev | FlowMotion::BlockNext => {
                    let inline =
                        preferred_inline.unwrap_or_else(|| p.lines[line].caret_stops[index].x);
                    let target = if flow == FlowMotion::BlockPrev {
                        line.checked_sub(1)
                    } else {
                        Some(line + 1).filter(|&l| l < p.lines.len())
                    };
                    match target {
                        Some(target) => {
                            CursorMove::Moved(self.hit_test_on_line(target, inline).position)
                        }
                        None => CursorMove::Exited {
                            edge: if flow == FlowMotion::BlockPrev {
                                ParagraphEdge::Start
                            } else {
                                ParagraphEdge::End
                            },
                            inline_position: Some(inline),
                        },
                    }
                }
            };
        }
        let l = &p.lines[line];
        let to_position = |s: &CaretStop| CursorPosition {
            byte_offset: s.offset,
            affinity: s.affinity,
        };
        match motion {
            Motion::LineStart | Motion::LineEnd => {
                let start_side = if rtl { HangSide::Right } else { HangSide::Left };
                let side = if motion == Motion::LineStart {
                    start_side
                } else if rtl {
                    HangSide::Left
                } else {
                    HangSide::Right
                };
                let stop = l
                    .extreme_caret_stop(side)
                    .expect("a validated line has caret stops");
                CursorMove::Moved(to_position(stop))
            }
            Motion::Previous | Motion::Next => {
                self.move_logical(position.byte_offset, motion == Motion::Next, None)
            }
            Motion::WordPrevious | Motion::WordNext => {
                let words = crate::frontend::word_boundaries(self.text());
                self.move_logical(
                    position.byte_offset,
                    motion == Motion::WordNext,
                    Some(&words),
                )
            }
            Motion::WordLeft | Motion::WordRight => {
                let words = crate::frontend::word_boundaries(self.text());
                let forward = (motion == Motion::WordRight) != rtl;
                self.move_logical(position.byte_offset, forward, Some(&words))
            }
            _ => unreachable!("physical arrows resolved above"),
        }
    }

    /// The line and caret stop that show `position`: a stop with its offset
    /// and affinity, else one with its offset, else the nearest offset
    /// before it on the line that holds it.
    fn locate(&self, position: CursorPosition) -> (usize, usize) {
        let lines = &self.paragraph().lines;
        let offset = position.byte_offset;
        let mut fallback = None;
        for (i, line) in lines.iter().enumerate() {
            for (k, stop) in line.caret_stops.iter().enumerate() {
                if stop.offset == offset {
                    if stop.affinity == position.affinity {
                        return (i, k);
                    }
                    fallback.get_or_insert((i, k));
                }
            }
        }
        if let Some(found) = fallback {
            return found;
        }
        let line = lines
            .iter()
            .position(|l| offset < l.byte_range.end())
            .unwrap_or(lines.len() - 1);
        let stops = &lines[line].caret_stops;
        let k = stops
            .iter()
            .enumerate()
            .filter(|(_, s)| s.offset <= offset)
            .max_by_key(|(_, s)| s.offset)
            .or_else(|| stops.iter().enumerate().min_by_key(|(_, s)| s.offset))
            .map_or(0, |(k, _)| k);
        (line, k)
    }

    /// One visual stop left or right (spec INT-13), onto the neighbouring
    /// line past a line edge, or out of the paragraph past its first or
    /// last line.
    fn move_visual(&self, line: usize, index: usize, rightwards: bool) -> CursorMove {
        let p = self.paragraph();
        let lines = &p.lines;
        let current = lines[line].caret_stops[index];
        let to_position = |s: &CaretStop| CursorPosition {
            byte_offset: s.offset,
            affinity: s.affinity,
        };
        let stops = &lines[line].caret_stops;
        let mut k = index;
        loop {
            let next = if rightwards {
                k.checked_add(1).filter(|&n| n < stops.len())
            } else {
                k.checked_sub(1)
            };
            match next {
                Some(n) if stops[n].offset == current.offset => k = n,
                Some(n) => return CursorMove::Moved(to_position(&stops[n])),
                None => break,
            }
        }
        // Past the edge: rightwards is forward in LTR, backward in RTL.
        let forward = rightwards != p.base_direction.is_rtl();
        let target = if forward {
            Some(line + 1).filter(|&l| l < lines.len())
        } else {
            line.checked_sub(1)
        };
        let Some(target) = target else {
            return CursorMove::Exited {
                edge: if forward {
                    ParagraphEdge::End
                } else {
                    ParagraphEdge::Start
                },
                inline_position: None,
            };
        };
        let side = if rightwards {
            HangSide::Left
        } else {
            HangSide::Right
        };
        let target_stops = &lines[target].caret_stops;
        let entry = lines[target]
            .extreme_caret_stop(side)
            .expect("a validated line has caret stops");
        if entry.offset != current.offset {
            return CursorMove::Moved(to_position(entry));
        }
        // The same offset on both sides of a wrap: one more step.
        let at = target_stops
            .iter()
            .position(|s| s == entry)
            .unwrap_or_default();
        let next = if rightwards {
            target_stops.get(at + 1)
        } else {
            at.checked_sub(1).and_then(|n| target_stops.get(n))
        };
        CursorMove::Moved(to_position(next.unwrap_or(entry)))
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
                edge: if forward {
                    ParagraphEdge::End
                } else {
                    ParagraphEdge::Start
                },
                inline_position: None,
            };
        }
        let mut offsets: Vec<usize> = self
            .paragraph()
            .lines
            .iter()
            .flat_map(|l| l.caret_stops.iter().map(|s| s.offset))
            .filter(|o| boundaries.is_none_or(|b| b.binary_search(o).is_ok()))
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        let target = if forward {
            offsets.iter().copied().find(|&o| o > offset)
        } else {
            offsets.iter().rev().copied().find(|&o| o < offset)
        };
        let target = target.unwrap_or(if forward { text_end } else { 0 });
        CursorMove::Moved(CursorPosition {
            byte_offset: target,
            affinity: CaretAffinity::Downstream,
        })
    }
}

/// The flow-space block interval of line `line`: its line box.
fn line_block(paragraph: &PositionedParagraph, line: usize) -> (f32, f32) {
    let l = &paragraph.lines[line];
    let above = if paragraph.policy.writing_mode == WritingMode::VerticalLr {
        l.descent
    } else {
        l.ascent
    };
    let top = l.baseline_y - above;
    (top, top + l.line_height)
}
