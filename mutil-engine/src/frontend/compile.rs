//! The frontend entry point (spec §3, FE-25 to FE-28).
//!
//! The Phase 1 analysis pass: horizontal, unidirectional text (spec RM-11).

use super::{
    BoxDecorationBreak, BoxNode, DiscretionaryNode, FrontendOptions, GlueKind, GlueNode,
    InlineDecorationSpan, InlineObjectInput, LocalePolicy, MAX_DECORATION_NESTING_DEPTH,
    ParagraphAst, ParagraphInput, SpanEdge, TextStyle, WordNode, emoji, is_paragraph_separator,
    is_word_space, locale, script, segment, url,
};
use crate::shaping::{LeadingEdit, TextShaper, TrailingEdit};
use crate::types::{
    AtomFlags, BidiClass, BidiDirection, BidiLevel, ByteIndex, FontInstanceRef, FontRef,
    GlyphOrientation, GraphemeAtom, LineBreakClass, LocaleTag, RawShapedGlyph, ScriptTag,
    ShapedCluster, ShapingRun, TextRange, WritingMode, is_extent, is_measurement,
};
use core::fmt;
use smallvec::SmallVec;

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
    /// A decoration span is out of order, overlaps without nesting, nests
    /// deeper than `MAX_DECORATION_NESTING_DEPTH`, repeats an id, is empty or
    /// outside the text, has an invalid padding, or has an edge off a
    /// grapheme boundary or inside an inline object (spec AST-7).
    InvalidDecorationSpan {
        /// Index into `ParagraphInput::decoration_spans`.
        span: usize,
    },
    /// An atomic span is out of order, overlaps its predecessor, repeats an
    /// id, is empty or outside the text, has an edge that is not the start
    /// or end of a text atom or inline object, or holds neither (spec AST-9).
    InvalidAtomicSpan {
        /// Index into `ParagraphInput::atomic_spans`.
        span: usize,
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
            Self::InvalidDecorationSpan { span } => write!(f, "decoration span {span} is invalid"),
            Self::InvalidAtomicSpan { span } => write!(f, "atomic span {span} is invalid"),
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
    if options.writing_mode.is_vertical() {
        return Err(unsupported(FrontendFeature::VerticalWritingMode));
    }
    if options.base_direction == Some(BidiDirection::Rtl) {
        return Err(unsupported(FrontendFeature::RtlBaseDirection));
    }
    check_decoration_spans(input)?;
    for (empty, feature) in [
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
            && sizes.into_iter().all(is_extent)
            && object.baseline_align.is_valid();
        if !well_formed {
            return Err(FrontendError::InvalidInlineObject { object: index });
        }
        previous_end = r.end();
    }
    // Single pass over the text for paragraph separators (AST-2) and bidi
    // detection; an object's bytes are neither atoms nor shaped (FE-27), so
    // they take no part in direction.
    let objects = input.inline_objects;
    let mut obj_idx = 0usize;
    let mut first_strong: Option<bool> = None;
    let mut has_bidi = false;
    for (offset, ch) in text.char_indices() {
        if is_paragraph_separator(ch) {
            return Err(FrontendError::ParagraphSeparatorInText { byte: offset });
        }
        while obj_idx < objects.len() && objects[obj_idx].byte_range.end() <= offset {
            obj_idx += 1;
        }
        if obj_idx < objects.len() && objects[obj_idx].byte_range.start() <= offset {
            continue;
        }
        if options.base_direction.is_none() && first_strong.is_none() {
            first_strong = segment::strong_direction(ch);
        }
        if !has_bidi && segment::is_bidi_significant(ch) {
            has_bidi = true;
        }
    }
    if options.base_direction.is_none() && first_strong == Some(true) {
        // UAX #9 P2 and P3 would make the paragraph right-to-left.
        return Err(unsupported(FrontendFeature::RtlBaseDirection));
    }
    if has_bidi {
        return Err(unsupported(FrontendFeature::BidiText));
    }
    Ok(())
}

