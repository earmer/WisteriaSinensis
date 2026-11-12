//! Frontend: input abstraction and the paragraph AST (spec §3, §2.4, §2.5).
//!
//! Defines the output and option types; the analysis pass that builds the AST
//! from text is not implemented yet. The AST stores atoms and clusters once per
//! paragraph; text boxes and runs name slices of them by range.

mod annotation;
mod ast;
mod options;
mod run;
mod span;

use crate::types::TextRange;

pub use annotation::{
    AnnotationAlignment, AnnotationKind, AnnotationSide, AnnotationSpan, BidiControlKind,
    ExplicitBidiSpan, MAX_BIDI_SPAN_DEPTH,
};
pub use ast::{
    BoxNode, BoxOverflowPolicy, BreakAction, DiscretionaryNode, GlueKind, GlueNode, GlueTier,
    HyphenOpportunity, ParagraphAst, RepeatedOperator, ReplacementText, ReshapeWindow, SpanEdge,
    TextBoxView, WordNode,
};
pub(crate) use ast::{atoms_byte_range, text_is_self_contained};
pub use options::{AutospaceProfile, FrontendOptions, LocalePolicy, QuotePolicy};
pub use run::{ShapingRun, TextDecoration};
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
/// separators (`General_Category=Zs`) except the ideographic space U+3000.
/// Tabs, line and paragraph separators are not word spaces; they have nodes of
/// their own.
///
/// U+3000 is a fixed full-width character in CJK typography (JLReq class
/// cl-14), neither stretched by justification nor collapsed, so the frontend
/// shapes it as ordinary text in a [`BoxNode::Text`]
/// followed by a break opportunity (UAX #14 class BA, break after).
#[must_use]
pub fn is_word_space(c: char) -> bool {
    matches!(
        c,
        ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}'
    )
}

/// `true` for the word spaces that never collapse and never offer a break:
/// NBSP U+00A0, figure space U+2007 and narrow NBSP U+202F.
#[must_use]
pub fn is_no_break_space(c: char) -> bool {
    matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}')
}

/// Splits document text, preserving empty paragraphs and source offsets.
/// Paragraph separators are [`is_paragraph_separator`] characters; a CRLF
/// pair is one separator. U+2028 is not a separator. Empty input yields one
/// empty slice; trailing separator yields a final empty slice.
pub fn split_paragraphs(text: &str) -> Vec<ParagraphSlice> {
    let bytes = text.as_bytes();
    let mut paragraphs = Vec::new();
    let mut content_start = 0usize;
    let mut offset = 0usize;

    while offset < bytes.len() {
        if let Some(separator_len) = separator_len_at(text, offset) {
            paragraphs.push(ParagraphSlice {
                content: TextRange::new(content_start, offset),
                separator: TextRange::new(offset, offset + separator_len),
            });
            offset += separator_len;
            content_start = offset;
            continue;
        }

        let char_len = text[offset..].chars().next().map_or(1, char::len_utf8);
        offset += char_len;
    }

    paragraphs.push(ParagraphSlice {
        content: TextRange::new(content_start, text.len()),
        separator: TextRange::empty_at(text.len()),
    });
    paragraphs
}

/// Length in bytes of the separator starting at `offset`, or `None` when the
/// character there is ordinary content. CRLF is a single two-byte separator.
fn separator_len_at(text: &str, offset: usize) -> Option<usize> {
    let c = text[offset..].chars().next()?;
    if !is_paragraph_separator(c) {
        return None;
    }
    if c == '\r' && text.as_bytes().get(offset + 1) == Some(&b'\n') {
        Some(2)
    } else {
        Some(c.len_utf8())
    }
}
