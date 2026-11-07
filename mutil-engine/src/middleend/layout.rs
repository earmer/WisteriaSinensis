//! The mid-end entry point (spec §5).
//!
//! **Phase 0 skeleton:** it assembles the validation context and checks its
//! inputs, but lays nothing out.

use super::{LayoutConstraints, LayoutOptions, LineContext, PositionedParagraph};
use crate::frontend::{HyphenOpportunity, ParagraphAst, WordNode};
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
    /// An item violates `WordNode::is_self_contained`.
    ItemNotSelfContained {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// An inline box has a non-finite or negative width, ascent or descent.
    InvalidInlineBox {
        /// Index into `ParagraphAst::items`.
        item: usize,
    },
    /// A hyphenation opportunity of a word lies outside the word, or it or one
    /// of its cuts does not end on an atom boundary.
    InvalidHyphenOpportunity {
        /// Index into `ParagraphAst::items`.
        item: usize,
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
            Self::InvalidHyphenOpportunity { item } => {
                write!(f, "item {item} has a hyphenation opportunity off its atoms")
            }
        }
    }
}

impl std::error::Error for LayoutInputError {}

/// Whether every hyphenation opportunity of a word lies strictly inside it and,
/// with both cuts, ends on an atom boundary.
fn hyphen_breaks_valid(atoms: &[crate::types::GraphemeAtom], breaks: &[HyphenOpportunity]) -> bool {
    let (Some(first), Some(last)) = (atoms.first(), atoms.last()) else {
        return breaks.is_empty();
    };
    let (start, end) = (first.byte_range.start, last.byte_range.end);
    let boundary = |pos: usize| pos == end || atoms.iter().any(|a| a.byte_range.start == pos);
    breaks.iter().all(|b| {
        let off = b.byte_offset;
        let prefix = off.checked_sub(b.prefix_cut_bytes as usize);
        let suffix = off.checked_add(b.suffix_cut_bytes as usize);
        start < off
            && off < end
            && boundary(off)
            && prefix.is_some_and(|p| p >= start && boundary(p))
            && suffix.is_some_and(|q| q <= end && boundary(q))
    })
}

fn fits(text: &str, r: crate::types::TextRange) -> bool {
    r.start <= r.end
        && r.end <= text.len()
        && text.is_char_boundary(r.start)
        && text.is_char_boundary(r.end)
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
    for (item, node) in ast.items.iter().enumerate() {
        if let WordNode::Word { run_index, .. } | WordNode::PunctuationRun { run_index, .. } = node
        {
            if *run_index >= ast.runs.len() {
                return Err(E::RunIndexOutOfRange { item });
            }
        }
        if let WordNode::Word { atoms, .. } | WordNode::PunctuationRun { atoms, .. } = node {
            // Every atom must fit, even when the node's overall span does.
            if !atoms.iter().all(|a| fits(text, a.byte_range)) {
                return Err(E::ItemOutsideText { item });
            }
        }
        match node.byte_span() {
            Some(span) if !fits(text, span) => return Err(E::ItemOutsideText { item }),
            // A node with atoms but no span has out-of-order atoms.
            None if matches!(
                node,
                WordNode::Word { atoms, .. } | WordNode::PunctuationRun { atoms, .. }
                    if !atoms.is_empty()
            ) =>
            {
                return Err(E::ItemOutsideText { item });
            }
            _ => {}
        }
        if let WordNode::InlineBox {
            width,
            ascent,
            descent,
            ..
        } = node
        {
            if ![width, ascent, descent]
                .iter()
                .all(|v| v.is_finite() && **v >= 0.0)
            {
                return Err(E::InvalidInlineBox { item });
            }
        }
        if !node.is_self_contained() {
            return Err(E::ItemNotSelfContained { item });
        }
        if let WordNode::Word {
            atoms,
            hyphen_breaks,
            ..
        } = node
        {
            if !hyphen_breaks_valid(atoms, hyphen_breaks) {
                return Err(E::InvalidHyphenOpportunity { item });
            }
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
            items: vec![WordNode::HardBreak {
                byte_range: TextRange::new(0, 1),
            }],
        }
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
        a.items = vec![WordNode::HardBreak {
            byte_range: TextRange::new(0, 1),
        }];
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

    use crate::frontend::HyphenOpportunity;
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

    fn word(atoms: Vec<GraphemeAtom>, breaks: Vec<HyphenOpportunity>) -> WordNode {
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
        WordNode::Word {
            run_index: 0,
            atoms,
            clusters,
            hyphen_breaks: breaks,
            total_width: 0.0,
        }
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

    fn with_items(items: Vec<WordNode>) -> ParagraphAst {
        ParagraphAst {
            runs: vec![run()],
            items,
            ..ast()
        }
    }

    #[test]
    fn inline_boxes_must_have_finite_non_negative_sizes() {
        let o = LayoutOptions::default();
        for (w, a, d) in [
            (f32::NAN, 1.0, 1.0),
            (1.0, -1.0, 1.0),
            (1.0, 1.0, f32::INFINITY),
        ] {
            let a = with_items(vec![WordNode::InlineBox {
                byte_range: TextRange::new(0, 1),
                box_id: 1,
                width: w,
                ascent: a,
                descent: d,
            }]);
            assert_eq!(
                check_layout_inputs(&a, "abcd", &constraints(), &o),
                Err(LayoutInputError::InvalidInlineBox { item: 0 })
            );
        }
    }

    #[test]
    fn hyphen_opportunities_must_sit_on_atom_boundaries_inside_the_word() {
        let o = LayoutOptions::default();
        // Word "ab" + "é" (2 bytes) = bytes 0..4 with atoms 0..1, 1..2, 2..4.
        let atoms = || vec![atom(0, 1), atom(1, 2), atom(2, 4)];
        let check = |b: HyphenOpportunity| {
            check_layout_inputs(
                &with_items(vec![word(atoms(), vec![b])]),
                "abé",
                &constraints(),
                &o,
            )
        };
        assert!(check(opportunity(1, 0, 0)).is_ok());
        assert!(check(opportunity(2, 1, 2)).is_ok()); // cuts end at 1 and at 4
        let bad = Err(LayoutInputError::InvalidHyphenOpportunity { item: 0 });
        assert_eq!(check(opportunity(0, 0, 0)), bad); // not strictly inside
        assert_eq!(check(opportunity(4, 0, 0)), bad);
        assert_eq!(check(opportunity(3, 0, 0)), bad); // inside the 2-byte atom
        assert_eq!(check(opportunity(2, 3, 0)), bad); // cut before the word
        assert_eq!(check(opportunity(1, 0, 2)), bad); // ends mid-atom (at 3)
        assert_eq!(check(opportunity(2, 0, 3)), bad); // cut past the word
    }

    #[test]
    fn every_atom_must_fit_and_be_in_order() {
        let o = LayoutOptions::default();
        // Overall span fits "abcd" but the middle atom does not end inside it.
        let a = with_items(vec![word(vec![atom(0, 1), atom(1, 9), atom(2, 4)], vec![])]);
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
        // First atom starts after the last one ends: no panic, an error.
        let a = with_items(vec![word(vec![atom(3, 4), atom(0, 1)], vec![])]);
        assert_eq!(
            check_layout_inputs(&a, "abcd", &constraints(), &o),
            Err(LayoutInputError::ItemOutsideText { item: 0 })
        );
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
