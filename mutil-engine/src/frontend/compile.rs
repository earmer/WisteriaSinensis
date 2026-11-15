//! The frontend entry point (spec §3, FE-25 to FE-28).
//!
//! The Phase 1 analysis pass: horizontal, unidirectional text (spec RM-11).

use super::{
    BoxNode, DiscretionaryNode, FrontendOptions, GlueKind, GlueNode, InlineObjectInput,
    ParagraphAst, ParagraphInput, TextStyle, WordNode, is_paragraph_separator, is_word_space,
    script, segment,
};
use crate::shaping::{LeadingEdit, TextShaper, TrailingEdit};
use crate::types::{
    AtomFlags, BaselineAlignment, BidiClass, BidiDirection, BidiLevel, ByteIndex, FontInstanceRef,
    FontRef, GlyphOrientation, GraphemeAtom, LineBreakClass, LocaleTag, RawShapedGlyph, ScriptTag,
    ShapedCluster, ShapingRun, TextRange, WritingMode, is_extent, is_measurement,
};
use core::fmt;

/// Why [`compile_paragraph`] refused its input (spec FE-28). Each variant has
/// one root cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrontendError {
    /// The text holds a paragraph separator (spec AST-2); split the document
    /// with `split_paragraphs` first.
    ParagraphSeparatorInText {
        /// Offset of the first separator.
        byte: ByteIndex,
    },
    /// The text is longer than `u32::MAX` bytes, the range of a shaping
    /// cluster value.
    TextTooLong,
    /// A style's font size is not finite and positive, or one of its spacings
    /// is not a measurement.
    InvalidStyle {
        /// Index into `ParagraphInput::style_spans`; `None` for the default
        /// style.
        span: Option<usize>,
    },
    /// A style span is empty, outside the text, off a character boundary, out
    /// of order or overlaps its predecessor.
    InvalidStyleSpan {
        /// Index into `ParagraphInput::style_spans`.
        span: usize,
    },
    /// An inline object's range is empty, outside the text, off a grapheme
    /// boundary, out of order or overlapping, or one of its sizes is not a
    /// non-negative measurement.
    InvalidInlineObject {
        /// Index into `ParagraphInput::inline_objects`.
        object: usize,
    },
    /// The input asks for something this implementation does not compile yet.
    Unsupported {
        /// What was asked for.
        feature: FrontendFeature,
    },
    /// The shaper's answer cannot be used: its clusters do not partition the
    /// atoms of a run in order (spec SHP-4, SHP-5).
    ShaperContract {
        /// Index of the run in compilation order.
        run: usize,
    },
}

/// A feature [`compile_paragraph`] rejects until it is implemented
/// (spec FE-28).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FrontendFeature {
    /// A vertical `FrontendOptions::writing_mode`.
    VerticalWritingMode,
    /// `FrontendOptions::base_direction` set to right-to-left.
    RtlBaseDirection,
    /// A non-empty `ParagraphInput::decoration_spans`.
    DecorationSpans,
    /// A non-empty `ParagraphInput::atomic_spans`.
    AtomicSpans,
    /// A non-empty `ParagraphInput::bidi_spans`.
    BidiSpans,
    /// A non-empty `ParagraphInput::annotations`.
    Annotations,
    /// Text whose UAX #9 levels are not all zero in a left-to-right
    /// paragraph: a strong right-to-left character, an Arabic number or an
    /// explicit directional formatting character (spec RM-11).
    BidiText,
    /// A style or an inline object aligned to a baseline other than the
    /// alphabetic one or an explicit offset from it.
    BaselineAlignment,
}

impl fmt::Display for FrontendFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::VerticalWritingMode => "a vertical writing mode",
            Self::RtlBaseDirection => "a right-to-left base direction",
            Self::DecorationSpans => "decoration spans",
            Self::AtomicSpans => "atomic edit spans",
            Self::BidiSpans => "explicit bidi spans",
            Self::Annotations => "annotations",
            Self::BidiText => "right-to-left or explicitly embedded text",
            Self::BaselineAlignment => "a baseline alignment other than alphabetic or an offset",
        })
    }
}

