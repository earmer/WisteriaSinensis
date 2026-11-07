//! The mid-end entry point (spec §5).
//!
//! **Phase 0 skeleton:** it assembles the validation context and checks its
//! inputs, but lays nothing out.

use super::{LayoutConstraints, LayoutOptions, LineContext, PositionedParagraph};
use crate::frontend::{
    BoxDecorationBreak, BoxNode, BreakAction, DiscretionaryNode, HyphenOpportunity, ParagraphAst,
    WordNode,
};
use crate::shaping::TextShaper;
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
    /// A glue node has a non-finite or negative width, stretch or shrink.
    InvalidGlue {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A discretionary node has a non-finite width, a forbidden penalty
    /// (`>= FORBIDDEN_BREAK_PENALTY`; a forbidden break is an omitted node), or
    /// a `HardBreak` action that disagrees with a mandatory penalty.
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
    /// A span boundary is not an empty range or has a non-finite or negative
    /// width.
    InvalidSpanBoundary {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An item names a span (`SpanBoundary`, or `CloneSpanPadding` which also
    /// requires `BoxDecorationBreak::Clone`) that `decoration_spans` lacks.
    UnknownSpan {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A decoration span has an invalid padding or range, does not fit the text
    /// or repeats a `span_id`.
    InvalidDecorationSpan {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
    },
    /// An atomic span is empty, does not fit the text or repeats a `span_id`.
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
    if off != d.byte_offset || prev.end != off || next.start != off {
        return false;
    }
    if prev.is_empty() || next.is_empty() {
        return false;
    }
    let boundary = |atoms: &[crate::types::GraphemeAtom], pos: usize, end: usize| {
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

/// Validates the two span streams: ranges fit the text, ids are unique within
/// a stream, decoration paddings are finite and non-negative, atomic spans are
/// non-empty.
fn check_spans(ast: &ParagraphAst, text: &str) -> Result<(), LayoutInputError> {
    use LayoutInputError as E;
    for (span, s) in ast.decoration_spans.iter().enumerate() {
        let duplicate = ast.decoration_spans[..span]
            .iter()
            .any(|o| o.span_id == s.span_id);
        if duplicate || !s.is_valid() || !fits(text, s.byte_range) {
            return Err(E::InvalidDecorationSpan { span });
        }
    }
    for (span, s) in ast.atomic_spans.iter().enumerate() {
        let duplicate = ast.atomic_spans[..span]
            .iter()
            .any(|o| o.span_id == s.span_id);
        if duplicate || s.byte_range.is_empty() || !fits(text, s.byte_range) {
            return Err(E::InvalidAtomicSpan { span });
        }
    }
    Ok(())
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
                width,
                ..
            }) => {
                if !(byte_range.is_empty() && width.is_finite() && *width >= 0.0) {
                    return Err(E::InvalidSpanBoundary { item });
                }
                if !ast.decoration_spans.iter().any(|s| s.span_id == *span_id) {
                    return Err(E::UnknownSpan { item });
                }
            }
            WordNode::Glue(g) => {
                if ![g.natural_width, g.max_stretch, g.max_shrink]
                    .iter()
                    .all(|v| v.is_finite() && *v >= 0.0)
                {
                    return Err(E::InvalidGlue { item });
                }
            }
            WordNode::Discretionary(d) => {
                let widths_ok = [d.pre_break_width, d.post_break_width, d.no_break_width]
                    .iter()
                    .all(|v| v.is_finite());
                let hard = matches!(d.action, BreakAction::HardBreak { .. });
                if !(widths_ok
                    && d.penalty < DiscretionaryNode::FORBIDDEN_BREAK_PENALTY
                    && hard == d.is_mandatory())
                {
                    return Err(E::InvalidDiscretionary { item });
                }
                match &d.action {
                    BreakAction::Hyphenate(h) if !hyphen_valid(&ast.items, item, d, h) => {
                        return Err(E::InvalidHyphenOpportunity { item });
                    }
                    BreakAction::CloneSpanPadding { span_id }
                        if !ast.decoration_spans.iter().any(|s| {
                            s.span_id == *span_id && s.break_mode == BoxDecorationBreak::Clone
                        }) =>
                    {
                        return Err(E::UnknownSpan { item });
                    }
                    _ => {}
                }
            }
            WordNode::Tab { .. } => {}
        }
    }
    let context = LineContext::from_options(m, ast.strut_font_size, ast.base_direction, options);
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
            items: vec![hard_break(1)],
            decoration_spans: vec![],
            atomic_spans: vec![],
        }
    }

    /// A mandatory break after the one-byte paragraph "\n".
    fn hard_break(offset: usize) -> WordNode {
        WordNode::Discretionary(DiscretionaryNode::hard_break(offset, true))
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
        a.items = vec![hard_break(1)];
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
        assert!(check(DiscretionaryNode::hard_break(4, true)).is_ok());
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
        let mut d = DiscretionaryNode::hard_break(4, false);
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
            byte_offset: at,
            penalty: DiscretionaryNode::HYPHEN_PENALTY,
            flagged: true,
            pre_break_width: 5.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Hyphenate(opportunity),
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
            d.byte_offset = 2;
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
            d.action = BreakAction::CloneSpanPadding { span_id: 5 };
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
