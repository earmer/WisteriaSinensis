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
use crate::types::{BidiLevel, GraphemeAtom, TextRange};
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
    /// A discretionary node has a non-finite width, a forbidden penalty
    /// (`>= FORBIDDEN_BREAK_PENALTY`; a forbidden break is an omitted node), a
    /// `HardBreak` action that disagrees with a mandatory penalty, a
    /// `byte_range` that breaks the ownership rule (soft: empty; hard break: a
    /// line terminator, or empty at the paragraph end), a repeated id in
    /// `cloned_spans`, or a `CjkPunctAdjust` override that is not a finite,
    /// non-negative advance or has no text box beside it to apply to.
    InvalidDiscretionary {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A hyphenation discretionary does not sit between two non-empty text
    /// boxes at its `byte_offset`, disagrees with its `HyphenOpportunity`, or
    /// one of the cuts does not end on an atom boundary of the box it cuts.
    InvalidHyphenOpportunity {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A span boundary is not an empty range, has a non-finite or negative
    /// width, or a `bidi_level` that differs from the level of the first (for a
    /// `Leading` edge) or last (`Trailing`) atom inside its span.
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
    /// must be proper containment) or nests deeper than
    /// `MAX_DECORATION_NESTING_DEPTH`.
    InvalidDecorationSpan {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
    },
    /// An atomic span is empty, does not fit the text, repeats a `span_id`,
    /// overlaps or nests with another atomic span, or has an end that is not a
    /// grapheme atom boundary.
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
            Self::InvalidAtomicSpan { span } => {
                write!(f, "atomic span {span} is invalid or duplicated")
            }
        }
    }
}

impl std::error::Error for LayoutInputError {}

/// Whether a hyphenation discretionary at `items[index]` is well formed: its
/// `HyphenOpportunity` agrees with it, it sits between two non-empty text
/// boxes that meet at its offset, and each cut ends on an atom boundary of the
/// box it cuts.
fn hyphen_valid(
    items: &[WordNode],
    index: usize,
    d: &DiscretionaryNode,
    h: &HyphenOpportunity,
) -> bool {
    let text_of = |i: Option<usize>| match i.and_then(|i| items.get(i)) {
        Some(WordNode::Box(BoxNode::Text {
            byte_range, atoms, ..
        })) => Some((*byte_range, atoms)),
        _ => None,
    };
    let (Some((prev, prev_atoms)), Some((next, next_atoms))) =
        (text_of(index.checked_sub(1)), text_of(index.checked_add(1)))
    else {
        return false;
    };
    let off = h.byte_offset;
    if d.byte_range != TextRange::empty_at(off) || prev.end != off || next.start != off {
        return false;
    }
    if prev.is_empty() || next.is_empty() {
        return false;
    }
    let boundary = |atoms: &[GraphemeAtom], pos: usize, end: usize| {
        pos == end || atoms.iter().any(|a| a.byte_range.start == pos)
    };
    let prefix = off.checked_sub(h.prefix_cut_bytes as usize);
    let suffix = off.checked_add(h.suffix_cut_bytes as usize);
    prefix.is_some_and(|p| p >= prev.start && boundary(prev_atoms, p, prev.end))
        && suffix.is_some_and(|q| q <= next.end && boundary(next_atoms, q, next.end))
}

fn fits(text: &str, r: crate::types::TextRange) -> bool {
    r.start <= r.end
        && r.end <= text.len()
        && text.is_char_boundary(r.start)
        && text.is_char_boundary(r.end)
}

/// Whether `offset` is a legal edge for an atomic span: not strictly inside a
/// grapheme atom of a text box, an inline object or a line terminator. Offsets
/// inside a glue run are accepted (a glue node carries no atoms).
fn is_atom_boundary(ast: &ParagraphAst, offset: usize) -> bool {
    let inside = |r: &TextRange| r.start < offset && offset < r.end;
    ast.items.iter().all(|n| match n {
        WordNode::Box(BoxNode::Text { atoms, .. }) => !atoms.iter().any(|a| inside(&a.byte_range)),
        WordNode::Box(BoxNode::InlineObject { byte_range, .. }) => !inside(byte_range),
        WordNode::Discretionary(d) => !inside(&d.byte_range),
        _ => true,
    })
}

/// Validates the two span streams.
///
/// * Both: ranges fit the text, ids are unique within the stream.
/// * Decoration spans: finite, non-negative paddings; **proper nesting**
///   (two spans are disjoint or one contains the other, never a partial
///   overlap) no deeper than [`MAX_DECORATION_NESTING_DEPTH`].
/// * Atomic spans: non-empty, **strictly disjoint**, both ends on grapheme
///   atom boundaries.
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
            || !is_atom_boundary(ast, s.byte_range.start)
            || !is_atom_boundary(ast, s.byte_range.end)
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