impl fmt::Display for FrontendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParagraphSeparatorInText { byte } => {
                write!(f, "the text holds a paragraph separator at byte {byte}")
            }
            Self::TextTooLong => f.write_str("the text is longer than u32::MAX bytes"),
            Self::InvalidStyle { span: Some(span) } => {
                write!(f, "the style of style span {span} is invalid")
            }
            Self::InvalidStyle { span: None } => f.write_str("the default style is invalid"),
            Self::InvalidStyleSpan { span } => write!(f, "style span {span} is invalid"),
            Self::InvalidInlineObject { object } => {
                write!(f, "inline object {object} is invalid")
            }
            Self::Unsupported { feature } => write!(f, "not supported yet: {feature}"),
            Self::ShaperContract { run } => {
                write!(f, "the shaper's clusters do not partition run {run}")
            }
        }
    }
}

impl std::error::Error for FrontendError {}

/// Compiles one paragraph into a `ParagraphAst` (spec FE-3, FE-25).
///
/// * `input`: the text, its styles and its span streams;
/// * `options`: writing mode, base direction and segmentation policy;
/// * `shaper`: resolves the strut, measures word spaces and shapes every run.
///
/// For an accepted input and a shaper that keeps its contract, the result
/// passes the input gate (spec FE-26).
///
/// Phase 1 compiles horizontal, unidirectional left-to-right text (spec
/// RM-11): grapheme atoms, script resolution, runs by style, script and
/// locale, shaping, word spaces, tabs, U+2028 hard breaks, inline objects and
/// the UAX #14 opportunities of the segmenter. What it does not compile yet is
/// refused with [`FrontendError::Unsupported`].
///
/// # Errors
/// The first problem found in the input, or a shaper answer that cannot be
/// used (spec FE-28).
pub fn compile_paragraph(
    input: &ParagraphInput<'_>,
    options: &FrontendOptions,
    shaper: &dyn TextShaper,
) -> Result<ParagraphAst, FrontendError> {
    check_input(input, options)?;
    Compiler::new(input, options, shaper).compile()
}

fn unsupported(feature: FrontendFeature) -> FrontendError {
    FrontendError::Unsupported { feature }
}

/// The checks of FE-28 that need no segmentation.
fn check_input(input: &ParagraphInput<'_>, options: &FrontendOptions) -> Result<(), FrontendError> {
    let text = input.text;
    if u32::try_from(text.len()).is_err() {
        return Err(FrontendError::TextTooLong);
    }
    if let Some((byte, _)) = text
        .char_indices()
        .find(|&(_, c)| is_paragraph_separator(c))
    {
        return Err(FrontendError::ParagraphSeparatorInText { byte });
    }
    if options.writing_mode.is_vertical() {
        return Err(unsupported(FrontendFeature::VerticalWritingMode));
    }
    if options.base_direction == Some(BidiDirection::Rtl) {
        return Err(unsupported(FrontendFeature::RtlBaseDirection));
    }
    if options.base_direction.is_none()
        && text.chars().find_map(segment::strong_direction) == Some(true)
    {
        // UAX #9 P2 and P3 would make the paragraph right-to-left.
        return Err(unsupported(FrontendFeature::RtlBaseDirection));
    }
    if text.chars().any(segment::is_bidi_significant) {
        return Err(unsupported(FrontendFeature::BidiText));
    }
    for (empty, feature) in [
        (
            input.decoration_spans.is_empty(),
            FrontendFeature::DecorationSpans,
        ),
        (input.atomic_spans.is_empty(), FrontendFeature::AtomicSpans),
        (input.bidi_spans.is_empty(), FrontendFeature::BidiSpans),
        (input.annotations.is_empty(), FrontendFeature::Annotations),
    ] {
        if !empty {
            return Err(unsupported(feature));
        }
    }
    check_style(input.default_style, None)?;
    let mut previous_end = 0;
    for (index, span) in input.style_spans.iter().enumerate() {
        let r = span.byte_range;
        let well_formed = r.start() < r.end()
            && r.end() <= text.len()
            && text.is_char_boundary(r.start())
            && text.is_char_boundary(r.end())
            && r.start() >= previous_end;
        if !well_formed {
            return Err(FrontendError::InvalidStyleSpan { span: index });
        }
        previous_end = r.end();
        check_style(&span.style, Some(index))?;
    }
    let mut previous_end = 0;
    for (index, object) in input.inline_objects.iter().enumerate() {
        let r = object.byte_range;
        let sizes = [object.width, object.ascent, object.descent];
        let well_formed = r.start() < r.end()
            && r.end() <= text.len()
            && r.start() >= previous_end
            && sizes.iter().all(|&v| is_extent(v));
        if !well_formed {
            return Err(FrontendError::InvalidInlineObject { object: index });
        }
        if !is_supported_alignment(object.baseline_align) {
            return Err(unsupported(FrontendFeature::BaselineAlignment));
        }
        previous_end = r.end();
    }
    Ok(())
}

