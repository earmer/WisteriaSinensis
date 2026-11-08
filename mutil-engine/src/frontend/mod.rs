//! Frontend: input abstraction and the paragraph AST (spec §3, §2.4, §2.5).
//!
//! Phase 0 defines the output and option types only; no analysis pass is
//! implemented.

mod ast;
mod options;
mod run;
mod span;

pub use ast::{
    BoxNode, BoxOverflowPolicy, BreakAction, DiscretionaryNode, GlueKind, GlueNode, GlueTier,
    HyphenOpportunity, ParagraphAst, ReplacementText, SpanEdge, WordNode,
};
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
    pub content: crate::types::TextRange,
    /// Following separator, or an empty range at document end.
    pub separator: crate::types::TextRange,
}
/// Splits document text, preserving empty paragraphs and source offsets.
/// Paragraph separators are exactly UAX #9: `\n`, `\r`, `\r\n`, U+0085,
/// U+000B, U+000C, U+2029. U+2028 is not a separator. Empty input yields one
/// empty slice; trailing separator yields a final empty slice.
pub fn split_paragraphs(text: &str) -> Vec<ParagraphSlice> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut content_start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let (len, is_sep) = match bytes[i] {
            b'\n' => (1, true),
            b'\r' => {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    (2, true)
                } else {
                    (1, true)
                }
            }
            0x0B | 0x0C => (1, true),
            0xC2 if i + 1 < bytes.len() && bytes[i + 1] == 0x85 => (2, true),
            0xE2 if i + 2 < bytes.len() && bytes[i + 1] == 0x80 && bytes[i + 2] == 0xA9 => {
                (3, true)
            }
            _ => (1, false),
        };
        if is_sep {
            out.push(ParagraphSlice {
                content: crate::types::TextRange::new(content_start, i),
                separator: crate::types::TextRange::new(i, i + len),
            });
            i += len;
            content_start = i;
        } else {
            // advance by one UTF-8 code point
            let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            i += ch_len;
        }
    }
    out.push(ParagraphSlice {
        content: crate::types::TextRange::new(content_start, text.len()),
        separator: crate::types::TextRange::empty_at(text.len()),
    });
    out
}
