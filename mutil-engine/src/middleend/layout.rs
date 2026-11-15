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

use super::shaper_gate::checked_line_box;
use super::{
    AtomicFragment, CaretAffinity, CaretStop, DecorationKind, DecorationLine,
    HangingWhitespaceMode, InlineSlotConstraint, LayoutConstraints, LayoutInputError,
    LayoutOptions, LineGeometry, LineHeight, PositionedGlyph, PositionedInlineBox, PositionedLine,
    PositionedParagraph, PositionedSlot, PositionedSpanFragment, PositionedTab, StrutMetrics,
    ValidatedInputs, VisualGlyphRun, resolve_ligature_carets,
};
use crate::frontend::{
    AnnotationSide, BoxNode, BoxOverflowPolicy, DiscretionaryNode, ParagraphAst, WordNode,
};
use crate::shaping::TextShaper;
use crate::types::{
    AtomFlags, BaselineAlignment, BidiDirection, BidiLevel, FontInstanceRef, FontMetrics,
    TextRange, WritingMode,
};
use smallvec::SmallVec;
use std::collections::HashMap;

/// How many obstructed block positions the first-line probe tries before it
/// gives up on a geometry provider.
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
    for _ in 0..MAX_CLEARANCE_PROBES {
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
        block_start = probe
            .next_feature_block_offset
            .expect("the provider gate proved an obstructed answer points ahead");
    }
    Err(LayoutInputError::InvalidLineGeometry)
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
enum PieceKind {
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
struct Piece {
    kind: PieceKind,
    bytes: TextRange,
    level: BidiLevel,
    /// Natural advance (a tab's is resolved at placement).
    width: f32,
}

impl Piece {
    /// Content (as opposed to whitespace): what ends a line's visible part.
    fn is_content(&self) -> bool {
        !matches!(self.kind, PieceKind::Glue)
    }
}

/// A break opportunity: a `Discretionary` item.
#[derive(Clone, Debug)]
struct Break {
    bytes: TextRange,
    mandatory: bool,
    pre: f32,
    post: f32,
    no_break: f32,
    cloned: SmallVec<[u64; 2]>,
    /// The break's tier on the overflow ladder (spec ST1-1): 0 for an
    /// untailored opportunity, 1 for a URL break at punctuation, 2 for one
    /// inside a long URL piece.
    tier: usize,
}

/// The tier of a discretionary on the overflow ladder (spec ST1-1): its
/// penalty net of cloned-span penalties (WN-21) in the URL band (WN-19)
/// below its maximum is tier 1, at the maximum tier 2; any other is tier 0.
fn ladder_tier(d: &DiscretionaryNode) -> usize {
    if d.is_mandatory() {
        return 0;
    }
    let clones = i32::try_from(d.cloned_spans.len()).unwrap_or(i32::MAX);
    let net = d
        .penalty
        .saturating_sub(clones.saturating_mul(DiscretionaryNode::CLONED_SPAN_PENALTY));
    match net {
        DiscretionaryNode::URL_PENALTY_MAX => 2,
        n if (DiscretionaryNode::URL_PENALTY_MIN..DiscretionaryNode::URL_PENALTY_MAX)
            .contains(&n) =>
        {
            1
        }
        _ => 0,
    }
}

#[derive(Clone, Debug)]
enum Token {
    Piece(Piece),
    Break(Break),
}

/// Where a line ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnd {
    /// At the break token with this index, inclusive.
    At(usize),
    /// Before the piece token with this index: an emergency cluster break.
    Before(usize),
}

/// A cluster whose glyphs an emergency re-shape replaced.
#[derive(Clone, Debug)]
struct Reshaped {
    glyphs: Vec<crate::types::RawShapedGlyph>,
    width: f32,
}

struct Layout<'i, 'a> {
    inputs: &'i ValidatedInputs<'a>,
    ast: &'a ParagraphAst,
    text: &'a str,
    shaper: &'i dyn TextShaper,
    options: &'a LayoutOptions,
    tokens: Vec<Token>,
    strut: StrutMetrics,
    base: BidiDirection,
    line_boxes: HashMap<(u32, u32, usize), StrutMetrics>,
    metrics: HashMap<(u32, u32, usize), FontMetrics>,
    reshaped: HashMap<usize, Reshaped>,
    /// The width of the slot being filled: an object's reference width
    /// (spec WN-7).
    reference: f32,
    /// The end of that slot, in line-local coordinates.
    slot_end: f32,
}

/// A line's content placed relative to its own origin, before the slot
/// offset is known.
struct Placed {
    line: PositionedLine,
    /// The relative left edge of the content block.
    content_lo: f32,
    /// Its width.
    content_width: f32,
}