fn check_style(style: &TextStyle, span: Option<usize>) -> Result<(), FrontendError> {
    let valid = is_extent(style.font_size)
        && style.font_size > 0.0
        && is_measurement(style.letter_spacing)
        && is_measurement(style.word_spacing)
        && style.baseline_align.is_valid()
        && style.script.is_none_or(|script| !script.is_unresolved());
    if !valid {
        return Err(FrontendError::InvalidStyle { span });
    }
    if !is_supported_alignment(style.baseline_align) {
        return Err(unsupported(FrontendFeature::BaselineAlignment));
    }
    Ok(())
}

/// The alignments Phase 1 places: the alphabetic baseline and an explicit
/// offset from it.
fn is_supported_alignment(align: BaselineAlignment) -> bool {
    matches!(
        align,
        BaselineAlignment::Auto | BaselineAlignment::Alphabetic | BaselineAlignment::Offset(_)
    )
}

/// One source unit, in text order.
#[derive(Clone, Copy, Debug)]
enum Unit {
    /// An index into `Compiler::atoms`.
    Atom(usize),
    /// A word space, with the index of its style.
    Glue(TextRange, usize),
    Tab(TextRange),
    HardBreak(TextRange),
    /// An index into `ParagraphInput::inline_objects`.
    Object(usize),
}

impl Unit {
    fn range(self, atoms: &[GraphemeAtom], objects: &[InlineObjectInput]) -> TextRange {
        match self {
            Self::Atom(i) => atoms[i].byte_range,
            Self::Glue(r, _) | Self::Tab(r) | Self::HardBreak(r) => r,
            Self::Object(i) => objects[i].byte_range,
        }
    }
}

struct Compiler<'a> {
    input: &'a ParagraphInput<'a>,
    options: &'a FrontendOptions,
    shaper: &'a dyn TextShaper,
    text: &'a str,
    /// Every style: the default first, then the spans' in order.
    styles: Vec<&'a TextStyle>,
    atoms: Vec<GraphemeAtom>,
    /// The style index of each atom.
    atom_styles: Vec<usize>,
    units: Vec<Unit>,
}

impl<'a> Compiler<'a> {
    fn new(
        input: &'a ParagraphInput<'a>,
        options: &'a FrontendOptions,
        shaper: &'a dyn TextShaper,
    ) -> Self {
        let styles = core::iter::once(input.default_style)
            .chain(input.style_spans.iter().map(|span| &span.style))
            .collect();
        Self {
            input,
            options,
            shaper,
            text: input.text,
            styles,
            atoms: Vec::new(),
            atom_styles: Vec::new(),
            units: Vec::new(),
        }
    }

    /// The index into `styles` of the style at `byte` (FE-25).
    fn style_at(&self, byte: usize) -> usize {
        let spans = self.input.style_spans;
        let after = spans.partition_point(|span| span.byte_range.start() <= byte);
        after
            .checked_sub(1)
            .filter(|&i| byte < spans[i].byte_range.end())
            .map_or(0, |i| i + 1)
    }

