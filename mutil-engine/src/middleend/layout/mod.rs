//! The mid-end entry point (spec §5): the `Greedy` line breaker, line
//! placement and vertical stacking.
//!
//! Scope (spec RM-3, ME-3): natural-width greedy breaking with the overflow
//! ladder of ST1-1 (UAX #14 opportunities, then the emergency cluster break
//! of ST1-5), tabs measured from the line-start edge, hard breaks and blank
//! lines (MET-3), the strut and line boxes (Stage 4), decorations (Stage 5),
//! and a geometry provider's first slot. Each line applies UAX #9 L1 and L2
//! to the frontend's resolved levels (Stage 2) and groups span fragments
//! visually and logically (ST2-3). Under `LayoutOptions::justify` the
//! breaker fits with shrink and Stage 3 spends the slack through the
//! priority ladder (ST3-2 to ST3-5); marks hang by the `HangMode` (ST1-3).
//! Not yet: hyphenation, the optimising strategies (every strategy breaks
//! greedily), annotations and clamping.

mod breaking;
mod hanging;
mod justify;
mod line_artifacts;
mod line_box;
mod line_builder;

use crate::frontend::{
    AnnotationSide, BoxNode, DiscretionaryNode, GlueNode, InlineObjectBox,
    MAX_DECORATION_NESTING_DEPTH, ParagraphAst, SpanEdge, WordNode,
};
use crate::middleend::shaper_gate::checked_line_box;
use crate::middleend::{
    LayoutConstraints, LayoutInputError, LayoutOptions, LineGeometry, LineHeight, PositionedLine,
    PositionedParagraph, StrutMetrics, ValidatedInputs,
};
use crate::shaping::TextShaper;
use crate::types::{BidiDirection, BidiLevel, FontMetrics, RawShapedGlyph, TextRange};
use breaking::{LineFit, within};
use smallvec::SmallVec;
use std::collections::HashMap;

/// How many obstructed block positions the clearance loop tries in a row
/// before giving up on a geometry provider (spec GEO-4).
const MAX_CLEARANCE_PROBES: usize = 1024;

/// How often one line may be refitted because a taller envelope changed its
/// slots before layout gives up (spec GEO-6: an explicit failure, not a
/// clipped line).
const MAX_ENVELOPE_REFITS: usize = 16;

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
/// passes the provider gate. Every strategy breaks greedily; `justify` and
/// the hang mode apply (spec ME-3), while the clamp is not applied yet, so
/// every line is rendered.
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
        if let WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
            ascent, descent, ..
        })) = node
        {
            content_height = content_height.max(ascent + descent);
        }
        let Some(view) = inputs.text_box(item) else {
            continue;
        };
        let run = &ast.runs[view.run_index];
        for cluster in view.clusters {
            let instance = run.font_instance(cluster.font);
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

/// One placeable unit of the item sequence, in logical order. Nodes of the
/// AST are borrowed; clusters stay indexes, because an emergency re-shape
/// replaces their glyphs in [`Layout::reshaped`] (ADR-0003, rule 5).
#[derive(Clone, Copy, Debug)]
pub(super) enum PieceKind<'a> {
    /// Paragraph-level cluster index and its run.
    Cluster {
        run: usize,
        cluster: usize,
    },
    Object(&'a InlineObjectBox),
    Tab,
    Glue(&'a GlueNode),
    /// A decoration span edge: its padding is advance no glyph covers.
    SpanEdge {
        /// Index into `ParagraphAst::decoration_spans`.
        span: usize,
        edge: SpanEdge,
    },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Piece<'a> {
    pub(super) kind: PieceKind<'a>,
    pub(super) bytes: TextRange,
    pub(super) level: BidiLevel,
    /// Natural advance (a tab's is resolved at placement).
    pub(super) width: f32,
}

impl Piece<'_> {
    /// Content (as opposed to whitespace): what ends a line's visible part.
    pub(super) fn is_content(&self) -> bool {
        !matches!(self.kind, PieceKind::Glue(_)) && !self.is_edge_transparent()
    }

    /// A zero-width span boundary: the only piece that may lie between a
    /// hanging mark and its line edge (ST1-3).
    pub(super) fn is_edge_transparent(&self) -> bool {
        matches!(self.kind, PieceKind::SpanEdge { .. }) && self.width == 0.0
    }
}

/// One element of the line-breaking sequence: a piece, or a break
/// opportunity (a `Discretionary` item).
#[derive(Clone, Copy, Debug)]
pub(super) enum Token<'a> {
    Piece(Piece<'a>),
    Break(&'a DiscretionaryNode),
}

impl Token<'_> {
    /// The source bytes the token owns.
    pub(super) fn bytes(&self) -> TextRange {
        match self {
            Self::Piece(piece) => piece.bytes,
            Self::Break(brk) => brk.byte_range,
        }
    }
}

/// Where a line ends.
#[derive(Clone, Copy, Debug)]
pub(super) enum LineEnd<'a> {
    /// At the break token with this index, inclusive.
    At {
        token: usize,
        brk: &'a DiscretionaryNode,
    },
    /// Before the piece token with this index: an emergency cluster break.
    Before(usize),
}

/// A cluster whose glyphs an emergency re-shape replaced.
#[derive(Clone, Debug)]
pub(super) struct Reshaped {
    pub(super) glyphs: Vec<RawShapedGlyph>,
    pub(super) width: f32,
}

/// Emergency re-shapes by paragraph-level cluster index.
pub(super) type Reshapes = HashMap<usize, Reshaped>;