/// AST-7 without segmentation: order, nesting, ids, paddings and ranges on
/// character boundaries. Grapheme and object edges are checked once units
/// exist.
fn check_decoration_spans(input: &ParagraphInput<'_>) -> Result<(), FrontendError> {
    let text = input.text;
    let mut ids = std::collections::HashSet::new();
    let mut stack: SmallVec<[TextRange; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
    for (index, span) in input.decoration_spans.iter().enumerate() {
        let r = span.byte_range;
        let invalid = Err(FrontendError::InvalidDecorationSpan { span: index });
        if index > 0 {
            let prev = input.decoration_spans[index - 1].byte_range;
            if r.start() < prev.start() || (r.start() == prev.start() && r.end() > prev.end()) {
                return invalid;
            }
        }
        while stack.last().is_some_and(|top| top.end() <= r.start()) {
            stack.pop();
        }
        if !ids.insert(span.span_id)
            || !span.is_valid()
            || r.end() > text.len()
            || !text.is_char_boundary(r.start())
            || !text.is_char_boundary(r.end())
            || stack.last().is_some_and(|top| !top.contains_range(r))
            || stack.len() >= MAX_DECORATION_NESTING_DEPTH
        {
            return invalid;
        }
        stack.push(r);
    }
    let mut ids = std::collections::HashSet::new();
    for (index, span) in input.atomic_spans.iter().enumerate() {
        let r = span.byte_range;
        let ordered = index == 0 || input.atomic_spans[index - 1].byte_range.end() <= r.start();
        if !ids.insert(span.span_id) || r.is_empty() || r.end() > text.len() || !ordered {
            return Err(FrontendError::InvalidAtomicSpan { span: index });
        }
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
    Ok(())
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
    /// Every style with normalized features: the default first, then the spans' in order.
    styles: Vec<TextStyle>,
    atoms: Vec<GraphemeAtom>,
    /// The style index of each atom.
    atom_styles: Vec<usize>,
    /// The locale of each atom after Step 4 (FE-16).
    atom_locales: Vec<LocaleTag>,
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
            .map(|style| {
                let mut normalized = style.clone();
                normalized.features.normalize();
                normalized
            })
            .collect();
        Self {
            input,
            options,
            shaper,
            text: input.text,
            styles,
            atoms: Vec::new(),
            atom_styles: Vec::new(),
            atom_locales: Vec::new(),
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
        self.check_span_edges()?;
        self.resolve_scripts();
        self.resolve_locales();
        let opportunities = segment::line_opportunities(
            text,
            self.options.line_break_strictness,
            self.options.word_break_option,
            &graphemes,
        );
        // The penalty of the discretionary after unit `i`, if any (FE-19,
        // WN-2, WN-3), with URL tailoring (FE-22), unbreakable dash runs
        // (FE-15) and the no-line-end words (FE-23).
        let urls = url::url_ranges(text);
        let added: Vec<(usize, i32)> = urls
            .iter()
            .flat_map(|&u| url::url_opportunities(text, u, &graphemes))
            .collect();
        let mut breaks_after = Vec::with_capacity(self.units.len());
        let mut opp_idx = 0usize;
        let mut add_idx = 0usize;
        let mut url_idx = 0usize;
        for i in 0..self.units.len() {
            let unit = self.units[i];
            let end = unit.range(&self.atoms, self.input.inline_objects).end();
            if end >= text.len() || matches!(unit, Unit::HardBreak(_)) {
                breaks_after.push(None);
                continue;
            }
            // Only directly adjacent `Atom` units within the same `ShapingRun`
            // may bind under FE-15 (never across `Glue`, `Tab`, `HardBreak`,
            // or `Object`).
            if let (Unit::Atom(a), Some(Unit::Atom(b))) = (unit, self.units.get(i + 1).copied())
                && b == a + 1
                && self.same_run(a, b)
                && script::binds_to_next(self.text, &self.atoms, a)
            {
                self.atoms[b].flags.insert(AtomFlags::BINDS_TO_PREVIOUS);
                breaks_after.push(None);
                continue;
            }
            if self.binds_one_letter_word(i) {
                breaks_after.push(None);
                continue;
            }
            while opp_idx < opportunities.len() && opportunities[opp_idx] < end {
                opp_idx += 1;
            }
            let natural = opportunities.get(opp_idx).copied() == Some(end);
            while add_idx < added.len() && added[add_idx].0 < end {
                add_idx += 1;
            }
            let tailored = added
                .get(add_idx)
                .filter(|&&(at, _)| at == end)
                .map(|&(_, p)| p);
            while url_idx < urls.len() && urls[url_idx].end() <= end {
                url_idx += 1;
            }
            let penalty = match (natural, tailored) {
                (_, Some(p)) => Some(p),
                (true, None)
                    if urls
                        .get(url_idx)
                        .is_some_and(|u| u.start() < end && end < u.end()) =>
                {
                    Some(DiscretionaryNode::URL_PENALTY_MIN)
                }
                (true, None) => Some(DiscretionaryNode::NATURAL_PENALTY),
                (false, None) => None,
            };
            breaks_after.push(penalty);
        }
        // Atom indices that start a text box: after any other unit, after a
        // discretionary, or at a style, script, locale or decoration edge.
        let mut box_starts = vec![false; self.atoms.len()];
        let dec_spans = self.input.decoration_spans;
        let mut dec_next = 0usize;
        let mut dec_ends: SmallVec<[usize; MAX_DECORATION_NESTING_DEPTH]> = SmallVec::new();
        let mut has_decoration_edge_at = |at: usize| {
            let mut edge = false;
            while let Some(&end) = dec_ends.last() {
                if end > at {
                    break;
                }
                if end == at {
                    edge = true;
                }
                dec_ends.pop();
            }
            while dec_next < dec_spans.len() && dec_spans[dec_next].byte_range.start() <= at {
                let r = dec_spans[dec_next].byte_range;
                if r.start() == at || r.end() == at {
                    edge = true;
                }
                if r.end() > at {
                    while dec_ends.last().is_some_and(|&end| end <= r.start()) {
                        dec_ends.pop();
                    }
                    dec_ends.push(r.end());
                }
                dec_next += 1;
            }
            edge
        };
        let mut previous: Option<(Unit, bool)> = None;
        for (i, unit) in self.units.iter().enumerate() {
            if let Unit::Atom(a) = *unit {
                let at = self.atoms[a].byte_range.start();
                box_starts[a] = match previous {
                    Some((Unit::Atom(p), broke)) => {
                        broke || !self.same_run(p, a) || has_decoration_edge_at(at)
                    }
                    _ => true,
                };
            }
            previous = Some((*unit, breaks_after[i].is_some()));
        }
        let runs = self.build_runs();
        let mut ast = self.empty_ast();
        self.shape_runs(&runs, &box_starts, &mut ast)?;
        self.assemble_items(&box_starts, &breaks_after, &mut ast);
        ast.atoms = core::mem::take(&mut self.atoms);
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
                    flags.set(
                        AtomFlags::IS_EMOJI_SEQUENCE,
                        emoji::is_emoji_sequence(grapheme),
                    );
                    flags.set(AtomFlags::IS_APOSTROPHE, script::is_apostrophe(text, range));
                    flags.set(
                        AtomFlags::IS_CJK_IDEOGRAPH,
                        script::is_wide(first)
                            && matches!(
                                &script::char_script(first).as_bytes(),
                                b"Hani" | b"Hira" | b"Kana"
                            ),
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

    /// The atoms as script resolution sees them.
    fn script_atoms(&self) -> Vec<script::ScriptAtom<'a>> {
        let text = self.text;
        self.atoms
            .iter()
            .enumerate()
            .map(|(index, atom)| script::ScriptAtom {
                text: &text[atom.byte_range.as_range()],
                explicit: self.styles[self.atom_styles[index]].script,
                is_emoji: atom.flags.contains(AtomFlags::IS_EMOJI_SEQUENCE),
                is_apostrophe: atom.flags.contains(AtomFlags::IS_APOSTROPHE),
                after_space: text[..atom.byte_range.start()].ends_with(' '),
                quotes: script::locale_quote_pairs(primary_language(
                    &self.locale_of(self.atom_styles[index]),
                )),
            })
            .collect()
    }

    /// Step 3 (FE-6, FE-11, FE-13 to FE-15): a style's script stands.
    fn resolve_scripts(&mut self) {
        let resolved = script::resolve(&self.script_atoms(), self.options.quote_policy);
        for (atom, script) in self.atoms.iter_mut().zip(resolved) {
            atom.script = script;
        }
    }

    /// Step 4 (SCO-5, FE-16): a style's locale stands; untagged atoms take
    /// `default_locale`, or under `ExperimentalHeuristics` the locale their
    /// sentence's scripts imply.
    fn resolve_locales(&mut self) {
        if self.options.locale_policy == LocalePolicy::ExplicitOnly {
            self.atom_locales = self
                .atom_styles
                .iter()
                .map(|&style| self.locale_of(style))
                .collect();
            return;
        }
        let text = self.text;
        let atoms: Vec<locale::LocaleAtom<'_>> = self
            .atoms
            .iter()
            .zip(&self.atom_styles)
            .map(|(atom, &style)| locale::LocaleAtom {
                byte_range: atom.byte_range,
                text: &text[atom.byte_range.as_range()],
                script: atom.script,
                explicit: self.styles[style].locale.as_ref(),
            })
            .collect();
        self.atom_locales = locale::resolve(
            text,
            &atoms,
            &self.options.default_locale,
            self.options.locale_policy,
        );
    }

    /// The edge checks of AST-7 and AST-9 that need units: a decoration
    /// edge lies between units, an atomic edge at the start or end of a text
    /// atom or object, and an atomic span holds one.
    fn check_span_edges(&self) -> Result<(), FrontendError> {
        let input = self.input;
        if !input.decoration_spans.is_empty() {
            let mut next_span = 0usize;
            let mut open: SmallVec<[(usize, usize); MAX_DECORATION_NESTING_DEPTH]> =
                SmallVec::new();
            let mut first_bad: Option<usize> = None;
            let mut record_bad = |idx: usize| {
                first_bad = Some(first_bad.map_or(idx, |cur| cur.min(idx)));
            };
            for unit in &self.units {
                let ur = unit.range(&self.atoms, input.inline_objects);
                while next_span < input.decoration_spans.len()
                    && input.decoration_spans[next_span].byte_range.start() == ur.start()
                {
                    open.push((
                        next_span,
                        input.decoration_spans[next_span].byte_range.end(),
                    ));
                    next_span += 1;
                }
                while next_span < input.decoration_spans.len()
                    && input.decoration_spans[next_span].byte_range.start() < ur.end()
                {
                    record_bad(next_span);
                    next_span += 1;
                }
                while let Some(&(span_idx, span_end)) = open.last() {
                    if span_end > ur.end() {
                        break;
                    }
                    open.pop();
                    if span_end < ur.end() {
                        record_bad(span_idx);
                    }
                }
            }
            if let Some(span) = first_bad {
                return Err(FrontendError::InvalidDecorationSpan { span });
            }
        }
        if !input.atomic_spans.is_empty() {
            let mut u = 0usize;
            for (index, span) in input.atomic_spans.iter().enumerate() {
                let r = span.byte_range;
                while u < self.units.len()
                    && self.units[u].range(&self.atoms, input.inline_objects).end() <= r.start()
                {
                    u += 1;
                }
                let Some(&start_unit) = self.units.get(u) else {
                    return Err(FrontendError::InvalidAtomicSpan { span: index });
                };
                let start_range = start_unit.range(&self.atoms, input.inline_objects);
                if start_range.start() != r.start()
                    || !matches!(start_unit, Unit::Atom(_) | Unit::Object(_))
                    || start_range.end() > r.end()
                {
                    return Err(FrontendError::InvalidAtomicSpan { span: index });
                }
                while u < self.units.len()
                    && self.units[u].range(&self.atoms, input.inline_objects).end() < r.end()
                {
                    u += 1;
                }
                let Some(&end_unit) = self.units.get(u) else {
                    return Err(FrontendError::InvalidAtomicSpan { span: index });
                };
                let end_range = end_unit.range(&self.atoms, input.inline_objects);
                if end_range.end() != r.end()
                    || !matches!(end_unit, Unit::Atom(_) | Unit::Object(_))
                {
                    return Err(FrontendError::InvalidAtomicSpan { span: index });
                }
            }
        }
        Ok(())
    }

    /// Whether unit `i` is the word space after a listed one-letter word of
    /// its locale, which binds the word to the next (FE-23).
    fn binds_one_letter_word(&self, i: usize) -> bool {
        let (Some(Unit::Glue(..)), Some(&Unit::Atom(a))) = (
            self.units.get(i),
            i.checked_sub(1).and_then(|p| self.units.get(p)),
        ) else {
            return false;
        };
        let word_start = i < 2 || !matches!(self.units[i - 2], Unit::Atom(_));
        let letter = &self.text[self.atoms[a].byte_range.as_range()];
        let language = primary_language(&self.atom_locales[a]);
        let mut chars = letter.chars();
        word_start
            && matches!((chars.next(), chars.next()), (Some(c), None)
                if url::no_line_end_words(language).contains(c))
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
        self.atoms[a].bidi_level == self.atoms[b].bidi_level
            && self.atoms[a].script == self.atoms[b].script
            && (sa == sb || self.styles[sa] == self.styles[sb])
            && self.atom_locales[a] == self.atom_locales[b]
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
            let style = &self.styles[self.atom_styles[start]];
            runs.push(ShapingRun {
                baseline_align: style.baseline_align,
                letter_spacing: style.letter_spacing,
                word_spacing: style.word_spacing,
                orientation: GlyphOrientation::Horizontal,
                features: style.features.clone(),
                byte_range: TextRange::new(
                    self.atoms[start].byte_range.start(),
                    self.atoms[end - 1].byte_range.end(),
                ),
                families: style.families.clone(),
                font_size: style.font_size,
                style: style.style,
                script: self.atoms[start].script,
                locale: self.atom_locales[start].clone(),
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
    fn empty_ast(&self) -> ParagraphAst {
        let style = &self.styles[0];
        let script = style
            .script
            .or_else(|| self.atoms.first().map(|atom| atom.script))
            .unwrap_or(ScriptTag::LATIN);
        let (_, strut_metrics) = self.shaper.resolve_primary_font_metrics(
            &style.families,
            style.style,
            script,
            &self.locale_of(0),
            style.font_size,
            style.features.variations(),
        );
        ParagraphAst {
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
        }
    }

    /// Shapes every run, splits clusters that straddle a text box boundary
    /// (CLU-8), applies letter spacing (FE-27) and stores the result (AST-4).
    fn shape_runs(
        &self,
        runs: &[ShapingRun],
        box_starts: &[bool],
        ast: &mut ParagraphAst,
    ) -> Result<(), FrontendError> {
        let mut clusters: Vec<ShapedCluster> = Vec::new();
        let mut glyphs: Vec<RawShapedGlyph> = Vec::new();
        let mut out_clusters: Vec<ShapedCluster> = Vec::new();
        let mut out_glyphs: Vec<RawShapedGlyph> = Vec::new();
        for (run_index, run) in runs.iter().enumerate() {
            let contract = FrontendError::ShaperContract { run: run_index };
            let atoms = &self.atoms[run.atom_range.clone()];
            clusters.clear();
            glyphs.clear();
            self.shaper
                .shape_run(self.text, run, atoms, &mut clusters, &mut glyphs);
            if !partitions(&clusters, &glyphs, 0..atoms.len(), 0) {
                return Err(contract);
            }
            // Cuts are the run-local atoms that start a box, except the first.
            let is_cut = |local: usize| local > 0 && box_starts[run.atom_range.start + local];
            let push_cluster = |mut cluster: ShapedCluster,
                                source_glyphs: &[RawShapedGlyph],
                                ast: &mut ParagraphAst| {
                let own = &source_glyphs[cluster.glyph_range.clone()];
                let advance: f32 = own.iter().map(|g| g.x_advance).sum();
                let raw = advance + run.letter_spacing;
                cluster.natural_width = if raw.is_finite() { raw.max(0.0) } else { raw };
                let at = ast.glyphs.len();
                ast.glyphs.extend_from_slice(own);
                cluster.glyph_range = at..ast.glyphs.len();
                ast.clusters.push(cluster);
            };
            let cluster_start = ast.clusters.len();
            for cluster in clusters.drain(..) {
                let range = cluster.atom_range.clone();
                let Some(first_cut) = (range.start + 1..range.end).find(|&a| is_cut(a)) else {
                    push_cluster(cluster, &glyphs, ast);
                    continue;
                };
                let mut w_start = range.start;
                let mut next_cut = Some(first_cut);
                let mut k = 0usize;
                loop {
                    let w_end = next_cut.unwrap_or(range.end);
                    out_clusters.clear();
                    out_glyphs.clear();
                    let bytes = TextRange::new(
                        atoms[w_start].byte_range.start(),
                        atoms[w_end - 1].byte_range.end(),
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
                    if !partitions(&out_clusters, &out_glyphs, w_start..w_end, 0) {
                        return Err(contract);
                    }
                    for (j, mut c) in out_clusters.drain(..).enumerate() {
                        if k > 0 && j == 0 {
                            c.safe_to_concat_before = false;
                        }
                        push_cluster(c, &out_glyphs, ast);
                    }
                    let Some(cut) = next_cut else {
                        break;
                    };
                    w_start = cut;
                    next_cut = (cut + 1..range.end).find(|&a| is_cut(a));
                    k += 1;
                }
            }
            ast.runs.push(ShapingRun {
                cluster_range: cluster_start..ast.clusters.len(),
                ..run.clone()
            });
        }
        Ok(())
    }

    /// Step 6: the item sequence (FE-19, FE-27, WN-17, WN-21).
    fn assemble_items(
        &self,
        box_starts: &[bool],
        breaks_after: &[Option<i32>],
        ast: &mut ParagraphAst,
    ) {
        let objects = self.input.inline_objects;
        let runs = &ast.runs;
        let clusters = &ast.clusters;
        let atoms = &self.atoms;
        let mut space_cache: Vec<Option<f32>> = vec![None; self.styles.len()];
        let mut items: Vec<WordNode> = Vec::new();
        // The open text box: its first atom, plus monotonic run and cluster cursors.
        let mut open: Option<usize> = None;
        let mut run_cursor = 0usize;
        let mut cluster_cursor = 0usize;
        let mut close = |open: &mut Option<usize>, end_atom: usize, items: &mut Vec<WordNode>| {
            let Some(first) = open.take() else {
                return;
            };
            // Box edges are cluster edges once CLU-8 has been applied.
            while runs[run_cursor].atom_range.end <= first {
                run_cursor += 1;
                cluster_cursor = runs[run_cursor].cluster_range.start;
            }
            let run = &runs[run_cursor];
            while cluster_cursor < run.cluster_range.end
                && run.atom_range.start + clusters[cluster_cursor].atom_range.end <= first
            {
                cluster_cursor += 1;
            }
            let c0 = cluster_cursor;
            let mut natural_width = 0.0;
            while cluster_cursor < run.cluster_range.end
                && run.atom_range.start + clusters[cluster_cursor].atom_range.start < end_atom
            {
                natural_width += clusters[cluster_cursor].natural_width;
                cluster_cursor += 1;
            }
            let c1 = cluster_cursor;
            items.push(WordNode::Box(BoxNode::Text {
                run_index: run_cursor,
                byte_range: TextRange::new(
                    atoms[first].byte_range.start(),
                    atoms[end_atom - 1].byte_range.end(),
                ),
                atom_range: first..end_atom,
                cluster_range: c0..c1,
                natural_width,
            }));
        };
        let mut atom_end = 0;
        let mut boundaries = BoundarySweep::new(self.input.decoration_spans);
        boundaries.emit_leading(0, &mut items);
        for (i, unit) in self.units.iter().enumerate() {
            match *unit {
                Unit::Atom(a) => {
                    if box_starts[a] {
                        close(&mut open, a, &mut items);
                        open = Some(a);
                    }
                    atom_end = a + 1;
                }
                other => {
                    close(&mut open, atom_end, &mut items);
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
                            let mut node = DiscretionaryNode::hard_break(range);
                            boundaries.apply_clones(&mut node);
                            WordNode::Discretionary(node)
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
            let end = unit.range(atoms, objects).end();
            let has_trailing = boundaries.has_trailing_at(end);
            let has_leading = end < self.text.len() && boundaries.has_leading_at(end);
            if breaks_after[i].is_some() || has_trailing || has_leading {
                close(&mut open, atom_end, &mut items);
            }
            if has_trailing {
                boundaries.emit_trailing(end, &mut items);
            }
            if let Some(penalty) = breaks_after[i] {
                let mut node = DiscretionaryNode::direct(end);
                node.penalty = penalty;
                boundaries.apply_clones(&mut node);
                items.push(WordNode::Discretionary(node));
            }
            if has_leading {
                boundaries.emit_leading(end, &mut items);
            }
        }
        close(&mut open, atom_end, &mut items);
        items.push(WordNode::Discretionary(DiscretionaryNode::paragraph_end(
            self.text.len(),
        )));
        ast.items = items;
        ast.decoration_spans = self.input.decoration_spans.to_vec();
        ast.atomic_spans = self.input.atomic_spans.to_vec();
    }

    /// `max(0, space advance + word_spacing)` for the primary font of a style
    /// (FE-27, SHP-20), leaving non-finite shaper advances intact for the input
    /// gate.
    fn glue_width(&self, style: usize) -> f32 {
        let s = &self.styles[style];
        let (font, _) = self.shaper.resolve_primary_font_metrics(
            &s.families,
            s.style,
            s.script.unwrap_or(ScriptTag::LATIN),
            &self.locale_of(style),
            s.font_size,
            s.features.variations(),
        );
        let advance = self.shaper.space_advance(
            FontInstanceRef {
                font: FontRef::unscaled(font),
                font_size: s.font_size,
                variations: s.features.variations(),
            },
            GlyphOrientation::Horizontal,
        );
        let raw = advance + s.word_spacing;
        if raw.is_finite() { raw.max(0.0) } else { raw }
    }
}

/// The `SpanBoundary` items and `Clone` discretionary padding of the
/// decoration spans in text order (WN-8, WN-21, AST-8), from one sweep: asked
/// at ascending offsets, among them every span edge (AST-7, checked by
/// `check_span_edges`), it emits leading boundaries outer first, trailing ones
/// inner first, and clones open `Clone` spans onto enclosed discretionaries.
struct BoundarySweep<'s> {
    spans: &'s [InlineDecorationSpan],
    next: usize,
    open: SmallVec<[&'s InlineDecorationSpan; MAX_DECORATION_NESTING_DEPTH]>,
}

impl<'s> BoundarySweep<'s> {
    fn new(spans: &'s [InlineDecorationSpan]) -> Self {
        Self {
            spans,
            next: 0,
            open: SmallVec::new(),
        }
    }

    fn node(span: &InlineDecorationSpan, offset: usize, edge: SpanEdge) -> WordNode {
        WordNode::Box(BoxNode::SpanBoundary {
            byte_range: TextRange::empty_at(offset),
            span_id: span.span_id,
            edge,
            width: match edge {
                SpanEdge::Leading => span.start_padding,
                SpanEdge::Trailing => span.end_padding,
            },
            bidi_level: BidiLevel::LTR,
        })
    }

    fn has_leading_at(&self, offset: usize) -> bool {
        self.spans
            .get(self.next)
            .is_some_and(|span| span.byte_range.start() == offset)
    }

    fn has_trailing_at(&self, offset: usize) -> bool {
        self.open
            .last()
            .is_some_and(|span| span.byte_range.end() == offset)
    }

    /// Emits the spans starting at `offset`, outer first.
    fn emit_leading(&mut self, offset: usize, items: &mut Vec<WordNode>) {
        while let Some(span) = self
            .spans
            .get(self.next)
            .filter(|s| s.byte_range.start() == offset)
        {
            items.push(Self::node(span, offset, SpanEdge::Leading));
            self.open.push(span);
            self.next += 1;
        }
    }

    /// Emits the spans ending at `offset`, inner first.
    fn emit_trailing(&mut self, offset: usize, items: &mut Vec<WordNode>) {
        while let Some(&span) = self.open.last().filter(|s| s.byte_range.end() == offset) {
            items.push(Self::node(span, offset, SpanEdge::Trailing));
            self.open.pop();
        }
    }

    /// Applies open `Clone` spans strictly containing `d`'s break offset.
    fn apply_clones(&self, d: &mut DiscretionaryNode) {
        let offset = d.byte_offset();
        for span in &self.open {
            if span.break_mode == BoxDecorationBreak::Clone
                && span.byte_range.start() < offset
                && offset < span.byte_range.end()
            {
                d.cloned_spans.push(span.span_id);
                d.pre_break_width += span.end_padding;
                d.post_break_width += span.start_padding;
                if !d.is_mandatory() {
                    d.penalty += DiscretionaryNode::CLONED_SPAN_PENALTY;
                }
            }
        }
    }
}

/// The primary language subtag of `locale`, in lower case.
fn primary_language(locale: &LocaleTag) -> &str {
    locale.as_str().split('-').next().unwrap_or_default()
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
