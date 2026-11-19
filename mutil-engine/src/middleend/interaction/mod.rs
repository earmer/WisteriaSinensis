//! Interactive types: caret, hit-test, movement and selection data (spec §6).
//!
//! The data types, and the queries on validated output: `hit_test`,
//! `hit_test_on_line`, `caret_geometry`, `rect_for_offset`, `move_cursor`,
//! `selection_geometry` and `range_rects`; on validated inputs, the
//! deletion ranges and the copied text; and the UTF-16 conversions.

use super::output::{
    AtomicFragment, CaretAffinity, CaretStop, PositionedLine, PositionedParagraph,
};
use super::tolerance::approx_eq;
use super::validated::{LocatedCaret, ValidatedLine};
use super::{ValidatedInputs, ValidatedLayout};
use crate::types::{BidiLevel, ByteIndex, Point, Rect, WritingMode};

mod editing;
mod motion;
mod selection;

pub use editing::{utf8_to_utf16, utf16_to_utf8};

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
    /// Union of selected font-instance ascent/descent, excluding gap and
    /// leading (`TextShaper::font_metrics` of every selected instance).
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
    let strictly_increasing = carets.is_sorted_by(|a, b| a < b);
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

/// A [`Motion`] after the physical arrows are resolved for a writing mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResolvedMotion {
    /// A physical arrow, as a direction in flow space.
    Flow(FlowMotion),
    /// A logical, word or line-edge command, unchanged.
    Command(Command),
}

/// The motions that need no writing mode to resolve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Previous,
    Next,
    WordLeft,
    WordRight,
    WordPrevious,
    WordNext,
    LineStart,
    LineEnd,
}

impl Motion {
    /// Resolves the four physical arrows; logical and word commands return `None`.
    #[must_use]
    pub const fn flow_direction(self, mode: WritingMode) -> Option<FlowMotion> {
        match self.resolve(mode) {
            ResolvedMotion::Flow(flow) => Some(flow),
            ResolvedMotion::Command(_) => None,
        }
    }

    /// Resolves the physical arrows against `mode`; every other motion is a
    /// [`Command`].
    const fn resolve(self, mode: WritingMode) -> ResolvedMotion {
        use FlowMotion::*;
        use ResolvedMotion::{Command as Cmd, Flow};
        use WritingMode::*;
        match self {
            Self::Left => Flow(match mode {
                HorizontalTb => InlinePrev,
                VerticalRl => BlockNext,
                VerticalLr => BlockPrev,
            }),
            Self::Right => Flow(match mode {
                HorizontalTb => InlineNext,
                VerticalRl => BlockPrev,
                VerticalLr => BlockNext,
            }),
            Self::Up => Flow(match mode {
                HorizontalTb => BlockPrev,
                VerticalRl | VerticalLr => InlinePrev,
            }),
            Self::Down => Flow(match mode {
                HorizontalTb => BlockNext,
                VerticalRl | VerticalLr => InlineNext,
            }),
            Self::Previous => Cmd(Command::Previous),
            Self::Next => Cmd(Command::Next),
            Self::WordLeft => Cmd(Command::WordLeft),
            Self::WordRight => Cmd(Command::WordRight),
            Self::WordPrevious => Cmd(Command::WordPrevious),
            Self::WordNext => Cmd(Command::WordNext),
            Self::LineStart => Cmd(Command::LineStart),
            Self::LineEnd => Cmd(Command::LineEnd),
        }
    }
}

