//! The mid-end entry point (spec §5): the `Greedy` line breaker, line
//! placement and vertical stacking.
//!
//! Phase 1 scope (spec RM-3): natural-width greedy breaking with the overflow
//! ladder of ST1-1 (UAX #14 opportunities, then the emergency cluster break
//! of ST1-5), tabs, hard breaks and blank lines (MET-3), the strut and line
//! boxes (Stage 4), decorations (Stage 5), and a geometry provider's first
//! slot. Lines are reordered by their resolved levels (UAX #9 L1 and L2,
//! Stage 2), which for Phase 1's single-level text is the identity. Not yet:
//! justification, hanging punctuation, hyphenation, the optimising
//! strategies (every strategy breaks greedily), annotations and clamping.

mod breaking;
mod line_artifacts;
mod line_builder;

use super::shaper_gate::checked_line_box;
use super::{
    LayoutConstraints, LayoutInputError, LayoutOptions, LineGeometry, LineHeight, PositionedLine,
    PositionedParagraph, StrutMetrics, ValidatedInputs,
};
use crate::frontend::{AnnotationSide, BoxNode, ParagraphAst, WordNode};
use crate::shaping::TextShaper;
use crate::types::{
    BidiDirection, BidiLevel, FontInstanceRef, FontMetrics, RawShapedGlyph, TextRange,
};
use smallvec::SmallVec;
use std::collections::HashMap;

/// How many obstructed block positions the clearance loop tries in a row
/// before giving up on a geometry provider (spec GEO-4).
const MAX_CLEARANCE_PROBES: usize = 1024;

/// How often one line may be refitted because a taller envelope changed its
/// slots before layout gives up (spec GEO-6: an explicit failure, not a
/// clipped line).
const MAX_ENVELOPE_REFITS: usize = 16;

/// The fitting tolerance (spec ST1-8's `FIT_EPSILON`).
const FIT_EPSILON: f32 = super::FIT_EPSILON;