    fn compile(mut self) -> Result<ParagraphAst, FrontendError> {
        let text = self.text;
        let graphemes = segment::grapheme_boundaries(text);
        self.build_units(&graphemes)?;
        self.resolve_scripts();
        let opportunities = segment::line_opportunities(
            text,
            self.options.line_break_strictness,
            self.options.word_break_option,
            &graphemes,
        );
        // The discretionary after unit `i`, if any (FE-19, WN-2, WN-3).
        let breaks_after: Vec<bool> = (0..self.units.len())
            .map(|i| {
                let unit = self.units[i];
                let end = unit.range(&self.atoms, self.input.inline_objects).end();
                end < text.len()
                    && !matches!(unit, Unit::HardBreak(_))
                    && opportunities.binary_search(&end).is_ok()
            })
            .collect();
        // Atom indices that start a text box: after any other unit, after a
        // discretionary, or at a style, script or locale change.
        let mut box_starts = vec![false; self.atoms.len()];
        let mut previous: Option<(Unit, bool)> = None;
        for (i, unit) in self.units.iter().enumerate() {
            if let Unit::Atom(a) = *unit {
                box_starts[a] = match previous {
                    Some((Unit::Atom(p), broke)) => broke || !self.same_run(p, a),
                    _ => true,
                };
            }
            previous = Some((*unit, breaks_after[i]));
        }
        let runs = self.build_runs();
        let mut ast = self.empty_ast()?;
        self.shape_runs(&runs, &box_starts, &mut ast)?;
        self.assemble_items(&box_starts, &breaks_after, &mut ast);
        Ok(ast)
    }

    /// Splits the text into atoms, glue, tabs, hard breaks and objects.
    fn build_units(&mut self, graphemes: &[usize]) -> Result<(), FrontendError> {
        let text = self.text;
        let objects = self.input.inline_objects;
        for (index, object) in objects.iter().enumerate() {
            let r = object.byte_range;
            if graphemes.binary_search(&r.start()).is_err()
                || graphemes.binary_search(&r.end()).is_err()
            {
                return Err(FrontendError::InvalidInlineObject { object: index });
            }
        }
        let mut next_object = 0;
        for pair in graphemes.windows(2) {
            let range = TextRange::new(pair[0], pair[1]);
            if let Some(object) = objects.get(next_object) {
                if range.start() >= object.byte_range.end() {
                    next_object += 1;
                } else if range.start() >= object.byte_range.start() {
                    if range.start() == object.byte_range.start() {
                        self.units.push(Unit::Object(next_object));
                    }
                    if range.end() == object.byte_range.end() {
                        next_object += 1;
                    }
                    continue;
                }
            }
            let grapheme = &text[range.as_range()];
            let mut chars = grapheme.chars();
            let first = chars.next().expect("a grapheme is not empty");
            let single = chars.next().is_none();
            let unit = match first {
                '\t' if single => Unit::Tab(range),
                '\u{2028}' if single => Unit::HardBreak(range),
                c if single && is_word_space(c) => Unit::Glue(range, self.style_at(range.start())),
                _ => {
                    let mut flags = AtomFlags::EMPTY;
                    let class = segment::line_break_class(first);
                    flags.set(AtomFlags::IS_OPEN_PUNCT, class == LineBreakClass::Op);
                    flags.set(
                        AtomFlags::IS_CLOSE_PUNCT,
                        matches!(class, LineBreakClass::Cl | LineBreakClass::Cp),
                    );
                    flags.set(
                        AtomFlags::HAS_VARIATION_SELECTOR,
                        grapheme.chars().any(|c| {
                            matches!(c, '\u{FE0E}' | '\u{FE0F}' | '\u{E0100}'..='\u{E01EF}')
                        }),
                    );
                    self.atoms.push(GraphemeAtom {
                        orientation: GlyphOrientation::Horizontal,
                        byte_range: range,
                        script: ScriptTag::COMMON,
                        bidi_level: BidiLevel::LTR,
                        linebreak_class: class,
                        flags,
                    });
                    self.atom_styles.push(self.style_at(range.start()));
                    Unit::Atom(self.atoms.len() - 1)
                }
            };
            self.units.push(unit);
        }
        Ok(())
    }

    /// Step 3: a style's script stands; otherwise FE-13's Phase 1 subset.
    fn resolve_scripts(&mut self) {
        let own: Vec<Option<ScriptTag>> = self
            .atoms
            .iter()
            .map(|atom| script::atom_script(&self.text[atom.byte_range.as_range()]))
            .collect();
        let resolved = script::resolve_common(&own);
        for (index, atom) in self.atoms.iter_mut().enumerate() {
            atom.script = self.styles[self.atom_styles[index]]
                .script
                .unwrap_or(resolved[index]);
        }
    }