impl<'i, 'a> Layout<'i, 'a> {
    fn new(inputs: &'i ValidatedInputs<'a>, shaper: &'i dyn TextShaper) -> Self {
        let ast = inputs.ast();
        let mut tokens = Vec::with_capacity(ast.items.len() + ast.clusters.len());
        for (index, item) in ast.items.iter().enumerate() {
            match item {
                WordNode::Box(BoxNode::Text { .. }) => {
                    let view = inputs
                        .text_box(index)
                        .expect("the input gate resolved every text box");
                    let first = match item {
                        WordNode::Box(BoxNode::Text { cluster_range, .. }) => cluster_range.start,
                        _ => unreachable!("matched above"),
                    };
                    for (k, cluster) in view.clusters.iter().enumerate() {
                        tokens.push(Token::Piece(Piece {
                            kind: PieceKind::Cluster {
                                run: view.run_index,
                                cluster: first + k,
                            },
                            bytes: cluster.byte_range,
                            level: cluster.bidi_level,
                            width: cluster.natural_width,
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
                    tier: ladder_tier(d),
                })),
            }
        }
        Self {
            inputs,
            ast,
            text: inputs.text(),
            shaper,
            options: inputs.options(),
            tokens,
            strut: inputs.policy().strut,
            base: ast.base_direction,
            line_boxes: HashMap::new(),
            metrics: HashMap::new(),
            reshaped: HashMap::new(),
            reference: f32::INFINITY,
            slot_end: f32::INFINITY,
        }
    }

    fn run(mut self) -> Result<PositionedParagraph, LayoutInputError> {
        let strut_height = self.strut.line_height();
        let mut lines: Vec<PositionedLine> = Vec::new();
        let mut start = 0;
        let mut post = 0.0;
        let mut byte_start = 0;
        let mut block = 0.0_f32;
        loop {
            let index = lines.len();
            let mut envelope = strut_height;
            let mut obstructed = 0;
            let (line, end) = 'place: loop {
                let geometry = self
                    .inputs
                    .line_geometry(index, block, block + envelope, None)?;
                if geometry.slots.is_empty() {
                    obstructed += 1;
                    if obstructed > MAX_CLEARANCE_PROBES {
                        return Err(LayoutInputError::InvalidLineGeometry);
                    }
                    block = geometry
                        .next_feature_block_offset
                        .ok_or(LayoutInputError::InvalidLineGeometry)?;
                    continue;
                }
                let mut geometry = geometry;
                for _ in 0..MAX_ENVELOPE_REFITS {
                    let slot = geometry.slots[0];
                    let end = self.fit(start, post, slot);
                    let line = self.place(index, start, post, byte_start, end, &geometry)?;
                    if line.line_height
                        <= envelope + super::tolerance::tolerance(line.line_height, envelope)
                        || self.inputs.constraints().geometry_provider.is_none()
                    {
                        break 'place (line, end);
                    }
                    envelope = line.line_height;
                    let expanded = self.inputs.line_geometry(
                        index,
                        block,
                        block + envelope,
                        Some(&geometry),
                    )?;
                    if expanded.slots.is_empty() {
                        // No room for a line this tall here: move past the
                        // obstruction and start again.
                        block = expanded
                            .next_feature_block_offset
                            .filter(|next| *next > block)
                            .ok_or(LayoutInputError::InvalidLineGeometry)?;
                        continue 'place;
                    }
                    if expanded == geometry {
                        break 'place (line, end);
                    }
                    geometry = expanded;
                }
                return Err(LayoutInputError::InvalidLineGeometry);
            };
            let mut line = line;
            let (top, _) = (block, ());
            line.baseline_y = top
                + if self.ast.writing_mode == WritingMode::VerticalLr {
                    line.descent
                } else {
                    line.ascent
                };
            block = top + line.line_height;
            let finished = match end {
                LineEnd::At(t) => {
                    let Token::Break(b) = &self.tokens[t] else {
                        unreachable!("a line ends at a break token")
                    };
                    post = b.post;
                    byte_start = b.bytes.end();
                    start = t + 1;
                    start >= self.tokens.len()
                }
                LineEnd::Before(t) => {
                    post = 0.0;
                    byte_start = self.piece(t).bytes.start();
                    start = t;
                    false
                }
            };
            lines.push(line);
            if finished {
                break;
            }
        }
        Ok(PositionedParagraph {
            policy: self.inputs.policy().clone(),
            base_direction: self.base,
            truncated_range: None,
            total_block_size: block,
            lines,
        })
    }

    fn piece(&self, token: usize) -> &Piece {
        match &self.tokens[token] {
            Token::Piece(piece) => piece,
            Token::Break(_) => unreachable!("token {token} is a piece"),
        }
    }

    /// The next tab stop after the absolute pen position `pen` that lies
    /// inside the current slot, if any (spec ST3-1): a stop beyond the slot's
    /// end is not reachable from it.
    fn reachable_stop(&self, pen: f32) -> Option<f32> {
        self.inputs
            .constraints()
            .tab_stops
            .next_stop(pen)
            .filter(|&stop| {
                stop <= self.slot_end + super::tolerance::tolerance(stop, self.slot_end)
            })
    }

    /// The advance and the reach (rightmost pen end, relative to the start)
    /// of a piece starting at the absolute pen position `pen`.
    fn measure(&self, piece: &Piece, pen: f32) -> (f32, f32) {
        match piece.kind {
            PieceKind::Tab => {
                // A tab with no reachable stop takes no advance (ST3-1, AX-6).
                let width = self.reachable_stop(pen).map_or(0.0, |stop| stop - pen);
                (width, width)
            }
            PieceKind::Cluster { cluster, run } => {
                let (glyphs, width) = self.cluster_glyphs(cluster, run);
                let mut x = 0.0_f32;
                let mut reach = width;
                for g in glyphs {
                    x += g.x_advance;
                    reach = reach.max(x);
                }
                (width, reach)
            }
            PieceKind::Object { item } => {
                let (width, _, _) = self.fitted_object(item);
                (width, width)
            }
            _ => (piece.width, piece.width),
        }
    }

    /// An object's width, ascent and descent under its overflow policy in
    /// the current slot (spec WN-7): `Clip` clamps to the reference width,
    /// `ScaleToFit` scales uniformly to it, `AllowOverflow` keeps its size.
    fn fitted_object(&self, item: usize) -> (f32, f32, f32) {
        let WordNode::Box(BoxNode::InlineObject {
            width,
            ascent,
            descent,
            overflow_policy,
            ..
        }) = &self.ast.items[item]
        else {
            unreachable!("an object piece names an object item")
        };
        let (width, ascent, descent) = (*width, *ascent, *descent);
        let reference = self.reference;
        match overflow_policy {
            BoxOverflowPolicy::AllowOverflow => (width, ascent, descent),
            _ if width <= reference => (width, ascent, descent),
            BoxOverflowPolicy::Clip => (reference, ascent, descent),
            BoxOverflowPolicy::ScaleToFit => {
                let s = reference / width;
                (reference, ascent * s, descent * s)
            }
        }
    }

    /// The glyphs and width of a cluster, after any emergency re-shape.
    fn cluster_glyphs(
        &self,
        cluster: usize,
        _run: usize,
    ) -> (Vec<crate::types::RawShapedGlyph>, f32) {
        if let Some(r) = self.reshaped.get(&cluster) {
            return (r.glyphs.clone(), r.width);
        }
        let c = &self.ast.clusters[cluster];
        (self.inputs.cluster_glyphs(c).to_vec(), c.natural_width)
    }

    /// Greedy fitting (spec ST1-1, ST1-5, WN-1): the last break of the
    /// lowest ladder tier that fits before the content overflows, a
    /// mandatory break, or an emergency cluster break when nothing fits.
    fn fit(&mut self, start: usize, post: f32, slot: InlineSlotConstraint) -> LineEnd {
        let available = slot.width();
        self.reference = available;
        self.slot_end = slot.end();
        let origin = slot.start();
        let mut pen = post;
        let mut body = post;
        let mut has_content = false;
        // The last fitting break of each ladder tier.
        let mut fits: [Option<usize>; 3] = [None; 3];
        let best = |fits: &[Option<usize>; 3]| fits.iter().find_map(|&b| b);
        let mut t = start;
        loop {
            match &self.tokens[t] {
                Token::Break(b) => {
                    if b.mandatory {
                        return LineEnd::At(t);
                    }
                    if body + b.pre <= available + FIT_EPSILON {
                        fits[b.tier] = Some(t);
                    }
                    pen += b.no_break;
                }
                Token::Piece(piece)
                    if matches!(piece.kind, PieceKind::Tab)
                        && self.reachable_stop(origin + pen).is_none() =>
                {
                    // No reachable stop (spec ST3-1, AX-6): break before the
                    // tab at an earlier opportunity, else consume it with no
                    // advance and end the line right after it.
                    if has_content && let Some(b) = best(&fits) {
                        return LineEnd::At(b);
                    }
                    return match self.tokens.get(t + 1) {
                        Some(Token::Break(_)) => LineEnd::At(t + 1),
                        _ => LineEnd::Before(t + 1),
                    };
                }
                Token::Piece(piece) => {
                    let piece = *piece;
                    let (advance, reach) = self.measure(&piece, origin + pen);
                    if piece.is_content() {
                        let end = pen + reach.max(advance);
                        if has_content && end > available + FIT_EPSILON {
                            return match best(&fits) {
                                Some(b) => LineEnd::At(b),
                                None => self.emergency(start, t, post, origin),
                            };
                        }
                        if !has_content && end > available + FIT_EPSILON {
                            if let Some(b) = best(&fits) {
                                return LineEnd::At(b);
                            }
                            return self.emergency(start, t, post, origin);
                        }
                        body = end;
                        has_content = true;
                    }
                    pen += advance;
                }
            }
            t += 1;
        }
    }

    /// The break token right after `t` and its trailing glue, if any.
    fn next_break_after(&self, t: usize) -> Option<usize> {
        let mut e = t + 1;
        while let Some(Token::Piece(p)) = self.tokens.get(e) {
            if !matches!(p.kind, PieceKind::Glue) {
                return None;
            }
            e += 1;
        }
        matches!(self.tokens.get(e), Some(Token::Break(_))).then_some(e)
    }

    /// The emergency break (spec ST1-5) of a line from `start` whose piece
    /// `t` overflows with no fitting opportunity: before `t` when something
    /// visible precedes it, else after `t` alone (one indivisible entity on
    /// its own overflowing line, SLT-5), together with the break right after
    /// it if only glue separates them.
    fn emergency(&mut self, start: usize, t: usize, post: f32, origin: f32) -> LineEnd {
        let first_content =
            (start..t).find(|&k| matches!(&self.tokens[k], Token::Piece(p) if p.is_content()));
        if let Some(first) = first_content {
            // Re-shaping an unsafe seam can widen its sides (kerning across
            // it is lost); while the line then overflows, the seam moves one
            // piece back and the abandoned re-shape is dropped.
            let mut t = t;
            loop {
                // A dash, ellipsis or middle-dot run (FE-15) moves whole to
                // the next line when something precedes it on this one. A
                // run that starts the line and is still wider than the slot
                // is split like any other unit (ST1-5): only one cluster may
                // overflow a slot (SLT-5).
                let run_start = self.dash_run_start(t);
                if run_start > first {
                    t = run_start;
                }
                let added = self.repair_seam(t);
                if t <= first + 1
                    || self.content_end(start, t, post, origin) <= self.reference + FIT_EPSILON
                {
                    return LineEnd::Before(t);
                }
                for c in added {
                    self.reshaped.remove(&c);
                }
                t -= 1;
            }
        }
        if let Some(b) = self.next_break_after(t) {
            return LineEnd::At(b);
        }
        self.repair_seam(t + 1);
        LineEnd::Before(t + 1)
    }

    /// Whether the seam before token `t` lies inside a run of one of U+2014,
    /// U+2026 and U+00B7 (spec FE-15).
    fn inside_dash_run(&self, t: usize) -> bool {
        let Some(Token::Piece(p)) = self.tokens.get(t) else {
            return false;
        };
        let at = p.bytes.start();
        self.text[at..]
            .chars()
            .next()
            .filter(|c| matches!(c, '\u{2014}' | '\u{2026}' | '\u{00B7}'))
            .is_some_and(|c| self.text[..at].ends_with(c))
    }

    /// The first token of the run whose seam `t` lies inside, or `t`.
    fn dash_run_start(&self, mut t: usize) -> usize {
        while t > 0 && self.inside_dash_run(t) {
            t -= 1;
        }
        t
    }

    /// An emergency break before piece `t` that is not safe to break
    /// re-shapes the clusters on both sides of the seam (spec ST1-5), with no
    /// edit. A re-shape that does not return exactly one cluster per side is
    /// not used.
    fn repair_seam(&mut self, t: usize) -> SmallVec<[usize; 2]> {
        let mut added = SmallVec::new();
        let Some(Token::Piece(Piece {
            kind: PieceKind::Cluster { run, cluster },
            ..
        })) = self.tokens.get(t)
        else {
            return added;
        };
        let (run, after) = (*run, *cluster);
        if self.ast.clusters[after].safe_to_break_before {
            return added;
        }
        let run_clusters = &self.ast.runs[run].cluster_range;
        let mut sides = vec![after];
        if after > run_clusters.start {
            sides.push(after - 1);
        }
        let atoms = self.inputs.run_atoms(run);
        for c in sides {
            let window = self.ast.clusters[c].byte_range;
            let mut clusters = Vec::new();
            let mut glyphs = Vec::new();
            self.shaper.reshape_window(
                self.text,
                &self.ast.runs[run],
                atoms,
                window,
                crate::shaping::LeadingEdit::None,
                crate::shaping::TrailingEdit::None,
                &mut clusters,
                &mut glyphs,
            );
            let [only] = clusters.as_slice() else {
                continue;
            };
            let usable = only.atom_range == self.ast.clusters[c].atom_range
                && only.glyph_range == (0..glyphs.len())
                && glyphs.iter().all(|g| {
                    g.x_advance.is_finite() && g.x_offset.is_finite() && g.y_offset.is_finite()
                })
                && only.natural_width.is_finite()
                && only.natural_width >= 0.0;
            if usable {
                let advance: f32 = glyphs.iter().map(|g| g.x_advance).sum();
                let width = (advance + self.ast.runs[run].letter_spacing).max(0.0);
                if self
                    .reshaped
                    .insert(c, Reshaped { glyphs, width })
                    .is_none()
                {
                    added.push(c);
                }
            }
        }
        added
    }

    /// Where the content of a line from `start` ends when it stops before
    /// token `end`, measured as [`fit`](Self::fit) measures it.
    fn content_end(&self, start: usize, end: usize, post: f32, origin: f32) -> f32 {
        let mut pen = post;
        let mut body = post;
        for token in &self.tokens[start..end] {
            match token {
                Token::Break(b) => pen += b.no_break,
                Token::Piece(piece) => {
                    let (advance, reach) = self.measure(piece, origin + pen);
                    if piece.is_content() {
                        body = pen + reach.max(advance);
                    }
                    pen += advance;
                }
            }
        }
        body
    }

    /// The line box of a cluster's font instance (spec MET-1, MET-2), through
    /// the shaper gate.
    fn cluster_box(
        &mut self,
        run: usize,
        cluster: usize,
    ) -> Result<StrutMetrics, LayoutInputError> {
        let font = self.ast.clusters[cluster].font;
        let key = (font.font_id.0, font.scale.to_bits(), run);
        if let Some(found) = self.line_boxes.get(&key) {
            return Ok(*found);
        }
        let r = &self.ast.runs[run];
        let instance = FontInstanceRef {
            font,
            font_size: r.font_size,
            variations: r.features.variations(),
        };
        let line_box = checked_line_box(
            self.shaper,
            instance,
            self.options.line_height,
            self.ast.writing_mode,
        )?;
        self.line_boxes.insert(key, line_box);
        Ok(line_box)
    }

    /// A cluster's font metrics, through the shaper gate (spec G2).
    fn cluster_metrics(
        &mut self,
        run: usize,
        cluster: usize,
    ) -> Result<FontMetrics, LayoutInputError> {
        let font = self.ast.clusters[cluster].font;
        let key = (font.font_id.0, font.scale.to_bits(), run);
        if let Some(found) = self.metrics.get(&key) {
            return Ok(*found);
        }
        let r = &self.ast.runs[run];
        let metrics = self.shaper.font_metrics(FontInstanceRef {
            font,
            font_size: r.font_size,
            variations: r.features.variations(),
        });
        if !metrics.is_valid() {
            return Err(LayoutInputError::InvalidFontMetrics);
        }
        self.metrics.insert(key, metrics);
        Ok(metrics)
    }

    /// Places the line `(start, end]` in the first slot of `geometry`.
    fn place(
        &mut self,
        index: usize,
        start: usize,
        post: f32,
        byte_start: usize,
        end: LineEnd,
        geometry: &LineGeometry,
    ) -> Result<PositionedLine, LayoutInputError> {
        let slot = geometry.slots[0];
        self.reference = slot.width();
        self.slot_end = slot.end();
        let (last_piece, end_break, byte_end) = match end {
            LineEnd::At(t) => {
                let Token::Break(b) = &self.tokens[t] else {
                    unreachable!("a line ends at a break token")
                };
                (t, Some(b.clone()), b.bytes.end())
            }
            LineEnd::Before(t) => (t, None, self.piece(t).bytes.start()),
        };
        let soft_wrap = end_break.as_ref().is_none_or(|b| !b.mandatory);
        let pre = end_break.as_ref().map_or(0.0, |b| b.pre);
        // The line's pieces, logical order, with their no-break gaps.
        let mut pieces: Vec<(Piece, f32)> = Vec::new();
        let mut gap = 0.0;
        for token in &self.tokens[start..last_piece] {
            match token {
                Token::Piece(p) => {
                    pieces.push((*p, gap));
                    gap = 0.0;
                }
                Token::Break(b) => gap += b.no_break,
            }
        }
        let body_len = pieces
            .iter()
            .rposition(|(p, _)| p.is_content())
            .map_or(0, |k| k + 1);
        let base_level = self.base.base_level();
        // UAX #9 L1: trailing whitespace, tabs and the whitespace before a
        // tab take the paragraph level.
        let mut levels: Vec<BidiLevel> = pieces.iter().map(|(p, _)| p.level).collect();
        for k in (0..pieces.len()).rev() {
            let resets = k >= body_len
                || matches!(pieces[k].0.kind, PieceKind::Tab)
                || (matches!(pieces[k].0.kind, PieceKind::Glue)
                    && pieces[k + 1..]
                        .iter()
                        .take_while(|(p, _)| matches!(p.kind, PieceKind::Glue | PieceKind::Tab))
                        .any(|(p, _)| matches!(p.kind, PieceKind::Tab)));
            if resets {
                levels[k] = base_level;
            }
        }
        let order = visual_order(&levels[..body_len]);
        let rtl = self.base.is_rtl();
        // Relative placement of the body, left to right.
        let (left_pad, right_pad) = if rtl { (pre, post) } else { (post, pre) };
        let origin = slot.start();
        let mut x = left_pad;
        let mut starts = vec![0.0_f32; pieces.len()];
        let mut widths = vec![0.0_f32; pieces.len()];
        for &k in &order {
            let (piece, gap) = pieces[k];
            x += gap;
            let (advance, _) = self.measure(&piece, origin + x);
            starts[k] = x;
            widths[k] = advance;
            x += advance;
        }
        let body_end = x + right_pad;
        // Trailing whitespace beyond the far edge (UAX #9 L1, HNG-3).
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        let trailing_width = |w: f32| if collapsible { 0.0 } else { w };
        if rtl {
            let mut tx = 0.0;
            for k in body_len..pieces.len() {
                let w = trailing_width(pieces[k].0.width);
                tx -= w;
                starts[k] = tx;
                widths[k] = w;
            }
        } else {
            let mut tx = body_end;
            for k in body_len..pieces.len() {
                let w = trailing_width(pieces[k].0.width);
                starts[k] = tx;
                widths[k] = w;
                tx += w;
            }
        }
        let placed = self.build_line(
            index,
            &pieces,
            &levels,
            &order,
            &starts,
            &widths,
            body_len,
            soft_wrap,
            TextRange::new(byte_start, byte_end),
            (left_pad, body_end),
            end_break.as_ref(),
            post,
        )?;
        let Placed {
            mut line,
            content_lo,
            content_width,
        } = placed;
        // Alignment: the line start of the base direction.
        let content_start = if rtl && slot.end().is_finite() && content_width <= slot.width() {
            slot.end() - content_width
        } else {
            slot.start()
        };
        let shift = content_start - content_lo;
        shift_line(&mut line, shift);
        let mut slots: SmallVec<[PositionedSlot; 1]> = geometry
            .slots
            .iter()
            .map(|&constraint| PositionedSlot {
                constraint,
                content_start: constraint.start(),
                visual_width: 0.0,
            })
            .collect();
        slots[0].content_start = content_start;
        slots[0].visual_width = content_width;
        line.slots = slots;
        Ok(line)
    }

    #[expect(clippy::too_many_arguments, reason = "one line's placement state")]
    fn build_line(
        &mut self,
        index: usize,
        pieces: &[(Piece, f32)],
        levels: &[BidiLevel],
        order: &[usize],
        starts: &[f32],
        widths: &[f32],
        body_len: usize,
        soft_wrap: bool,
        byte_range: TextRange,
        (body_lo, body_hi): (f32, f32),
        end_break: Option<&Break>,
        post: f32,
    ) -> Result<Placed, LayoutInputError> {
        let ast = self.ast;
        let mut visual_runs: Vec<VisualGlyphRun> = Vec::new();
        let mut visual_boxes: Vec<PositionedInlineBox> = Vec::new();
        let mut tabs: Vec<PositionedTab> = Vec::new();
        let mut decorations: Vec<DecorationLine> = Vec::new();
        // Line box.
        let normal = matches!(self.options.line_height, LineHeight::Normal);
        let half = |m: StrutMetrics| (m.ascent + m.line_gap / 2.0, m.descent + m.line_gap / 2.0);
        let (mut above, mut below) = half(self.strut);
        let mut visible_end = byte_range.start();
        // Extents of the content, for the content block (SLT-3).
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut extend = |a: f32, b: f32| {
            lo = lo.min(a.min(b));
            hi = hi.max(a.max(b));
        };
        // Baseline alignment (§5 Stage 4): every item's shift towards
        // line-over. Named baselines and offsets are known before the line
        // box; line-top and line-bottom items are placed against it after.
        let mut shifts = vec![0.0_f32; pieces.len()];
        let mut line_relative: Vec<(usize, bool, f32, f32)> = Vec::new();
        for &k in order {
            let (piece, _) = pieces[k];
            let (align, a, d) = match piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    let (a, d) = if normal || is_line_relative(ast.runs[run].baseline_align) {
                        half(self.cluster_box(run, cluster)?)
                    } else {
                        (0.0, 0.0)
                    };
                    (ast.runs[run].baseline_align, a, d)
                }
                PieceKind::Object { item } => {
                    let WordNode::Box(BoxNode::InlineObject { baseline_align, .. }) =
                        &ast.items[item]
                    else {
                        unreachable!("an object piece names an object item")
                    };
                    let (_, a, d) = self.fitted_object(item);
                    (*baseline_align, a, d)
                }
                _ => continue,
            };
            let is_object = matches!(piece.kind, PieceKind::Object { .. });
            match align {
                BaselineAlignment::LineTop | BaselineAlignment::LineBottom => {
                    line_relative.push((k, align == BaselineAlignment::LineTop, a, d));
                }
                _ => {
                    let shift = match piece.kind {
                        PieceKind::Cluster { run, cluster } => {
                            let own = self.cluster_metrics(run, cluster)?;
                            run_shift(align, &self.ast.strut_metrics, &own)
                        }
                        _ => object_shift(align, &self.ast.strut_metrics),
                    };
                    shifts[k] = shift;
                    if normal || is_object {
                        above = above.max(a + shift);
                        below = below.max(d - shift);
                    }
                }
            }
        }
        // A line-relative item taller than the line deepens it on its free
        // side under `Normal` (spec BAS-1).
        if normal {
            for &(_, top, a, d) in &line_relative {
                let room = above + below;
                if a + d > room {
                    if top {
                        below += a + d - room;
                    } else {
                        above += a + d - room;
                    }
                }
            }
        }
        let (ascent, descent, line_height) = match self.options.line_height {
            LineHeight::Fixed(_) => (
                self.strut.ascent,
                self.strut.descent,
                self.strut.line_height(),
            ),
            LineHeight::Normal if byte_range.is_empty() => {
                let (a, d) = half(self.strut);
                (a, d, self.strut.line_height())
            }
            LineHeight::Normal => (above, below, above + below),
        };
        for &(k, top, a, d) in &line_relative {
            shifts[k] = if top { ascent - a } else { d - descent };
        }
        for &k in order {
            let (piece, _) = pieces[k];
            let x = starts[k];
            match piece.kind {
                PieceKind::Cluster { run, cluster } => {
                    let r = &ast.runs[run];
                    let c = &ast.clusters[cluster];
                    let shift = shifts[k];
                    let (mut glyphs, _) = self.cluster_glyphs(cluster, run);
                    if glyphs.is_empty() {
                        continue;
                    }
                    if levels[k].direction().is_rtl() {
                        glyphs.reverse();
                    }
                    let mut pen = x;
                    let positioned: Vec<PositionedGlyph> = glyphs
                        .iter()
                        .map(|g| {
                            let glyph = PositionedGlyph {
                                glyph_id: g.glyph_id,
                                x: pen,
                                x_offset: g.x_offset,
                                y_offset: g.y_offset + shift,
                                advance: g.x_advance,
                                byte_range: if g.is_synthesized {
                                    None
                                } else {
                                    Some(c.byte_range)
                                },
                                is_hanging: false,
                            };
                            pen += g.x_advance;
                            glyph
                        })
                        .collect();
                    if k < body_len {
                        for g in positioned.iter().filter(|g| g.advance != 0.0) {
                            extend(g.x, g.x + g.advance);
                        }
                        if !positioned.iter().all(PositionedGlyph::is_synthesized) {
                            visible_end = visible_end.max(c.byte_range.end());
                        }
                    }
                    let same_run = visual_runs.last().is_some_and(|v| {
                        v.font == c.font
                            && v.bidi_level == c.bidi_level
                            && v.font_size == r.font_size
                            && v.orientation == c.orientation
                            && v.variations.as_slice() == r.features.variations()
                    });
                    if same_run {
                        let v = visual_runs.last_mut().expect("checked above");
                        v.byte_range = v.byte_range.cover(c.byte_range);
                        v.glyphs.extend(positioned);
                    } else {
                        visual_runs.push(VisualGlyphRun {
                            orientation: c.orientation,
                            font_size: r.font_size,
                            variations: r.features.variations().iter().copied().collect(),
                            font: c.font,
                            bidi_level: c.bidi_level,
                            byte_range: c.byte_range,
                            glyphs: positioned,
                        });
                    }
                }
                PieceKind::Object { item } => {
                    let WordNode::Box(BoxNode::InlineObject {
                        baseline_align,
                        byte_range,
                        box_id,
                        overflow_policy,
                        bidi_level,
                        ..
                    }) = &ast.items[item]
                    else {
                        unreachable!("an object piece names an object item")
                    };
                    let shift = shifts[k];
                    let (w, a, d) = self.fitted_object(item);
                    visual_boxes.push(PositionedInlineBox {
                        baseline_align: *baseline_align,
                        box_id: *box_id,
                        byte_range: *byte_range,
                        x,
                        y: -shift,
                        width: w,
                        ascent: a,
                        descent: d,
                        overflow_policy: *overflow_policy,
                        bidi_level: *bidi_level,
                    });
                    extend(x, x + w);
                    visible_end = visible_end.max(byte_range.end());
                }
                PieceKind::Tab => {
                    tabs.push(PositionedTab {
                        byte_range: piece.bytes,
                        x,
                        width: widths[k],
                    });
                    extend(x, x + widths[k]);
                    visible_end = visible_end.max(piece.bytes.end());
                }
                PieceKind::Glue | PieceKind::SpanEdge { .. } => {}
            }
        }
        // Boxes and tabs are stored in logical order.
        visual_boxes.sort_by_key(|b| b.byte_range.start());
        tabs.sort_by_key(|t| t.byte_range.start());
        let span_fragments = self.span_fragments(
            pieces,
            order,
            starts,
            widths,
            body_len,
            (body_lo, body_hi),
            end_break,
            post,
            byte_range,
        );
        for f in &span_fragments {
            extend(f.x, f.x + f.width);
        }
        let has_content = lo <= hi;
        let rtl = self.base.is_rtl();
        let (content_lo, content_width) = if !has_content {
            (if rtl { body_hi } else { 0.0 }, 0.0)
        } else if rtl {
            (lo, hi.max(body_hi) - lo)
        } else {
            (0.0, hi)
        };
        // Decorations (Stage 5).
        for kind in [DecorationKind::Underline, DecorationKind::Strikethrough] {
            self.decorations(
                kind,
                pieces,
                order,
                (starts, widths, &shifts),
                body_len,
                &mut decorations,
            )?;
        }
        let atomic_fragments = self.atomic_fragments(pieces, order, starts, widths, byte_range);
        let caret_stops = self.caret_stops(
            index,
            pieces,
            levels,
            order,
            starts,
            widths,
            body_len,
            soft_wrap,
            byte_range,
            visible_end,
            content_lo,
            &visual_runs,
            &visual_boxes,
            &tabs,
        );
        Ok(Placed {
            line: PositionedLine {
                slots: SmallVec::new(),
                is_ellipsized: false,
                annotations: Vec::new(),
                atomic_fragments,
                byte_range,
                visible_byte_end: visible_end,
                baseline_y: 0.0,
                ascent,
                descent,
                line_height,
                is_justified: false,
                visual_runs,
                visual_boxes,
                tabs,
                span_fragments,
                caret_stops,
                decorations,
            },
            content_lo,
            content_width,
        })
    }

    /// One fragment per visual group of each decoration span on the line
    /// (spec ST2-3, FRG-1), padding included; a span cloned across this
    /// line's breaks reaches the line's edge.
    #[expect(clippy::too_many_arguments, reason = "one line's placement state")]
    fn span_fragments(
        &self,
        pieces: &[(Piece, f32)],
        order: &[usize],
        starts: &[f32],
        widths: &[f32],
        body_len: usize,
        (body_lo, body_hi): (f32, f32),
        end_break: Option<&Break>,
        post: f32,
        line_bytes: TextRange,
    ) -> Vec<PositionedSpanFragment> {
        let mut fragments = Vec::new();
        let (ascent, descent) = (self.strut.ascent.max(0.0), self.strut.descent.max(0.0));
        for span in &self.ast.decoration_spans {
            let range = span.byte_range;
            let member = |p: &Piece| match p.kind {
                PieceKind::SpanEdge { span_id } => span_id == span.span_id,
                _ => {
                    !p.bytes.is_empty()
                        && range.start() <= p.bytes.start()
                        && p.bytes.end() <= range.end()
                }
            };
            let cloned_at_end = end_break.is_some_and(|b| b.cloned.contains(&span.span_id));
            let cloned_at_start = post > 0.0
                && range.start() < line_bytes.start()
                && line_bytes.start() < range.end();
            let mut group: Option<(f32, f32, TextRange)> = None;
            let mut flush = |group: &mut Option<(f32, f32, TextRange)>| {
                if let Some((a, b, bytes)) = group.take() {
                    fragments.push(PositionedSpanFragment {
                        span_id: span.span_id,
                        span_range: range,
                        byte_range: bytes,
                        x: a,
                        width: (b - a).max(0.0),
                        ascent,
                        descent,
                        bidi_level: self.base.base_level(),
                    });
                }
            };
            for &k in order.iter().filter(|&&k| k < body_len) {
                let (p, _) = pieces[k];
                // Another span's edge neither joins nor splits this span's
                // fragment: a nested span's padding lies inside it.
                if matches!(p.kind, PieceKind::SpanEdge { span_id } if span_id != span.span_id) {
                    continue;
                }
                if member(&p) {
                    let (a, b) = (starts[k], starts[k] + widths[k]);
                    group = Some(match group {
                        Some((ga, gb, bytes)) => (ga.min(a), gb.max(b), bytes.cover(p.bytes)),
                        None => (a, b, p.bytes),
                    });
                } else {
                    flush(&mut group);
                }
            }
            if let Some((a, b, bytes)) = group.as_mut() {
                if cloned_at_end {
                    *b = b.max(body_hi);
                }
                if cloned_at_start {
                    *a = a.min(body_lo - post).max(0.0_f32.min(*a));
                }
                // A fragment's bytes stay inside the span and the line.
                let start = bytes.start().max(range.start()).max(line_bytes.start());
                let end = bytes
                    .end()
                    .min(range.end())
                    .min(line_bytes.end())
                    .max(start);
                *bytes = TextRange::new(start, end);
            }
            flush(&mut group);
        }
        fragments
    }

    /// One fragment per visual group of each atomic span on the line (spec
    /// FRG-3), together covering exactly the part of the span on the line.
    fn atomic_fragments(
        &self,
        pieces: &[(Piece, f32)],
        order: &[usize],
        starts: &[f32],
        widths: &[f32],
        line_bytes: TextRange,
    ) -> Vec<AtomicFragment> {
        let mut out = Vec::new();
        for span in &self.ast.atomic_spans {
            let full = span.byte_range;
            let part_start = full.start().max(line_bytes.start());
            let part_end = full.end().min(line_bytes.end());
            if part_start >= part_end {
                continue;
            }
            let mut groups: Vec<(f32, f32, TextRange)> = Vec::new();
            let mut open = false;
            for &k in order {
                let (p, _) = pieces[k];
                let inside = !p.bytes.is_empty()
                    && p.bytes.start() < full.end()
                    && p.bytes.end() > full.start();
                if inside {
                    let (a, b) = (starts[k], starts[k] + widths[k]);
                    if open {
                        let g = groups.last_mut().expect("open group");
                        *g = (g.0.min(a), g.1.max(b), g.2.cover(p.bytes));
                    } else {
                        groups.push((a, b, p.bytes));
                        open = true;
                    }
                } else {
                    open = false;
                }
            }
            if groups.is_empty() {
                continue;
            }
            groups.sort_by_key(|g| g.2.start());
            let n = groups.len();
            let mut covered = part_start;
            for (i, (a, b, bytes)) in groups.into_iter().enumerate() {
                let end = if i + 1 == n {
                    part_end
                } else {
                    bytes.end().clamp(covered + 1, part_end)
                };
                if covered >= end {
                    continue;
                }
                out.push(AtomicFragment {
                    atomic_id: span.span_id,
                    full_byte_range: full,
                    fragment_byte_range: TextRange::new(covered, end),
                    x: a.min(b),
                    width: (b - a).abs(),
                    bidi_level: self.base.base_level(),
                });
                covered = end;
            }
        }
        out
    }

    /// Underline or strike-through segments merged across adjacent decorated
    /// clusters and the glue between them (spec ST5-1).
    fn decorations(
        &mut self,
        kind: DecorationKind,
        pieces: &[(Piece, f32)],
        order: &[usize],
        (starts, widths, shifts): (&[f32], &[f32], &[f32]),
        body_len: usize,
        out: &mut Vec<DecorationLine>,
    ) -> Result<(), LayoutInputError> {
        let mut open: Option<DecorationLine> = None;
        let mut pending_glue: Option<(f32, TextRange)> = None;
        for &k in order.iter().filter(|&&k| k < body_len) {
            let (p, _) = pieces[k];
            let decorated = match p.kind {
                PieceKind::Cluster { run, .. } => {
                    let d = self.ast.runs[run].decoration;
                    match kind {
                        DecorationKind::Underline => d.underline,
                        DecorationKind::Strikethrough => d.strikethrough,
                    }
                }
                PieceKind::Glue => {
                    if open.is_some() {
                        let end = starts[k] + widths[k];
                        pending_glue = Some(match pending_glue {
                            Some((_, bytes)) => (end, bytes.cover(p.bytes)),
                            None => (end, p.bytes),
                        });
                    }
                    continue;
                }
                _ => false,
            };
            if !decorated {
                out.extend(open.take());
                pending_glue = None;
                continue;
            }
            let PieceKind::Cluster { run, cluster } = p.kind else {
                unreachable!("only clusters are decorated")
            };
            let m = self.cluster_metrics(run, cluster)?;
            let shift = shifts[k];
            let (position, thickness) = match kind {
                DecorationKind::Underline => (m.underline_position, m.underline_thickness),
                DecorationKind::Strikethrough => (m.strikeout_position, m.underline_thickness),
            };
            let (a, b) = (starts[k], starts[k] + widths[k]);
            match open.as_mut() {
                Some(line) => {
                    if let Some((_, bytes)) = pending_glue.take() {
                        line.byte_range = line.byte_range.cover(bytes);
                    }
                    line.width = b.max(line.x + line.width) - line.x;
                    line.byte_range = line.byte_range.cover(p.bytes);
                    // The lowest underline of the fonts it crosses (ST5-1).
                    if kind == DecorationKind::Underline && position + shift < line.y_offset {
                        line.y_offset = position + shift;
                        line.thickness = thickness;
                    }
                }
                None => {
                    open = Some(DecorationLine {
                        kind,
                        byte_range: p.bytes,
                        x: a,
                        width: b - a,
                        y_offset: position + shift,
                        thickness,
                    });
                }
            }
        }
        out.extend(open);
        Ok(())
    }

    /// The line's caret stops in visual order (spec §6.1, CRT-1, CRT-2,
    /// INT-4 to INT-6, INT-9).
    #[expect(clippy::too_many_arguments, reason = "one line's placement state")]
    fn caret_stops(
        &self,
        index: usize,
        pieces: &[(Piece, f32)],
        levels: &[BidiLevel],
        order: &[usize],
        starts: &[f32],
        widths: &[f32],
        body_len: usize,
        soft_wrap: bool,
        byte_range: TextRange,
        visible_end: usize,
        content_lo: f32,
        runs: &[VisualGlyphRun],
        boxes: &[PositionedInlineBox],
        tabs: &[PositionedTab],
    ) -> Vec<CaretStop> {
        let base_level = self.base.base_level();
        // The level of an entity touching `offset`, else the paragraph's.
        let level_at = |offset: usize| -> BidiLevel {
            runs.iter()
                .find(|r| {
                    r.glyphs.iter().any(|g| {
                        g.byte_range
                            .is_some_and(|b| b.start() <= offset && offset <= b.end())
                    })
                })
                .map(|r| r.bidi_level)
                .or_else(|| {
                    boxes
                        .iter()
                        .find(|b| b.byte_range.start() <= offset && offset <= b.byte_range.end())
                        .map(|b| b.bidi_level)
                })
                .or_else(|| {
                    tabs.iter()
                        .any(|t| t.byte_range.start() <= offset && offset <= t.byte_range.end())
                        .then_some(base_level)
                })
                .unwrap_or(base_level)
        };
        let mut raw: Vec<(f32, usize)> = Vec::new();
        let collapsible = self.options.whitespace == HangingWhitespaceMode::Collapsible;
        if byte_range.is_empty() || body_len == 0 && (pieces.is_empty() || collapsible && soft_wrap)
        {
            raw.push((content_lo, byte_range.start()));
        }
        let push_piece = |k: usize, raw: &mut Vec<(f32, usize)>| {
            let (p, _) = pieces[k];
            let (a, b) = (starts[k], starts[k] + widths[k]);
            let rtl = levels[k].direction().is_rtl();
            let (left, right) = if rtl {
                (p.bytes.end(), p.bytes.start())
            } else {
                (p.bytes.start(), p.bytes.end())
            };
            raw.push((a, left));
            if let PieceKind::Cluster { run, cluster } = p.kind {
                let c = &self.ast.clusters[cluster];
                let k_atoms = c.atom_count();
                if k_atoms > 1 && !c.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE) {
                    let atoms = &self.inputs.run_atoms(run)[c.atom_range.clone()];
                    let first_glyph = self
                        .inputs
                        .cluster_glyphs(c)
                        .first()
                        .map_or(0, |g| g.glyph_id);
                    let carets = self.shaper.ligature_carets(
                        FontInstanceRef {
                            font: c.font,
                            font_size: self.ast.runs[run].font_size,
                            variations: self.ast.runs[run].features.variations(),
                        },
                        first_glyph,
                        levels[k].direction(),
                    );
                    let width = c.natural_width;
                    let positions = resolve_ligature_carets(width, k_atoms, &carets);
                    let mut inner: Vec<(f32, usize)> = positions
                        .iter()
                        .zip(atoms.iter().skip(1))
                        .map(|(&pos, atom)| {
                            let x = if rtl { a + width - pos } else { a + pos };
                            (x, atom.byte_range.start())
                        })
                        .collect();
                    if rtl {
                        inner.reverse();
                    }
                    raw.extend(inner);
                }
            }
            raw.push((b, right));
        };
        for &k in order {
            push_piece(k, &mut raw);
        }
        let trailing_stops = !collapsible || !soft_wrap;
        if trailing_stops && body_len < pieces.len() {
            // Beyond the far edge of the base direction (HNG-3, INT-6). Under
            // `Collapsible` they all sit at the edge, so that the end of a
            // hard-broken line or of the paragraph stays reachable.
            let trailing: Vec<usize> = (body_len..pieces.len()).collect();
            if self.base.is_rtl() {
                for &k in trailing.iter().rev() {
                    push_piece(k, &mut raw);
                }
            } else {
                for &k in &trailing {
                    push_piece(k, &mut raw);
                }
            }
        }
        // Visual order, one stop per distinct (x, offset).
        let mut stops: Vec<CaretStop> = Vec::new();
        for (x, offset) in raw {
            if !(byte_range.start() <= offset && offset <= byte_range.end()) {
                continue;
            }
            if self
                .ast
                .atomic_span_containing(offset)
                .is_some_and(|span| span.is_interior(offset))
            {
                continue;
            }
            // Offsets past the visible end of a soft-wrapped collapsible line
            // belong to the next line (INT-6).
            if collapsible && soft_wrap && offset > visible_end && offset != byte_range.start() {
                continue;
            }
            let duplicate = stops.iter().rev().take(4).any(|s| {
                s.offset == offset && (s.x - x).abs() <= super::tolerance::tolerance(s.x, x)
            });
            if duplicate {
                continue;
            }
            stops.push(CaretStop {
                x,
                offset,
                affinity: CaretAffinity::Downstream,
                level: level_at(offset),
                line: index,
            });
        }
        // A hard break's own bytes start the next line.
        if !soft_wrap && !byte_range.is_empty() {
            let hard_end = byte_range.end();
            let is_terminated = self.text[byte_range.as_range()].ends_with('\u{2028}');
            if is_terminated {
                stops.retain(|s| s.offset != hard_end);
            }
        }
        stops.sort_by(|a, b| a.x.total_cmp(&b.x));
        // At a soft wrap the line's last logical stop is upstream (INT-1, INT-9).
        if soft_wrap && let Some(last) = stops.iter_mut().max_by_key(|s| s.offset) {
            last.affinity = CaretAffinity::Upstream;
        }
        stops
    }
}

