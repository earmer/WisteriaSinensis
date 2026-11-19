//! An emergency break (ST1-5) whose line the geometry provider then narrows
//! (GEO-3, GEO-4): the refit must equal a fresh fit against the final
//! envelope, with nothing left over from the rejected attempt (ME-2).
//!
//! A line taller than the strut re-queries an expanded envelope (ME-2), and
//! GEO-4 lets the provider answer that expansion with less space. The tall
//! characters below are a larger style span, so every line holds one.

use crate::common::*;
use mutil_engine::*;
use std::sync::Arc;

const WIDE: f32 = 150.0;
const NARROW: f32 = 61.0;
const SMALL: f32 = 10.0;
const TALL: f32 = 20.0;
/// Every `TALL_EVERY`th byte of a word, counted back from its last, is set in
/// the tall style, so that every line (the last one included) holds one and
/// ends with the same envelope under both providers.
const TALL_EVERY: usize = 3;

/// Answers `wide` to an envelope no taller than `limit` and `narrow` to any
/// taller one; both are single slots starting at zero.
#[derive(Debug)]
struct Shrinking {
    limit: f32,
    wide: f32,
    narrow: f32,
}

impl LineGeometryProvider for Shrinking {
    fn query_line_geometry(&self, _: usize, block_start: f32, block_end: f32) -> LineGeometry {
        let end = if block_end - block_start > self.limit {
            self.narrow
        } else {
            self.wide
        };
        slot(end)
    }
}

/// The reverse of [`Shrinking`]: answers `narrow` to an envelope no taller
/// than `limit` and `wide` to any taller one.
#[derive(Debug)]
struct Growing {
    limit: f32,
    narrow: f32,
    wide: f32,
}

impl LineGeometryProvider for Growing {
    fn query_line_geometry(&self, _: usize, block_start: f32, block_end: f32) -> LineGeometry {
        let end = if block_end - block_start > self.limit {
            self.wide
        } else {
            self.narrow
        };
        slot(end)
    }
}

/// Answers `end` whatever the envelope: the final envelope given directly.
#[derive(Debug)]
struct Direct(f32);

impl LineGeometryProvider for Direct {
    fn query_line_geometry(&self, _: usize, _: f32, _: f32) -> LineGeometry {
        slot(self.0)
    }
}

fn slot(end: f32) -> LineGeometry {
    LineGeometry {
        slots: [InlineSlotConstraint::try_new(0.0, end).unwrap()]
            .into_iter()
            .collect(),
        next_feature_block_offset: None,
    }
}

fn provided(provider: impl LineGeometryProvider + 'static) -> LayoutConstraints {
    LayoutConstraints {
        geometry_provider: Some(Arc::new(provider)),
        ..column(WIDE)
    }
}

/// The envelope height of a line holding only strut-sized text.
fn strut_height() -> f32 {
    lay_out("a", &column(WIDE), &LayoutOptions::default()).lines[0].line_height
}

/// `unit` is the byte length of the pieces the tall style is applied to, so
/// that a multi-character operator is never split across two styles.
fn lay_out_word(word: &str, unit: usize, constraints: &LayoutConstraints) -> PositionedParagraph {
    let base = style(SMALL);
    let spans: Vec<StyleSpan> = (0..word.len() / unit)
        .rev()
        .step_by(TALL_EVERY)
        .rev()
        .map(|n| StyleSpan {
            byte_range: TextRange::new(n * unit, (n + 1) * unit),
            style: style(TALL),
        })
        .collect();
    let input = ParagraphInput {
        style_spans: &spans,
        ..input(word, &base)
    };
    let ast = compile_paragraph(&input, &FrontendOptions::default(), &*SHAPER)
        .expect("the frontend accepts the word");
    let paragraph = layout_paragraph(&ast, word, &*SHAPER, constraints, &LayoutOptions::default())
        .unwrap_or_else(|e| panic!("{word:?}: {e}"));
    if let Err((line, error)) = ValidatedLayout::try_from_paragraph(&paragraph, word) {
        panic!("{word:?}: line {line}: {error}\n{paragraph:#?}");
    }
    paragraph
}

/// Lays `word` out through a provider that narrows on refit and through one
/// that gives the narrow answer at once, and requires identical output.
#[track_caller]
fn assert_refit_equals_fresh_fit(word: &str, unit: usize) -> PositionedParagraph {
    let limit = strut_height() + 0.5;
    let refit = lay_out_word(
        word,
        unit,
        &provided(Shrinking {
            limit,
            wide: WIDE,
            narrow: NARROW,
        }),
    );
    let fresh = lay_out_word(word, unit, &provided(Direct(NARROW)));
    assert!(refit.lines.len() > 1, "{word:?} must need emergency breaks");
    assert_eq!(
        line_texts(&refit, word),
        line_texts(&fresh, word),
        "{word:?}"
    );
    assert_eq!(refit, fresh, "{word:?}");
    refit
}

#[test]
fn a_narrowed_first_line_equals_a_fresh_fit() {
    // One emergency seam on the first line, then a short tail.
    let word = "abcdefghijklmnopqrstuvwxy";
    let p = assert_refit_equals_fresh_fit(word, 1);
    for line in &p.lines {
        assert!(
            line.visual_width() <= NARROW + 1e-3,
            "{}",
            line.visual_width()
        );
    }
}

