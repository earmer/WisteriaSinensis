//! Mid-end layout entry point (spec §5).

mod breaking;
mod clamp;
mod hanging;
mod justify;
mod line_artifacts;
mod line_box;
mod line_builder;
mod scratch;
mod trim;
mod visual_runs;

use crate::frontend::{
    AnnotationSide, BoxNode, DiscretionaryNode, GlueNode, InlineObjectBox, ParagraphAst, SpanEdge,
    WordNode,
};
use crate::middleend::options::BreakingParameters;
use crate::middleend::shaper_gate::checked_line_box;
use crate::middleend::validated::{ClusterId, RunId, SentinelTokens, SpanId, TokenIdx};
use crate::middleend::{
    Clearance, GeometryAnswer, LayoutConstraints, LayoutInputError, LayoutOptions, LineHeight,
    OpenGeometry, PositionedLine, PositionedParagraph, StrutMetrics, ValidatedInputs,
};
use crate::shaping::TextShaper;
use crate::types::{BidiDirection, BidiLevel, FontMetrics, RawShapedGlyph, TextRange};
use breaking::within;
use scratch::LayoutScratch;
use std::collections::HashMap;

const MAX_CLEARANCE_PROBES: usize = 1024;
const MAX_ENVELOPE_REFITS: usize = 16;

/// Lays out one paragraph under `options.strategy` (spec §5, ST1-6).
pub fn layout_paragraph(
    ast: &ParagraphAst,
    text: &str,
    shaper: &dyn TextShaper,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
) -> Result<PositionedParagraph, LayoutInputError> {
    let parameters = options.strategy.parameters();
    break_lines_parameterized(
        ast,
        text,
        shaper,
        constraints,
        options,
        parameters.beam_width,
        parameters.window_depth,
    )
}

/// Runs the parameterised line breaker with `beam_width` and `window_depth` (spec ST1-6).
pub fn break_lines_parameterized(
    ast: &ParagraphAst,
    text: &str,
    shaper: &dyn TextShaper,
    constraints: &LayoutConstraints,
    options: &LayoutOptions,
    beam_width: usize,
    window_depth: usize,
) -> Result<PositionedParagraph, LayoutInputError> {
    let inputs = ValidatedInputs::try_from_ast(ast, text, constraints, options)?;
    if constraints.geometry_provider.is_some() {
        probe_first_line(&inputs, shaper)?;
    }
    let parameters = BreakingParameters {
        beam_width: beam_width.max(1),
        window_depth: window_depth.max(1),
    };
    Layout::new(&inputs, shaper, parameters).run()
}

fn advance_clearance(
    obstructed: &mut usize,
    clearance: Clearance,
) -> Result<f64, LayoutInputError> {
    *obstructed += 1;
    if *obstructed > MAX_CLEARANCE_PROBES {
        return Err(LayoutInputError::ClearanceExhausted);
    }
    Ok(f64::from(clearance.clear_at()))
}

fn probe_first_line(
    inputs: &ValidatedInputs<'_>,
    shaper: &dyn TextShaper,
) -> Result<(), LayoutInputError> {
    let strut_height = inputs.policy().strut.line_height();
    let tallest = tallest_first_line(inputs, shaper, strut_height)?;
    let mut block_start = 0.0;
    let mut obstructed = 0;
    loop {
        let probe = inputs.line_geometry(0, block_start, strut_height, None)?;
        if tallest > strut_height {
            inputs.line_geometry(0, block_start, tallest, Some(&probe.geometry()))?;
        }
        let GeometryAnswer::Obstructed(clearance) = probe else {
            return Ok(());
        };
        block_start = advance_clearance(&mut obstructed, clearance)? as f32;
    }
}