/// Lays out one paragraph (spec §5).
///
/// * `ast`: frontend output; `text`: the paragraph text it was built from
///   (shapers receive it per call).
/// * `shaper`: answers font metrics for line boxes and decorations through
///   the shaper gate, ligature carets, and re-shapes an unsafe emergency
///   seam.
/// * `constraints`: container geometry; `options`: layout policy.
///
/// The inputs pass the input gate ([`ValidatedInputs::new`]) first; with a
/// geometry provider, the first line is probed (spec GEO-5) and every answer
/// passes the provider gate. Every strategy breaks greedily in Phase 1, and
/// `justify`, the hang mode and the clamp are not applied yet: the result is
/// ragged, nothing hangs and every line is rendered.
///
/// # Errors
/// The first invalid input, or a shaper or geometry provider that breaks its
/// contract.
pub fn layout_paragraph(
    ast: &ParagraphAst,
    text: &str,
    shaper: &dyn TextShaper,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> Result<PositionedParagraph, LayoutInputError> {
    let inputs = ValidatedInputs::new(ast, text, constraints, options)?;
    if constraints.geometry_provider.is_some() {
        probe_first_line(&inputs, shaper)?;
    }
    Layout::new(&inputs, shaper).run()
}

/// Advances `obstructed` past an empty provider answer, failing once more
/// than [`MAX_CLEARANCE_PROBES`] obstructed attempts occur in a row
/// (spec GEO-4).
fn advance_clearance(
    obstructed: &mut usize,
    geometry: &LineGeometry,
) -> Result<f32, LayoutInputError> {
    *obstructed += 1;
    if *obstructed > MAX_CLEARANCE_PROBES {
        return Err(LayoutInputError::InvalidLineGeometry);
    }
    Ok(geometry
        .next_feature_block_offset
        .expect("the provider gate proved an obstructed answer points ahead"))
}

/// Checks that a geometry provider offers the first line a place to start.
///
/// Only the geometry that can begin at the block start is probed: the strut
/// and the input up to the first break opportunity. A tall object later in the
/// paragraph is not evidence about the first line, so it must not turn a legal
/// provider into a clearance failure.
fn probe_first_line(
    inputs: &ValidatedInputs<'_>,
    shaper: &dyn TextShaper,
) -> Result<(), LayoutInputError> {
    let strut_height = inputs.policy().strut.line_height();
    let tallest = tallest_first_line(inputs, shaper, strut_height)?;
    let mut block_start = 0.0;
    let mut obstructed = 0;
    loop {
        let probe = inputs.line_geometry(0, block_start, block_start + strut_height, None)?;
        if tallest > strut_height {
            // Contract check only: an expanded envelope must stay inside the
            // original one. Its contents do not decide whether the initial
            // position is legal.
            inputs.line_geometry(0, block_start, block_start + tallest, Some(&probe))?;
        }
        if !probe.slots.is_empty() {
            return Ok(());
        }
        block_start = advance_clearance(&mut obstructed, &probe)?;
    }
}

/// The tallest line box the first line could need: the strut, the content up
/// to the first break opportunity, and the annotations over that content.
///
/// Every term is a measurement (spec MSR-1: the input gate bounds the AST's,
/// the shaper gate the shaper's), so the sum cannot overflow.
fn tallest_first_line(
    inputs: &ValidatedInputs<'_>,
    shaper: &dyn TextShaper,
    strut_height: f32,
) -> Result<f32, LayoutInputError> {
    let ast = inputs.ast();
    let options = inputs.options();
    let first_break = ast
        .items
        .iter()
        .position(WordNode::is_break_opportunity)
        .unwrap_or(ast.items.len());
    let content_end = ast
        .items
        .get(first_break)
        .map_or(usize::MAX, |node| node.byte_span().start);
    let mut content_height = strut_height;
    for (item, node) in ast.items[..first_break].iter().enumerate() {
        if let WordNode::Box(BoxNode::InlineObject {
            ascent, descent, ..
        }) = node
        {
            content_height = content_height.max(ascent + descent);
        }
        let Some(view) = inputs.text_box(item) else {
            continue;
        };
        let run = &ast.runs[view.run_index];
        for cluster in view.clusters {
            let instance = FontInstanceRef {
                font: cluster.font,
                font_size: run.font_size,
                variations: run.features.variations(),
            };
            let line_box =
                checked_line_box(shaper, instance, options.line_height, ast.writing_mode)?;
            content_height = content_height.max(line_box.line_height());
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
    Ok(content_height + over + under)
}

/// One placeable unit of the item sequence, in logical order.
#[derive(Clone, Copy, Debug)]
pub(super) enum PieceKind {
    /// Paragraph-level cluster index and its run.
    Cluster {
        run: usize,
        cluster: usize,
    },
    /// Index into `ParagraphAst::items`.
    Object {
        item: usize,
    },
    Tab,
    Glue,
    /// A decoration span edge: its padding is advance no glyph covers.
    SpanEdge {
        span_id: u64,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Piece {
    pub(super) kind: PieceKind,
    pub(super) bytes: TextRange,
    pub(super) level: BidiLevel,
    /// Natural advance (a tab's is resolved at placement).
    pub(super) width: f32,
}

impl Piece {
    /// Content (as opposed to whitespace): what ends a line's visible part.
    pub(super) fn is_content(&self) -> bool {
        !matches!(self.kind, PieceKind::Glue)
    }
}

/// A break opportunity: a `Discretionary` item.
#[derive(Clone, Debug)]
pub(super) struct Break {
    pub(super) bytes: TextRange,
    pub(super) mandatory: bool,
    pub(super) pre: f32,
    pub(super) post: f32,
    pub(super) no_break: f32,
    pub(super) cloned: SmallVec<[u64; 2]>,
    /// The break's tier on the overflow ladder (spec ST1-1): 0 for an
    /// untailored opportunity, 1 for a URL break at punctuation, 2 for one
    /// inside a long URL piece.
    pub(super) tier: usize,
}

#[derive(Clone, Debug)]
pub(super) enum Token {
    Piece(Piece),
    Break(Break),
}

/// Where a line ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LineEnd {
    /// At the break token with this index, inclusive.
    At(usize),
    /// Before the piece token with this index: an emergency cluster break.
    Before(usize),
}

/// A cluster whose glyphs an emergency re-shape replaced.
#[derive(Clone, Debug)]
pub(super) struct Reshaped {
    pub(super) glyphs: Vec<RawShapedGlyph>,
    pub(super) width: f32,
}

pub(super) struct Layout<'i, 'a> {
    pub(super) inputs: &'i ValidatedInputs<'a>,
    pub(super) ast: &'a ParagraphAst,
    pub(super) shaper: &'i dyn TextShaper,
    pub(super) options: &'a LayoutOptions,
    pub(super) tokens: Vec<Token>,
    pub(super) strut: StrutMetrics,
    pub(super) base: BidiDirection,
    pub(super) metrics: HashMap<(u32, u32, usize), FontMetrics>,
    pub(super) reshaped: HashMap<usize, Reshaped>,
}

/// One piece of the current line together with its L1-resolved bidi level,
/// preceding no-break gap, horizontal placement and baseline shift.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlacedPiece {
    pub(super) piece: Piece,
    pub(super) level: BidiLevel,
    pub(super) gap: f32,
    pub(super) x: f32,
    pub(super) width: f32,
    pub(super) y_shift: f32,
}

/// Cursor tracking the current line's position in tokens, bytes, and block space.
pub(super) struct LineCursor {
    pub(super) token_start: usize,
    pub(super) post_break_width: f32,
    pub(super) byte_start: usize,
    pub(super) block_top: f32,
}

impl<'i, 'a> Layout<'i, 'a> {
    fn new(inputs: &'i ValidatedInputs<'a>, shaper: &'i dyn TextShaper) -> Self {
        let ast = inputs.ast();
        let mut tokens = Vec::with_capacity(ast.items.len() + ast.clusters.len());
        for (index, item) in ast.items.iter().enumerate() {
            match item {
                WordNode::Box(BoxNode::Text { cluster_range, .. }) => {
                    let view = inputs
                        .text_box(index)
                        .expect("the input gate resolved every text box");
                    for (cluster, info) in cluster_range.clone().zip(view.clusters) {
                        tokens.push(Token::Piece(Piece {
                            kind: PieceKind::Cluster {
                                run: view.run_index,
                                cluster,
                            },
                            bytes: info.byte_range,
                            level: info.bidi_level,
                            width: info.natural_width,
                        }));
                    }
                }
                WordNode::Box(BoxNode::InlineObject {
                    byte_range,
                    width,
                    bidi_level,
                    ..
                }) => tokens.push(Token::Piece(Piece {
                    kind: PieceKind::Object { item: index },
                    bytes: *byte_range,
                    level: *bidi_level,
                    width: *width,
                })),
                WordNode::Box(BoxNode::SpanBoundary {
                    byte_range,
                    span_id,
                    width,
                    bidi_level,
                    ..
                }) => tokens.push(Token::Piece(Piece {
                    kind: PieceKind::SpanEdge { span_id: *span_id },
                    bytes: *byte_range,
                    level: *bidi_level,
                    width: *width,
                })),
                WordNode::Glue(glue) => tokens.push(Token::Piece(Piece {
                    kind: PieceKind::Glue,
                    bytes: glue.byte_range,
                    level: glue.bidi_level,
                    width: glue.natural_width,
                })),
                WordNode::Tab { byte_range } => tokens.push(Token::Piece(Piece {
                    kind: PieceKind::Tab,
                    bytes: *byte_range,
                    level: ast.base_direction.base_level(),
                    width: 0.0,
                })),
                WordNode::Discretionary(d) => tokens.push(Token::Break(Break {
                    bytes: d.byte_range,
                    mandatory: d.is_mandatory(),
                    pre: d.pre_break_width,
                    post: d.post_break_width,
                    no_break: d.no_break_width,
                    cloned: d.cloned_spans.clone(),
                    tier: breaking::ladder_tier(d),
                })),
            }
        }
        Self {
            inputs,
            ast,
            shaper,
            options: inputs.options(),
            tokens,
            strut: inputs.policy().strut,
            base: ast.base_direction,
            metrics: HashMap::new(),
            reshaped: HashMap::new(),
        }
    }

    fn run(mut self) -> Result<PositionedParagraph, LayoutInputError> {
        let mut lines: Vec<PositionedLine> = Vec::new();
        let mut cursor = LineCursor {
            token_start: 0,
            post_break_width: 0.0,
            byte_start: 0,
            block_top: 0.0,
        };
        loop {
            let (line, end) = self.place_next_line(lines.len(), &mut cursor)?;
            cursor.block_top += line.line_height;
            lines.push(line);
            match end {
                LineEnd::At(token_idx) => {
                    let Token::Break(brk) = &self.tokens[token_idx] else {
                        unreachable!("a line ends at a break token")
                    };
                    cursor.post_break_width = brk.post;
                    cursor.byte_start = brk.bytes.end();
                    cursor.token_start = token_idx + 1;
                    if cursor.token_start >= self.tokens.len() {
                        break;
                    }
                }
                LineEnd::Before(token_idx) => {
                    cursor.post_break_width = 0.0;
                    cursor.byte_start = self.piece(token_idx).bytes.start();
                    cursor.token_start = token_idx;
                }
            }
        }
        Ok(PositionedParagraph {
            policy: self.inputs.policy().clone(),
            base_direction: self.base,
            total_block_size: cursor.block_top,
            lines,
            truncated_range: None,
        })
    }

    /// Probes geometry and refits until the line fits its envelope (spec GEO-3–6).
    fn place_next_line(
        &mut self,
        index: usize,
        cursor: &mut LineCursor,
    ) -> Result<(PositionedLine, LineEnd), LayoutInputError> {
        let strut_height = self.strut.line_height();
        let has_provider = self.inputs.constraints().geometry_provider.is_some();
        let mut obstructed = 0;
        'place: loop {
            let mut envelope = strut_height;
            let mut geometry = self.inputs.line_geometry(
                index,
                cursor.block_top,
                cursor.block_top + envelope,
                None,
            )?;
            if geometry.slots.is_empty() {
                cursor.block_top = advance_clearance(&mut obstructed, &geometry)?;
                continue;
            }
            for _ in 0..MAX_ENVELOPE_REFITS {
                let slot = geometry.slots[0];
                let saved_reshaped = has_provider.then(|| self.reshaped.clone());
                let end = self.fit(cursor.token_start, cursor.post_break_width, slot);
                let line = self.place(index, cursor, end, &geometry)?;
                if line.line_height
                    <= envelope + super::tolerance::tolerance(line.line_height, envelope)
                    || !has_provider
                {
                    return Ok((line, end));
                }
                envelope = line.line_height;
                let expanded = self.inputs.line_geometry(
                    index,
                    cursor.block_top,
                    cursor.block_top + envelope,
                    Some(&geometry),
                )?;
                if expanded == geometry {
                    return Ok((line, end));
                }
                // This attempt was not committed: discard speculative seam
                // re-shapes before refitting in `expanded` or relocating.
                if let Some(saved) = saved_reshaped {
                    self.reshaped = saved;
                }
                if expanded.slots.is_empty() {
                    cursor.block_top = advance_clearance(&mut obstructed, &expanded)?;
                    continue 'place;
                }
                geometry = expanded;
            }
            return Err(LayoutInputError::InvalidLineGeometry);
        }
    }

    pub(super) fn piece(&self, token: usize) -> &Piece {
        match &self.tokens[token] {
            Token::Piece(piece) => piece,
            Token::Break(_) => unreachable!("token {token} is a piece"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::DiscretionaryNode;
    use crate::middleend::{LayoutPolicySnapshot, TabStops};
    use crate::shaping::NoopShaper;
    use crate::types::{BidiDirection, FontMetrics, WritingMode};

    #[test]
    fn ladder_tiers_read_the_penalty_net_of_cloned_spans() {
        let with = |penalty: i32, clones: &[u64]| {
            let mut d = DiscretionaryNode::direct(0);
            d.penalty = penalty;
            d.cloned_spans = clones.iter().copied().collect();
            breaking::ladder_tier(&d)
        };
        let (min, max, clone) = (
            DiscretionaryNode::URL_PENALTY_MIN,
            DiscretionaryNode::URL_PENALTY_MAX,
            DiscretionaryNode::CLONED_SPAN_PENALTY,
        );
        assert_eq!(with(0, &[]), 0);
        assert_eq!(with(min, &[]), 1);
        assert_eq!(with(max - 1, &[]), 1);
        assert_eq!(with(max, &[]), 2);
        // Hyphenation and math breaks are not tailored.
        assert_eq!(with(DiscretionaryNode::HYPHEN_PENALTY, &[]), 0);
        // Inside two Clone spans, a URL punctuation break is still tier 1,
        // and a natural break with the same total is tier 0.
        assert_eq!(with(min + 2 * clone, &[1, 2]), 1);
        assert_eq!(with(max + clone, &[1]), 2);
        assert_eq!(with(clone, &[1]), 0);
        let mandatory = DiscretionaryNode::hard_break(TextRange::new(0, 3));
        assert_eq!(breaking::ladder_tier(&mandatory), 0);
    }

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
    fn an_empty_paragraph_is_one_strut_high_empty_line() {
        let options = LayoutOptions::default();
        let paragraph = layout_paragraph(&ast(), "", &NoopShaper, &constraints(), &options)
            .expect("valid inputs");
        assert_eq!(
            paragraph.policy,
            LayoutPolicySnapshot::resolve(&ast(), &constraints(), &options).unwrap()
        );
        assert_eq!(paragraph.base_direction, BidiDirection::Rtl);
        assert_eq!(paragraph.lines.len(), 1);
        let line = &paragraph.lines[0];
        assert_eq!(line.line_height, 11.0);
        assert_eq!(paragraph.total_block_size, 11.0);
        assert_eq!(line.caret_stops.len(), 1);
        assert_eq!(line.caret_stops[0].level, BidiLevel::RTL);
        assert_eq!(paragraph.validate(""), Ok(()));
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

    #[test]
    fn levels_reverse_from_the_highest_to_the_lowest_odd_one() {
        let piece = |v: u8| PlacedPiece {
            piece: Piece {
                kind: PieceKind::Tab,
                bytes: TextRange::empty_at(0),
                level: BidiLevel::new(v).unwrap(),
                width: 0.0,
            },
            level: BidiLevel::new(v).unwrap(),
            gap: 0.0,
            x: 0.0,
            width: 0.0,
            y_shift: 0.0,
        };
        let order = |levels: &[u8]| {
            let pieces: Vec<PlacedPiece> = levels.iter().copied().map(piece).collect();
            line_builder::visual_order(&pieces)
        };
        assert_eq!(order(&[0, 0]), [0, 1]);
        assert_eq!(order(&[1, 1, 1]), [2, 1, 0]);
        assert_eq!(order(&[0, 1, 1, 0]), [0, 2, 1, 3]);
        assert_eq!(order(&[1, 2, 2, 1]), [3, 1, 2, 0]);
    }
}
