//! Frontend: input abstraction and the paragraph AST (spec §3, §2.4, §2.5).
//!
//! Defines the input, output and option types and the entry point
//! [`compile_paragraph`], whose Phase 1 analysis compiles horizontal
//! left-to-right text (spec FE-29). The AST stores atoms and clusters once per
//! paragraph; text boxes and runs name slices of them by range.

mod annotation;
mod ast;
pub(crate) mod bidi;
mod compile;
#[cfg(test)]
mod conformance;
mod emoji;
mod input;
mod locale;
mod options;
mod script;
mod segment;
mod spacing;
mod span;
mod url;

use crate::types::TextRange;
// The run and the hyphenation opportunity are defined in `types`, below the
// shaping bridge (spec SCO-6); they stay part of the frontend's vocabulary.
pub use crate::types::{HyphenOpportunity, ReplacementText, ShapingRun, TextDecoration};

pub use annotation::{
    AnnotationAlignment, AnnotationKind, AnnotationSide, AnnotationSpan, BidiControlKind,
    ExplicitBidiSpan, MAX_BIDI_SPAN_DEPTH,
};
pub(crate) use ast::text_is_self_contained;
pub use ast::{
    BoxNode, BoxOverflowPolicy, BreakAction, DiscretionaryNode, GlueKind, GlueNode, GlueTier,
    InlineObjectBox, ParagraphAst, RepeatedOperator, ReshapeWindow, SpanEdge, TextBoxView,
    WordNode,
};
pub use compile::{FrontendError, FrontendFeature, compile_paragraph};
pub use input::{AnnotationInput, InlineObjectInput, ParagraphInput, StyleSpan, TextStyle};
pub use options::{
    AutospaceProfile, FrontendOptions, LineBreakStrictness, LocalePolicy, QuotePolicy,
    WordBreakOption,
};
pub(crate) use segment::word_boundaries;
pub use span::{
    AtomicEditSpan, BoxDecorationBreak, CompositionSpan, InlineDecorationSpan,
    MAX_DECORATION_NESTING_DEPTH,
};

/// Source-document byte ranges for one paragraph and its following separator.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParagraphSlice {
    /// Paragraph content; U+2028 remains within this range.
    pub content: TextRange,
    /// Following separator, or an empty range at document end.
    pub separator: TextRange,
}

/// `true` for a character that ends a paragraph.
///
/// The UAX #9 class B characters are `\n`, `\r`, U+0085, U+2029 and
/// U+001C–U+001E. U+000B and U+000C are application-policy separators (their
/// bidi classes are `S` and `WS`), included so that a document split treats
/// them like the other line/paragraph terminators.
#[must_use]
pub fn is_paragraph_separator(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{85}' | '\u{2029}' | '\u{1c}'..='\u{1e}' | '\u{b}' | '\u{c}'
    )
}

/// `true` for a paragraph separator or the line separator U+2028: any
/// character that may not appear inside a single-line payload such as an
/// annotation text or a clamp ellipsis.
#[must_use]
pub fn is_line_or_paragraph_separator(c: char) -> bool {
    is_paragraph_separator(c) || c == '\u{2028}'
}

/// `true` for a character a [`GlueKind::WordSpace`] may own: the Unicode space
/// separators (`General_Category=Zs`) except the ideographic space U+3000 and
/// the no-break spaces ([`is_no_break_space`]). Tabs, line and paragraph
/// separators are not word spaces; they have nodes of their own.
///
/// A no-break space is a fixed-width character that never ends a line (UAX #14
/// class GL), so the frontend shapes it as ordinary text in a
/// [`BoxNode::Text`] with no break opportunity on either side (spec WN-12): a
/// line's visible content then always ends on a glyph, a box or a tab.
///
/// U+3000 is a fixed full-width character in CJK typography (JLReq class
/// cl-14), neither stretched by justification nor collapsed, so the frontend
/// shapes it as ordinary text in a [`BoxNode::Text`]
/// followed by a break opportunity (UAX #14 class BA, break after).
#[must_use]
pub fn is_word_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\u{1680}' | '\u{2000}'..='\u{2006}' | '\u{2008}'..='\u{200a}' | '\u{205f}'
    )
}

/// `true` for the space separators that never collapse and never offer a
/// break: NBSP U+00A0, figure space U+2007 and narrow NBSP U+202F. They are
/// text, not glue (spec WN-12): [`is_word_space`] is `false` for them.
#[must_use]
pub fn is_no_break_space(c: char) -> bool {
    matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}')
}

/// Splits document text, preserving empty paragraphs and source offsets.
/// Paragraph separators are [`is_paragraph_separator`] characters; a CRLF
/// pair is one separator. U+2028 is not a separator. Empty input yields one
/// empty slice; trailing separator yields a final empty slice.
pub fn split_paragraphs(text: &str) -> Vec<ParagraphSlice> {
    let mut paragraphs = Vec::new();
    let mut content_start = 0usize;
    let mut iter = text.char_indices().peekable();

    while let Some((offset, c)) = iter.next() {
        if !is_paragraph_separator(c) {
            continue;
        }
        let mut sep_end = offset + c.len_utf8();
        if c == '\r' && iter.peek().is_some_and(|&(_, next)| next == '\n') {
            iter.next();
            sep_end += 1;
        }
        paragraphs.push(ParagraphSlice {
            content: TextRange::new(content_start, offset),
            separator: TextRange::new(offset, sep_end),
        });
        content_start = sep_end;
    }

    paragraphs.push(ParagraphSlice {
        content: TextRange::new(content_start, text.len()),
        separator: TextRange::empty_at(text.len()),
    });
    paragraphs
}
