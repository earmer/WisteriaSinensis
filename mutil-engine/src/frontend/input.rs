//! Caller input of the frontend (spec §3, FE-25 to FE-27): the text of one
//! paragraph, its styles and the span streams laid over it.

use super::{AnnotationKind, AnnotationSide, AtomicEditSpan, ExplicitBidiSpan};
use super::{BoxOverflowPolicy, InlineDecorationSpan};
use crate::types::{
    BaselineAlignment, FamilyStack, FontFeatures, FontStyle, LocaleTag, ScriptTag, TextDecoration,
    TextRange,
};

/// A complete inline style (spec FE-25): everything a shaping run takes from
/// its caller. A style is never partial; nothing cascades between styles.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    /// Ordered family stack.
    pub families: FamilyStack,
    /// Font size in layout units; finite and positive.
    pub font_size: f32,
    /// Weight and slant.
    pub style: FontStyle,
    /// OpenType features and variation coordinates. Canonical as
    /// built (spec FNT-2).
    pub features: FontFeatures,
    /// Underline and strike-through.
    pub decoration: TextDecoration,
    /// Alignment relative to the paragraph's main baseline.
    pub baseline_align: BaselineAlignment,
    /// Extra advance after each eligible cluster; a measurement.
    pub letter_spacing: f32,
    /// Extra advance of each word space; a measurement.
    pub word_spacing: f32,
    /// Explicit script, which stands over script resolution (spec FE-13).
    pub script: Option<ScriptTag>,
    /// Explicit locale, which wins over `FrontendOptions::default_locale`
    /// (spec SCO-5).
    pub locale: Option<LocaleTag>,
}

/// A stretch of text in one [`TextStyle`] (spec FE-25).
#[derive(Clone, Debug, PartialEq)]
pub struct StyleSpan {
    /// Bytes the style applies to: non-empty, on character boundaries.
    pub byte_range: TextRange,
    /// The style of those bytes.
    pub style: TextStyle,
}

/// An inline object the caller measures and draws (spec FE-27, WN-7).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InlineObjectInput {
    /// The bytes that stand for the object, usually one U+FFFC; never empty.
    pub byte_range: TextRange,
    /// Caller-assigned identifier, returned on the positioned box.
    pub box_id: u64,
    /// Inline extent; finite and non-negative.
    pub width: f32,
    /// Extent towards line-over; finite and non-negative.
    pub ascent: f32,
    /// Extent towards line-under; finite and non-negative.
    pub descent: f32,
    /// Alignment relative to the paragraph's main baseline.
    pub baseline_align: BaselineAlignment,
    /// What happens when the object is wider than its line.
    pub overflow_policy: BoxOverflowPolicy,
}

/// A ruby or emphasis annotation before shaping (spec FE-27, §2.9).
#[derive(Clone, Debug, PartialEq)]
pub struct AnnotationInput {
    /// Caller-assigned identifier, unique among annotations.
    pub annotation_id: u64,
    /// The base text the annotation attaches to.
    pub base_byte_range: TextRange,
    /// The side of the line it sits on.
    pub side: AnnotationSide,
    /// Ruby or emphasis.
    pub kind: AnnotationKind,
    /// The annotation's own text, shaped on its own.
    pub text: Box<str>,
    /// The style the annotation text is shaped in.
    pub style: TextStyle,
}

/// Everything the frontend compiles into one `ParagraphAst` (spec FE-25).
///
/// `text` is one paragraph (spec AST-2). Every span stream is in ascending
/// byte order, with offsets local to `text`.
#[derive(Clone, Copy, Debug)]
pub struct ParagraphInput<'a> {
    /// The paragraph text.
    pub text: &'a str,
    /// The style of every byte no [`StyleSpan`] covers, and the source of the
    /// paragraph strut, so that an empty paragraph has one too (spec AST-6).
    pub default_style: &'a TextStyle,
    /// Styled stretches: non-empty, ascending and disjoint.
    pub style_spans: &'a [StyleSpan],
    /// Inline objects: ascending and disjoint.
    pub inline_objects: &'a [InlineObjectInput],
    /// Decoration spans, as stored in `ParagraphAst::decoration_spans`.
    pub decoration_spans: &'a [InlineDecorationSpan],
    /// Atomic edit spans, as stored in `ParagraphAst::atomic_spans`.
    pub atomic_spans: &'a [AtomicEditSpan],
    /// Explicit bidi spans, as stored in `ParagraphAst::bidi_spans`.
    pub bidi_spans: &'a [ExplicitBidiSpan],
    /// Annotations to shape and attach.
    pub annotations: &'a [AnnotationInput],
}
