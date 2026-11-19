//! The frontend entry point (spec §3, FE-25 to FE-28): the Phase 1
//! analysis (FE-29) with Phase 2's bidi resolution (FE-30).

use crate::frontend::{
    FrontendOptions, ParagraphAst, ParagraphInput, TextStyle, WordNode, bidi, segment,
};
use crate::shaping::TextShaper;
use crate::types::{
    ByteIndex, FontMetrics, GraphemeAtom, LocaleTag, RawShapedGlyph, ScriptTag, ShapedCluster,
    ShapingRun, TextRange, WritingMode,
};
use core::fmt;

mod breaks;
mod checks;
mod items;
mod runs;
mod shape;
mod spacing;
mod units;

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
    /// A non-empty `ParagraphInput::annotations`.
    Annotations,
}

impl fmt::Display for FrontendFeature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::VerticalWritingMode => "a vertical writing mode",
            Self::Annotations => "annotations",
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
    let compiler = Compiler::new(input, options, shaper);
    let levels = resolve_levels(input, options);
    let graphemes = segment::grapheme_boundaries(input.text);
    let mut units = compiler.build_units(&graphemes, &levels)?;
    compiler.check_span_edges(&units)?;
    compiler.resolve_scripts(&mut units.atoms, &units.atom_styles);
    let locales = compiler.resolve_locales(&units);
    // The next passes write unrelated flags, so statement order is enough;
    // `mark_cjk_justification` reads the candidates it is given.
    compiler.mark_spacing_flags(&mut units.atoms);
    let candidates = compiler.candidate_seams(&units, &locales);
    compiler.mark_cjk_justification(&units.units, &mut units.atoms, &candidates);
    let bound = compiler.bound_atoms(&units, &locales);
    breaks::mark_bound_atoms(&mut units.atoms, &bound);
    let breaks_after = compiler.breaks_after(&units, &locales, &graphemes);
    let box_starts = compiler.box_starts(&units, &locales, &breaks_after, &candidates);
    let runs = compiler.build_runs(&units, &locales);
    let shaped = compiler.shape_runs(&units.atoms, &runs, &box_starts)?;
    let seams = compiler.confirmed_seams(&candidates, &shaped);
    let items =
        compiler.assemble_items(&units, &levels, &box_starts, &breaks_after, &seams, &shaped);
    let strut = compiler.strut(&units.atoms);
    Ok(compiler.paragraph_ast(&levels, strut, units.atoms, shaped, items))
}

/// Step 2 (FE-10, FE-30): the UAX #9 levels of the text, with its inline
/// objects and explicit bidi spans.
fn resolve_levels(input: &ParagraphInput<'_>, options: &FrontendOptions) -> bidi::TextLevels {
    let objects: Vec<TextRange> = input.inline_objects.iter().map(|o| o.byte_range).collect();
    bidi::resolve_text(
        input.text,
        &objects,
        input.bidi_spans,
        options.base_direction,
    )
}

/// The read-only inputs of the compiler phases, with the styles.
struct Compiler<'a> {
    input: &'a ParagraphInput<'a>,
    options: &'a FrontendOptions,
    shaper: &'a dyn TextShaper,
    text: &'a str,
    /// Every style (features are canonical by construction, FNT-2): the default first, then the spans' in order.
    styles: Vec<&'a TextStyle>,
}

/// The strut's raw metrics and the font size they were requested at
/// (FE-25, AST-6).
struct Strut {
    metrics: FontMetrics,
    font_size: f32,
}

/// What shaping stores (AST-4): the runs with their cluster ranges, and the
/// flat cluster and glyph arrays they slice.
struct Shaped {
    runs: Vec<ShapingRun>,
    clusters: Vec<ShapedCluster>,
    glyphs: Vec<RawShapedGlyph>,
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

    fn locale_of(&self, style: usize) -> LocaleTag {
        self.styles[style]
            .locale
            .clone()
            .unwrap_or_else(|| self.options.default_locale.clone())
    }

    /// The strut of the default style, in the script it names or that of the
    /// first atom (FE-25, AST-6).
    fn strut(&self, atoms: &[GraphemeAtom]) -> Strut {
        let style = &self.styles[0];
        let script = style
            .script
            .or_else(|| atoms.first().map(|atom| atom.script))
            .unwrap_or(ScriptTag::LATIN);
        let (_, metrics) = self.shaper.resolve_primary_font_metrics(
            &style.families,
            style.style,
            script,
            &self.locale_of(0),
            style.font_size,
            style.features.variations(),
        );
        Strut {
            metrics,
            font_size: style.font_size,
        }
    }

    /// The AST, every field built here once: the compiled storage and items,
    /// the strut, and the fields the input passes through (AST-6).
    fn paragraph_ast(
        &self,
        levels: &bidi::TextLevels,
        strut: Strut,
        atoms: Vec<GraphemeAtom>,
        shaped: Shaped,
        items: Vec<WordNode>,
    ) -> ParagraphAst {
        ParagraphAst {
            writing_mode: WritingMode::HorizontalTb,
            bidi_spans: self.input.bidi_spans.to_vec(),
            annotations: Vec::new(),
            base_direction: levels.direction,
            strut_metrics: strut.metrics,
            strut_font_size: strut.font_size,
            runs: shaped.runs,
            atoms,
            clusters: shaped.clusters,
            glyphs: shaped.glyphs,
            items,
            decoration_spans: self.input.decoration_spans.to_vec(),
            atomic_spans: self.input.atomic_spans.to_vec(),
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
            feature: FrontendFeature::VerticalWritingMode,
        });
        assert_eq!(
            error.to_string(),
            "not supported yet: a vertical writing mode"
        );
        assert_eq!(
            FrontendError::InvalidStyle { span: None }.to_string(),
            "the default style is invalid"
        );
    }
}
