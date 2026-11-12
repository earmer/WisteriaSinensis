//! The mid-end entry point (spec §5).
//!
//! **Validation-only skeleton:** it validates its inputs, resolves the policy
//! and probes the first line's geometry, but lays nothing out.

use super::input_validation::validate_layout_inputs;
use super::{LayoutConstraints, LayoutInputError, LayoutOptions, LineHeight, PositionedParagraph};
use crate::frontend::{AnnotationSide, BoxNode, ParagraphAst, WordNode};
use crate::shaping::TextShaper;
use crate::types::FontInstanceRef;

/// How many obstructed block positions the first-line probe tries before it
/// gives up on a geometry provider.
const MAX_CLEARANCE_PROBES: usize = 1024;
/// The least block progress a provider must offer past an obstructed position.
const MIN_CLEARANCE_STEP: f32 = 1e-4;

/// Lays out one paragraph (spec §5).
///
/// * `ast`: frontend output; `text`: the paragraph text it was built from
///   (shapers receive it per call).
/// * `shaper`: used for re-shaping at breaks and for ligature carets.
/// * `constraints`: container geometry; `options`: layout policy.
///
/// **Validation-only skeleton.** It validates the inputs
/// ([`check_layout_inputs`](super::check_layout_inputs)) and, with a geometry
/// provider, the provider's answers for the first line, and performs no
/// layout: the result records the policy and the AST's base direction and has
/// no lines, so it does not pass [`PositionedParagraph::validate`] yet.
///
/// # Errors
/// The first invalid input, or a geometry provider that breaks its contract.
pub fn layout_paragraph(
    ast: &ParagraphAst,
    text: &str,
    shaper: &dyn TextShaper,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> Result<PositionedParagraph, LayoutInputError> {
    let policy = validate_layout_inputs(ast, text, constraints, options)?;
    if constraints.geometry_provider.is_some() {
        probe_first_line(
            ast,
            shaper,
            constraints,
            options,
            policy.strut.line_height(),
        )?;
    }
    Ok(PositionedParagraph {
        policy,
        base_direction: ast.base_direction,
        truncated_range: None,
        total_block_size: 0.0,
        lines: Vec::new(),
    })
}

/// Checks that a geometry provider offers the first line a place to start.
///
/// Only the geometry that can begin at the block start is probed: the strut
/// and the input up to the first break opportunity. A tall object later in the
/// paragraph is not evidence about the first line, so it must not turn a legal
/// provider into a clearance failure.
fn probe_first_line(
    ast: &ParagraphAst,
    shaper: &dyn TextShaper,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
    strut_height: f32,
) -> Result<(), LayoutInputError> {
    let tallest = tallest_first_line(ast, shaper, options, strut_height)?;
    let mut block_start = 0.0;
    for _ in 0..MAX_CLEARANCE_PROBES {
        let probe =
            constraints.query_checked_geometry(0, block_start, block_start + strut_height, None)?;
        if tallest > strut_height {
            // Contract check only: an expanded envelope must stay inside the
            // original one. Its contents do not decide whether the initial
            // position is legal.
            constraints.query_checked_geometry(
                0,
                block_start,
                block_start + tallest,
                Some(&probe),
            )?;
        }
        if !probe.slots.is_empty() {
            return Ok(());
        }
        let next = probe
            .next_feature_block_offset
            .ok_or(LayoutInputError::InvalidLineGeometry)?;
        if next - block_start < MIN_CLEARANCE_STEP {
            return Err(LayoutInputError::InvalidLineGeometry);
        }
        block_start = next;
    }
    Err(LayoutInputError::InvalidLineGeometry)
}

/// The tallest line box the first line could need: the strut, the content up
/// to the first break opportunity, and the annotations over that content.
fn tallest_first_line(
    ast: &ParagraphAst,
    shaper: &dyn TextShaper,
    options: &LayoutOptions,
    strut_height: f32,
) -> Result<f32, LayoutInputError> {
    let first_break = ast
        .items
        .iter()
        .position(WordNode::is_break_opportunity)
        .unwrap_or(ast.items.len());
    let content_end = ast
        .item_range(first_break)
        .map_or(usize::MAX, |range| range.start());
    let mut content_height = strut_height;
    for (item, node) in ast.items[..first_break].iter().enumerate() {
        if let WordNode::Box(BoxNode::InlineObject {
            ascent, descent, ..
        }) = node
        {
            content_height = content_height.max(ascent + descent);
        }
        let Some(view) = ast.text_box(item) else {
            continue;
        };
        let run = &ast.runs[view.run_index];
        for cluster in view.clusters {
            let metrics = shaper.font_metrics(FontInstanceRef {
                font: cluster.font,
                font_size: run.font_size,
                variations: run.features.variations(),
            });
            let height = options
                .line_height
                .resolve_strut(&metrics, run.font_size, ast.writing_mode)
                .line_height();
            if !height.is_finite() {
                return Err(LayoutInputError::InvalidFontMetrics);
            }
            content_height = content_height.max(height);
        }
    }
    // A fixed line box does not grow for annotations; they overflow it.
    let (mut over, mut under) = (0.0_f32, 0.0_f32);
    if matches!(options.line_height, LineHeight::Normal) {
        for annotation in ast
            .annotations
            .iter()
            .filter(|annotation| annotation.base_byte_range.start() < content_end)
        {
            match annotation.side {
                AnnotationSide::Over => over = over.max(annotation.block_extent),
                AnnotationSide::Under => under = under.max(annotation.block_extent),
            }
        }
    }
    let tallest = content_height + over + under;
    if tallest.is_finite() {
        Ok(tallest)
    } else {
        Err(LayoutInputError::InvalidFontMetrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::DiscretionaryNode;
    use crate::middleend::{LayoutPolicySnapshot, TabStops};
    use crate::shaping::NoopShaper;
    use crate::types::{BidiDirection, FontMetrics, WritingMode};

    fn constraints() -> LayoutConstraints {
        LayoutConstraints {
            wrap_width: 100.0,
            tab_stops: TabStops::DEFAULT,
            ..Default::default()
        }
    }

    /// An empty RTL paragraph.
    fn ast() -> ParagraphAst {
        ParagraphAst {
            annotations: Vec::new(),
            bidi_spans: Vec::new(),
            writing_mode: WritingMode::HorizontalTb,
            base_direction: BidiDirection::Rtl,
            strut_metrics: FontMetrics {
                vertical_ascent: 5.0,
                vertical_descent: 5.0,
                ascent: 8.0,
                descent: 2.0,
                line_gap: 1.0,
                ..Default::default()
            },
            strut_font_size: 10.0,
            runs: vec![],
            atoms: vec![],
            clusters: vec![],
            glyphs: vec![],
            items: vec![WordNode::Discretionary(DiscretionaryNode::paragraph_end(0))],
            decoration_spans: vec![],
            atomic_spans: vec![],
        }
    }

    #[test]
    fn skeleton_returns_the_policy_and_direction_without_lines() {
        let options = LayoutOptions::default();
        let paragraph = layout_paragraph(&ast(), "", &NoopShaper, &constraints(), &options)
            .expect("valid inputs");
        assert_eq!(
            paragraph.policy,
            LayoutPolicySnapshot::resolve(&ast(), &constraints(), &options).unwrap()
        );
        assert_eq!(paragraph.policy.strut.line_height(), 11.0);
        assert_eq!(paragraph.base_direction, BidiDirection::Rtl);
        assert!(paragraph.lines.is_empty());
        // A paragraph has at least one line: the skeleton's output is not
        // valid yet.
        assert!(paragraph.validate("").is_err());
    }

    #[test]
    fn invalid_inputs_are_returned_to_the_caller() {
        let bad = LayoutConstraints {
            wrap_width: -1.0,
            ..constraints()
        };
        assert_eq!(
            layout_paragraph(&ast(), "\n", &NoopShaper, &bad, &LayoutOptions::default()),
            Err(LayoutInputError::InvalidConstraints)
        );
        assert_eq!(
            layout_paragraph(
                &ast(),
                "\n",
                &NoopShaper,
                &constraints(),
                &LayoutOptions::default()
            ),
            Err(LayoutInputError::ParagraphSeparatorInText)
        );
    }
}