/// Visual indices of a line's pieces from their levels (UAX #9 L2): from
/// the highest level to the lowest odd one, every maximal sequence at that
/// level or above is reversed.
fn visual_order(levels: &[BidiLevel]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let Some(highest) = levels.iter().map(|l| l.get()).max() else {
        return order;
    };
    let lowest_odd = levels
        .iter()
        .map(|l| l.get())
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(highest + 1);
    let mut level = highest;
    while level >= lowest_odd && level > 0 {
        let mut i = 0;
        while i < order.len() {
            if levels[order[i]].get() >= level {
                let mut j = i;
                while j < order.len() && levels[order[j]].get() >= level {
                    j += 1;
                }
                order[i..j].reverse();
                i = j;
            } else {
                i += 1;
            }
        }
        level -= 1;
    }
    order
}

/// Whether an alignment is placed against the finished line box.
fn is_line_relative(align: BaselineAlignment) -> bool {
    matches!(
        align,
        BaselineAlignment::LineTop | BaselineAlignment::LineBottom
    )
}

/// A font's offset of a named baseline from its alphabetic one, positive
/// towards line-over (FNT-4); `None` for the other alignments.
fn baseline_offset(align: BaselineAlignment, m: &FontMetrics) -> Option<f32> {
    match align {
        BaselineAlignment::Auto | BaselineAlignment::Alphabetic => Some(0.0),
        BaselineAlignment::Ideographic => Some(m.ideographic_baseline_offset),
        BaselineAlignment::Central => Some(m.central_baseline_offset),
        BaselineAlignment::Hanging => Some(m.hanging_baseline_offset),
        BaselineAlignment::Mathematical => Some(m.mathematical_baseline_offset),
        BaselineAlignment::LineTop
        | BaselineAlignment::LineBottom
        | BaselineAlignment::Offset(_) => None,
    }
}

