//! The frontend entry point (spec §3, FE-25 to FE-28): the Phase 1
//! analysis (FE-29) with Phase 2's bidi resolution (FE-30).

use super::{
    FrontendOptions, InlineObjectInput, LocalePolicy, ParagraphAst, ParagraphInput, TextStyle,
    bidi, emoji, is_word_space, locale, script, segment, url,
};
use crate::shaping::TextShaper;
use crate::types::{
    AtomFlags, BidiLevel, ByteIndex, GlyphOrientation, GraphemeAtom, LineBreakClass, LocaleTag,
    ScriptTag, ShapingRun, TextRange, WritingMode,
};
use core::fmt;

mod breaks;
mod checks;
mod items;
mod shape;
mod spacing;

use checks::check_input;

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
    /// An explicit bidi span repeats an id, is empty or outside the text, has
    /// an edge off a grapheme boundary or inside an inline object, is out of
    /// order, overlaps without nesting, has its predecessor's range, or nests
    /// deeper than `MAX_BIDI_SPAN_DEPTH` (spec BID-2).
    InvalidBidiSpan {
        /// Index into `ParagraphInput::bidi_spans`.
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
            Self::InvalidBidiSpan { span } => write!(f, "explicit bidi span {span} is invalid"),
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
/// It compiles horizontal text in either direction: grapheme atoms, bidi
/// levels and explicit bidi spans (FE-30), script resolution, runs by style,
/// script, locale and level, shaping, word spaces, tabs, U+2028 hard breaks,
/// inline objects and the UAX #14 opportunities of the segmenter. What it
/// does not compile yet (vertical writing modes and annotations) is refused
/// with [`FrontendError::Unsupported`].
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
    /// The paragraph's resolved bidi levels (Step 2, FE-30).
    levels: bidi::TextLevels,
    /// Where synthetic glue goes, ascending (FE-20, FE-31).
    seams: Vec<spacing::Seam>,
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
        let objects: Vec<TextRange> = input.inline_objects.iter().map(|o| o.byte_range).collect();
        let levels = bidi::resolve_text(
            input.text,
            &objects,
            input.bidi_spans,
            options.base_direction,
        );
        Self {
            input,
            options,
            shaper,
            levels,
            text: input.text,
            styles,
            atoms: Vec::new(),
            atom_styles: Vec::new(),
            atom_locales: Vec::new(),
            units: Vec::new(),
            seams: Vec::new(),
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
        self.resolve_levels();
        self.resolve_scripts();
        self.resolve_locales();
        self.resolve_spacing();
        let breaks_after = self.breaks_after(&graphemes);
        let box_starts = self.box_starts(&breaks_after);
        let runs = self.build_runs();
        let mut ast = self.empty_ast();
        self.shape_runs(&runs, &box_starts, &mut ast)?;
        self.confirm_compression(&ast);
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

    /// Step 2 (FE-10, FE-30): every atom takes the level of its first code
    /// point; an atom that L1 resets with whitespace is flagged for Stage 2.
    fn resolve_levels(&mut self) {
        for atom in &mut self.atoms {
            let start = atom.byte_range.start();
            atom.bidi_level = self.levels.at(start);
            let first = self.text[start..]
                .chars()
                .next()
                .expect("an atom is not empty");
            atom.flags.set(
                AtomFlags::L1_RESETTABLE,
                bidi::line_class(bidi::char_class(first)) != bidi::LineClass::Other,
            );
        }
    }

    /// The atoms as script resolution sees them.
    fn script_atoms(&self) -> Vec<script::ScriptAtom<'a>> {
        let text = self.text;
        self.atoms
            .iter()
            .enumerate()
            .map(|(index, atom)| script::ScriptAtom {
                byte_range: atom.byte_range,
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

    /// Whether unit `i` is the word space after a listed one-letter word of
    /// its locale, which binds the word to the next (FE-23).
    fn binds_one_letter_word(&self, i: usize) -> bool {
        let (Some(Unit::Glue(..)), Some(&Unit::Atom(a))) = (
            self.units.get(i),
            i.checked_sub(1).and_then(|p| self.units.get(p)),
        ) else {
            return false;
        };
        let word_start = self.units[..i - 1]
            .iter()
            .rev()
            .find(|u| {
                !matches!(
                    u,
                    Unit::Atom(p) if self.atoms[*p].flags.contains(AtomFlags::IS_OPEN_PUNCT)
                )
            })
            .is_none_or(|u| !matches!(u, Unit::Atom(_)));
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
            // A cursive run takes no letter spacing (FE-27).
            let cursive = self.atoms[start]
                .flags
                .contains(AtomFlags::NO_LETTER_SPACING);
            runs.push(ShapingRun {
                baseline_align: style.baseline_align,
                letter_spacing: if cursive { 0.0 } else { style.letter_spacing },
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
                bidi_level: self.atoms[start].bidi_level,
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
            bidi_spans: self.input.bidi_spans.to_vec(),
            annotations: Vec::new(),
            base_direction: self.levels.direction,
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
}

/// The primary language subtag of `locale`, in lower case.
fn primary_language(locale: &LocaleTag) -> &str {
    locale.as_str().split('-').next().unwrap_or_default()
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