    fn locale_of(&self, style: usize) -> LocaleTag {
        self.styles[style]
            .locale
            .clone()
            .unwrap_or_else(|| self.options.default_locale.clone())
    }

    /// Whether atoms `a` and `b` can share a run (FE-17).
    fn same_run(&self, a: usize, b: usize) -> bool {
        let (sa, sb) = (self.atom_styles[a], self.atom_styles[b]);
        self.atoms[a].script == self.atoms[b].script
            && (sa == sb || self.styles[sa] == self.styles[sb])
            && self.locale_of(sa) == self.locale_of(sb)
    }

    /// Step 5: maximal sequences of atoms that agree on style, script and
    /// locale. Glue, tabs and objects between them stay inside the run's
    /// bytes (AST-5).
    fn build_runs(&self) -> Vec<ShapingRun> {
        let mut runs: Vec<ShapingRun> = Vec::new();
        let mut start = 0;
        for end in 1..=self.atoms.len() {
            if end < self.atoms.len() && self.same_run(start, end) {
                continue;
            }
            let style = self.styles[self.atom_styles[start]];
            let mut features = style.features.clone();
            features.normalize();
            runs.push(ShapingRun {
                baseline_align: style.baseline_align,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                orientation: GlyphOrientation::Horizontal,
                features,
                byte_range: TextRange::new(
                    self.atoms[start].byte_range.start(),
                    self.atoms[end - 1].byte_range.end(),
                ),
                families: style.families.clone(),
                font_size: style.font_size,
                style: style.style,
                script: self.atoms[start].script,
                locale: self.locale_of(self.atom_styles[start]),
                bidi_level: BidiLevel::LTR,
                decoration: style.decoration,
                atom_range: start..end,
                cluster_range: 0..0,
            });
            start = end;
        }
        runs
    }

    /// The AST with its strut and nothing else (FE-25, AST-6).
    fn empty_ast(&self) -> Result<ParagraphAst, FrontendError> {
        let style = self.input.default_style;
        let script = style
            .script
            .or_else(|| self.atoms.first().map(|atom| atom.script))
            .unwrap_or(ScriptTag::LATIN);
        let mut features = style.features.clone();
        features.normalize();
        let (_, strut_metrics) = self.shaper.resolve_primary_font_metrics(
            &style.families,
            style.style,
            script,
            &self.locale_of(0),
            style.font_size,
            features.variations(),
        );
        Ok(ParagraphAst {
            writing_mode: WritingMode::HorizontalTb,
            bidi_spans: Vec::new(),
            annotations: Vec::new(),
            base_direction: BidiDirection::Ltr,
            strut_metrics,
            strut_font_size: style.font_size,
            runs: Vec::new(),
            atoms: Vec::new(),
            clusters: Vec::new(),
            glyphs: Vec::new(),
            items: Vec::new(),
            decoration_spans: Vec::new(),
            atomic_spans: Vec::new(),
        })
    }

