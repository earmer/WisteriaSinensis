//! Interactive types: caret, hit-test, movement and selection data (spec §6).
//!
//! The data types, and the queries on validated output: `hit_test`,
//! `hit_test_on_line`, `caret_geometry`, `rect_for_offset`, `move_cursor`,
//! `selection_geometry` and `range_rects`; on validated inputs, the
//! deletion ranges and the copied text; and the UTF-16 conversions.

use super::output::{AtomicFragment, CaretAffinity, CaretStop, HangSide, PositionedParagraph};
use super::tolerance::{approx_eq, tolerance};
use super::{ValidatedInputs, ValidatedLayout};
use crate::types::{AtomFlags, BidiLevel, ByteIndex, Point, Rect, TextRange, WritingMode};

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
        let is_in_left_half = x < self.x + 0.5 * self.width;
        // Even levels map the left half to the start; odd levels swap halves.
        if is_in_left_half != self.bidi_level.direction().is_rtl() {
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
        let line = (0..lines.len())
            .position(|i| flow.y < line_block(p, i).1)
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
        // Beyond an edge: the extreme stop on that side (INT-8).
        if x < first.x {
            let stop = line.extreme_caret_stop(HangSide::Left).unwrap_or(first);
            return HitResult {
                position: stop_position(stop),
                inside: false,
            };
        }
        if x > last.x {
            let stop = line.extreme_caret_stop(HangSide::Right).unwrap_or(last);
            return HitResult {
                position: stop_position(stop),
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
            position: stop_position(stop),
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
        let l = &p.lines[line];
        match motion {
            Motion::LineStart | Motion::LineEnd => {
                // The line start is on the left in LTR text, on the right in RTL.
                let side = if (motion == Motion::LineStart) != rtl {
                    HangSide::Left
                } else {
                    HangSide::Right
                };
                let stop = l
                    .extreme_caret_stop(side)
                    .expect("a validated line has caret stops");
                CursorMove::Moved(stop_position(stop))
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
                Some(n) => return CursorMove::Moved(stop_position(&stops[n])),
                None => break,
            }
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
        let entry = lines[target]
            .extreme_caret_stop(side)
            .expect("a validated line has caret stops");
        if entry.offset != current.offset {
            return CursorMove::Moved(stop_position(entry));
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

/// The caret position a stop shows.
fn stop_position(stop: &CaretStop) -> CursorPosition {
    CursorPosition {
        byte_offset: stop.offset,
        affinity: stop.affinity,
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

/// The flow-space block interval of line `line`: its line box.
fn line_block(paragraph: &PositionedParagraph, line: usize) -> (f32, f32) {
    let l = &paragraph.lines[line];
    paragraph
        .policy
        .writing_mode
        .line_bounds(l.baseline_y, l.ascent, l.descent)
}

/// A visual slot of a line: the stretch between two neighbouring caret
/// stops, with the bytes between their offsets.
struct Slot {
    x0: f32,
    x1: f32,
    lo: ByteIndex,
    hi: ByteIndex,
    level: BidiLevel,
}

fn slots(stops: &[CaretStop]) -> impl Iterator<Item = Slot> + '_ {
    stops.windows(2).map(|w| Slot {
        x0: w[0].x.min(w[1].x),
        x1: w[0].x.max(w[1].x),
        lo: w[0].offset.min(w[1].offset),
        hi: w[0].offset.max(w[1].offset),
        level: w[0].level,
    })
}

/// The inline intervals of line `line` that `range` selects (spec INT-16):
/// visually adjacent selected slots merge, a zero-width slot between two
/// selected ones bridges them, and an unselected slot breaks the merge.
fn selected_intervals(
    paragraph: &PositionedParagraph,
    line: usize,
    range: TextRange,
) -> Vec<(f32, f32, BidiLevel)> {
    let mut out = Vec::new();
    let mut open: Option<(f32, f32, BidiLevel)> = None;
    for slot in slots(&paragraph.lines[line].caret_stops) {
        if slot.lo == slot.hi {
            continue;
        }
        if range.start() <= slot.lo && slot.hi <= range.end() {
            open = Some(match open {
                Some((a, b, level)) => (a.min(slot.x0), b.max(slot.x1), level),
                None => (slot.x0, slot.x1, slot.level),
            });
        } else if let Some(done) = open.take() {
            out.push(done);
        }
    }
    out.extend(open);
    out
}

impl ValidatedLayout<'_> {
    /// The selection rectangles of `range` (spec INT-16), physical and
    /// paragraph-local, line by line in visual order. `Tight` spans the
    /// ascent and descent of the font instances and inline boxes the piece
    /// crosses, asked of `shaper` through the shaper gate; `Max` the line
    /// box, which holds the line gap and the tallest inline box; `Strut` the
    /// resolved strut. An empty range gives none, and no line-end extension
    /// is drawn.
    #[must_use]
    pub fn selection_geometry(
        &self,
        shaper: &dyn crate::shaping::TextShaper,
        range: TextRange,
        height: BoxHeightStyle,
    ) -> Vec<SelectionRect> {
        let p = self.paragraph();
        if range.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (index, line) in p.lines.iter().enumerate() {
            if line.byte_range.end() <= range.start() && !line.byte_range.is_empty() {
                continue;
            }
            if line.byte_range.start() >= range.end() {
                break;
            }
            for (x0, x1, level) in selected_intervals(p, index, range) {
                let (top, bottom) = match height {
                    BoxHeightStyle::Max => line_block(p, index),
                    BoxHeightStyle::Strut => (
                        line.baseline_y - p.policy.strut.ascent,
                        line.baseline_y + p.policy.strut.descent,
                    ),
                    BoxHeightStyle::Tight => self.tight_block(shaper, index, x0, x1, range),
                };
                out.push(SelectionRect {
                    rect: p.policy.writing_mode.flow_to_physical_rect(
                        Rect::new(x0, top, x1 - x0, (bottom - top).max(0.0)),
                        p.total_block_size,
                    ),
                    line: index,
                    level,
                    is_line_end_extension: false,
                });
            }
        }
        out
    }

    /// The rectangles of `range` over full line boxes, for placing an IME
    /// candidate window (spec INT-18).
    #[must_use]
    pub fn range_rects(&self, range: TextRange) -> Vec<Rect> {
        let p = self.paragraph();
        let mut out = Vec::new();
        if range.is_empty() {
            return out;
        }
        for index in 0..p.lines.len() {
            let (top, bottom) = line_block(p, index);
            for (x0, x1, _) in selected_intervals(p, index, range) {
                out.push(p.policy.writing_mode.flow_to_physical_rect(
                    Rect::new(x0, top, x1 - x0, bottom - top),
                    p.total_block_size,
                ));
            }
        }
        out
    }

    /// The block interval of a `Tight` piece `[x0, x1]` of line `line`: the
    /// union of the ascents and descents of the selected glyph runs' font
    /// instances (through the shaper gate) and inline boxes, else the strut.
    fn tight_block(
        &self,
        shaper: &dyn crate::shaping::TextShaper,
        line: usize,
        x0: f32,
        x1: f32,
        range: TextRange,
    ) -> (f32, f32) {
        let p = self.paragraph();
        let l = &p.lines[line];
        let (mut above, mut below) = (f32::NEG_INFINITY, f32::NEG_INFINITY);
        let eps = tolerance(x0, x1);
        let inside = |x: f32| x >= x0 - eps && x <= x1 + eps;
        for run in &l.visual_runs {
            // A glyph counts when its extent meets the piece and its bytes
            // meet the selection: part of a ligature selects its font.
            let selected = run.glyphs.iter().any(|g| {
                let (lo, hi) = g.pen_interval();
                lo < x1 - eps
                    && hi > x0 + eps
                    && g.byte_range
                        .is_some_and(|b| b.start() < range.end() && range.start() < b.end())
            });
            if !selected {
                continue;
            }
            let Ok(metrics) = super::shaper_gate::checked_font_metrics(
                shaper,
                crate::types::FontInstanceRef {
                    font: run.font,
                    font_size: run.font_size,
                    variations: &run.variations,
                },
            ) else {
                continue;
            };
            let (ascent, descent) = if p.policy.writing_mode.is_vertical() {
                (metrics.vertical_ascent, metrics.vertical_descent)
            } else {
                (metrics.ascent, metrics.descent)
            };
            above = above.max(ascent);
            below = below.max(descent);
        }
        for b in &l.visual_boxes {
            if inside(b.x) && range.contains_range(b.byte_range) {
                above = above.max(b.ascent - b.y);
                below = below.max(b.descent + b.y);
            }
        }
        if !above.is_finite() || !below.is_finite() {
            above = p.policy.strut.ascent;
            below = p.policy.strut.descent;
        }
        p.policy
            .writing_mode
            .line_bounds(l.baseline_y, above, below)
    }
}

impl ValidatedInputs<'_> {
    /// The range Backspace deletes before `offset` (spec INT-12): a whole
    /// atomic span when the unit before the offset lies in one; else a whole
    /// emoji sequence, an atom with a variation selector or an inline object;
    /// else the last code point before the offset. Empty at the start of
    /// the text; an offset past the text or off a character boundary is
    /// first moved back to one.
    #[must_use]
    pub fn delete_backward_range(&self, offset: ByteIndex) -> TextRange {
        let text = self.text();
        let ast = self.ast();
        let offset = floor_boundary(text, offset);
        if offset == 0 {
            return TextRange::empty_at(0);
        }
        if let Some(span) = ast.atomic_span_containing(offset - 1) {
            return span.byte_range;
        }
        if let Some(object) = object_holding(ast, offset - 1) {
            return object;
        }
        let k = ast.atoms.partition_point(|a| a.byte_range.end() < offset);
        if let Some(atom) = ast.atoms.get(k).filter(|a| a.byte_range.end() == offset)
            && atom
                .flags
                .intersects(AtomFlags::IS_EMOJI_SEQUENCE.union(AtomFlags::HAS_VARIATION_SELECTOR))
        {
            return atom.byte_range;
        }
        let last = text[..offset].chars().next_back().map_or(1, char::len_utf8);
        TextRange::new(offset - last, offset)
    }

    /// The range Delete removes after `offset` (spec INT-12): a whole atomic
    /// span when the unit after the offset lies in one; else the whole atom
    /// or inline object after it; else the next code point. Empty at the end
    /// of the text.
    #[must_use]
    pub fn delete_forward_range(&self, offset: ByteIndex) -> TextRange {
        let text = self.text();
        let ast = self.ast();
        let offset = floor_boundary(text, offset);
        if offset >= text.len() {
            return TextRange::empty_at(text.len());
        }
        if let Some(span) = ast.atomic_span_containing(offset) {
            return span.byte_range;
        }
        if let Some(object) = object_holding(ast, offset) {
            return object;
        }
        let k = ast.atoms.partition_point(|a| a.byte_range.start() < offset);
        if let Some(atom) = ast.atoms.get(k).filter(|a| a.byte_range.start() == offset) {
            return atom.byte_range;
        }
        let next = text[offset..].chars().next().map_or(1, char::len_utf8);
        TextRange::new(offset, offset + next)
    }

    /// The text a copy of `range` puts on the clipboard (spec INT-17): the
    /// source bytes, so no synthesized glyph is copied, with U+00AD dropped
    /// under `SoftHyphenPolicy::Strip`. The range is clamped to the text and
    /// to character boundaries.
    #[must_use]
    pub fn copied_text(&self, range: TextRange) -> String {
        let text = self.text();
        let start = floor_boundary(text, range.start());
        let end = floor_boundary(text, range.end()).max(start);
        let source = &text[start..end];
        match self.options().soft_hyphen {
            super::SoftHyphenPolicy::Keep => source.to_owned(),
            super::SoftHyphenPolicy::Strip => source.replace('\u{AD}', ""),
        }
    }
}

/// The range of the inline object holding `byte`, if any.
fn object_holding(ast: &crate::frontend::ParagraphAst, byte: ByteIndex) -> Option<TextRange> {
    let i = ast
        .items
        .partition_point(|item| item.byte_span().end <= byte);
    match ast.items.get(i)? {
        crate::frontend::WordNode::Box(crate::frontend::BoxNode::InlineObject {
            byte_range,
            ..
        }) if byte_range.contains(byte) => Some(*byte_range),
        _ => None,
    }
}

/// `offset` clamped to `text` and moved back to a character boundary.
fn floor_boundary(text: &str, offset: ByteIndex) -> ByteIndex {
    let mut at = offset.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The UTF-16 offset of byte `offset` of `text`, or `None` when the offset
/// lies past the text or inside a code point (spec INT-18).
#[must_use]
pub fn utf8_to_utf16(text: &str, offset: ByteIndex) -> Option<usize> {
    text.get(..offset)
        .map(|prefix| prefix.chars().map(char::len_utf16).sum())
}

/// The byte offset of UTF-16 offset `offset` of `text`, or `None` when it
/// lies past the text or between the two halves of a surrogate pair
/// (spec INT-18).
#[must_use]
pub fn utf16_to_utf8(text: &str, offset: usize) -> Option<ByteIndex> {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += c.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}