/// The queries of §6 for laid-out, validated output (spec INT-1 to INT-15,
/// INT-18 `rect_for_offset`). Points and rectangles are paragraph-local and
/// physical (HST-8); inline positions are in flow space.
impl<'a> ValidatedLayout<'a> {
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
        let lines = self.lines();
        let (top, _) = line_box(p, lines.first());
        let (_, bottom) = line_box(p, lines.last());
        // The output gate stacks line boxes without overlap (LineBoxesOverlap),
        // so their bottoms ascend and the first one below the point is found
        // by bisection.
        let line = lines.partition_point_clamped(|l| line_box(p, l).1 <= flow.y);
        let hit = hit_on_line(self.line(line), flow.x);
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
        hit_on_line(self.line(line), inline_position)
    }

    /// The caret rectangles of `position` (spec INT-11): zero-width
    /// rectangles spanning the line box, physical and paragraph-local.
    #[must_use]
    pub fn caret_geometry(&self, position: CursorPosition) -> CaretVisual {
        let p = self.paragraph();
        let rect_on = |line: &PositionedLine, x: f32| {
            let (top, bottom) = line_box(p, line);
            p.policy
                .writing_mode
                .flow_to_physical_rect(Rect::new(x, top, 0.0, bottom - top), p.total_block_size)
        };
        let Some(located) = self.locate(position) else {
            // No line has a stop: the caret stands at the first line's start.
            let first = self.line(0);
            return CaretVisual {
                primary_rect: rect_on(first.line, first.first_slot().content_start),
                secondary_rect: None,
            };
        };
        let rect = |x: f32| rect_on(located.line.line, x);
        let primary = located.caret();
        let secondary = located
            .line
            .stops
            .iter()
            .find(|s| {
                s.offset == primary.offset
                    && s.affinity != primary.affinity
                    && !approx_eq(s.x, primary.x)
            })
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

    /// The line and caret stop that show `position`: a stop with its offset
    /// and affinity, else one with its offset, else the nearest offset
    /// before it on the line that holds it. A line may have no stops (CRT-1),
    /// as one inside an atomic span: the nearest earlier line with stops
    /// shows it, else the nearest later one. `None` when no line has stops.
    ///
    /// G4 proved that the lines tile the text in order (`SourceNotCovered`)
    /// and that every stop lies in its line's range, ends included
    /// (`InvalidCaretStop`). So the lines that can hold a stop at the offset
    /// are one run, found by bisection: O(log L) plus the stops of those
    /// lines, usually one or two.
    pub(crate) fn locate(&self, position: CursorPosition) -> Option<LocatedCaret<'a>> {
        let offset = position.byte_offset;
        let first = self
            .lines()
            .partition_point(|l| l.byte_range.end() < offset);
        let candidates = self
            .walk_lines(first, true)
            .take_while(|l| l.line.byte_range.start() <= offset)
            .filter_map(ValidatedLine::with_stops);
        let mut fallback = None;
        for line in candidates {
            for (stop, s) in line.stops.indexed() {
                if s.offset == offset {
                    if s.affinity == position.affinity {
                        return Some(LocatedCaret { line, stop });
                    }
                    fallback.get_or_insert(LocatedCaret { line, stop });
                }
            }
        }
        fallback.or_else(|| self.locate_nearest(offset))
    }

    /// The stop nearest `offset` when no stop has it: on the line that
    /// holds the offset, else the nearest earlier line with stops, else the
    /// nearest later one, the stop at the nearest offset before it, else
    /// the first stop with the least offset.
    fn locate_nearest(&self, offset: ByteIndex) -> Option<LocatedCaret<'a>> {
        // Line ends ascend (`SourceNotCovered`), so the holder is bisected.
        let holder = self
            .lines()
            .partition_point_clamped(|l| l.byte_range.end() <= offset);
        let line = self
            .walk_lines(holder, false)
            .find_map(ValidatedLine::with_stops)
            .or_else(|| self.adjacent_line_with_stops(self.line(holder).index, true))?;
        let (stop, _) = line
            .stops
            .indexed()
            .filter(|(_, s)| s.offset <= offset)
            .max_by_key(|(_, s)| s.offset)
            .unwrap_or_else(|| line.stops.min_by_offset());
        Some(LocatedCaret { line, stop })
    }
}

/// The caret position on `line` nearest the flow-space inline position `x`
/// (spec INT-7 to INT-10).
fn hit_on_line(line: ValidatedLine<'_>, x: f32) -> HitResult {
    let ValidatedLine { line, stops, .. } = line;
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
    let Some(stops) = stops else {
        // A line may have no stops, as one wholly inside an atomic span
        // (CRT-1): it snaps to the span's ends.
        let position = line.atomic_fragments.first().map_or(
            CursorPosition {
                byte_offset: line.byte_range.start(),
                affinity: CaretAffinity::Downstream,
            },
            |fragment| fragment.hit_test(x),
        );
        return HitResult {
            position,
            inside: false,
        };
    };
    let (first, last) = (stops.first(), stops.last());
    // At or beyond an edge: the extreme stop on that side (INT-8). The
    // output gate proves the stops finite and in non-decreasing `x`, so
    // `first` and `last` are the extreme stops with their ties resolved
    // by visual order. A position that is not a number lies beyond the
    // left edge, never inside.
    if x.is_nan() || x <= first.x {
        return HitResult {
            position: stop_position(first),
            inside: x == first.x,
        };
    }
    if x >= last.x {
        return HitResult {
            position: stop_position(last),
            inside: x == last.x,
        };
    }
    HitResult {
        position: stop_position(stops.nearest_in_bounds(x)),
        inside: true,
    }
}

/// The caret position a stop shows.
fn stop_position(stop: &CaretStop) -> CursorPosition {
    CursorPosition {
        byte_offset: stop.offset,
        affinity: stop.affinity,
    }
}

/// The flow-space block interval of `line`, a line of `paragraph`.
fn line_box(paragraph: &PositionedParagraph, line: &PositionedLine) -> (f32, f32) {
    paragraph
        .policy
        .writing_mode
        .line_bounds(line.baseline_y, line.ascent, line.descent)
}