    /// Shapes every run, splits clusters that straddle a text box boundary
    /// (CLU-8), applies letter spacing (FE-27) and stores the result (AST-4).
    fn shape_runs(
        &self,
        runs: &[ShapingRun],
        box_starts: &[bool],
        ast: &mut ParagraphAst,
    ) -> Result<(), FrontendError> {
        for (run_index, run) in runs.iter().enumerate() {
            let contract = FrontendError::ShaperContract { run: run_index };
            let atoms = &self.atoms[run.atom_range.clone()];
            let mut clusters = Vec::new();
            let mut glyphs = Vec::new();
            self.shaper
                .shape_run(self.text, run, atoms, &mut clusters, &mut glyphs);
            if !partitions(&clusters, &glyphs, 0..atoms.len(), 0) {
                return Err(contract);
            }
            // Cuts are the run-local atoms that start a box, except the first.
            let is_cut = |local: usize| local > 0 && box_starts[run.atom_range.start + local];
            let mut pieces: Vec<(ShapedCluster, Vec<RawShapedGlyph>)> = Vec::new();
            for cluster in clusters {
                let own = glyphs[cluster.glyph_range.clone()].to_vec();
                let range = cluster.atom_range.clone();
                let cuts: Vec<usize> = (range.start + 1..range.end)
                    .filter(|&a| is_cut(a))
                    .collect();
                if cuts.is_empty() {
                    pieces.push((cluster, own));
                    continue;
                }
                let bounds: Vec<usize> = core::iter::once(range.start)
                    .chain(cuts)
                    .chain(core::iter::once(range.end))
                    .collect();
                for (k, window) in bounds.windows(2).enumerate() {
                    let mut out_clusters = Vec::new();
                    let mut out_glyphs = Vec::new();
                    let bytes = TextRange::new(
                        atoms[window[0]].byte_range.start(),
                        atoms[window[1] - 1].byte_range.end(),
                    );
                    self.shaper.reshape_window(
                        self.text,
                        run,
                        atoms,
                        bytes,
                        LeadingEdit::None,
                        TrailingEdit::None,
                        &mut out_clusters,
                        &mut out_glyphs,
                    );
                    if !partitions(&out_clusters, &out_glyphs, window[0]..window[1], 0) {
                        return Err(contract);
                    }
                    for (j, mut c) in out_clusters.into_iter().enumerate() {
                        if k > 0 && j == 0 {
                            c.safe_to_concat_before = false;
                        }
                        let own = out_glyphs[c.glyph_range.clone()].to_vec();
                        pieces.push((c, own));
                    }
                }
            }
            let mut stored = run.clone();
            stored.atom_range = ast.atoms.len()..ast.atoms.len() + atoms.len();
            ast.atoms.extend_from_slice(atoms);
            let cluster_start = ast.clusters.len();
            for (mut cluster, own) in pieces {
                let advance: f32 = own.iter().map(|g| g.x_advance).sum();
                cluster.natural_width = (advance + run.letter_spacing).max(0.0);
                if !cluster.natural_width.is_finite() {
                    cluster.natural_width = 0.0;
                }
                let at = ast.glyphs.len();
                ast.glyphs.extend(own);
                cluster.glyph_range = at..ast.glyphs.len();
                ast.clusters.push(cluster);
            }
            stored.cluster_range = cluster_start..ast.clusters.len();
            ast.runs.push(stored);
        }
        Ok(())
    }