fn tallest_first_line(
    inputs: &ValidatedInputs<'_>,
    shaper: &dyn TextShaper,
    strut_height: f32,
) -> Result<f32, LayoutInputError> {
    let ast = inputs.ast();
    let options = inputs.options();
    if matches!(options.line_height, LineHeight::Fixed(_)) {
        return Ok(strut_height);
    }
    let (before, first_break) = inputs.before_first_break();
    let content_end = first_break.byte_range.start();
    let mut content_height = strut_height;
    for node in before {
        match node {
            WordNode::Box(BoxNode::InlineObject(InlineObjectBox {
                ascent, descent, ..
            })) => {
                content_height = content_height.max(ascent + descent);
            }
            WordNode::Box(BoxNode::Text(text)) => {
                let (_, run, view) = inputs.resolve_text(text);
                for cluster in view.clusters {
                    let instance = run.font_instance(cluster.font);
                    let line_box =
                        checked_line_box(shaper, instance, options.line_height, ast.writing_mode)?;
                    content_height = content_height.max(line_box.line_height());
                }
            }
            _ => {}
        }
    }
    let (mut over, mut under) = (0.0_f32, 0.0_f32);
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
    Ok(content_height + over + under)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum PieceKind<'a> {
    Cluster { run: RunId, cluster: ClusterId },
    Object(&'a InlineObjectBox),
    Tab,
    Glue(&'a GlueNode),
    SpanEdge { span: SpanId, edge: SpanEdge },
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Piece<'a> {
    pub(super) kind: PieceKind<'a>,
    pub(super) bytes: TextRange,
    pub(super) level: BidiLevel,
    pub(super) width: f64,
}

impl Piece<'_> {
    pub(super) fn is_content(&self) -> bool {
        !matches!(self.kind, PieceKind::Glue(_)) && !self.is_edge_transparent()
    }

    pub(super) fn is_edge_transparent(&self) -> bool {
        matches!(self.kind, PieceKind::SpanEdge { .. }) && self.width == 0.0
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Token<'a> {
    Piece(Piece<'a>),
    Break(&'a DiscretionaryNode),
}

impl Token<'_> {
    pub(super) fn bytes(&self) -> TextRange {
        match self {
            Self::Piece(piece) => piece.bytes,
            Self::Break(brk) => brk.byte_range,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum LineEnd<'a> {
    At {
        token: TokenIdx,
        brk: &'a DiscretionaryNode,
    },
    Before(TokenIdx),
}

#[derive(Clone, Debug)]
pub(super) struct Reshaped {
    pub(super) glyphs: Vec<RawShapedGlyph>,
    pub(super) width: f64,
    pub(super) wdth: Option<f32>,
}

pub(super) type Reshapes = HashMap<ClusterId, Reshaped>;

pub(super) struct Layout<'i, 'a> {
    pub(super) inputs: &'i ValidatedInputs<'a>,
    pub(super) ast: &'a ParagraphAst,
    pub(super) shaper: &'i dyn TextShaper,
    pub(super) options: &'a LayoutOptions,
    pub(super) parameters: BreakingParameters,
    pub(super) tokens: SentinelTokens<Token<'a>>,
    span_reach: Vec<usize>,
    pub(super) strut: StrutMetrics,
    pub(super) font_scale: f64,
    pub(super) effective_wrap_width: Option<f32>,
    pub(super) base: BidiDirection,
    pub(super) metrics: HashMap<(u32, u32, RunId), FontMetrics>,
    pub(super) reshaped: Reshapes,
    pub(super) has_underline: bool,
    pub(super) has_strikethrough: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PlacedPiece<'a> {
    pub(super) piece: Piece<'a>,
    pub(super) x: f64,
    pub(super) width: f64,
    pub(super) expansion: f64,
    pub(super) y_shift: f64,
    pub(super) lead_trim: f64,
    pub(super) trail_trim: f64,
}

#[derive(Clone, Copy)]
pub(super) struct LineCursor<'a> {
    pub(super) token_start: TokenIdx,
    pub(super) start_break: Option<&'a DiscretionaryNode>,
    pub(super) byte_start: usize,
    pub(super) block_top: f64,
    pub(super) index: usize,
}

impl<'a> LineCursor<'a> {
    fn paragraph_start() -> Self {
        Self {
            token_start: TokenIdx::START,
            start_break: None,
            byte_start: 0,
            block_top: 0.0,
            index: 0,
        }
    }

    pub(super) fn first_line(&self) -> bool {
        self.index == 0
    }

    pub(super) fn is_soft_wrapped_start(&self) -> bool {
        self.index > 0 && self.start_break.is_none_or(|b| !b.is_mandatory())
    }

    pub(super) fn post_break_width(&self) -> f64 {
        self.start_break
            .map_or(0.0, |b| f64::from(b.post_break_width))
    }

    fn advance(&mut self, end: LineEnd<'a>, tokens: &SentinelTokens<Token<'_>>, line_height: f64) {
        self.block_top += line_height;
        self.index += 1;
        match end {
            LineEnd::At { token, brk } => {
                self.start_break = Some(brk);
                self.byte_start = brk.byte_range.end();
                self.token_start = token.next();
            }
            LineEnd::Before(token) => {
                self.start_break = None;
                self.byte_start = tokens.at_or_sentinel(token).bytes().start();
                self.token_start = token;
            }
        }
    }

    fn is_paragraph_end(&self, tokens: &SentinelTokens<Token<'_>>) -> bool {
        tokens.is_exhausted(self.token_start)
    }
}

impl<'i, 'a> Layout<'i, 'a> {
    fn new(
        inputs: &'i ValidatedInputs<'a>,
        shaper: &'i dyn TextShaper,
        parameters: BreakingParameters,
    ) -> Self {
        let ast = inputs.ast();
        let mut span_reach = Vec::with_capacity(ast.decoration_spans.len());
        let mut reach = 0;
        for s in &ast.decoration_spans {
            reach = reach.max(s.byte_range.end());
            span_reach.push(reach);
        }
        Self {
            inputs,
            ast,
            shaper,
            options: inputs.options(),
            parameters,
            tokens: tokenize(inputs),
            span_reach,
            strut: inputs.policy().strut,
            font_scale: 1.0,
            effective_wrap_width: None,
            base: ast.base_direction,
            metrics: HashMap::new(),
            reshaped: HashMap::new(),
            has_underline: ast.runs.iter().any(|r| r.decoration.underline),
            has_strikethrough: ast.runs.iter().any(|r| r.decoration.strikethrough),
        }
    }

    pub(super) fn spans_touching(&self, range: TextRange) -> impl Iterator<Item = SpanId> {
        let lo = self.span_reach.partition_point(|&end| end < range.start());
        let hi = self
            .ast
            .decoration_spans
            .partition_point(|s| s.byte_range.start() <= range.end());
        self.inputs.span_ids_in(lo..hi.max(lo))
    }

    fn run(mut self) -> Result<PositionedParagraph, LayoutInputError> {
        self.apply_shape_fit();
        let (mut lines, total_block_size, truncated_range) = self.place_lines()?;
        backfill_span_ribbons(&mut lines);
        let mut policy = self.inputs.policy().clone();
        policy.strut = self.strut;
        if let Some(w) = self.effective_wrap_width {
            policy.wrap_width = w;
        }
        Ok(PositionedParagraph {
            policy,
            base_direction: self.base,
            total_block_size,
            lines,
            truncated_range,
        })
    }

    fn apply_shape_fit(&mut self) {
        let Some(sf) = self.options.shape_fit else {
            return;
        };
        let (h1, c_min) = (
            f64::from(self.strut.line_height()),
            f64::from(self.options.min_horizontal_scale),
        );
        let (w_max, s_lo, s_hi0) = (
            f64::from(sf.max_width),
            f64::from(sf.min_font_scale),
            f64::from(sf.max_font_scale),
        );
        let total_w: f64 = self
            .tokens
            .iter_from(TokenIdx::START)
            .map(|(_, t)| match t {
                Token::Piece(p) => p.width,
                Token::Break(_) => 0.0,
            })
            .sum();
        let mut best: Option<(f64, f64, f64)> = None;
        for k in 1..=sf.max_lines {
            let (target, mut lines_fu) = (total_w / k as f64, [(0.0, 0.0); 16]);
            let (mut li, mut f_acc, mut u_acc, mut cw) = (0usize, 0.0, 0.0, 0.0);
            for (_, tok) in self.tokens.iter_from(TokenIdx::START) {
                match tok {
                    Token::Piece(p) => {
                        match p.kind {
                            PieceKind::Cluster { .. } | PieceKind::Glue(_) => f_acc += p.width,
                            PieceKind::Object(_) | PieceKind::SpanEdge { .. } => u_acc += p.width,
                            PieceKind::Tab => {}
                        }
                        cw += p.width;
                    }
                    Token::Break(b)
                        if !b.is_paragraph_end()
                            && (b.is_mandatory() || (cw >= target && li + 1 < k)) =>
                    {
                        if let Some(s) = lines_fu.get_mut(li) {
                            *s = (f_acc, u_acc);
                        }
                        li = (li + 1).min(k - 1);
                        (f_acc, u_acc, cw) = (0.0, 0.0, 0.0);
                    }
                    Token::Break(_) => {}
                }
            }
            if let Some(s) = lines_fu.get_mut(li) {
                *s = (f_acc, u_acc);
            }
            let uk = (li + 1).max(1) as f64;
            let mut s_hi = sf
                .max_height
                .map_or(s_hi0, |h| s_hi0.min(f64::from(h) / (uk * h1)));
            for &(fj, uj) in lines_fu.iter().take(li + 1) {
                if fj > 0.0 {
                    s_hi = s_hi.min((w_max - uj).max(0.0) / (c_min * fj));
                }
            }
            let s = sf
                .target_aspect_ratio
                .map_or(s_hi, |r| s_hi.min(w_max / (f64::from(r) * uk * h1)))
                .clamp(s_lo, s_hi0);
            let w_min = lines_fu
                .iter()
                .take(li + 1)
                .map(|&(fj, uj)| uj + c_min * s * fj)
                .fold(0.0_f64, f64::max);
            let w = sf.target_aspect_ratio.map_or(w_max, |r| {
                (f64::from(r) * uk * h1 * s).clamp(w_min.min(w_max), w_max)
            });
            let dist = lines_fu
                .iter()
                .take(li + 1)
                .map(|&(fj, uj)| {
                    if fj <= 0.0 {
                        0.0
                    } else {
                        (1.0 - ((w - uj) / (s * fj)).min(1.0)).powi(2)
                    }
                })
                .fold(0.0_f64, f64::max);
            let infeas = (w_min - w).max(0.0) * 1e6;
            let score = sf.target_aspect_ratio.map_or(-s + dist * 0.1, |r| {
                ((w / (uk * h1 * s).max(1e-9) - f64::from(r)) / f64::from(r)).abs() + dist
            }) + infeas;
            if best.is_none_or(|(b, _, _)| score < b) {
                best = Some((score, s, w));
            }
        }
        if let Some((_, s, w)) = best {
            self.font_scale = s;
            self.effective_wrap_width = Some(w.max(1e-3) as f32);
            self.strut = StrutMetrics {
                ascent: (f64::from(self.strut.ascent) * s) as f32,
                descent: (f64::from(self.strut.descent) * s) as f32,
                line_gap: (f64::from(self.strut.line_gap) * s) as f32,
            };
        }
    }

    fn place_lines(
        &mut self,
    ) -> Result<(Vec<PositionedLine>, f32, Option<TextRange>), LayoutInputError> {
        let clamp = self.inputs.policy().clamp.as_ref();
        let mut retained: Vec<PlacedNextLine<'a>> = Vec::new();
        let mut cursor = LineCursor::paragraph_start();
        let mut scratch = LayoutScratch::default();
        loop {
            let mut line_cursor = cursor;
            let placed = self.place_next_line(&mut line_cursor, &mut scratch)?;
            let over_block = clamp
                .and_then(|c| c.max_block_size)
                .is_some_and(|max| line_cursor.block_top + placed.line_height > f64::from(max));
            if over_block && !retained.is_empty() {
                break;
            }
            self.reshaped.extend(placed.reshapes);
            cursor = line_cursor;
            cursor.advance(placed.end, &self.tokens, placed.line_height);
            retained.push(PlacedNextLine {
                cursor: line_cursor,
                reshapes: Reshapes::new(),
                ..placed
            });
            if cursor.is_paragraph_end(&self.tokens) {
                let lines = retained.into_iter().map(|r| r.line).collect();
                return Ok((lines, cursor.block_top as f32, None));
            }
            if over_block
                || clamp
                    .and_then(|c| c.max_lines)
                    .is_some_and(|m| retained.len() >= m)
            {
                break;
            }
        }
        let ellipsis = clamp.map_or("", |c| c.ellipsis.as_ref());
        let mut obstructed = 0;
        while let Some(mut last) = retained.pop() {
            if let Some((clamped, clamped_height, _)) = self.refit_line(
                &mut last.cursor,
                last.geometry,
                last.line_height,
                &mut obstructed,
                |this, c, g| this.clamp_line(c, last.end, ellipsis, g, &mut scratch),
            )? {
                let end_top = last.cursor.block_top + clamped_height;
                let tail = TextRange::new(clamped.byte_range.end(), self.inputs.text().len());
                let mut lines: Vec<_> = retained.into_iter().map(|r| r.line).collect();
                lines.push(clamped);
                return Ok((lines, end_top as f32, Some(tail)));
            }
        }
        Err(LayoutInputError::RefitExhausted)
    }

    pub(super) fn open_geometry(
        &self,
        cursor: &mut LineCursor<'a>,
        height: f64,
        obstructed: &mut usize,
    ) -> Result<OpenGeometry, LayoutInputError> {
        if let Some(w) = self.effective_wrap_width
            && self.inputs.constraints().geometry_provider.is_none()
            && let Some(slot) = self
                .inputs
                .policy()
                .slot_source
                .standard_slot(cursor.index, w)
        {
            return Ok(OpenGeometry::single(slot));
        }
        loop {
            match self.inputs.line_geometry(
                cursor.index,
                cursor.block_top as f32,
                height as f32,
                None,
            )? {
                GeometryAnswer::Open(geometry) => return Ok(geometry),
                GeometryAnswer::Obstructed(clearance) => {
                    cursor.block_top = advance_clearance(obstructed, clearance)?;
                }
            }
        }
    }

    fn refit_line<T>(
        &mut self,
        cursor: &mut LineCursor<'a>,
        mut geometry: OpenGeometry,
        mut envelope: f64,
        obstructed: &mut usize,
        mut attempt: impl FnMut(
            &mut Self,
            &LineCursor<'a>,
            &OpenGeometry,
        ) -> Result<Option<(T, f64)>, LayoutInputError>,
    ) -> Result<Option<(T, f64, OpenGeometry)>, LayoutInputError> {
        let has_provider = self.inputs.constraints().geometry_provider.is_some();
        for _ in 0..MAX_ENVELOPE_REFITS {
            let Some((value, height)) = attempt(self, cursor, &geometry)? else {
                return Ok(None);
            };
            if within(height, envelope) || !has_provider {
                return Ok(Some((value, height, geometry)));
            }
            envelope = height;
            match self.inputs.line_geometry(
                cursor.index,
                cursor.block_top as f32,
                envelope as f32,
                Some(geometry.geometry()),
            )? {
                GeometryAnswer::Open(open) if open == geometry => {
                    return Ok(Some((value, height, geometry)));
                }
                GeometryAnswer::Open(open) => geometry = open,
                GeometryAnswer::Obstructed(clearance) => {
                    cursor.block_top = advance_clearance(obstructed, clearance)?;
                    geometry = self.open_geometry(cursor, envelope, obstructed)?;
                }
            }
        }
        Err(LayoutInputError::RefitExhausted)
    }

    fn place_next_line(
        &mut self,
        cursor: &mut LineCursor<'a>,
        scratch: &mut LayoutScratch<'a>,
    ) -> Result<PlacedNextLine<'a>, LayoutInputError> {
        let strut_height = self.strut_height();
        let mut obstructed = 0;
        let geometry = self.open_geometry(cursor, strut_height, &mut obstructed)?;
        let Some(((line, end, reshapes), line_height, geometry)) = self.refit_line(
            cursor,
            geometry,
            strut_height,
            &mut obstructed,
            |this, c, g| {
                let (end, reshapes) = this.break_line(c, g.first_slot())?;
                let (line, h) = this.place(c, end, g, &reshapes, scratch, None)?;
                Ok(Some(((line, end, reshapes), h)))
            },
        )?
        else {
            return Err(LayoutInputError::RefitExhausted);
        };
        Ok(PlacedNextLine {
            line,
            cursor: *cursor,
            end,
            geometry,
            line_height,
            reshapes,
        })
    }
}

struct PlacedNextLine<'a> {
    line: PositionedLine,
    cursor: LineCursor<'a>,
    end: LineEnd<'a>,
    geometry: OpenGeometry,
    line_height: f64,
    reshapes: Reshapes,
}

fn tokenize<'a>(inputs: &ValidatedInputs<'a>) -> SentinelTokens<Token<'a>> {
    let ast = inputs.ast();
    let (body_items, sentinel) = inputs.split_paragraph_end();
    let mut tokens = Vec::with_capacity(ast.items.len() + ast.clusters.len());
    for item in body_items {
        match item {
            WordNode::Box(BoxNode::Text(text)) => {
                let (run, _, _) = inputs.resolve_text(text);
                tokens.extend(inputs.text_clusters(text).map(|(cluster, info)| {
                    Token::Piece(Piece {
                        kind: PieceKind::Cluster { run, cluster },
                        bytes: info.byte_range,
                        level: info.bidi_level,
                        width: f64::from(info.natural_width),
                    })
                }));
            }
            WordNode::Box(BoxNode::InlineObject(object)) => tokens.push(Token::Piece(Piece {
                kind: PieceKind::Object(object),
                bytes: object.byte_range,
                level: object.bidi_level,
                width: f64::from(object.width),
            })),
            WordNode::Box(BoxNode::SpanBoundary {
                byte_range,
                span_id,
                edge,
                width,
                bidi_level,
            }) => tokens.push(Token::Piece(Piece {
                kind: PieceKind::SpanEdge {
                    span: inputs.boundary_span_id(*span_id),
                    edge: *edge,
                },
                bytes: *byte_range,
                level: *bidi_level,
                width: f64::from(*width),
            })),
            WordNode::Glue(glue) => tokens.push(Token::Piece(Piece {
                kind: PieceKind::Glue(glue),
                bytes: glue.byte_range,
                level: glue.bidi_level,
                width: f64::from(glue.natural_width),
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
    SentinelTokens::from_body_and_sentinel(tokens, Token::Break(sentinel))
}

fn backfill_span_ribbons(lines: &mut [PositionedLine]) {
    let mut totals: HashMap<u64, f64> = HashMap::new();
    for line in lines.iter_mut() {
        if line.span_fragments.is_empty() {
            continue;
        }
        let mut ordered: Vec<&mut crate::middleend::PositionedSpanFragment> =
            line.span_fragments.iter_mut().collect();
        ordered.sort_by_key(|f| (f.byte_range.start(), f.byte_range.end()));
        for f in ordered {
            let acc = totals.entry(f.span_id).or_insert(0.0);
            f.ribbon_offset = *acc as f32;
            *acc += f64::from(f.width);
        }
    }
    if totals.is_empty() {
        return;
    }
    for line in lines.iter_mut() {
        for f in &mut line.span_fragments {
            if let Some(&total) = totals.get(&f.span_id) {
                f.total_span_advance = total as f32;
            }
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
        let piece = |v: u8| scratch::LogicalPiece {
            piece: Piece {
                kind: PieceKind::Tab,
                bytes: TextRange::empty_at(0),
                level: BidiLevel::try_new(v).unwrap(),
                width: 0.0,
            },
            gap: 0.0,
        };
        let order = |levels: &[u8]| {
            let pieces: Vec<_> = levels.iter().copied().map(piece).collect();
            line_builder::visual_order(&pieces, &mut Vec::new())
        };
        assert_eq!(order(&[0, 0]), [0, 1]);
        assert_eq!(order(&[1, 1, 1]), [2, 1, 0]);
        assert_eq!(order(&[0, 1, 1, 0]), [0, 2, 1, 3]);
        assert_eq!(order(&[1, 2, 2, 1]), [3, 1, 2, 0]);
    }

    #[test]
    fn layout_piece_and_token_sizes_stay_compact() {
        // 8-byte typed IDs keep `PieceKind` at 16B, `Piece`/`Token` at 48B
        // (with `f64` width), and `PlacedPiece` at 96B (six `f64` fields).
        assert_eq!(core::mem::size_of::<Token<'_>>(), 48);
        assert_eq!(core::mem::size_of::<PlacedPiece<'_>>(), 96);
    }
}