#[test]
fn consecutive_narrowed_lines_equal_a_fresh_fit() {
    // Many seams in a row: each line is fitted wide, rejected, refitted.
    let word = "abcdefghij".repeat(12);
    let p = assert_refit_equals_fresh_fit(&word, 1);
    assert!(p.lines.len() >= 4, "{} lines", p.lines.len());
}

#[test]
fn ligature_seams_equal_a_fresh_fit() {
    // Operators the font may ligate make some seams unsafe to break (ST1-5),
    // which re-shapes both sides; the refit must not keep the wide fit's.
    for unit in [
        "->", "==", "!=", "<=", "www", "|>", "=>", "::", "..", "<!--",
    ] {
        let word = unit.repeat(40);
        assert_refit_equals_fresh_fit(&word, unit.len());
    }
}

#[test]
fn an_unbreakable_word_after_a_space_equals_a_fresh_fit() {
    let word = format!("ab {}", "->=".repeat(30));
    assert_refit_equals_fresh_fit(&word, 1);
}

#[test]
fn a_narrowed_layout_passes_the_output_gate() {
    let word = "->".repeat(60);
    let p = assert_refit_equals_fresh_fit(&word, 2);
    ValidatedLayout::try_from_paragraph(&p, &word).expect("validates");
    // Every byte is laid out exactly once, in order.
    let mut next = 0;
    for line in &p.lines {
        assert_eq!(line.byte_range.start(), next);
        next = line.byte_range.end();
    }
    assert_eq!(next, word.len());
}

// GEO-4 lets an expanded envelope only remove space, so "the envelope grows"
// means a taller block envelope whose answer is the same or smaller, never a
// wider one. A wider answer is a contract violation, not a refit.

/// Lays `word` out through a provider that gives `first` to the strut-height
/// envelope and `expanded` to a taller one, and through one that gives
/// `expanded` at once, and requires identical output.
#[track_caller]
fn assert_expanded_refit_equals_fresh_fit(
    word: &str,
    unit: usize,
    first: f32,
    expanded: f32,
) -> PositionedParagraph {
    let refit = lay_out_word(
        word,
        unit,
        &provided(Shrinking {
            limit: strut_height() + 0.5,
            wide: first,
            narrow: expanded,
        }),
    );
    let fresh = lay_out_word(word, unit, &provided(Direct(expanded)));
    assert!(refit.lines.len() > 1, "{word:?} must need emergency breaks");
    assert_eq!(
        line_texts(&refit, word),
        line_texts(&fresh, word),
        "{word:?}"
    );
    assert_eq!(refit, fresh, "{word:?}");
    refit
}

#[test]
fn an_expanded_envelope_that_keeps_the_answer_equals_a_fresh_fit() {
    // The taller envelope changes nothing, so the refit's seams are exactly
    // the first attempt's; no line may differ from a fresh fit.
    let word = "abcdefghij".repeat(30);
    let p = assert_expanded_refit_equals_fresh_fit(&word, 1, WIDE, WIDE);
    assert!(p.lines.iter().any(|l| l.visual_width() > NARROW));
}

#[test]
fn a_slightly_narrowed_envelope_equals_a_fresh_fit() {
    // The first attempt fits 150, the refit only 105: lines that were broken
    // at the wide fit's seams are broken again at the narrower one's.
    let word = "abcdefghij".repeat(30);
    assert_expanded_refit_equals_fresh_fit(&word, 1, WIDE, 105.0);
}

#[test]
fn slightly_narrowed_ligature_seams_equal_a_fresh_fit() {
    for unit in ["->", "==", "!=", "<=", "www", "|>", "=>", "::", "<!--"] {
        let word = unit.repeat(80);
        assert_expanded_refit_equals_fresh_fit(&word, unit.len(), WIDE, 105.0);
    }
}

#[test]
fn a_provider_that_widens_with_the_envelope_is_rejected() {
    // Narrow for the strut-height envelope, wide for the taller one: the
    // refit must stop with `NonMonotonicGeometry`, not lay out the wide fit.
    let word = "abcdefghij".repeat(30);
    let base = style(SMALL);
    let spans: Vec<StyleSpan> = (0..word.len())
        .rev()
        .step_by(TALL_EVERY)
        .rev()
        .map(|n| StyleSpan {
            byte_range: TextRange::new(n, n + 1),
            style: style(TALL),
        })
        .collect();
    let input = ParagraphInput {
        style_spans: &spans,
        ..input(&word, &base)
    };
    let ast = compile_paragraph(&input, &FrontendOptions::default(), &*SHAPER)
        .expect("the frontend accepts the word");
    let constraints = provided(Growing {
        limit: strut_height() + 0.5,
        narrow: NARROW,
        wide: WIDE,
    });
    let result = layout_paragraph(
        &ast,
        &word,
        &*SHAPER,
        &constraints,
        &LayoutOptions::default(),
    );
    assert_eq!(result, Err(LayoutInputError::NonMonotonicGeometry));
}