    /// Step 6: the item sequence (FE-19, FE-27, WN-17).
    fn assemble_items(&self, box_starts: &[bool], breaks_after: &[bool], ast: &mut ParagraphAst) {
        let objects = self.input.inline_objects;
        // The paragraph-level cluster of every atom start, and the run of
        // every atom.
        let mut cluster_of_atom = vec![usize::MAX; self.atoms.len()];
        let mut run_of_atom = vec![0; self.atoms.len()];
        for (run_index, run) in ast.runs.iter().enumerate() {
            for a in run.atom_range.clone() {
                run_of_atom[a] = run_index;
            }
            for c in run.cluster_range.clone() {
                let first = run.atom_range.start + ast.clusters[c].atom_range.start;
                cluster_of_atom[first] = c;
            }
        }
        let mut space_cache: Vec<Option<f32>> = vec![None; self.styles.len()];
        let mut items: Vec<WordNode> = Vec::new();
        // The open text box: its first atom.
        let mut open: Option<usize> = None;
        let close = |open: &mut Option<usize>,
                     end_atom: usize,
                     items: &mut Vec<WordNode>,
                     ast: &ParagraphAst| {
            if let Some(first) = open.take() {
                // Box edges are cluster edges once CLU-8 has been applied.
                let run = run_of_atom[first];
                let c0 = cluster_of_atom[first];
                let c1 = if end_atom < run_of_atom.len() && run_of_atom[end_atom] == run {
                    cluster_of_atom[end_atom]
                } else {
                    ast.runs[run].cluster_range.end
                };
                let natural_width = ast.clusters[c0..c1].iter().map(|c| c.natural_width).sum();
                items.push(WordNode::Box(BoxNode::Text {
                    run_index: run,
                    byte_range: TextRange::new(
                        ast.atoms[first].byte_range.start(),
                        ast.atoms[end_atom - 1].byte_range.end(),
                    ),
                    atom_range: first..end_atom,
                    cluster_range: c0..c1,
                    natural_width,
                }));
            }
        };
        let mut atom_end = 0;
        for (i, unit) in self.units.iter().enumerate() {
            match *unit {
                Unit::Atom(a) => {
                    if box_starts[a] {
                        close(&mut open, a, &mut items, ast);
                        open = Some(a);
                    }
                    atom_end = a + 1;
                }
                other => {
                    close(&mut open, atom_end, &mut items, ast);
                    items.push(match other {
                        Unit::Glue(range, style) => {
                            let width =
                                *space_cache[style].get_or_insert_with(|| self.glue_width(style));
                            WordNode::Glue(GlueNode {
                                original_bidi_class: BidiClass::Ws,
                                bidi_level: BidiLevel::LTR,
                                is_collapsible: true,
                                byte_range: range,
                                kind: GlueKind::WordSpace,
                                natural_width: width,
                                max_stretch: width / 2.0,
                                max_shrink: width / 3.0,
                            })
                        }
                        Unit::Tab(range) => WordNode::Tab { byte_range: range },
                        Unit::HardBreak(range) => {
                            WordNode::Discretionary(DiscretionaryNode::hard_break(range))
                        }
                        Unit::Object(index) => {
                            let o = &objects[index];
                            WordNode::Box(BoxNode::InlineObject {
                                baseline_align: o.baseline_align,
                                byte_range: o.byte_range,
                                box_id: o.box_id,
                                width: o.width,
                                ascent: o.ascent,
                                descent: o.descent,
                                overflow_policy: o.overflow_policy,
                                bidi_level: BidiLevel::LTR,
                            })
                        }
                        Unit::Atom(_) => unreachable!("handled above"),
                    });
                }
            }
            if breaks_after[i] {
                close(&mut open, atom_end, &mut items, ast);
                let end = unit.range(&self.atoms, objects).end();
                items.push(WordNode::Discretionary(DiscretionaryNode::direct(end)));
            }
        }
        close(&mut open, atom_end, &mut items, ast);
        items.push(WordNode::Discretionary(DiscretionaryNode::paragraph_end(
            self.text.len(),
        )));
        ast.items = items;
    }

    /// `max(0, space advance + word_spacing)` for the primary font of a
    /// style (FE-27, SHP-20).
    fn glue_width(&self, style: usize) -> f32 {
        let s = self.styles[style];
        let mut features = s.features.clone();
        features.normalize();
        let (font, _) = self.shaper.resolve_primary_font_metrics(
            &s.families,
            s.style,
            s.script.unwrap_or(ScriptTag::LATIN),
            &self.locale_of(style),
            s.font_size,
            features.variations(),
        );
        let advance = self.shaper.space_advance(
            FontInstanceRef {
                font: FontRef::unscaled(font),
                font_size: s.font_size,
                variations: features.variations(),
            },
            GlyphOrientation::Horizontal,
        );
        let width = (advance + s.word_spacing).max(0.0);
        if width.is_finite() { width } else { 0.0 }
    }
}

/// Whether `clusters` partition `atoms` in order and their glyph ranges
/// tile `glyphs` from `glyph_base` (SHP-4, SHP-5).
fn partitions(
    clusters: &[ShapedCluster],
    glyphs: &[RawShapedGlyph],
    atoms: core::ops::Range<usize>,
    glyph_base: usize,
) -> bool {
    let mut next_atom = atoms.start;
    let mut next_glyph = glyph_base;
    for c in clusters {
        if c.atom_range.start != next_atom
            || c.atom_range.end <= c.atom_range.start
            || c.glyph_range.start != next_glyph
            || c.glyph_range.end < c.glyph_range.start
        {
            return false;
        }
        next_atom = c.atom_range.end;
        next_glyph = c.glyph_range.end;
    }
    next_atom == atoms.end && next_glyph == glyphs.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_display_and_implement_std_error() {
        let error: Box<dyn std::error::Error> = Box::new(FrontendError::Unsupported {
            feature: FrontendFeature::RtlBaseDirection,
        });
        assert_eq!(
            error.to_string(),
            "not supported yet: a right-to-left base direction"
        );
        assert_eq!(
            FrontendError::InvalidStyle { span: None }.to_string(),
            "the default style is invalid"
        );
    }
}