pub(super) struct Layout<'i, 'a> {
    pub(super) inputs: &'i ValidatedInputs<'a>,
    pub(super) ast: &'a ParagraphAst,
    pub(super) shaper: &'i dyn TextShaper,
    pub(super) options: &'a LayoutOptions,
    pub(super) tokens: Vec<Token<'a>>,
    pub(super) strut: StrutMetrics,
    pub(super) base: BidiDirection,
    pub(super) metrics: HashMap<(u32, u32, usize), FontMetrics>,
    pub(super) reshaped: Reshapes,
}

/// One piece of the current line, its level resolved by UAX #9 L1 in
/// `piece.level`, with its preceding no-break gap, horizontal placement and
/// baseline shift.
#[derive(Clone, Copy, Debug)]
pub(super) struct PlacedPiece<'a> {
    pub(super) piece: Piece<'a>,
    pub(super) gap: f32,
    pub(super) x: f32,
    pub(super) width: f32,
    /// Justification space after a cluster, inside `width` (ST3-2).
    pub(super) expansion: f32,
    pub(super) y_shift: f32,
}

/// Where the next line starts: its first token, the cloned padding before
/// it, its first source byte, its block offset and its index.
pub(super) struct LineCursor {
    pub(super) token_start: usize,
    pub(super) post_break_width: f32,
    pub(super) byte_start: usize,
    pub(super) block_top: f32,
    pub(super) index: usize,
}

impl LineCursor {
    fn paragraph_start() -> Self {
        Self {
            token_start: 0,
            post_break_width: 0.0,
            byte_start: 0,
            block_top: 0.0,
            index: 0,
        }
    }

    pub(super) fn first_line(&self) -> bool {
        self.index == 0
    }

    /// Moves past the line that ended at `end` and was `line_height` tall.
    fn advance(&mut self, end: LineEnd<'_>, tokens: &[Token<'_>], line_height: f32) {
        self.block_top += line_height;
        self.index += 1;
        match end {
            LineEnd::At { token, brk } => {
                self.post_break_width = brk.post_break_width;
                self.byte_start = brk.byte_range.end();
                self.token_start = token + 1;
            }
            LineEnd::Before(token) => {
                self.post_break_width = 0.0;
                self.byte_start = tokens[token].bytes().start();
                self.token_start = token;
            }
        }
    }

    /// Whether no token is left. Only the paragraph's final break ends a
    /// line with none left (every other line ends before it), so this is
    /// the end of the paragraph.
    fn is_paragraph_end(&self, tokens: &[Token<'_>]) -> bool {
        self.token_start >= tokens.len()
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    fn new(inputs: &'i ValidatedInputs<'a>, shaper: &'i dyn TextShaper) -> Self {
        let ast = inputs.ast();
        let mut tokens = Vec::with_capacity(ast.items.len() + ast.clusters.len());
        // Boundaries open in span order and close LIFO (AST-8), so the sweep
        // names each one's span without a lookup.
        let mut next_span = 0;
        let mut open_spans: SmallVec<[usize; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
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
                WordNode::Box(BoxNode::InlineObject(object)) => tokens.push(Token::Piece(Piece {
                    kind: PieceKind::Object(object),
                    bytes: object.byte_range,
                    level: object.bidi_level,
                    width: object.width,
                })),
                WordNode::Box(BoxNode::SpanBoundary {
                    byte_range,
                    edge,
                    width,
                    bidi_level,
                    ..
                }) => {
                    let span = match edge {
                        SpanEdge::Leading => {
                            open_spans.push(next_span);
                            next_span += 1;
                            next_span - 1
                        }
                        SpanEdge::Trailing => open_spans
                            .pop()
                            .expect("the input gate pairs every trailing boundary"),
                    };
                    tokens.push(Token::Piece(Piece {
                        kind: PieceKind::SpanEdge { span, edge: *edge },
                        bytes: *byte_range,
                        level: *bidi_level,
                        width: *width,
                    }));
                }
                WordNode::Glue(glue) => tokens.push(Token::Piece(Piece {
                    kind: PieceKind::Glue(glue),
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
                WordNode::Discretionary(d) => tokens.push(Token::Break(d)),
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
        let mut cursor = LineCursor::paragraph_start();
        loop {
            let (line, end) = self.place_next_line(&mut cursor)?;
            cursor.advance(end, &self.tokens, line.line_height);
            lines.push(line);
            if cursor.is_paragraph_end(&self.tokens) {
                break;
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
        cursor: &mut LineCursor,
    ) -> Result<(PositionedLine, LineEnd<'a>), LayoutInputError> {
        let strut_height = self.strut.line_height();
        let has_provider = self.inputs.constraints().geometry_provider.is_some();
        let mut obstructed = 0;
        'place: loop {
            let mut envelope = strut_height;
            let mut geometry = self.inputs.line_geometry(
                cursor.index,
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
                let LineFit { end, reshapes } = self.fit(cursor, slot);
                let added = self.commit_reshapes(reshapes);
                let line = self.place(cursor, end, &geometry)?;
                if within(line.line_height, envelope) || !has_provider {
                    return Ok((line, end));
                }
                envelope = line.line_height;
                let expanded = self.inputs.line_geometry(
                    cursor.index,
                    cursor.block_top,
                    cursor.block_top + envelope,
                    Some(&geometry),
                )?;
                if expanded == geometry {
                    return Ok((line, end));
                }
                // This attempt was not committed: discard its speculative
                // seam re-shapes before refitting in `expanded` or relocating.
                self.discard_reshapes(&added);
                if expanded.slots.is_empty() {
                    cursor.block_top = advance_clearance(&mut obstructed, &expanded)?;
                    continue 'place;
                }
                geometry = expanded;
            }
            return Err(LayoutInputError::InvalidLineGeometry);
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
            gap: 0.0,
            x: 0.0,
            width: 0.0,
            expansion: 0.0,
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