/// Whether item `index` is a text box with at least one cluster.
fn has_clusters(items: &[WordNode], index: Option<usize>) -> bool {
    matches!(
        index.and_then(|i| items.get(i)),
        Some(WordNode::Box(BoxNode::Text { clusters, .. })) if !clusters.is_empty()
    )
}

/// Checks the inputs of [`layout_paragraph`] and assembles the paragraph-level
/// [`LineContext`]: the active strut is resolved here from the AST's **raw**
/// strut metrics, `ParagraphAst::strut_font_size` and
/// `options.line_height` (`LineHeight::resolve_strut`).
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
    for (item, node) in ast.items.iter().enumerate() {
        match node.byte_span() {
            Some(span) if fits(text, span) => {}
            _ => return Err(E::ItemOutsideText { item }),
        }
        match node {
            WordNode::Box(BoxNode::Text {
                run_index, atoms, ..
            }) => {
                if *run_index >= ast.runs.len() {
                    return Err(E::RunIndexOutOfRange { item });
                }
                // Every atom must fit, even when the node's overall span does.
                if !atoms.iter().all(|a| fits(text, a.byte_range)) {
                    return Err(E::ItemOutsideText { item });
                }
                if !node.is_self_contained() {
                    return Err(E::ItemNotSelfContained { item });
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
                if !ast.decoration_spans.iter().any(|s| s.span_id == *span_id) {
                    return Err(E::UnknownSpan { item });
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
                if !(sizes_ok && g.tier != GlueTier::Tier2 && pool_ok) {
                    return Err(E::InvalidGlue { item });
                }
            }
            WordNode::Discretionary(d) => {
                let widths_ok = [d.pre_break_width, d.post_break_width, d.no_break_width]
                    .iter()
                    .all(|v| v.is_finite());
                let hard = matches!(d.action, BreakAction::HardBreak { .. });
                // Ownership: a soft opportunity owns nothing; a hard break owns
                // its terminator, except that the paragraph end may own nothing.
                let range_ok = match d.action {
                    BreakAction::HardBreak { is_paragraph_end } => {
                        (d.byte_range.is_empty() && is_paragraph_end)
                            || is_line_terminator(&text[d.byte_range.start..d.byte_range.end])
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
                for id in &d.cloned_spans {
                    // A clone only makes sense for a Clone-mode span the break
                    // falls strictly inside.
                    let off = d.byte_offset();
                    if !ast.decoration_spans.iter().any(|s| {
                        s.span_id == *id
                            && s.break_mode == BoxDecorationBreak::Clone
                            && s.byte_range.start < off
                            && off < s.byte_range.end
                    }) {
                        return Err(E::UnknownSpan { item });
                    }
                }
                match &d.action {
                    BreakAction::Hyphenate(h) if !hyphen_valid(&ast.items, item, d, h) => {
                        return Err(E::InvalidHyphenOpportunity { item });
                    }
                    BreakAction::CjkPunctAdjust {
                        trailing_advance_override,
                        leading_advance_override,
                    } => {
                        let advance_ok =
                            |o: &Option<f32>| o.is_none_or(|v| v.is_finite() && v >= 0.0);
                        let trailing_ok = trailing_advance_override.is_none()
                            || has_clusters(&ast.items, item.checked_sub(1));
                        let leading_ok = leading_advance_override.is_none()
                            || has_clusters(&ast.items, item.checked_add(1));
                        if !(advance_ok(trailing_advance_override)
                            && advance_ok(leading_advance_override)
                            && trailing_ok
                            && leading_ok)
                        {
                            return Err(E::InvalidDiscretionary { item });
                        }
                    }
                    _ => {}
                }
            }
            WordNode::Tab { .. } => {}
        }
    }
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
            items: vec![hard_break()],
            decoration_spans: vec![],
            atomic_spans: vec![],
        }
    }

    /// A mandatory break that owns the one-byte paragraph "\n".
    fn hard_break() -> WordNode {
        WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(0, 1), false))
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

    fn with_items(items: Vec<WordNode>) -> ParagraphAst {
        ParagraphAst {
            runs: vec![run()],
            items,
            ..ast()
        }
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
        assert!(check(DiscretionaryNode::hard_break(TextRange::empty_at(4), true)).is_ok());
        // A negative width is a legal delta (trim); a non-finite one is not.
        let mut d = DiscretionaryNode::direct(2);
        d.pre_break_width = -2.5;
        assert!(check(d.clone()).is_ok());
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
        let ok = with_span(vec![boundary(2.0, TextRange::empty_at(1))], Slice);
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
            with_span(vec![WordNode::Discretionary(d)], mode)
        };
        assert!(check_layout_inputs(&clone_pad(Clone), "abcd", &constraints(), &o).is_ok());
        // Padding is cloned only by a span that was declared to clone it.
        assert_eq!(
            check_layout_inputs(&clone_pad(Slice), "abcd", &constraints(), &o),
            Err(LayoutInputError::UnknownSpan { item: 0 })
        );
    }

    #[test]
    fn cloned_spans_are_orthogonal_to_the_action_and_must_enclose_the_break() {
        use crate::frontend::BoxDecorationBreak::Clone;
        let o = LayoutOptions::default();
        // A hyphenation break at 2 inside a cloning chip over 1..3: both apply.
        let mut a = hyphenated(opportunity(2, 0, 0));
        let mut over = chip(5, Clone);
        over.byte_range = TextRange::new(1, 4); // "b" and "é"
        a.decoration_spans = vec![over];
        if let WordNode::Discretionary(d) = &mut a.items[1] {
            d.cloned_spans.push(5);
            d.penalty += DiscretionaryNode::CLONED_SPAN_PENALTY;
        }
        assert!(check_layout_inputs(&a, "abé", &constraints(), &o).is_ok());
        // A break on the span's edge is not inside it; a repeated id is wrong.
        for (cloned, at) in [(vec![5], 1), (vec![5], 3), (vec![5, 5], 2)] {
            let mut d = DiscretionaryNode::direct(at);
            d.cloned_spans.extend(cloned);
            let mut b = with_items(vec![WordNode::Discretionary(d)]);
            b.decoration_spans = vec![chip(5, Clone)];
            assert!(check_layout_inputs(&b, "abcd", &constraints(), &o).is_err());
        }
    }

    #[test]
    fn soft_breaks_own_nothing_and_hard_breaks_own_their_terminator() {
        let o = LayoutOptions::default();
        let check = |text: &str, d: DiscretionaryNode| {
            let mut a = with_items(vec![WordNode::Discretionary(d)]);
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
        assert!(
            check(
                "abcd",
                DiscretionaryNode::hard_break(TextRange::empty_at(4), true)
            )
            .is_ok()
        );
        // A soft opportunity owns nothing.
        let mut soft = DiscretionaryNode::direct(1);
        soft.byte_range = TextRange::new(1, 2);
        assert_eq!(check("abcd", soft), bad);
        assert_eq!(DiscretionaryNode::direct(3).byte_offset(), 3);
    }

    #[test]
    fn cjk_punct_adjust_overrides_are_geometric_and_need_a_neighbour() {
        let o = LayoutOptions::default();
        let adjust = |t, l| {
            let mut d = DiscretionaryNode::direct(2);
            d.action = BreakAction::CjkPunctAdjust {
                trailing_advance_override: t,
                leading_advance_override: l,
            };
            with_items(vec![
                text_box(vec![atom(0, 2)]),
                WordNode::Discretionary(d),
                text_box(vec![atom(2, 4)]),
            ])
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        assert!(check(adjust(Some(5.0), Some(0.0))).is_ok());
        assert!(check(adjust(None, None)).is_ok());
        let bad = Err(LayoutInputError::InvalidDiscretionary { item: 1 });
        for (t, l) in [
            (Some(-1.0), None),
            (None, Some(f32::NAN)),
            (Some(f32::INFINITY), None),
        ] {
            assert_eq!(check(adjust(t, l)), bad);
        }
        // An override needs the text box it overrides.
        let mut a = adjust(Some(1.0), None);
        a.items.remove(0);
        assert_eq!(
            check(a),
            Err(LayoutInputError::InvalidDiscretionary { item: 0 })
        );
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
        let build = |edge, level| {
            let mut a = with_items(vec![
                text_box(vec![atom(0, 1)]),
                WordNode::Box(BoxNode::SpanBoundary {
                    byte_range: TextRange::empty_at(1),
                    span_id: 5,
                    edge,
                    width: 2.0,
                    bidi_level: level,
                }),
                text_box(vec![inner.clone(), atom(2, 3)]),
            ]);
            a.decoration_spans = vec![chip(5, Slice)]; // bytes 1..3
            a
        };
        let check = |a: ParagraphAst| check_layout_inputs(&a, "abcd", &constraints(), &o);
        // First atom inside the span is level 1, the last is level 0.
        assert!(check(build(SpanEdge::Leading, 1)).is_ok());
        assert!(check(build(SpanEdge::Trailing, 0)).is_ok());
        let bad = Err(LayoutInputError::InvalidSpanBoundary { item: 1 });
        assert_eq!(check(build(SpanEdge::Leading, 0)), bad);
        assert_eq!(check(build(SpanEdge::Trailing, 1)), bad);
    }

    #[test]
    fn span_streams_are_validated() {
        use crate::frontend::BoxDecorationBreak::Slice;
        let o = LayoutOptions::default();
        let check = |deco: Vec<InlineDecorationSpan>, atomic: Vec<AtomicEditSpan>| {
            let mut a = with_items(vec![]);
            a.decoration_spans = deco;
            a.atomic_spans = atomic;
            check_layout_inputs(&a, "abcd", &constraints(), &o)
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
        let check = |a: ParagraphAst| check_layout_inputs(&a, "aéb", &constraints(), &o);
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
