//! The mid-end entry point (spec §5).
//!
//! **Phase 0 skeleton:** it assembles the validation context and checks its
//! inputs, but lays nothing out.

use super::{LayoutConstraints, LayoutOptions, LineContext, PositionedParagraph};
use crate::frontend::MAX_DECORATION_NESTING_DEPTH;
use crate::frontend::{
    BoxDecorationBreak, BoxNode, BreakAction, DiscretionaryNode, GlueKind, GlueTier,
    HyphenOpportunity, ParagraphAst, SpanEdge, WordNode,
};
use crate::shaping::TextShaper;
use crate::types::{AtomFlags, BIDI_MAX_DEPTH, BidiLevel, GraphemeAtom, TextRange};
use core::fmt;

/// An input that [`check_layout_inputs`] rejects.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayoutInputError {
    /// `wrap_width` is negative or not finite, or the tab step is not a finite
    /// positive number.
    InvalidConstraints,
    /// The raw strut metrics, the strut font size or a `LineHeight::Fixed`
    /// multiplier is not finite (or the size is negative).
    InvalidStrut,
    /// A run's `byte_range` exceeds `text` or splits a UTF-8 sequence.
    RunOutsideText {
        /// Index into `ParagraphAst::runs`.
        run: usize,
    },
    /// An item's `run_index` is not a valid index into `ParagraphAst::runs`.
    RunIndexOutOfRange {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An item's bytes exceed `text` or split a UTF-8 sequence.
    ItemOutsideText {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A text box (or one of its atoms) is not contained in the byte range of
    /// the shaping run it names.
    ItemOutsideRun {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A text box's cached `natural_width` is not finite, or differs from the
    /// sum of its cluster widths.
    InvalidTextWidth {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A `BoxNode::Text` violates `WordNode::is_self_contained`.
    ItemNotSelfContained {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An inline object has a non-finite or negative width, ascent or descent.
    InvalidInlineBox {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A glue node has a non-finite or negative width, stretch or shrink, is
    /// `Tier2` (derived from cluster flags, never emitted), or a
    /// `CjkPunctCompression` node that is not `Tier0` with zero natural width
    /// and zero stretch.
    InvalidGlue {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A discretionary node is malformed:
    /// * a non-finite width, a forbidden penalty (`>= FORBIDDEN_BREAK_PENALTY`;
    ///   a forbidden break is an omitted node), or a `HardBreak` that disagrees
    ///   with a mandatory penalty;
    /// * a `byte_range` that breaks the ownership rule (soft: empty; real hard
    ///   break: exactly a line terminator; sentinel: `text.len()..text.len()`,
    ///   the last discretionary, at most once);
    /// * a repeated or incomplete `cloned_spans`;
    /// * break widths that do not match the cloned padding: `Direct`, a real
    ///   `HardBreak` and `MathOperator { repeat_glyph_id: None }` must equal the
    ///   clone sums, and `CjkPunctAdjust` must carry a real net delta whose
    ///   neighbouring cluster stays non-negative.
    InvalidDiscretionary {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A hyphenation discretionary is not `flagged`, does not sit between two
    /// non-empty text boxes at its `byte_offset`, disagrees with its
    /// `HyphenOpportunity`, or has a cut that leaves its same-run hyphen chain
    /// or ends off an atom boundary.
    InvalidHyphenOpportunity {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A span boundary is not an empty range, has a non-finite or negative
    /// width, not at its span's `byte_range.start` (`Leading`) or `.end`
    /// (`Trailing`), a `bidi_level` that differs from the level of the first (for
    /// a `Leading` edge) or last (`Trailing`) atom inside its span, or an order
    /// that breaks LIFO nesting (a `Trailing` edge closing a span that is not the
    /// innermost open one, a repeated `Leading`, or, for two spans with equal
    /// ranges, the higher-index span not nested inside the lower-index one).
    InvalidSpanBoundary {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An item names a span (`SpanBoundary`, or an entry of
    /// `DiscretionaryNode::cloned_spans`, which must also be
    /// `BoxDecorationBreak::Clone` and contain the break strictly inside) that
    /// `decoration_spans` lacks.
    UnknownSpan {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A decoration span has an invalid padding or range, does not fit the text,
    /// repeats a `span_id`, partially overlaps another decoration span (nesting
    /// must be proper containment; equal ranges are allowed, the lower index
    /// being the outer span) or nests deeper than `MAX_DECORATION_NESTING_DEPTH`.
    InvalidDecorationSpan {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
    },
    /// `ParagraphAst::items` does not end with the paragraph-end sentinel
    /// `HardBreak { is_paragraph_end: true }` (an empty paragraph included).
    MissingParagraphEnd,
    /// A decoration span lacks its `Leading` boundary at `byte_range.start` or
    /// its `Trailing` boundary at `byte_range.end` in `items` (both are
    /// mandatory, even with zero padding).
    MissingSpanBoundary {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
    },
    /// An item starts before the previous item ends: `items` must progress
    /// monotonically through the text (`prev.end <= next.start`).
    ItemsOutOfOrder {
        /// Index into `ParagraphAst::items` of the later item.
        item: usize,
    },
    /// An item is on the wrong side of a decoration span's boundaries: an item
    /// inside `byte_range` is not strictly between the `Leading` and `Trailing`
    /// nodes, an item outside it is between them, a break opportunity at the
    /// span's start is not before the `Leading` node (or one at its end not
    /// after the `Trailing` node), or an item straddles a span edge.
    SpanEnclosure {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A text box holds an atom whose `AtomFlags::IN_ATOMIC_EDIT_SPAN` disagrees
    /// with `atomic_spans`: the flag is missing on an atom inside an atomic span,
    /// or set on one outside every atomic span.
    AtomicFlagMismatch {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An atomic span is empty, does not fit the text, repeats a `span_id`,
    /// overlaps or nests with another atomic span, or has an edge that is not
    /// the start or end of a text atom or of an inline object (an edge inside
    /// glue, a tab or a line terminator, or on an unowned offset, is rejected),
    /// or wraps no atom or object at all.
    InvalidAtomicSpan {
        /// Index into `ParagraphAst::atomic_spans`.
        span: usize,
    },
}

impl fmt::Display for LayoutInputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConstraints => f.write_str("invalid wrap width or tab stops"),
            Self::InvalidStrut => f.write_str("invalid strut metrics, font size or line height"),
            Self::RunOutsideText { run } => write!(f, "run {run} does not fit the text"),
            Self::RunIndexOutOfRange { item } => {
                write!(f, "item {item} refers to a run that does not exist")
            }
            Self::ItemOutsideText { item } => write!(f, "item {item} does not fit the text"),
            Self::ItemOutsideRun { item } => {
                write!(f, "item {item} lies outside the shaping run it names")
            }
            Self::InvalidTextWidth { item } => {
                write!(f, "item {item} has a width that is not its clusters' total")
            }
            Self::ItemNotSelfContained { item } => {
                write!(f, "item {item} does not partition its atoms")
            }
            Self::InvalidInlineBox { item } => {
                write!(f, "inline box {item} has a non-finite or negative size")
            }
            Self::InvalidGlue { item } => {
                write!(f, "glue {item} has a non-finite or negative size")
            }
            Self::InvalidDiscretionary { item } => {
                write!(f, "discretionary {item} has an invalid penalty or width")
            }
            Self::InvalidHyphenOpportunity { item } => {
                write!(f, "item {item} has a hyphenation opportunity off its atoms")
            }
            Self::InvalidSpanBoundary { item } => {
                write!(
                    f,
                    "span boundary {item} is not empty or has an invalid width"
                )
            }
            Self::UnknownSpan { item } => {
                write!(f, "item {item} refers to a span that does not exist")
            }
            Self::InvalidDecorationSpan { span } => {
                write!(f, "decoration span {span} is invalid or duplicated")
            }
            Self::ItemsOutOfOrder { item } => {
                write!(f, "item {item} starts before the previous item ends")
            }
            Self::SpanEnclosure { span, item } => {
                write!(
                    f,
                    "item {item} is misplaced relative to the boundaries of decoration span {span}"
                )
            }
            Self::MissingParagraphEnd => {
                f.write_str("the items do not end with the paragraph-end sentinel")
            }
            Self::MissingSpanBoundary { span } => {
                write!(
                    f,
                    "decoration span {span} lacks a leading or trailing boundary"
                )
            }
            Self::AtomicFlagMismatch { item } => {
                write!(f, "item {item} has an atom whose atomic-span flag is wrong")
            }
            Self::InvalidAtomicSpan { span } => {
                write!(f, "atomic span {span} is invalid or duplicated")
            }
        }
    }
}

impl std::error::Error for LayoutInputError {}

/// The text box at `items[index]`: `(run_index, byte_range, atoms)`.
fn text_at(
    items: &[WordNode],
    index: Option<usize>,
) -> Option<(usize, TextRange, &[GraphemeAtom])> {
    match index.and_then(|i| items.get(i)) {
        Some(WordNode::Box(BoxNode::Text {
            run_index,
            byte_range,
            atoms,
            ..
        })) => Some((*run_index, *byte_range, atoms.as_slice())),
        _ => None,
    }
}

/// The atom boundaries a hyphenation cut may land on, on one side of the
/// opportunity at `items[index]`: those of the **contiguous same-run word
/// chain** that starts at the neighbouring text box (`items[index - 1]`, going
/// backwards, or `items[index + 1]`, going forwards) and continues through
/// further text boxes of the **same `run_index`** joined solely by
/// `BreakAction::Hyphenate` discretionaries that meet the boxes end to end. The
/// chain stops at glue, a tab, an inline object, a span boundary, a run change
/// or any other discretionary: `TextShaper::reshape_window` takes one
/// `ShapingRun`, and a box cannot straddle a span boundary.
///
/// Returns the chain's far byte bound (its start going backwards, its end going
/// forwards) and its atom boundaries (starts and ends), or `None` when there is
/// no neighbouring text box.
fn hyphen_chain(
    items: &[WordNode],
    index: usize,
    backwards: bool,
    target: usize,
) -> Option<(usize, Vec<usize>)> {
    let step = |i: usize| {
        if backwards {
            i.checked_sub(1)
        } else {
            i.checked_add(1)
        }
    };
    let mut cursor = step(index);
    let (run, mut range, atoms) = text_at(items, cursor)?;
    let mut bound = if backwards { range.start } else { range.end };
    let mut edges: Vec<usize> = atoms
        .iter()
        .flat_map(|a| [a.byte_range.start, a.byte_range.end])
        .collect();
    // The walk ends once the chain reaches `target`, so a short cut never scans
    // a long chain (validation stays linear overall). `items[cursor]` is the
    // last text box added; look past it for a `Hyphenate` joining it to another
    // text box of the same run.
    let reached = |bound: usize| {
        if backwards {
            bound <= target
        } else {
            bound >= target
        }
    };
    while !reached(bound) {
        let Some(disc_at) = cursor.and_then(step) else {
            break;
        };
        let Some(WordNode::Discretionary(d)) = items.get(disc_at) else {
            break;
        };
        if !matches!(d.action, BreakAction::Hyphenate(_)) {
            break;
        }
        let Some((next_run, next, next_atoms)) = text_at(items, step(disc_at)) else {
            break;
        };
        let meets = if backwards {
            next.end == d.byte_range.start && d.byte_range.end == range.start
        } else {
            range.end == d.byte_range.start && d.byte_range.end == next.start
        };
        if next_run != run || !meets {
            break;
        }
        edges.extend(
            next_atoms
                .iter()
                .flat_map(|a| [a.byte_range.start, a.byte_range.end]),
        );
        bound = if backwards { next.start } else { next.end };
        range = next;
        cursor = step(disc_at);
    }
    Some((bound, edges))
}

/// Whether a hyphenation discretionary at `items[index]` is well formed: its
/// `HyphenOpportunity` agrees with it, it sits between two non-empty text
/// boxes that meet at its offset, and each cut stays within, and ends on an
/// atom boundary of, the contiguous same-run hyphen-connected chain of text
/// boxes on its side (see [`hyphen_chain`]). A cut may therefore cross an
/// unchosen hyphenation point of the same word (a spelling change reaching two
/// bytes back across a point one byte away) but never a run or span boundary.
fn hyphen_valid(
    items: &[WordNode],
    index: usize,
    d: &DiscretionaryNode,
    h: &HyphenOpportunity,
) -> bool {
    let (Some((_, prev, _)), Some((_, next, _))) = (
        text_at(items, index.checked_sub(1)),
        text_at(items, index.checked_add(1)),
    ) else {
        return false;
    };
    let off = h.byte_offset;
    if d.byte_range != TextRange::empty_at(off) || prev.end != off || next.start != off {
        return false;
    }
    if prev.is_empty() || next.is_empty() {
        return false;
    }
    let (Some(prefix), Some(suffix)) = (
        off.checked_sub(h.prefix_cut_bytes as usize),
        off.checked_add(h.suffix_cut_bytes as usize),
    ) else {
        return false;
    };
    let (Some((left_bound, left_edges)), Some((right_bound, right_edges))) = (
        hyphen_chain(items, index, true, prefix),
        hyphen_chain(items, index, false, suffix),
    ) else {
        return false;
    };
    prefix >= left_bound
        && left_edges.contains(&prefix)
        && suffix <= right_bound
        && right_edges.contains(&suffix)
}

fn fits(text: &str, r: crate::types::TextRange) -> bool {
    r.start <= r.end
        && r.end <= text.len()
        && text.is_char_boundary(r.start)
        && text.is_char_boundary(r.end)
}

/// The offsets an atomic span may start or end at: the start or end of a text
/// atom or of an inline object. Glue, tabs and line terminators contribute
/// none, so an edge inside whitespace (or on an unowned offset) is rejected.
fn atomic_edges(ast: &ParagraphAst) -> std::collections::HashSet<usize> {
    let mut edges = std::collections::HashSet::new();
    for r in concrete_content(ast) {
        edges.insert(r.start);
        edges.insert(r.end);
    }
    edges
}

/// The byte ranges of concrete content: every text atom and inline object.
fn concrete_content(ast: &ParagraphAst) -> Vec<TextRange> {
    let mut out = Vec::new();
    for n in &ast.items {
        match n {
            WordNode::Box(BoxNode::Text { atoms, .. }) => {
                out.extend(atoms.iter().map(|a| a.byte_range));
            }
            WordNode::Box(BoxNode::InlineObject { byte_range, .. }) => out.push(*byte_range),
            _ => {}
        }
    }
    out
}

/// Validates the two span streams.
///
/// * Both: ranges fit the text, ids are unique within the stream.
/// * Decoration spans: finite, non-negative paddings; **proper nesting**
///   (two spans are disjoint or one contains the other, never a partial
///   overlap) no deeper than [`MAX_DECORATION_NESTING_DEPTH`].
/// * Atomic spans: non-empty, **strictly disjoint**, both edges at the start
///   or end of a text atom or an inline object, and wrapping at least one
///   atom or object (a span of whitespace alone is not atomic content).
fn check_spans(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    use LayoutInputError as E;
    for (span, s) in ast.decoration_spans.iter().enumerate() {
        let earlier = &ast.decoration_spans[..span];
        let duplicate = earlier.iter().any(|o| o.span_id == s.span_id);
        let crossing = earlier.iter().any(|o| {
            let (a, b) = (o.byte_range, s.byte_range);
            a.start < b.end && b.start < a.end && !a.contains_range(b) && !b.contains_range(a)
        });
        let depth = 1 + ast
            .decoration_spans
            .iter()
            .enumerate()
            .filter(|(i, o)| *i != span && o.byte_range.contains_range(s.byte_range))
            .count();
        if duplicate
            || crossing
            || depth > MAX_DECORATION_NESTING_DEPTH
            || !s.is_valid()
            || !fits(text, s.byte_range)
        {
            return Err(E::InvalidDecorationSpan { span });
        }
    }
    let edges = atomic_edges(ast);
    let content = concrete_content(ast);
    for (span, s) in ast.atomic_spans.iter().enumerate() {
        let earlier = &ast.atomic_spans[..span];
        let duplicate = earlier.iter().any(|o| o.span_id == s.span_id);
        let overlapping = earlier.iter().any(|o| {
            o.byte_range.start < s.byte_range.end && s.byte_range.start < o.byte_range.end
        });
        if duplicate
            || overlapping
            || s.byte_range.is_empty()
            || !fits(text, s.byte_range)
            || !edges.contains(&s.byte_range.start)
            || !edges.contains(&s.byte_range.end)
            || !content
                .iter()
                .any(|c| !c.is_empty() && s.byte_range.contains_range(*c))
        {
            return Err(E::InvalidAtomicSpan { span });
        }
    }
    Ok(())
}

/// The UAX #14 mandatory line terminators (BK, CR, LF, NL): the only text a
/// non-empty hard-break `byte_range` may cover.
fn is_line_terminator(s: &str) -> bool {
    matches!(
        s,
        "\n" | "\r" | "\r\n" | "\u{b}" | "\u{c}" | "\u{85}" | "\u{2028}" | "\u{2029}"
    )
}

/// The bidi level a span boundary must carry: the first (`Leading`) or last
/// (`Trailing`) atom whose bytes lie inside the span. `None` when the span
/// holds no text atom (only objects), so there is nothing to compare with.
fn expected_boundary_level(ast: &ParagraphAst, span_id: u64, edge: SpanEdge) -> Option<BidiLevel> {
    let span = ast.decoration_spans.iter().find(|s| s.span_id == span_id)?;
    let mut inside = ast
        .items
        .iter()
        .filter_map(|n| match n {
            WordNode::Box(BoxNode::Text { atoms, .. }) => Some(atoms.iter()),
            _ => None,
        })
        .flatten()
        .filter(|a| span.byte_range.contains_range(a.byte_range));
    match edge {
        SpanEdge::Leading => inside.next().map(|a| a.bidi_level),
        SpanEdge::Trailing => inside.next_back().map(|a| a.bidi_level),
    }
}

/// The `natural_width` of the last (`last == true`) or first cluster of the
/// text box at `items[index]`, or `None` if that item is not a text box with a
/// cluster.
fn edge_cluster_width(items: &[WordNode], index: Option<usize>, last: bool) -> Option<f32> {
    match index.and_then(|i| items.get(i)) {
        Some(WordNode::Box(BoxNode::Text { clusters, .. })) => {
            let c = if last {
                clusters.last()
            } else {
                clusters.first()
            };
            c.map(|c| c.natural_width)
        }
        _ => None,
    }
}

/// Monotone progression: each item starts at or after the end of the previous
/// one (zero-width nodes may share an offset). Among the zero-width nodes at one
/// offset a `CjkAutospace` glue precedes every break opportunity, as a
/// breakable space's glue precedes its discretionary (so the spacing stays on
/// the line that ends there): `[Glue(CjkAutospace), Discretionary]`.
fn check_item_order(ast: &ParagraphAst) -> Result<(), LayoutInputError> {
    for (i, w) in ast.items.windows(2).enumerate() {
        if let (Some(prev), Some(next)) = (w[0].byte_span(), w[1].byte_span()) {
            if prev.end > next.start {
                return Err(LayoutInputError::ItemsOutOfOrder { item: i + 1 });
            }
        }
    }
    // Offset of the break opportunity seen in the current run of zero-width
    // nodes, if any.
    let mut break_at: Option<usize> = None;
    for (item, n) in ast.items.iter().enumerate() {
        let Some(r) = n.byte_span() else { continue };
        if !r.is_empty() || break_at != Some(r.start) {
            break_at = None;
        }
        if n.is_break_opportunity() && r.is_empty() {
            break_at = Some(r.start);
        } else if matches!(n, WordNode::Glue(g) if g.kind == GlueKind::CjkAutospace)
            && break_at == Some(r.start)
        {
            return Err(LayoutInputError::ItemsOutOfOrder { item });
        }
    }
    Ok(())
}

/// Enclosure of the boundaries (run after `check_boundary_order`, so both nodes
/// of every span exist). With `lead` and `trail` the indices of a span's
/// boundary nodes:
///
/// * a break opportunity or a `CjkAutospace` glue at `s..s` (`s =
///   byte_range.start`) sits before `lead`, one at `e..e` after `trail`, so
///   breaking before or after the chip leaves both boundaries on the chip's
///   line and the seam spacing outside it. At a seam shared by two adjacent
///   spans the canonical order is `[A Trailing, Glue(CjkAutospace),
///   Discretionary, B Leading]`: LIFO and this rule fix the boundaries'
///   places, `check_item_order` the glue before the discretionary;
/// * any other zero-width node at `s` or `e` (a neighbouring span's boundary)
///   is unconstrained;
/// * every other item inside `[s, e]` is strictly between `lead` and `trail`,
///   every item outside is not between them, and none straddles an edge.
fn check_span_enclosure(ast: &ParagraphAst) -> Result<(), LayoutInputError> {
    let find = |id: u64, wanted: SpanEdge| {
        ast.items.iter().position(|n| {
            matches!(n, WordNode::Box(BoxNode::SpanBoundary { span_id, edge, .. })
                if *span_id == id && *edge == wanted)
        })
    };
    for (span, s) in ast.decoration_spans.iter().enumerate() {
        let (Some(lead), Some(trail)) = (
            find(s.span_id, SpanEdge::Leading),
            find(s.span_id, SpanEdge::Trailing),
        ) else {
            return Err(LayoutInputError::MissingSpanBoundary { span });
        };
        let (start, end) = (s.byte_range.start, s.byte_range.end);
        for (item, n) in ast.items.iter().enumerate() {
            if matches!(n, WordNode::Box(BoxNode::SpanBoundary { span_id, .. }) if *span_id == s.span_id)
            {
                continue;
            }
            let Some(r) = n.byte_span() else { continue };
            let between = lead < item && item < trail;
            let zero = r.is_empty();
            let autospace = matches!(n, WordNode::Glue(g) if g.kind == GlueKind::CjkAutospace);
            // Row 101: a break opportunity or a CJK autospace at the start sits
            // before `Leading`, at the end after `Trailing`.
            let ok = if zero && (n.is_break_opportunity() || autospace) && r.start == start {
                item < lead
            } else if zero && (n.is_break_opportunity() || autospace) && r.start == end {
                item > trail
            } else if zero && (r.start == start || r.start == end) {
                true
            } else if r.start >= start && r.end <= end {
                between
            } else if r.end <= start || r.start >= end {
                !between
            } else {
                false
            };
            if !ok {
                return Err(LayoutInputError::SpanEnclosure { span, item });
            }
        }
    }
    Ok(())
}

/// Span boundaries: every decoration span has exactly one `Leading` and one
/// `Trailing` node, in strict LIFO nesting order, and for two decoration spans
/// with equal ranges the lower index is the outer span (its `Leading` first, its
/// `Trailing` last).
fn check_boundary_order(ast: &ParagraphAst) -> Result<(), LayoutInputError> {
    use LayoutInputError as E;
    let mut open: Vec<u64> = Vec::new();
    let mut opened: Vec<u64> = Vec::new();
    for (item, n) in ast.items.iter().enumerate() {
        let WordNode::Box(BoxNode::SpanBoundary { span_id, edge, .. }) = n else {
            continue;
        };
        match edge {
            SpanEdge::Leading => {
                if opened.contains(span_id) {
                    return Err(E::InvalidSpanBoundary { item });
                }
                opened.push(*span_id);
                open.push(*span_id);
            }
            SpanEdge::Trailing => {
                if open.last() != Some(span_id) {
                    return Err(E::InvalidSpanBoundary { item });
                }
                open.pop();
            }
        }
    }
    for (span, s) in ast.decoration_spans.iter().enumerate() {
        if !opened.contains(&s.span_id) || open.contains(&s.span_id) {
            return Err(E::MissingSpanBoundary { span });
        }
    }
    let position = |id: u64, wanted: SpanEdge| {
        ast.items.iter().position(|n| {
            matches!(n, WordNode::Box(BoxNode::SpanBoundary { span_id, edge, .. })
                if *span_id == id && *edge == wanted)
        })
    };
    for (i, outer) in ast.decoration_spans.iter().enumerate() {
        for inner in &ast.decoration_spans[i + 1..] {
            if inner.byte_range != outer.byte_range {
                continue;
            }
            let leading = (
                position(outer.span_id, SpanEdge::Leading),
                position(inner.span_id, SpanEdge::Leading),
            );
            if let (Some(o), Some(n)) = leading {
                if o > n {
                    return Err(E::InvalidSpanBoundary { item: n });
                }
            }
            let trailing = (
                position(outer.span_id, SpanEdge::Trailing),
                position(inner.span_id, SpanEdge::Trailing),
            );
            if let (Some(o), Some(n)) = trailing {
                if n > o {
                    return Err(E::InvalidSpanBoundary { item: o });
                }
            }
        }
    }
    Ok(())
}

/// Checks the inputs of [`layout_paragraph`] and assembles the paragraph-level
/// [`LineContext`]: the active strut is resolved here from the AST's **raw**
/// strut metrics, `ParagraphAst::strut_font_size` and `options.line_height`
/// (`LineHeight::resolve_strut`).
///
/// The checks, in order (each failure has its own [`LayoutInputError`] variant):
/// 1. constraints, strut inputs and run ranges;
/// 2. the span streams (`check_spans`);
/// 3. each item on its own: text boxes (run, atoms, cached width, atomic flag),
///    inline objects, span boundaries, glue, and discretionaries (ownership,
///    cloned spans and their widths, hyphenation and punctuation deltas);
/// 4. the sentinel last, monotone item order, `Leading` / `Trailing` boundary
///    order, and span enclosure including seam spacing.
///
/// # Errors
/// The first problem found.
pub fn check_layout_inputs(
    ast: &ParagraphAst,
    text: &str,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> Result<LineContext, LayoutInputError> {
    use LayoutInputError as E;
    if !constraints.is_valid() {
        return Err(E::InvalidConstraints);
    }
    let m = &ast.strut_metrics;
    let fixed_ok = match options.line_height {
        super::LineHeight::Normal => true,
        super::LineHeight::Fixed(v) => v.is_finite() && v >= 0.0,
    };
    if !([m.ascent, m.descent, m.line_gap]
        .iter()
        .all(|v| v.is_finite())
        && ast.strut_font_size.is_finite()
        && ast.strut_font_size >= 0.0
        && fixed_ok)
    {
        return Err(E::InvalidStrut);
    }
    for (run, r) in ast.runs.iter().enumerate() {
        if !fits(text, r.byte_range) {
            return Err(E::RunOutsideText { run });
        }
    }
    check_spans(ast, text)?;
    let last_disc = ast.items.iter().rposition(|n| n.is_break_opportunity());
    for (item, node) in ast.items.iter().enumerate() {
        match node.byte_span() {
            Some(span) if fits(text, span) => {}
            _ => return Err(E::ItemOutsideText { item }),
        }
        match node {
            WordNode::Box(BoxNode::Text {
                run_index,
                byte_range,
                atoms,
                clusters,
                natural_width,
            }) => {
                let Some(run) = ast.runs.get(*run_index) else {
                    return Err(E::RunIndexOutOfRange { item });
                };
                // Every atom must fit, even when the node's overall span does.
                if !atoms.iter().all(|a| fits(text, a.byte_range)) {
                    return Err(E::ItemOutsideText { item });
                }
                if !node.is_self_contained() {
                    return Err(E::ItemNotSelfContained { item });
                }
                // IN_ATOMIC_EDIT_SPAN mirrors `atomic_spans` exactly.
                if atoms.iter().any(|a| {
                    let inside = ast
                        .atomic_spans
                        .iter()
                        .any(|s| s.byte_range.contains_range(a.byte_range));
                    inside != a.flags.contains(AtomFlags::IN_ATOMIC_EDIT_SPAN)
                }) {
                    return Err(E::AtomicFlagMismatch { item });
                }
                if !(run.byte_range.contains_range(*byte_range)
                    && atoms
                        .iter()
                        .all(|a| run.byte_range.contains_range(a.byte_range)))
                {
                    return Err(E::ItemOutsideRun { item });
                }
                let total: f64 = clusters.iter().map(|c| f64::from(c.natural_width)).sum();
                let cached = f64::from(*natural_width);
                if !(natural_width.is_finite()
                    && total.is_finite()
                    && (cached - total).abs() <= 1e-3_f64.max(1e-5 * total.abs()))
                {
                    return Err(E::InvalidTextWidth { item });
                }
            }
            WordNode::Box(BoxNode::InlineObject {
                width,
                ascent,
                descent,
                ..
            }) => {
                if ![width, ascent, descent]
                    .iter()
                    .all(|v| v.is_finite() && **v >= 0.0)
                {
                    return Err(E::InvalidInlineBox { item });
                }
            }
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range,
                span_id,
                edge,
                width,
                bidi_level,
            }) => {
                if !(byte_range.is_empty() && width.is_finite() && *width >= 0.0) {
                    return Err(E::InvalidSpanBoundary { item });
                }
                let Some(span) = ast.decoration_spans.iter().find(|s| s.span_id == *span_id) else {
                    return Err(E::UnknownSpan { item });
                };
                let anchor = match edge {
                    SpanEdge::Leading => span.byte_range.start,
                    SpanEdge::Trailing => span.byte_range.end,
                };
                let padding = match edge {
                    SpanEdge::Leading => span.start_padding,
                    SpanEdge::Trailing => span.end_padding,
                };
                if byte_range.start != anchor
                    || *bidi_level > BIDI_MAX_DEPTH + 1
                    || (*width - padding).abs() > 1e-3_f32.max(1e-5 * padding.abs())
                {
                    return Err(E::InvalidSpanBoundary { item });
                }
                if expected_boundary_level(ast, *span_id, *edge).is_some_and(|l| l != *bidi_level) {
                    return Err(E::InvalidSpanBoundary { item });
                }
            }
            WordNode::Glue(g) => {
                let sizes_ok = [g.natural_width, g.max_stretch, g.max_shrink]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0);
                // Tier 2 is derived from cluster flags; the compression pool
                // is shrink only and belongs to Tier 0.
                let pool_ok = g.kind != GlueKind::CjkPunctCompression
                    || (g.tier == GlueTier::Tier0
                        && g.natural_width == 0.0
                        && g.max_stretch == 0.0);
                // Autospace and the compression pool are synthetic: no source bytes.
                let range_ok = g.kind == GlueKind::WordSpace || g.byte_range.is_empty();
                if !(sizes_ok && g.tier != GlueTier::Tier2 && pool_ok && range_ok) {
                    return Err(E::InvalidGlue { item });
                }
            }
            WordNode::Discretionary(d) => {
                let widths_ok = [d.pre_break_width, d.post_break_width, d.no_break_width]
                    .iter()
                    .all(|v| v.is_finite());
                let hard = matches!(d.action, BreakAction::HardBreak { .. });
                // Ownership: a soft opportunity owns nothing; a real hard break owns
                // exactly its non-empty terminator; the paragraph-end sentinel is
                // the empty range at the end of the text, the last discretionary.
                let range_ok = match d.action {
                    BreakAction::HardBreak {
                        is_paragraph_end: true,
                    } => d.byte_range == TextRange::empty_at(text.len()) && Some(item) == last_disc,
                    BreakAction::HardBreak {
                        is_paragraph_end: false,
                    } => {
                        let (a, b) = (d.byte_range.start, d.byte_range.end);
                        // A lone CR or LF must not be half of a CRLF pair.
                        let splits_crlf = (text.as_bytes().get(a.wrapping_sub(1)) == Some(&b'\r')
                            && text.as_bytes().get(a) == Some(&b'\n'))
                            || (text.as_bytes().get(b.wrapping_sub(1)) == Some(&b'\r')
                                && text.as_bytes().get(b) == Some(&b'\n'));
                        !d.byte_range.is_empty() && is_line_terminator(&text[a..b]) && !splits_crlf
                    }
                    _ => d.byte_range.is_empty(),
                };
                let clones_unique = d
                    .cloned_spans
                    .iter()
                    .enumerate()
                    .all(|(i, id)| !d.cloned_spans[..i].contains(id));
                if !(widths_ok
                    && d.penalty < DiscretionaryNode::FORBIDDEN_BREAK_PENALTY
                    && hard == d.is_mandatory()
                    && range_ok
                    && clones_unique)
                {
                    return Err(E::InvalidDiscretionary { item });
                }
                // Completeness: every Clone-mode span the break is strictly inside
                // is listed (a chip split without its cloned padding is wrong).
                // `cloned_spans` is exactly the set of Clone-mode spans the break is
                // strictly inside: a chip split without its cloned padding is
                // wrong, and so is cloning a span the break does not cut.
                let off = d.byte_offset();
                let cut_by_break = |sp: &crate::frontend::InlineDecorationSpan| {
                    sp.break_mode == BoxDecorationBreak::Clone
                        && sp.byte_range.start < off
                        && off < sp.byte_range.end
                };
                if ast
                    .decoration_spans
                    .iter()
                    .any(|sp| cut_by_break(sp) && !d.cloned_spans.contains(&sp.span_id))
                {
                    return Err(E::InvalidDiscretionary { item });
                }
                for id in &d.cloned_spans {
                    if !ast
                        .decoration_spans
                        .iter()
                        .any(|s| s.span_id == *id && cut_by_break(s))
                    {
                        return Err(E::UnknownSpan { item });
                    }
                }
                // Cloned padding is part of the break widths (line `k` gains each
                // listed span's `end_padding`, line `k + 1` its `start_padding`);
                // what remains (`net_*`) is the action's own delta.
                let (mut clone_pre, mut clone_post) = (0.0_f64, 0.0_f64);
                for id in &d.cloned_spans {
                    if let Some(sp) = ast.decoration_spans.iter().find(|s| s.span_id == *id) {
                        clone_pre += f64::from(sp.end_padding);
                        clone_post += f64::from(sp.start_padding);
                    }
                }
                let (pre, post) = (f64::from(d.pre_break_width), f64::from(d.post_break_width));
                let (net_pre, net_post) = (pre - clone_pre, post - clone_post);
                let near =
                    |a: f64, b: f64| (a - b).abs() <= 1e-3_f64.max(1e-5 * a.abs().max(b.abs()));
                match &d.action {
                    BreakAction::Hyphenate(h)
                        if !d.flagged || !hyphen_valid(&ast.items, item, d, h) =>
                    {
                        return Err(E::InvalidHyphenOpportunity { item });
                    }
                    // A hyphenation's net deltas are its reshaping deltas.
                    BreakAction::Hyphenate(_) => {}
                    BreakAction::CjkPunctAdjust => {
                        // At least one net delta is real; each needs its
                        // neighbouring text box and may not drive that edge
                        // cluster's width below zero.
                        let live_pre = net_pre.abs() > 1e-3;
                        let live_post = net_post.abs() > 1e-3;
                        let side_ok = |live: bool, net: f64, width: Option<f32>| {
                            !live || width.is_some_and(|w| f64::from(w) + net >= -1e-3)
                        };
                        let trailing = edge_cluster_width(&ast.items, item.checked_sub(1), true);
                        let leading = edge_cluster_width(&ast.items, item.checked_add(1), false);
                        if !((live_pre || live_post)
                            && side_ok(live_pre, net_pre, trailing)
                            && side_ok(live_post, net_post, leading))
                        {
                            return Err(E::InvalidDiscretionary { item });
                        }
                    }
                    BreakAction::Direct
                    | BreakAction::HardBreak { .. }
                    | BreakAction::MathOperator {
                        repeat_glyph_id: None,
                    } => {
                        if !(near(pre, clone_pre) && near(post, clone_post)) {
                            return Err(E::InvalidDiscretionary { item });
                        }
                    }
                    BreakAction::MathOperator {
                        repeat_glyph_id: Some(_),
                    } => {}
                }
            }
            WordNode::Tab { .. } => {}
        }
    }
    match ast.items.last() {
        Some(WordNode::Discretionary(d))
            if matches!(
                d.action,
                BreakAction::HardBreak {
                    is_paragraph_end: true
                }
            ) => {}
        _ => return Err(E::MissingParagraphEnd),
    }
    check_item_order(ast)?;
    check_boundary_order(ast)?;
    check_span_enclosure(ast)?;
    let context = LineContext::from_options(
        m,
        ast.strut_font_size,
        ast.base_direction,
        options,
        constraints.wrap_width,
    );
    let r = &context.strut;
    if ![r.ascent, r.descent, r.line_gap, r.natural_line_height()]
        .iter()
        .all(|v| v.is_finite())
    {
        return Err(E::InvalidStrut);
    }
    Ok(context)
}

/// Lays out one paragraph (spec §5).
///
/// * `ast`: frontend output; `text`: the paragraph text it was built from
///   (shapers receive it per call).
/// * `shaper`: used for re-shaping at breaks and for ligature carets.
/// * `constraints`: container geometry; `options`: layout policy.
///
/// **Phase 0 skeleton.** It runs [`check_layout_inputs`] and performs no
/// layout: the result has the AST's base direction and no lines. Because the
/// signature returns no `Result`, invalid input is not reported either; call
/// `check_layout_inputs` first to learn why.
#[must_use]
pub fn layout_paragraph(
    ast: &ParagraphAst,
    text: &str,
    shaper: &dyn TextShaper,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> PositionedParagraph {
    let _ = shaper;
    let _context = check_layout_inputs(ast, text, constraints, options);
    PositionedParagraph::empty(ast.base_direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::middleend::TabStops;
    use crate::shaping::NoopShaper;
    use crate::types::{BidiDirection, FontMetrics, TextRange};

    fn constraints() -> LayoutConstraints {
        LayoutConstraints {
            wrap_width: 100.0,
            tab_stops: TabStops { step: 8.0 },
        }
    }

    fn ast() -> ParagraphAst {
        ParagraphAst {
            base_direction: BidiDirection::Rtl,
            strut_metrics: FontMetrics {
                ascent: 8.0,
                descent: 2.0,
                line_gap: 1.0,
                ..Default::default()
            },
            strut_font_size: 10.0,
            runs: vec![],
            items: vec![hard_break(), sentinel(1)],
            decoration_spans: vec![],
            atomic_spans: vec![],
        }
    }

    /// A mandatory break that owns the one-byte paragraph "\n".
    fn hard_break() -> WordNode {
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(0, 1), false))
    }

    /// The paragraph-end sentinel for a text of `len` bytes.
    fn sentinel(len: usize) -> WordNode {
        WordNode::Discretionary(DiscretionaryNode::hard_break(
            TextRange::empty_at(len),
            true,
        ))
    }

    #[test]
    fn context_resolves_the_strut_from_raw_metrics_and_options() {
        let normal =
            check_layout_inputs(&ast(), "\n", &constraints(), &LayoutOptions::default()).unwrap();
        assert_eq!(normal.strut.natural_line_height(), 11.0);
        assert_eq!(normal.base_direction, BidiDirection::Rtl);
        let fixed_options = LayoutOptions {
            line_height: crate::middleend::LineHeight::Fixed(1.5),
            ..Default::default()
        };
        let fixed = check_layout_inputs(&ast(), "\n", &constraints(), &fixed_options).unwrap();
        // 1.5 * 10 = 15, half-leading folded in, gap gone.
        assert_eq!(fixed.strut.natural_line_height(), 15.0);
        assert_eq!(fixed.strut.line_gap, 0.0);
        // The AST itself is untouched by the policy.
        assert_eq!(ast().strut_metrics.line_gap, 1.0);
    }

    #[test]
    fn bad_inputs_are_named() {
        let o = LayoutOptions::default();
        let bad_width = LayoutConstraints {
            wrap_width: f32::NAN,
            ..constraints()
        };
        assert_eq!(
            check_layout_inputs(&ast(), "\n", &bad_width, &o),
            Err(LayoutInputError::InvalidConstraints)
        );
        assert_eq!(
            check_layout_inputs(&ast(), "", &constraints(), &o),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
        let mut a = ast();
        a.strut_metrics.ascent = f32::INFINITY;
        assert_eq!(
            check_layout_inputs(&a, "\n", &constraints(), &o),
            Err(LayoutInputError::InvalidStrut)
        );
        // Splitting a UTF-8 sequence is outside the text.
        let mut a = ast();
        a.items = vec![hard_break()];
        assert_eq!(
            check_layout_inputs(&a, "é", &constraints(), &o),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
        assert!(!LayoutInputError::InvalidStrut.to_string().is_empty());
    }

    #[test]
    fn skeleton_returns_an_empty_paragraph_with_the_ast_direction() {
        let p = layout_paragraph(
            &ast(),
            "\n",
            &NoopShaper,
            &constraints(),
            &LayoutOptions::default(),
        );
        assert_eq!(p, PositionedParagraph::empty(BidiDirection::Rtl));
        // Invalid input yields the same value; check_layout_inputs explains it.
        let bad = LayoutConstraints {
            wrap_width: -1.0,
            ..constraints()
        };
        let p = layout_paragraph(&ast(), "\n", &NoopShaper, &bad, &LayoutOptions::default());
        assert!(p.lines.is_empty());
    }

    use crate::frontend::{
        AtomicEditSpan, BoxOverflowPolicy, GlueKind, GlueNode, GlueTier, HyphenOpportunity,
        InlineDecorationSpan, SpanEdge,
    };
    use crate::types::{AtomFlags, GraphemeAtom, LineBreakClass, ScriptTag, ShapedCluster};

    fn atom(start: usize, end: usize) -> GraphemeAtom {
        GraphemeAtom {
            byte_range: TextRange::new(start, end),
            script: ScriptTag::LATIN,
            bidi_level: 0,
            linebreak_class: LineBreakClass::Al,
            flags: AtomFlags::EMPTY,
        }
    }

    /// A text box over `atoms`, one cluster per atom. The range is built
    /// without the `start <= end` assertion so out-of-order atoms can be tested.
    fn text_box(atoms: Vec<GraphemeAtom>) -> WordNode {
        let clusters = atoms
            .iter()
            .enumerate()
            .map(|(i, a)| ShapedCluster {
                byte_range: a.byte_range,
                atom_range: i..i + 1,
                font: crate::types::FontRef::unscaled(crate::types::FontId(0)),
                bidi_level: 0,
                flags: AtomFlags::EMPTY,
                glyphs: Default::default(),
                natural_width: 0.0,
                uncompressed_width: 0.0,
                safe_to_break_before: true,
                safe_to_concat_before: true,
            })
            .collect();
        let byte_range = match (atoms.first(), atoms.last()) {
            (Some(f), Some(l)) => TextRange {
                start: f.byte_range.start,
                end: l.byte_range.end,
            },
            _ => TextRange::EMPTY,
        };
        WordNode::Box(BoxNode::Text {
            run_index: 0,
            byte_range,
            atoms,
            clusters,
            natural_width: 0.0,
        })
    }

    fn run() -> crate::frontend::ShapingRun {
        crate::frontend::ShapingRun {
            byte_range: TextRange::new(0, 4),
            families: std::sync::Arc::from([]),
            font_size: 10.0,
            style: Default::default(),
            script: ScriptTag::LATIN,
            locale: Default::default(),
            bidi_level: 0,
            decoration: Default::default(),
        }
    }

    /// `items` exactly as given (the caller supplies the sentinel).
    fn with_raw_items(items: Vec<WordNode>) -> ParagraphAst {
        ParagraphAst {
            runs: vec![run()],
            items,
            ..ast()
        }
    }

    /// `items` followed by the sentinel of a `len`-byte text.
    fn with_items_for(len: usize, mut items: Vec<WordNode>) -> ParagraphAst {
        items.push(sentinel(len));
        let mut a = with_raw_items(items);
        a.runs[0].byte_range = TextRange::new(0, len.max(4));
        a
    }

    /// `items` followed by the sentinel of the four-byte text most tests use.
    fn with_items(items: Vec<WordNode>) -> ParagraphAst {
        with_items_for(4, items)
    }

    /// Adds the mandatory `Leading` / `Trailing` boundary pair of every
    /// decoration span where the enclosure rule wants it: a `Leading` before the
    /// first item that starts at or after the span start (a break opportunity
    /// at the start stays before it), a `Trailing` before the first item that
    /// starts at or after the span end. Equal edges come out nested (`Leading`s
    /// inserted innermost first, `Trailing`s outermost last).
    fn closed(mut a: ParagraphAst) -> ParagraphAst {
        let spans = a.decoration_spans.clone();
        for sp in spans.iter().rev() {
            let at = a
                .items
                .iter()
                .position(|n| {
                    let r = n.byte_span().unwrap();
                    let seam = n.is_break_opportunity()
                        || matches!(n, WordNode::Glue(g) if g.kind == GlueKind::CjkAutospace);
                    r.start >= sp.byte_range.start
                        && !(seam && r.is_empty() && r.start == sp.byte_range.start)
                })
                .unwrap_or(a.items.len() - 1);
            a.items
                .insert(at, bnd(sp.span_id, SpanEdge::Leading, sp.byte_range.start));
        }
        for sp in &spans {
            let at = a
                .items
                .iter()
                .position(|n| n.byte_span().unwrap().start >= sp.byte_range.end)
                .unwrap_or(a.items.len() - 1);
            a.items
                .insert(at, bnd(sp.span_id, SpanEdge::Trailing, sp.byte_range.end));
        }
        a
    }

    #[test]
    fn inline_objects_must_have_finite_non_negative_sizes() {
        let o = LayoutOptions::default();
        for (w, a, d) in [
            (f32::NAN, 1.0, 1.0),
            (1.0, -1.0, 1.0),
            (1.0, 1.0, f32::INFINITY),
        ] {
            let a = with_items(vec![WordNode::Box(BoxNode::InlineObject {
                byte_range: TextRange::new(0, 1),
                box_id: 1,
                width: w,
                ascent: a,
                descent: d,
                overflow_policy: BoxOverflowPolicy::AllowOverflow,
            })]);
            assert_eq!(
                check_layout_inputs(&a, "abcd", &constraints(), &o),
                Err(LayoutInputError::InvalidInlineBox { item: 0 })
            );
        }
    }

    fn glue(natural: f32, stretch: f32, shrink: f32) -> WordNode {
        WordNode::Glue(GlueNode {
            byte_range: TextRange::new(1, 2),
            kind: GlueKind::WordSpace,
            tier: GlueTier::Tier0,
            natural_width: natural,
            max_stretch: stretch,
            max_shrink: shrink,
        })
    }

    #[test]
    fn glue_sizes_must_be_finite_and_non_negative() {
        let o = LayoutOptions::default();
        let check = |g| check_layout_inputs(&with_items(vec![g]), "abcd", &constraints(), &o);
        assert!(check(glue(3.0, 1.5, 1.0)).is_ok());
        let bad = Err(LayoutInputError::InvalidGlue { item: 0 });
        assert_eq!(check(glue(f32::NAN, 0.0, 0.0)), bad);
        assert_eq!(check(glue(3.0, -1.0, 0.0)), bad);
        assert_eq!(check(glue(3.0, 0.0, f32::INFINITY)), bad);
    }

    #[test]
    fn discretionary_penalty_and_widths_are_checked() {
        let o = LayoutOptions::default();
        let check = |d: DiscretionaryNode| {
            check_layout_inputs(
                &with_items(vec![WordNode::Discretionary(d)]),
                "abcd",
                &constraints(),
                &o,
            )
        };
        let bad = Err(LayoutInputError::InvalidDiscretionary { item: 0 });
        assert!(check(DiscretionaryNode::direct(2)).is_ok());
        // A plain break owns no width of its own (outside a cloned span): any
        // non-zero pre/post width, and any non-finite one, is rejected.
        let mut d = DiscretionaryNode::direct(2);
        d.pre_break_width = -2.5;
        assert_eq!(check(d.clone()), bad);
        d.pre_break_width = 0.0;
        d.post_break_width = 1.0;
        assert_eq!(check(d.clone()), bad);
        d.post_break_width = f32::NAN;
        assert_eq!(check(d), bad);
        // A forbidden break is an omitted node, never a node.
        let mut d = DiscretionaryNode::direct(2);
        d.penalty = DiscretionaryNode::FORBIDDEN_BREAK_PENALTY;
        assert_eq!(check(d), bad);
        // HardBreak and a mandatory penalty go together.
        let mut d = DiscretionaryNode::hard_break(TextRange::empty_at(4), true);
        d.penalty = 0;
        assert_eq!(check(d), bad);
        let mut d = DiscretionaryNode::direct(2);
        d.penalty = DiscretionaryNode::MANDATORY_BREAK_PENALTY;
        assert_eq!(check(d), bad);
        // The offset must lie in the text, on a code point boundary.
        assert_eq!(
            check_layout_inputs(
                &with_items(vec![WordNode::Discretionary(DiscretionaryNode::direct(9))]),
                "abcd",
                &constraints(),
                &o
            ),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
    }

    /// Word "ab" + "é" (bytes 0..4) split at `at` into two boxes around a
    /// hyphenation discretionary.
    fn hyphenated(opportunity: HyphenOpportunity) -> ParagraphAst {
        let atoms = vec![atom(0, 1), atom(1, 2), atom(2, 4)];
        let at = opportunity.byte_offset;
        let (head, tail): (Vec<_>, Vec<_>) =
            atoms.into_iter().partition(|a| a.byte_range.end <= at);
        let d = DiscretionaryNode {
            byte_range: TextRange::empty_at(at),
            penalty: DiscretionaryNode::HYPHEN_PENALTY,
            flagged: true,
            pre_break_width: 5.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Hyphenate(opportunity),
            cloned_spans: Default::default(),
        };
        with_items(vec![
            text_box(head),
            WordNode::Discretionary(d),
            text_box(tail),
        ])
    }

    fn opportunity(offset: usize, prefix_cut: u32, suffix_cut: u32) -> HyphenOpportunity {
        HyphenOpportunity {
            byte_offset: offset,
            prefix_replacement: None,
            suffix_replacement: None,
            prefix_cut_bytes: prefix_cut,
            suffix_cut_bytes: suffix_cut,
            penalty: 0,
        }
    }

    #[test]
    fn hyphen_opportunities_must_sit_on_atom_boundaries_between_two_boxes() {
        let o = LayoutOptions::default();
        let check =
            |h: HyphenOpportunity| check_layout_inputs(&hyphenated(h), "abé", &constraints(), &o);
        assert!(check(opportunity(1, 0, 0)).is_ok());
        // Split at 2: head "a" "b" (0..2), tail "é" (2..4). Cuts end at 1 and 4.
        assert!(check(opportunity(2, 1, 2)).is_ok());
        let bad = Err(LayoutInputError::InvalidHyphenOpportunity { item: 1 });
        assert_eq!(check(opportunity(2, 3, 0)), bad); // cut before the head
        assert_eq!(check(opportunity(1, 0, 2)), bad); // ends mid-atom (at 3)
        assert_eq!(check(opportunity(2, 0, 3)), bad); // cut past the tail
        // The discretionary and the opportunity must agree on the offset.
        let mut a = hyphenated(opportunity(1, 0, 0));
        if let WordNode::Discretionary(d) = &mut a.items[1] {
            d.byte_range = TextRange::empty_at(2);
        }
        assert_eq!(check_layout_inputs(&a, "abé", &constraints(), &o), bad);
        // It needs a text box on each side.
        let mut a = hyphenated(opportunity(1, 0, 0));
        a.items.remove(2);
        assert_eq!(check_layout_inputs(&a, "abé", &constraints(), &o), bad);
    }

    /// "abcd" as `"a" | "b" | "cd"`: boxes at 0..1, 1..2 and 2..4 around two
    /// hyphenation points. `first` is the item between the first two boxes,
    /// `second_cut` the `(prefix, suffix)` cut of the point at 2.
    fn three_part_word(first: WordNode, second_cut: (u32, u32)) -> ParagraphAst {
        let hyphen = |at: usize, prefix: u32, suffix: u32| {
            WordNode::Discretionary(DiscretionaryNode {
                byte_range: TextRange::empty_at(at),
                penalty: DiscretionaryNode::HYPHEN_PENALTY,
                flagged: true,
                pre_break_width: 5.0,
                post_break_width: 0.0,
                no_break_width: 0.0,
                action: BreakAction::Hyphenate(opportunity(at, prefix, suffix)),
                cloned_spans: Default::default(),
            })
        };
        with_items(vec![
            text_box(vec![atom(0, 1)]),
            first,
            text_box(vec![atom(1, 2)]),
            hyphen(2, second_cut.0, second_cut.1),
            text_box(vec![atom(2, 3), atom(3, 4)]),
        ])
    }

    #[test]
    fn hyphen_cuts_stay_within_the_same_run_hyphen_chain() {
        let o = LayoutOptions::default();
        let hyphen_at_1 = || {
            WordNode::Discretionary(DiscretionaryNode {
                byte_range: TextRange::empty_at(1),
                penalty: DiscretionaryNode::HYPHEN_PENALTY,
                flagged: true,
                pre_break_width: 5.0,
                post_break_width: 0.0,
                no_break_width: 0.0,
                action: BreakAction::Hyphenate(opportunity(1, 0, 0)),
                cloned_spans: Default::default(),
            })
        };
        let check = |a: &ParagraphAst| check_layout_inputs(a, "abcd", &constraints(), &o);
        let bad = Err(LayoutInputError::InvalidHyphenOpportunity { item: 3 });
        // A 2-byte spelling-change cut crosses the unchosen point one byte away.
        assert!(check(&three_part_word(hyphen_at_1(), (2, 1))).is_ok());
        // The suffix cut crosses a later point just the same.
        let mut first = hyphen_at_1();
        if let WordNode::Discretionary(d) = &mut first {
            d.action = BreakAction::Hyphenate(opportunity(1, 0, 2));
        }
        assert!(check(&three_part_word(first, (0, 0))).is_ok());
        // ... but not the start of the word, nor a cut past its end.
        assert_eq!(check(&three_part_word(hyphen_at_1(), (3, 0))), bad);
        assert_eq!(check(&three_part_word(hyphen_at_1(), (0, 3))), bad);
        // Anything but a hyphenation point ends the chain.
        let space = WordNode::Glue(GlueNode {
            byte_range: TextRange::empty_at(1),
            kind: GlueKind::CjkAutospace,
            tier: GlueTier::Tier0,
            natural_width: 1.0,
            max_stretch: 0.0,
            max_shrink: 0.0,
        });
        assert_eq!(check(&three_part_word(space, (2, 0))), bad);
        assert!(check(&three_part_word(hyphen_at_1(), (1, 0))).is_ok());
        let direct = WordNode::Discretionary(DiscretionaryNode::direct(1));
        assert_eq!(check(&three_part_word(direct, (2, 0))), bad);
        // A change of shaping run ends it too: one reshape window, one run.
        let mut a = three_part_word(hyphen_at_1(), (2, 0));
        a.runs.push(run());
        if let WordNode::Box(BoxNode::Text { run_index, .. }) = &mut a.items[0] {
            *run_index = 1;
        }
        assert_eq!(check(&a), bad);
        assert!(check(&three_part_word(hyphen_at_1(), (1, 0))).is_ok());
    }

    #[test]
    fn break_widths_carry_the_padding_of_the_cloned_spans() {
        use crate::frontend::BoxDecorationBreak::Clone;
        let o = LayoutOptions::default();
        // chip(5) spans 1..3 with 2.0 padding on each side; a break at 2 inside it.
        let build = |action: BreakAction, pre: f32, post: f32, cloned: bool| {
            let mut d = DiscretionaryNode::direct(2);
            d.action = action;
            d.pre_break_width = pre;
            d.post_break_width = post;
            if cloned {
                d.cloned_spans.push(5);
            }
            let mut a = with_items(vec![
                with_width(text_box(vec![atom(0, 1)])),
                with_width(text_box(vec![atom(1, 2)])),
                WordNode::Discretionary(d),
                with_width(text_box(vec![atom(2, 3)])),
                with_width(text_box(vec![atom(3, 4)])),
            ]);
            a.decoration_spans = vec![chip(5, Clone)];
            closed(a)
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        let math = |glyph| BreakAction::MathOperator {
            repeat_glyph_id: glyph,
        };
        // The sums must match exactly for the plain actions.
        for action in [BreakAction::Direct, math(None)] {
            assert!(check(build(action.clone(), 2.0, 2.0, true)).is_ok());
            assert!(check(build(action.clone(), 0.0, 2.0, true)).is_err());
            assert!(check(build(action.clone(), 2.0, 0.0, true)).is_err());
            assert!(check(build(action, 3.0, 2.0, true)).is_err());
        }
        // A repeated operator glyph adds its own width: not constrained here.
        assert!(check(build(math(Some(7)), 9.0, 2.0, true)).is_ok());
        // Without a listed span the widths are zero, and a Clone span the break
        // is inside must be listed.
        assert!(check(build(BreakAction::Direct, 2.0, 2.0, false)).is_err());
        // Punctuation adjustment: the clone padding is subtracted first, and
        // what remains must be a real, affordable delta (edge clusters are 4 wide).
        let adjust = |pre, post| check(build(BreakAction::CjkPunctAdjust, pre, post, true));
        assert!(adjust(1.0, 2.0).is_ok()); // net_pre = -1
        assert!(adjust(2.0, 5.0).is_ok()); // net_post = +3
        assert!(adjust(-2.0, 2.0).is_ok()); // net_pre = -4: exactly zero left
        assert!(adjust(2.0, 2.0).is_err()); // padding alone adjusts nothing
        assert!(adjust(-2.5, 2.0).is_err()); // net_pre = -4.5
        assert!(adjust(2.0, -2.5).is_err()); // net_post = -4.5
    }

    #[test]
    fn a_non_finite_cluster_total_cannot_match_a_finite_cached_width() {
        let o = LayoutOptions::default();
        let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
        if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
            clusters[0].natural_width = f32::INFINITY;
        }
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::InvalidTextWidth { item: 0 })
        );
    }

    #[test]
    fn punctuation_deltas_are_judged_net_of_large_clone_padding() {
        use crate::frontend::BoxDecorationBreak::Clone;
        let o = LayoutOptions::default();
        // A chip with 1e6 of padding on each side: the padded widths are huge,
        // the net deltas small, and only the net deltas decide.
        let big = |pre: f32, post: f32| {
            let mut d = DiscretionaryNode::direct(2);
            d.action = BreakAction::CjkPunctAdjust;
            d.pre_break_width = pre;
            d.post_break_width = post;
            d.cloned_spans.push(5);
            let mut a = with_items(vec![
                with_width(text_box(vec![atom(0, 1)])),
                with_width(text_box(vec![atom(1, 2)])),
                WordNode::Discretionary(d),
                with_width(text_box(vec![atom(2, 3)])),
                with_width(text_box(vec![atom(3, 4)])),
            ]);
            let mut sp = chip(5, Clone);
            sp.start_padding = 1.0e6;
            sp.end_padding = 1.0e6;
            a.decoration_spans = vec![sp];
            // Boundary widths follow the padding.
            let mut a = closed(a);
            for n in &mut a.items {
                if let WordNode::Box(BoxNode::SpanBoundary { width, .. }) = n {
                    *width = 1.0e6;
                }
            }
            check_layout_inputs(&a, "abcd", &constraints(), &o)
        };
        assert!(big(1.0e6 - 1.0, 1.0e6).is_ok()); // net_pre = -1 of 4
        assert!(big(1.0e6, 1.0e6 + 0.5).is_ok()); // a small restore is real
        assert!(big(1.0e6 - 5.0, 1.0e6).is_err()); // net_pre = -5 empties the cluster
        assert!(big(1.0e6, 1.0e6).is_err()); // nothing adjusted
    }

    #[test]
    fn the_atomic_flag_mirrors_the_atomic_spans() {
        let o = LayoutOptions::default();
        let build = |flag_inside: bool, flag_outside: bool| {
            let mut a = with_items(vec![text_box(vec![
                atom(0, 1),
                atom(1, 2),
                atom(2, 3),
                atom(3, 4),
            ])]);
            a.atomic_spans = vec![AtomicEditSpan {
                span_id: 1,
                byte_range: TextRange::new(1, 3),
            }];
            if let WordNode::Box(BoxNode::Text {
                atoms, clusters, ..
            }) = &mut a.items[0]
            {
                for (i, at) in atoms.iter_mut().enumerate() {
                    let inside = (1..3).contains(&i);
                    if (inside && flag_inside) || (!inside && flag_outside) {
                        at.flags = AtomFlags::IN_ATOMIC_EDIT_SPAN;
                    }
                }
                for c in clusters.iter_mut() {
                    c.flags = AtomFlags::for_cluster(&atoms[c.atom_range.clone()]);
                }
            }
            a
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        assert!(check(build(true, false)).is_ok());
        let bad = Err(LayoutInputError::AtomicFlagMismatch { item: 0 });
        assert_eq!(check(build(false, false)), bad); // flag missing inside
        assert_eq!(check(build(true, true)), bad); // flag stray outside
        assert_eq!(check(build(false, true)), bad);
    }

    #[test]
    fn every_atom_must_fit_and_be_in_order() {
        let o = LayoutOptions::default();
        // Overall span fits "abcd" but the middle atom does not end inside it.
        let a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 9), atom(2, 4)])]);
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
        // First atom starts after the last one ends: no panic, an error.
        let a = with_items(vec![text_box(vec![atom(3, 4), atom(0, 1)])]);
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
        // A byte range that disagrees with its atoms is not self-contained.
        let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 2)])]);
        if let WordNode::Box(BoxNode::Text { byte_range, .. }) = &mut a.items[0] {
            *byte_range = TextRange::new(0, 3);
        }
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::ItemNotSelfContained { item: 0 })
        );
        // A text box naming a run that does not exist.
        let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
        a.runs.clear();
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::RunIndexOutOfRange { item: 0 })
        );
    }

    fn chip(span_id: u64, mode: crate::frontend::BoxDecorationBreak) -> InlineDecorationSpan {
        InlineDecorationSpan {
            span_id,
            byte_range: TextRange::new(1, 3),
            start_padding: 2.0,
            end_padding: 2.0,
            break_mode: mode,
        }
    }

    #[test]
    fn span_boundaries_and_clone_padding_must_name_a_known_span() {
        use crate::frontend::BoxDecorationBreak::{Clone, Slice};
        let o = LayoutOptions::default();
        let boundary = |width: f32, range: TextRange| {
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range: range,
                span_id: 5,
                edge: SpanEdge::Leading,
                width,
                bidi_level: 0,
            })
        };
        let with_span = |items, mode| {
            let mut a = with_items(items);
            a.decoration_spans = vec![chip(5, mode)];
            a
        };
        let ok = with_span(
            vec![
                boundary(2.0, TextRange::empty_at(1)),
                bnd(5, SpanEdge::Trailing, 3),
            ],
            Slice,
        );
        assert!(check_layout_inputs(&ok, "abcd", &constraints(), &o).is_ok());
        let unknown = with_items(vec![boundary(2.0, TextRange::empty_at(1))]);
        assert_eq!(
            check_layout_inputs(&unknown, "abcd", &constraints(), &o),
            Err(LayoutInputError::UnknownSpan { item: 0 })
        );
        for bad in [
            boundary(2.0, TextRange::new(1, 2)), // not empty
            boundary(-1.0, TextRange::empty_at(1)),
            boundary(f32::NAN, TextRange::empty_at(1)),
        ] {
            let a = with_span(vec![bad], Slice);
            assert_eq!(
                check_layout_inputs(&a, "abcd", &constraints(), &o),
                Err(LayoutInputError::InvalidSpanBoundary { item: 0 })
            );
        }
        let clone_pad = |mode| {
            let mut d = DiscretionaryNode::direct(2);
            d.penalty = DiscretionaryNode::CLONED_SPAN_PENALTY;
            d.cloned_spans.push(5);
            // The cloned chip's padding rides in the break widths: line k gains
            // its end padding, line k + 1 its start padding (2.0 each).
            d.pre_break_width = 2.0;
            d.post_break_width = 2.0;
            closed(with_span(vec![WordNode::Discretionary(d)], mode))
        };
        assert!(check_layout_inputs(&clone_pad(Clone), "abcd", &constraints(), &o).is_ok());
        // Padding is cloned only by a span that was declared to clone it.
        assert_eq!(
            check_layout_inputs(&clone_pad(Slice), "abcd", &constraints(), &o),
            Err(LayoutInputError::UnknownSpan { item: 1 }) // item 0 is the Leading edge
        );
    }

    #[test]
    fn cloned_spans_are_orthogonal_to_the_action_and_must_enclose_the_break() {
        use crate::frontend::BoxDecorationBreak::Clone;
        let o = LayoutOptions::default();
        // A hyphenation break at 2 inside a cloning chip over 1..3: both apply.
        let mut a = hyphenated(opportunity(2, 0, 0));
        // Boxes are split at the chip's start: "a" | "b" - "é".
        a.items[0] = text_box(vec![atom(0, 1)]);
        a.items.insert(1, text_box(vec![atom(1, 2)]));
        let mut over = chip(5, Clone);
        over.byte_range = TextRange::new(1, 4); // "b" and "é"
        a.decoration_spans = vec![over];
        if let WordNode::Discretionary(d) = &mut a.items[2] {
            d.cloned_spans.push(5);
            d.penalty += DiscretionaryNode::CLONED_SPAN_PENALTY;
        }
        let a = closed(a);
        assert!(check_layout_inputs(&a, "abé", &constraints(), &o).is_ok());
        // A break on the span's edge is not inside it; a repeated id is wrong.
        for (cloned, at) in [(vec![5], 1), (vec![5], 3), (vec![5, 5], 2)] {
            let mut d = DiscretionaryNode::direct(at);
            d.cloned_spans.extend(cloned);
            let mut b = with_items(vec![WordNode::Discretionary(d)]);
            b.decoration_spans = vec![chip(5, Clone)];
            let b = closed(b);
            let r = check_layout_inputs(&b, "abcd", &constraints(), &o);
            assert!(
                matches!(
                    r,
                    Err(LayoutInputError::UnknownSpan { .. }
                        | LayoutInputError::InvalidDiscretionary { .. })
                ),
                "{r:?}"
            );
        }
    }

    #[test]
    fn soft_breaks_own_nothing_and_hard_breaks_own_their_terminator() {
        let o = LayoutOptions::default();
        let check = |text: &str, d: DiscretionaryNode| {
            let mut a = with_items_for(text.len(), vec![WordNode::Discretionary(d)]);
            a.runs[0].byte_range = TextRange::new(0, text.len());
            check_layout_inputs(&a, text, &constraints(), &o)
        };
        let bad = Err(LayoutInputError::InvalidDiscretionary { item: 0 });
        for (text, len) in [("a\nb", 1), ("a\r\nb", 2), ("a\u{2028}b", 3)] {
            let nl = TextRange::new(1, 1 + len);
            let r = check(text, DiscretionaryNode::hard_break(nl, false));
            assert!(r.is_ok(), "{text:?}: {r:?}");
            // The terminator is the whole range: a shorter one splits `\r\n`
            // or a code point, a longer one swallows text.
            assert!(
                check(
                    text,
                    DiscretionaryNode::hard_break(TextRange::new(1, 2 + len), false)
                )
                .is_err()
            );
        }
        // Not a terminator.
        assert_eq!(
            check(
                "abcd",
                DiscretionaryNode::hard_break(TextRange::new(1, 2), false)
            ),
            bad
        );
        // A mid-paragraph hard break must own its newline; only the end may not.
        assert_eq!(
            check(
                "abcd",
                DiscretionaryNode::hard_break(TextRange::empty_at(1), false)
            ),
            bad
        );
        // A soft opportunity owns nothing.
        let mut soft = DiscretionaryNode::direct(1);
        soft.byte_range = TextRange::new(1, 2);
        assert_eq!(check("abcd", soft), bad);
        assert_eq!(DiscretionaryNode::direct(3).byte_offset(), 3);
    }

    /// A text box with every cluster 4 wide.
    fn with_width(n: WordNode) -> WordNode {
        match n {
            WordNode::Box(BoxNode::Text {
                run_index,
                byte_range,
                atoms,
                mut clusters,
                ..
            }) => {
                clusters.iter_mut().for_each(|c| c.natural_width = 4.0);
                let natural_width = clusters.len() as f32 * 4.0;
                WordNode::Box(BoxNode::Text {
                    run_index,
                    byte_range,
                    atoms,
                    clusters,
                    natural_width,
                })
            }
            other => other,
        }
    }

    #[test]
    fn cjk_punct_adjust_deltas_need_a_neighbour_and_stay_non_negative() {
        let o = LayoutOptions::default();
        // Two boxes whose edge clusters are 4 wide, around the break.
        let adjust = |pre: f32, post: f32| {
            let mut d = DiscretionaryNode::direct(2);
            d.action = BreakAction::CjkPunctAdjust;
            d.pre_break_width = pre;
            d.post_break_width = post;
            with_items(vec![
                with_width(text_box(vec![atom(0, 2)])),
                WordNode::Discretionary(d),
                with_width(text_box(vec![atom(2, 4)])),
            ])
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        // A trim to exactly zero is allowed; a restore is positive.
        assert!(check(adjust(-4.0, -4.0)).is_ok());
        assert!(check(adjust(2.0, 0.0)).is_ok());
        let bad = Err(LayoutInputError::InvalidDiscretionary { item: 1 });
        // An adjustment that adjusts nothing is no adjustment.
        assert_eq!(check(adjust(0.0, 0.0)), bad);
        assert_eq!(check(adjust(-4.5, 0.0)), bad); // last cluster would go negative
        assert_eq!(check(adjust(0.0, -4.5)), bad); // first cluster would go negative
        // A non-zero delta needs the text box it applies to.
        let mut a = adjust(-1.0, 0.0);
        a.items.remove(0);
        assert_eq!(
            check(a),
            Err(LayoutInputError::InvalidDiscretionary { item: 0 })
        );
        let mut a = adjust(0.0, -1.0);
        a.items.remove(2);
        assert_eq!(check(a), bad);
        // A delta on one side needs no neighbour on the other.
        let mut a = adjust(-1.0, 0.0);
        a.items.remove(2);
        assert!(check(a).is_ok());
    }

    /// Sets `IN_ATOMIC_EDIT_SPAN` on exactly the atoms inside an atomic span, as
    /// the frontend does, and re-aggregates the cluster flags to match.
    fn flag_atomic(a: &mut ParagraphAst) {
        let spans = a.atomic_spans.clone();
        for n in &mut a.items {
            if let WordNode::Box(BoxNode::Text {
                atoms, clusters, ..
            }) = n
            {
                for at in atoms.iter_mut() {
                    let inside = spans
                        .iter()
                        .any(|s| s.byte_range.contains_range(at.byte_range));
                    at.flags = if inside {
                        AtomFlags::IN_ATOMIC_EDIT_SPAN
                    } else {
                        AtomFlags::EMPTY
                    };
                }
                for c in clusters.iter_mut() {
                    c.flags = AtomFlags::for_cluster(&atoms[c.atom_range.clone()]);
                }
            }
        }
    }

    fn bnd(id: u64, edge: SpanEdge, at: usize) -> WordNode {
        WordNode::Box(BoxNode::SpanBoundary {
            byte_range: TextRange::empty_at(at),
            span_id: id,
            edge,
            width: 2.0,
            bidi_level: 0,
        })
    }

    #[test]
    fn span_boundaries_nest_in_lifo_order_and_equal_ranges_follow_the_slice_order() {
        use crate::frontend::BoxDecorationBreak::Slice;
        use SpanEdge::{Leading as L, Trailing as T};
        let o = LayoutOptions::default();
        let ast = |boundaries: [(u64, SpanEdge, usize); 4]| {
            let [b0, b1, b2, b3] = boundaries.map(|(id, e, at)| bnd(id, e, at));
            // "a" [ b0 b1 "bc" b2 b3 ] "d"; spans 1 (outer) and 2 (inner) share 1..3.
            let mut a = with_items(vec![
                text_box(vec![atom(0, 1)]),
                b0,
                b1,
                text_box(vec![atom(1, 2), atom(2, 3)]),
                b2,
                b3,
                text_box(vec![atom(3, 4)]),
            ]);
            a.decoration_spans = vec![chip(1, Slice), chip(2, Slice)];
            a
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        // The lower index is the outer span: L1 L2 .. T2 T1.
        assert!(check(ast([(1, L, 1), (2, L, 1), (2, T, 3), (1, T, 3)])).is_ok());
        // Consistent LIFO but inverted (span 2 outside span 1): rejected.
        assert!(matches!(
            check(ast([(2, L, 1), (1, L, 1), (1, T, 3), (2, T, 3)])),
            Err(LayoutInputError::InvalidSpanBoundary { .. })
        ));
        // Crossing (L1 L2 T1 T2) breaks LIFO.
        assert_eq!(
            check(ast([(1, L, 1), (2, L, 1), (1, T, 3), (2, T, 3)])),
            Err(LayoutInputError::InvalidSpanBoundary { item: 4 })
        );
        // A span opened twice.
        assert!(matches!(
            check(ast([(1, L, 1), (1, L, 1), (1, T, 3), (1, T, 3)])),
            Err(LayoutInputError::InvalidSpanBoundary { .. })
        ));
        // Nested spans with different ranges obey the same stack.
        let mut nested = ast([(1, L, 1), (2, L, 1), (2, T, 3), (1, T, 3)]);
        nested.decoration_spans[1].byte_range = TextRange::new(1, 3);
        nested.decoration_spans[0].byte_range = TextRange::new(1, 4);
        assert_eq!(
            check_boundary_order(&nested),
            Ok(()),
            "inner span closes first"
        );
    }

    #[test]
    fn the_paragraph_end_sentinel_is_empty_last_and_unique() {
        let o = LayoutOptions::default();
        let check = |text: &str, items: Vec<WordNode>| {
            let mut a = with_raw_items(items);
            a.runs[0].byte_range = TextRange::new(0, text.len());
            check_layout_inputs(&a, text, &constraints(), &o)
        };
        let newline = |a: usize, b: usize| {
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(a, b), false))
        };
        assert!(check("abcd", vec![sentinel(4)]).is_ok());
        // An empty paragraph is just the sentinel.
        assert!(check("", vec![sentinel(0)]).is_ok());
        // Text ending in a newline: the real break, then the sentinel.
        assert!(check("ab\n", vec![newline(2, 3), sentinel(3)]).is_ok());
        let bad = |item| Err(LayoutInputError::InvalidDiscretionary { item });
        // The sentinel sits at the end of the text, not before it.
        assert_eq!(check("abcd", vec![sentinel(3)]), bad(0));
        // It must be the final discretionary.
        let soft = WordNode::Discretionary(DiscretionaryNode::direct(4));
        assert_eq!(check("abcd", vec![sentinel(4), soft]), bad(0));
        // At most once.
        assert_eq!(check("abcd", vec![sentinel(4), sentinel(4)]), bad(0));
        // A sentinel owns no bytes; a real break owns some.
        assert_eq!(check("ab\n", vec![newline(2, 3), newline(3, 3)]), bad(1));
        let mut owning = DiscretionaryNode::hard_break(TextRange::new(2, 3), true);
        owning.byte_range = TextRange::new(2, 3);
        assert_eq!(check("ab\n", vec![WordNode::Discretionary(owning)]), bad(0));
    }

    #[test]
    fn the_items_must_end_with_the_sentinel() {
        let o = LayoutOptions::default();
        let check = |text: &str, items: Vec<WordNode>| {
            let mut a = with_raw_items(items);
            a.runs[0].byte_range = TextRange::new(0, text.len());
            check_layout_inputs(&a, text, &constraints(), &o)
        };
        let missing = Err(LayoutInputError::MissingParagraphEnd);
        // No items at all, even for an empty paragraph.
        assert_eq!(check("", vec![]), missing);
        // A real hard break is not the sentinel: "ab\n" needs both.
        let newline =
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(2, 3), false));
        assert_eq!(check("ab\n", vec![newline]), missing);
        // A soft opportunity or a box last.
        let soft = WordNode::Discretionary(DiscretionaryNode::direct(2));
        assert_eq!(check("abcd", vec![soft]), missing);
        let text = text_box(vec![atom(0, 1)]);
        assert_eq!(check("abcd", vec![sentinel(4), text]), missing);
        // Four traces from the line-emission rule: N newlines, N + 1 mandatory
        // breaks, so N + 1 lines.
        let mandatory = |items: &[WordNode]| {
            items
                .iter()
                .filter(|n| matches!(n, WordNode::Discretionary(d) if d.is_mandatory()))
                .count()
        };
        let nl = |a: usize, b: usize| {
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(a, b), false))
        };
        let cases: [(&str, Vec<WordNode>); 4] = [
            ("", vec![sentinel(0)]),
            (
                "abc",
                vec![
                    text_box(vec![atom(0, 1), atom(1, 2), atom(2, 3)]),
                    sentinel(3),
                ],
            ),
            (
                "abc\n",
                vec![
                    text_box(vec![atom(0, 1), atom(1, 2), atom(2, 3)]),
                    nl(3, 4),
                    sentinel(4),
                ],
            ),
            (
                "a\n\nb",
                vec![
                    text_box(vec![atom(0, 1)]),
                    nl(1, 2),
                    nl(2, 3),
                    text_box(vec![atom(3, 4)]),
                    sentinel(4),
                ],
            ),
        ];
        for (text, items) in cases {
            let newlines = text.matches('\n').count();
            assert_eq!(mandatory(&items), newlines + 1, "{text:?}");
            assert!(check(text, items).is_ok(), "{text:?}");
        }
    }

    #[test]
    fn glue_tiers_and_the_compression_pool_follow_the_kind() {
        let o = LayoutOptions::default();
        let node = |kind, tier, natural, stretch, shrink| {
            WordNode::Glue(GlueNode {
                byte_range: TextRange::empty_at(1),
                kind,
                tier,
                natural_width: natural,
                max_stretch: stretch,
                max_shrink: shrink,
            })
        };
        let check = |g| check_layout_inputs(&with_items(vec![g]), "abcd", &constraints(), &o);
        assert_eq!(
            GlueKind::CjkPunctCompression.default_tier(),
            GlueTier::Tier0
        );
        let pool = |t, n, st, sh| node(GlueKind::CjkPunctCompression, t, n, st, sh);
        assert!(check(pool(GlueTier::Tier0, 0.0, 0.0, 5.0)).is_ok());
        let bad = Err(LayoutInputError::InvalidGlue { item: 0 });
        assert_eq!(check(pool(GlueTier::Tier1, 0.0, 0.0, 5.0)), bad);
        assert_eq!(check(pool(GlueTier::Tier0, 1.0, 0.0, 5.0)), bad);
        assert_eq!(check(pool(GlueTier::Tier0, 0.0, 1.0, 5.0)), bad);
        // Tier 2 is derived from cluster flags, never emitted as glue.
        assert_eq!(
            check(node(GlueKind::CjkAutospace, GlueTier::Tier2, 0.0, 1.0, 0.0)),
            bad
        );
    }

    #[test]
    fn span_boundary_bidi_level_follows_the_atom_inside_the_span() {
        use crate::frontend::BoxDecorationBreak::Slice;
        let o = LayoutOptions::default();
        let mut inner = atom(1, 2);
        inner.bidi_level = 1;
        // Edge `edge` carries `level`; the other edge carries its correct level.
        let build = |edge: SpanEdge, level: u8| {
            let boundary = |e: SpanEdge, correct: u8, at: usize| {
                WordNode::Box(BoxNode::SpanBoundary {
                    byte_range: TextRange::empty_at(at),
                    span_id: 5,
                    edge: e,
                    width: 2.0,
                    bidi_level: if e == edge { level } else { correct },
                })
            };
            let mut a = with_items(vec![
                text_box(vec![atom(0, 1)]),
                boundary(SpanEdge::Leading, 1, 1),
                text_box(vec![inner.clone(), atom(2, 3)]),
                boundary(SpanEdge::Trailing, 0, 3),
            ]);
            a.decoration_spans = vec![chip(5, Slice)]; // bytes 1..3
            a
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        // First atom inside the span is level 1, the last is level 0.
        assert!(check(build(SpanEdge::Leading, 1)).is_ok());
        assert!(check(build(SpanEdge::Trailing, 0)).is_ok());
        assert_eq!(
            check(build(SpanEdge::Leading, 0)),
            Err(LayoutInputError::InvalidSpanBoundary { item: 1 })
        );
        assert_eq!(
            check(build(SpanEdge::Trailing, 1)),
            Err(LayoutInputError::InvalidSpanBoundary { item: 3 })
        );
    }

    #[test]
    fn items_progress_monotonically_and_spans_enclose_their_items() {
        use crate::frontend::BoxDecorationBreak::Slice;
        use SpanEdge::{Leading as L, Trailing as T};
        let o = LayoutOptions::default();
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        let a_ = || text_box(vec![atom(0, 1)]);
        let b_ = || text_box(vec![atom(1, 2)]);
        let c_ = || text_box(vec![atom(2, 3)]);
        let d_ = || text_box(vec![atom(3, 4)]);
        let brk = |at: usize| WordNode::Discretionary(DiscretionaryNode::direct(at));
        let span = |a: ParagraphAst| {
            let mut a = a;
            a.decoration_spans = vec![chip(5, Slice)]; // bytes 1..3
            a
        };
        // Monotone progression: a later item may not start before the previous
        // one ends (zero-width nodes may share an offset).
        assert_eq!(
            check(with_items(vec![b_(), a_()])),
            Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
        );
        assert!(check(with_items(vec![a_(), brk(1), b_(), brk(2), c_()])).is_ok());
        assert_eq!(
            check(with_items(vec![a_(), b_(), brk(1)])),
            Err(LayoutInputError::ItemsOutOfOrder { item: 2 })
        );
        // Reopening a closed span is caught by the order check.
        assert_eq!(
            check(span(with_items(vec![
                a_(),
                bnd(5, L, 1),
                b_(),
                c_(),
                bnd(5, T, 3),
                bnd(5, L, 1),
                bnd(5, T, 3),
                d_(),
            ]))),
            Err(LayoutInputError::ItemsOutOfOrder { item: 5 })
        );
        // The well-formed chip: break before it, content between, break after.
        let good = || {
            vec![
                a_(),
                brk(1),
                bnd(5, L, 1),
                b_(),
                brk(2),
                c_(),
                bnd(5, T, 3),
                brk(3),
                d_(),
            ]
        };
        assert!(check(span(with_items(good()))).is_ok());
        // A break at the start after Leading would strand the boundary on the
        // previous line; one at the end before Trailing likewise.
        let mut late = good();
        late.swap(1, 2);
        assert_eq!(
            check(span(with_items(late))),
            Err(LayoutInputError::SpanEnclosure { span: 0, item: 2 })
        );
        let mut early = good();
        early.swap(6, 7);
        assert_eq!(
            check(span(with_items(early))),
            Err(LayoutInputError::SpanEnclosure { span: 0, item: 6 })
        );
        // Content outside the chip may not sit between its boundaries, and
        // content inside may not sit outside them.
        let outside = span(with_items(vec![
            a_(),
            bnd(5, L, 1),
            b_(),
            c_(),
            d_(),
            bnd(5, T, 3),
        ]));
        assert!(check(outside).is_err());
        let inside = span(with_items(vec![
            a_(),
            b_(),
            bnd(5, L, 1),
            c_(),
            bnd(5, T, 3),
            d_(),
        ]));
        assert!(check(inside).is_err());
        // A box that straddles a span edge must have been split by the frontend.
        let straddle = closed(span(with_items(vec![text_box(vec![
            atom(0, 1),
            atom(1, 2),
        ])])));
        // (No boundary can sit inside a box's range, so monotone order catches it.)
        assert!(matches!(
            check(straddle),
            Err(LayoutInputError::ItemsOutOfOrder { .. })
        ));
        // Nested spans: the inner chip's nodes are inside the outer one.
        let mut nested = with_items(vec![
            bnd(1, L, 0),
            a_(),
            bnd(2, L, 1),
            b_(),
            bnd(2, T, 2),
            c_(),
            d_(),
            bnd(1, T, 4),
        ]);
        let mut inner = chip(2, Slice);
        inner.byte_range = TextRange::new(1, 2);
        let mut outer = chip(1, Slice);
        outer.byte_range = TextRange::new(0, 4);
        nested.decoration_spans = vec![outer, inner];
        // The sentinel at 4 follows Trailing(1) at 4: no stranded boundary.
        let r = check(nested);
        assert!(r.is_ok(), "{r:?}");
    }

    #[test]
    fn cjk_autospace_at_a_span_edge_sits_outside_its_boundaries() {
        use crate::frontend::BoxDecorationBreak::Slice;
        use SpanEdge::{Leading as L, Trailing as T};
        let o = LayoutOptions::default();
        let autospace = |at: usize| {
            WordNode::Glue(GlueNode {
                byte_range: TextRange::empty_at(at),
                kind: GlueKind::CjkAutospace,
                tier: GlueTier::Tier1,
                natural_width: 2.0,
                max_stretch: 1.0,
                max_shrink: 1.0,
            })
        };
        let brk = |at: usize| WordNode::Discretionary(DiscretionaryNode::direct(at));
        let a_ = || text_box(vec![atom(0, 1)]);
        let b_ = || text_box(vec![atom(1, 2)]);
        let c_ = || text_box(vec![atom(2, 3)]);
        let d_ = || text_box(vec![atom(3, 4)]);
        let span = |items| {
            let mut a = with_items(items);
            a.decoration_spans = vec![chip(5, Slice)]; // bytes 1..3
            a
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        // Seam spacing, then the break, then the chip; the chip's end mirrors it.
        let good = vec![
            a_(),
            autospace(1),
            brk(1),
            bnd(5, L, 1),
            b_(),
            c_(),
            bnd(5, T, 3),
            autospace(3),
            brk(3),
            d_(),
        ];
        assert!(check(span(good.clone())).is_ok());
        // The start seam after Leading would strand the spacing inside the chip.
        let mut late = good.clone();
        late.swap(1, 3);
        late.swap(2, 3); // a_, bnd L, autospace, brk ... (autospace now after Leading)
        assert!(matches!(
            check(span(late)),
            Err(LayoutInputError::SpanEnclosure { span: 0, .. })
        ));
        // The end seam before Trailing likewise.
        let mut early = good.clone();
        early.swap(6, 7);
        assert_eq!(
            check(span(early)),
            Err(LayoutInputError::SpanEnclosure { span: 0, item: 6 })
        );
        // Two adjacent chips share the seam at 2: canonical order
        // [A Trailing, Glue(CjkAutospace), Discretionary, B Leading].
        let mut other = chip(6, Slice);
        other.byte_range = TextRange::new(2, 4);
        let mut a = with_items(vec![
            a_(),
            bnd(5, L, 1),
            b_(),
            bnd(5, T, 3),
            autospace(3),
            brk(3),
            bnd(6, L, 3),
            d_(),
            bnd(6, T, 4),
        ]);
        a.decoration_spans = vec![chip(5, Slice), other];
        // Chip 5 spans 1..3 and chip 6 starts at 2: fix the second to 3..4.
        a.decoration_spans[1].byte_range = TextRange::new(3, 4);
        let r = check(a.clone());
        assert!(r.is_ok(), "{r:?}");
        // The break before the seam glue is the wrong way round.
        let mut swapped = a.clone();
        swapped.items.swap(4, 5);
        assert_eq!(
            check(swapped),
            Err(LayoutInputError::ItemsOutOfOrder { item: 5 })
        );
        // B's Leading before the seam glue or break strands them inside B.
        a.items.swap(5, 6);
        assert!(check(a).is_err());
    }

    #[test]
    fn row_checks_from_the_coderabbit_pass() {
        use crate::frontend::BoxDecorationBreak::{Clone, Slice};
        let o = LayoutOptions::default();
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);

        // Hyphenation breaks are flagged.
        let mut a = hyphenated(opportunity(1, 0, 0));
        if let WordNode::Discretionary(d) = &mut a.items[1] {
            d.flagged = false;
        }
        assert_eq!(
            check_layout_inputs(&a, "abé", &constraints(), &o),
            Err(LayoutInputError::InvalidHyphenOpportunity { item: 1 })
        );

        // A Clone span the break is strictly inside must be listed.
        let mut d = DiscretionaryNode::direct(2);
        d.penalty = DiscretionaryNode::CLONED_SPAN_PENALTY;
        let mut a = with_items(vec![WordNode::Discretionary(d)]);
        a.decoration_spans = vec![chip(5, Clone)]; // 1..3
        assert_eq!(
            check(closed(a)),
            Err(LayoutInputError::InvalidDiscretionary { item: 1 })
        );
        // A Slice span needs no listing.
        let mut a = with_items(vec![WordNode::Discretionary(DiscretionaryNode::direct(2))]);
        a.decoration_spans = vec![chip(5, Slice)];
        assert!(check(closed(a)).is_ok());

        // Synthetic glue owns no bytes.
        for kind in [GlueKind::CjkAutospace, GlueKind::CjkPunctCompression] {
            let tier = kind.default_tier();
            let a = with_items(vec![WordNode::Glue(GlueNode {
                byte_range: TextRange::new(1, 2),
                kind,
                tier,
                natural_width: 0.0,
                max_stretch: 0.0,
                max_shrink: 1.0,
            })]);
            assert_eq!(check(a), Err(LayoutInputError::InvalidGlue { item: 0 }));
        }

        // A boundary's width is its edge's padding; its level is in range even
        // when the span holds no atom to compare with.
        let mut inner = chip(5, Slice);
        inner.start_padding = 2.0;
        inner.end_padding = 6.0;
        let build = |lead_w: f32, trail_w: f32, level: u8| {
            let node = |edge, width, bidi_level| {
                WordNode::Box(BoxNode::SpanBoundary {
                    byte_range: TextRange::empty_at(if edge == SpanEdge::Leading { 1 } else { 3 }),
                    span_id: 5,
                    edge,
                    width,
                    bidi_level,
                })
            };
            let mut a = with_items(vec![
                node(SpanEdge::Leading, lead_w, level),
                node(SpanEdge::Trailing, trail_w, level),
            ]);
            a.decoration_spans = vec![inner];
            a
        };
        assert!(check(build(2.0, 6.0, 0)).is_ok());
        let bad = |item| Err(LayoutInputError::InvalidSpanBoundary { item });
        assert_eq!(check(build(6.0, 6.0, 0)), bad(0));
        assert_eq!(check(build(2.0, 2.0, 0)), bad(1));
        assert_eq!(check(build(2.0, 6.0, 127)), bad(0));

        // Half of a CRLF pair is not a terminator.
        let run_over = |text: &str, items: Vec<WordNode>| {
            let mut a = with_items_for(text.len(), items);
            a.runs[0].byte_range = TextRange::new(0, text.len());
            check_layout_inputs(&a, text, &constraints(), &o)
        };
        let nl = |a: usize, b: usize| {
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(a, b), false))
        };
        assert!(run_over("a\r\nb", vec![nl(1, 3)]).is_ok());
        assert!(run_over("a\r\nb", vec![nl(1, 2)]).is_err()); // lone CR of a CRLF
        assert!(run_over("a\r\nb", vec![nl(2, 3)]).is_err()); // lone LF of a CRLF
        assert!(run_over("a\rb", vec![nl(1, 2)]).is_ok());

        // A text box lies in its run and caches the total of its clusters.
        let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 2)])]);
        a.runs[0].byte_range = TextRange::new(0, 1);
        assert_eq!(check(a), Err(LayoutInputError::ItemOutsideRun { item: 0 }));
        let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
        if let WordNode::Box(BoxNode::Text { natural_width, .. }) = &mut a.items[0] {
            *natural_width = 9.0; // the cluster is 0 wide
        }
        assert_eq!(
            check(a),
            Err(LayoutInputError::InvalidTextWidth { item: 0 })
        );
        let mut a = with_items(vec![text_box(vec![atom(0, 1)])]);
        if let WordNode::Box(BoxNode::Text { natural_width, .. }) = &mut a.items[0] {
            *natural_width = f32::NAN;
        }
        assert_eq!(
            check(a),
            Err(LayoutInputError::InvalidTextWidth { item: 0 })
        );
    }

    #[test]
    fn every_decoration_span_needs_both_boundaries_even_without_padding() {
        use crate::frontend::BoxDecorationBreak::Slice;
        use SpanEdge::{Leading as L, Trailing as T};
        let o = LayoutOptions::default();
        // Zero padding: the boundary nodes are zero wide.
        let zb = |id: u64, e: SpanEdge, at: usize| match bnd(id, e, at) {
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range,
                span_id,
                edge,
                bidi_level,
                ..
            }) => WordNode::Box(BoxNode::SpanBoundary {
                byte_range,
                span_id,
                edge,
                width: 0.0,
                bidi_level,
            }),
            other => other,
        };
        let zero = |id: u64| {
            let mut s = chip(id, Slice);
            s.start_padding = 0.0;
            s.end_padding = 0.0;
            s
        };
        let build = |boundaries: Vec<WordNode>| {
            // "a", the boundaries (an empty chip), "d": boxes split at the edges.
            let mut items = vec![text_box(vec![atom(0, 1)])];
            items.extend(boundaries);
            items.push(text_box(vec![atom(3, 4)]));
            let mut a = with_items(items);
            a.decoration_spans = vec![zero(5)]; // bytes 1..3
            a
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        assert!(check(build(vec![zb(5, L, 1), zb(5, T, 3)])).is_ok());
        // Zero padding does not excuse a missing node.
        let missing = Err(LayoutInputError::MissingSpanBoundary { span: 0 });
        assert_eq!(check(build(vec![])), missing);
        assert_eq!(check(build(vec![zb(5, L, 1)])), missing);
        // A Trailing with no open Leading is a misplaced boundary.
        assert_eq!(
            check(build(vec![zb(5, T, 3)])),
            Err(LayoutInputError::InvalidSpanBoundary { item: 1 })
        );
        // Exactly one of each, at the span's own edges.
        for wrong in [
            vec![zb(5, L, 0), zb(5, T, 3)],
            vec![zb(5, L, 1), zb(5, T, 2)],
            vec![zb(5, L, 1), zb(5, L, 1), zb(5, T, 3)],
            vec![zb(5, L, 1), zb(5, T, 3), zb(5, T, 3)],
        ] {
            let r = check(build(wrong));
            assert!(
                matches!(r, Err(LayoutInputError::InvalidSpanBoundary { .. })),
                "{r:?}"
            );
        }
    }

    #[test]
    fn span_streams_are_validated() {
        use crate::frontend::BoxDecorationBreak::Slice;
        let o = LayoutOptions::default();
        let check = |deco: Vec<InlineDecorationSpan>, atomic: Vec<AtomicEditSpan>| {
            let mut a = with_items(
                (0..4)
                    .map(|i| text_box(vec![atom(i, i + 1)]))
                    .collect::<Vec<_>>(),
            );
            a.decoration_spans = deco;
            a.atomic_spans = atomic;
            flag_atomic(&mut a);
            check_layout_inputs(&closed(a), "abcd", &constraints(), &o)
        };
        let atomic = |id, r: TextRange| AtomicEditSpan {
            span_id: id,
            byte_range: r,
        };
        assert!(check(vec![chip(1, Slice)], vec![atomic(1, TextRange::new(1, 3))]).is_ok());
        // Decoration: duplicate id, bad padding, outside the text.
        let mut padded = chip(2, Slice);
        padded.start_padding = f32::NAN;
        let mut outside = chip(3, Slice);
        outside.byte_range = TextRange::new(1, 9);
        for (list, index) in [
            (vec![chip(1, Slice), chip(1, Slice)], 1),
            (vec![padded], 0),
            (vec![chip(1, Slice), outside], 1),
        ] {
            assert_eq!(
                check(list, vec![]),
                Err(LayoutInputError::InvalidDecorationSpan { span: index })
            );
        }
        // Nesting: containment is fine (also at equal edges), a crossing is not,
        // nor is going deeper than the limit.
        let span = |id, a, b| {
            let mut s = chip(id, Slice);
            s.byte_range = TextRange::new(a, b);
            s
        };
        assert!(check(vec![span(1, 0, 4), span(2, 1, 3), span(3, 1, 2)], vec![]).is_ok());
        assert!(check(vec![span(1, 0, 2), span(2, 2, 4)], vec![]).is_ok());
        assert_eq!(
            check(vec![span(1, 0, 3), span(2, 1, 4)], vec![]),
            Err(LayoutInputError::InvalidDecorationSpan { span: 1 })
        );
        let too_deep: Vec<_> = (0..=crate::frontend::MAX_DECORATION_NESTING_DEPTH)
            .map(|i| span(i as u64, 0, 4))
            .collect();
        assert!(matches!(
            check(too_deep, vec![]),
            Err(LayoutInputError::InvalidDecorationSpan { .. })
        ));
        // Atomic spans are strictly disjoint: touching is fine, nesting is not.
        assert!(
            check(
                vec![],
                vec![
                    atomic(1, TextRange::new(0, 2)),
                    atomic(2, TextRange::new(2, 4))
                ]
            )
            .is_ok()
        );
        for (a, b) in [((0, 3), (2, 4)), ((0, 4), (1, 2))] {
            assert_eq!(
                check(
                    vec![],
                    vec![
                        atomic(1, TextRange::new(a.0, a.1)),
                        atomic(2, TextRange::new(b.0, b.1))
                    ]
                ),
                Err(LayoutInputError::InvalidAtomicSpan { span: 1 })
            );
        }
        // Atomic: empty, outside the text, duplicate id.
        for (list, index) in [
            (vec![atomic(1, TextRange::empty_at(2))], 0),
            (vec![atomic(1, TextRange::new(1, 9))], 0),
            (
                vec![
                    atomic(1, TextRange::new(0, 1)),
                    atomic(1, TextRange::new(2, 3)),
                ],
                1,
            ),
        ] {
            assert_eq!(
                check(vec![], list),
                Err(LayoutInputError::InvalidAtomicSpan { span: index })
            );
        }
    }

    #[test]
    fn atomic_span_edges_must_be_atom_boundaries() {
        let o = LayoutOptions::default();
        let build = |start, end| {
            let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 4)])]);
            a.atomic_spans = vec![AtomicEditSpan {
                span_id: 1,
                byte_range: TextRange::new(start, end),
            }];
            a
        };
        let check = |mut a: ParagraphAst| {
            flag_atomic(&mut a);
            check_layout_inputs(&a, "aéb", &constraints(), &o)
        };
        assert!(check(build(1, 4)).is_ok());
        // Offset 2 is inside the atom 1..4 (a char boundary of "é" is 1 and 3,
        // so use 3: still strictly inside the atom).
        assert_eq!(
            check(build(1, 3)),
            Err(LayoutInputError::InvalidAtomicSpan { span: 0 })
        );
        assert_eq!(
            check(build(3, 4)),
            Err(LayoutInputError::InvalidAtomicSpan { span: 0 })
        );
    }

    #[test]
    fn atomic_spans_wrap_content_not_whitespace() {
        let o = LayoutOptions::default();
        let ws = |kind: &str, range: (usize, usize)| match kind {
            "glue" => WordNode::Glue(GlueNode {
                byte_range: TextRange::new(range.0, range.1),
                kind: GlueKind::WordSpace,
                tier: GlueTier::Tier0,
                natural_width: 3.0,
                max_stretch: 0.0,
                max_shrink: 0.0,
            }),
            _ => WordNode::Tab {
                byte_range: TextRange::new(range.0, range.1),
            },
        };
        // "ab", two spaces as one glue (2..4), "cd".
        let build = |kind: &str, ws_range: (usize, usize), span: (usize, usize)| {
            let mut a = with_items_for(
                6,
                vec![
                    text_box(vec![atom(0, 1), atom(1, 2)]),
                    ws(kind, ws_range),
                    text_box(vec![atom(4, 5), atom(5, 6)]),
                ],
            );
            a.atomic_spans = vec![AtomicEditSpan {
                span_id: 1,
                byte_range: TextRange::new(span.0, span.1),
            }];
            a
        };
        let check = |mut a: ParagraphAst| {
            flag_atomic(&mut a);
            check_layout_inputs(&a, "ab  cd", &constraints(), &o)
        };
        let bad = Err(LayoutInputError::InvalidAtomicSpan { span: 0 });
        // Both edges at atom edges, with or without glue between them.
        assert!(check(build("glue", (2, 4), (0, 2))).is_ok());
        assert!(check(build("glue", (2, 4), (1, 5))).is_ok());
        assert!(check(build("glue", (2, 4), (0, 6))).is_ok());
        // An edge strictly inside the glue run, or the tab, is invalid ...
        assert_eq!(check(build("glue", (2, 4), (0, 3))), bad);
        assert_eq!(check(build("glue", (2, 4), (3, 6))), bad);
        assert_eq!(check(build("tab", (2, 4), (3, 5))), bad);
        // ... and so is a span made of whitespace alone (its inner edge is not
        // the edge of any atom or object), or one ending on an unowned offset.
        assert_eq!(check(build("glue", (2, 4), (2, 4))), bad);
        let mut a = with_items(vec![text_box(vec![atom(0, 1), atom(1, 2)])]);
        a.atomic_spans = vec![AtomicEditSpan {
            span_id: 1,
            byte_range: TextRange::new(1, 3),
        }];
        flag_atomic(&mut a);
        assert_eq!(check_layout_inputs(&a, "abcd", &constraints(), &o), bad);
        // An inline object's edges count.
        let mut a = with_items(vec![
            text_box(vec![atom(0, 1)]),
            WordNode::Box(BoxNode::InlineObject {
                byte_range: TextRange::new(1, 3),
                box_id: 1,
                width: 1.0,
                ascent: 1.0,
                descent: 0.0,
                overflow_policy: BoxOverflowPolicy::Clip,
            }),
        ]);
        a.atomic_spans = vec![AtomicEditSpan {
            span_id: 1,
            byte_range: TextRange::new(1, 3),
        }];
        flag_atomic(&mut a);
        assert!(check_layout_inputs(&a, "abcd", &constraints(), &o).is_ok());
    }

    #[test]
    fn wrap_width_reaches_the_line_context() {
        let c =
            check_layout_inputs(&ast(), "\n", &constraints(), &LayoutOptions::default()).unwrap();
        assert_eq!(c.wrap_width, constraints().wrap_width);
    }

    #[test]
    fn strut_inputs_are_validated_after_resolution() {
        let negative = LayoutOptions {
            line_height: crate::middleend::LineHeight::Fixed(-1.0),
            ..Default::default()
        };
        assert_eq!(
            check_layout_inputs(&ast(), "\n", &constraints(), &negative),
            Err(LayoutInputError::InvalidStrut)
        );
        // Finite inputs whose resolved height overflows.
        let huge = LayoutOptions {
            line_height: crate::middleend::LineHeight::Fixed(f32::MAX),
            ..Default::default()
        };
        let mut a = ast();
        a.strut_font_size = 10.0;
        assert_eq!(
            check_layout_inputs(&a, "\n", &constraints(), &huge),
            Err(LayoutInputError::InvalidStrut)
        );
    }
}