/// The shift of a run towards line-over (spec BAS-1): its font's named
/// baseline onto the strut's, or an explicit offset.
fn run_shift(align: BaselineAlignment, strut: &FontMetrics, own: &FontMetrics) -> f32 {
    match align {
        BaselineAlignment::Offset(offset) => offset,
        _ => match (baseline_offset(align, strut), baseline_offset(align, own)) {
            (Some(s), Some(o)) => s - o,
            _ => 0.0,
        },
    }
}

/// The shift of an object's baseline towards line-over (spec BAS-1): to
/// the strut's named baseline, or an explicit offset.
fn object_shift(align: BaselineAlignment, strut: &FontMetrics) -> f32 {
    match align {
        BaselineAlignment::Offset(offset) => offset,
        _ => baseline_offset(align, strut).unwrap_or(0.0),
    }
}

/// Moves every inline coordinate of a line by `dx`.
fn shift_line(line: &mut PositionedLine, dx: f32) {
    for run in &mut line.visual_runs {
        for g in &mut run.glyphs {
            g.x += dx;
        }
    }
    for b in &mut line.visual_boxes {
        b.x += dx;
    }
    for t in &mut line.tabs {
        t.x += dx;
    }
    for f in &mut line.span_fragments {
        f.x += dx;
    }
    for f in &mut line.atomic_fragments {
        f.x += dx;
    }
    for c in &mut line.caret_stops {
        c.x += dx;
    }
    for d in &mut line.decorations {
        d.x += dx;
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
            ladder_tier(&d)
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
        assert_eq!(ladder_tier(&mandatory), 0);
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
        let l = |v: u8| BidiLevel::new(v).unwrap();
        assert_eq!(visual_order(&[l(0), l(0)]), [0, 1]);
        assert_eq!(visual_order(&[l(1), l(1), l(1)]), [2, 1, 0]);
        assert_eq!(visual_order(&[l(0), l(1), l(1), l(0)]), [0, 2, 1, 3]);
        assert_eq!(visual_order(&[l(1), l(2), l(2), l(1)]), [3, 1, 2, 0]);
    }
}
