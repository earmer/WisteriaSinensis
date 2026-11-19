//! Interactive types: caret, hit-test, movement and selection data (spec §6).
//!
//! The data types, and the queries on validated output: `hit_test`,
//! `hit_test_on_line`, `caret_geometry`, `rect_for_offset`, `move_cursor`,
//! `selection_geometry` and `range_rects`; on validated inputs, the
//! deletion ranges and the copied text; and the UTF-16 conversions.

use super::output::{
    AtomicFragment, CaretAffinity, CaretStop, HangSide, PositionedLine, PositionedParagraph,
};
use super::tolerance::approx_eq;
use super::{ValidatedInputs, ValidatedLayout};
use crate::types::{AtomFlags, BidiLevel, ByteIndex, Point, Rect, TextRange, WritingMode};

mod motion;
mod selection;

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
        // The output gate stacks line boxes without overlap (LineBoxesOverlap),
        // so their bottoms ascend and the first one below the point is found
        // by bisection.
        let line = lines
            .partition_point(|l| line_box(p, l).1 <= flow.y)
            .min(lines.len() - 1);
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
}

/// The caret position a stop shows.
fn stop_position(stop: &CaretStop) -> CursorPosition {
    CursorPosition {
        byte_offset: stop.offset,
        affinity: stop.affinity,
    }
}

/// The flow-space block interval of line `line`: its line box.
fn line_block(paragraph: &PositionedParagraph, line: usize) -> (f32, f32) {
    line_box(paragraph, &paragraph.lines[line])
}

/// The flow-space block interval of `line`, a line of `paragraph`.
fn line_box(paragraph: &PositionedParagraph, line: &PositionedLine) -> (f32, f32) {
    paragraph
        .policy
        .writing_mode
        .line_bounds(line.baseline_y, line.ascent, line.descent)
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
        if let Some(unit) = anchored_unit_range(ast, offset - 1) {
            return unit;
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
        if let Some(unit) = anchored_unit_range(ast, offset) {
            return unit;
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

/// The atomic span holding `byte`, else the inline object holding it: the
/// units that editing removes whole (spec INT-12).
fn anchored_unit_range(ast: &crate::frontend::ParagraphAst, byte: ByteIndex) -> Option<TextRange> {
    ast.atomic_span_containing(byte)
        .map(|span| span.byte_range)
        .or_else(|| object_holding(ast, byte))
}

/// The range of the inline object holding `byte`, if any.
fn object_holding(ast: &crate::frontend::ParagraphAst, byte: ByteIndex) -> Option<TextRange> {
    let i = ast
        .items
        .partition_point(|item| item.byte_span().end() <= byte);
    match ast.items.get(i)? {
        crate::frontend::WordNode::Box(crate::frontend::BoxNode::InlineObject(
            crate::frontend::InlineObjectBox { byte_range, .. },
        )) if byte_range.contains(byte) => Some(*byte_range),
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
