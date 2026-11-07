//! The mid-end entry point (spec §5).
//!
//! **Phase 0 skeleton:** it assembles the validation context and checks its
//! inputs, but lays nothing out.

use super::{LayoutConstraints, LayoutOptions, LineContext, PositionedParagraph};
use crate::frontend::{ParagraphAst, WordNode};
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
        }
    }
}

impl std::error::Error for LayoutInputError {}

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
        super::LineHeight::Fixed(v) => v.is_finite(),
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
            && *run_index >= ast.runs.len()
        {
            return Err(E::RunIndexOutOfRange { item });
        }
        if let Some(span) = node.byte_span()
            && !fits(text, span)
        {
            return Err(E::ItemOutsideText { item });
        }
        if !node.is_self_contained() {
            return Err(E::ItemNotSelfContained { item });
        }
    }
    Ok(LineContext::from_options(
        m,
        ast.strut_font_size,
        ast.base_direction,
        options,
    ))
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
}
