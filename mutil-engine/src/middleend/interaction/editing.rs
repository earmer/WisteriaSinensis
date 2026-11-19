//! Editing queries on validated inputs: deletion ranges, copied text and the
//! UTF-16 conversions (spec §6.5, INT-12, INT-18).

use super::ValidatedInputs;
use crate::types::{AtomFlags, ByteIndex, TextRange};

impl ValidatedInputs<'_> {
    /// The range Backspace deletes before `offset` (spec INT-12): a whole
    /// atomic span when the unit before the offset lies in one; else a whole
    /// emoji sequence, an atom with a variation selector or an inline object;
    /// else the last code point before the offset. Empty at the start of
    /// the text; an offset past the text or off a character boundary is
    /// first moved back to one.
    #[must_use]
    pub fn delete_backward_range(&self, offset: ByteIndex) -> TextRange {
        let text = self.text();
        let ast = self.ast();
        let offset = floor_boundary(text, offset);
        if offset == 0 {
            return TextRange::empty_at(0);
        }
        if let Some(unit) = anchored_unit_range(ast, offset - 1) {
            return unit;
        }
        let k = ast.atoms.partition_point(|a| a.byte_range.end() < offset);
        if let Some(atom) = ast.atoms.get(k).filter(|a| a.byte_range.end() == offset)
            && atom
                .flags
                .intersects(AtomFlags::IS_EMOJI_SEQUENCE.union(AtomFlags::HAS_VARIATION_SELECTOR))
        {
            return atom.byte_range;
        }
        let last = text[..offset].chars().next_back().map_or(1, char::len_utf8);
        TextRange::new(offset - last, offset)
    }

    /// The range Delete removes after `offset` (spec INT-12): a whole atomic
    /// span when the unit after the offset lies in one; else the whole atom
    /// or inline object after it; else the next code point. Empty at the end
    /// of the text.
    #[must_use]
    pub fn delete_forward_range(&self, offset: ByteIndex) -> TextRange {
        let text = self.text();
        let ast = self.ast();
        let offset = floor_boundary(text, offset);
        if offset >= text.len() {
            return TextRange::empty_at(text.len());
        }
        if let Some(unit) = anchored_unit_range(ast, offset) {
            return unit;
        }
        let k = ast.atoms.partition_point(|a| a.byte_range.start() < offset);
        if let Some(atom) = ast.atoms.get(k).filter(|a| a.byte_range.start() == offset) {
            return atom.byte_range;
        }
        let next = text[offset..].chars().next().map_or(1, char::len_utf8);
        TextRange::new(offset, offset + next)
    }

    /// The text a copy of `range` puts on the clipboard (spec INT-17): the
    /// source bytes, so no synthesized glyph is copied, with U+00AD dropped
    /// under `SoftHyphenPolicy::Strip`. The range is clamped to the text and
    /// to character boundaries.
    #[must_use]
    pub fn copied_text(&self, range: TextRange) -> String {
        let text = self.text();
        let start = floor_boundary(text, range.start());
        let end = floor_boundary(text, range.end()).max(start);
        let source = &text[start..end];
        match self.options().soft_hyphen {
            crate::middleend::SoftHyphenPolicy::Keep => source.to_owned(),
            crate::middleend::SoftHyphenPolicy::Strip => source.replace('\u{AD}', ""),
        }
    }
}

/// The atomic span holding `byte`, else the inline object holding it: the
/// units that editing removes whole (spec INT-12).
fn anchored_unit_range(ast: &crate::frontend::ParagraphAst, byte: ByteIndex) -> Option<TextRange> {
    ast.atomic_span_containing(byte)
        .map(|span| span.byte_range)
        .or_else(|| object_holding(ast, byte))
}

/// The range of the inline object holding `byte`, if any.
fn object_holding(ast: &crate::frontend::ParagraphAst, byte: ByteIndex) -> Option<TextRange> {
    let i = ast
        .items
        .partition_point(|item| item.byte_span().end() <= byte);
    match ast.items.get(i)? {
        crate::frontend::WordNode::Box(crate::frontend::BoxNode::InlineObject(
            crate::frontend::InlineObjectBox { byte_range, .. },
        )) if byte_range.contains(byte) => Some(*byte_range),
        _ => None,
    }
}

/// `offset` clamped to `text` and moved back to a character boundary.
fn floor_boundary(text: &str, offset: ByteIndex) -> ByteIndex {
    let mut at = offset.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The UTF-16 offset of byte `offset` of `text`, or `None` when the offset
/// lies past the text or inside a code point (spec INT-18).
#[must_use]
pub fn utf8_to_utf16(text: &str, offset: ByteIndex) -> Option<usize> {
    text.get(..offset)
        .map(|prefix| prefix.chars().map(char::len_utf16).sum())
}

/// The byte offset of UTF-16 offset `offset` of `text`, or `None` when it
/// lies past the text or between the two halves of a surrogate pair
/// (spec INT-18).
#[must_use]
pub fn utf16_to_utf8(text: &str, offset: usize) -> Option<ByteIndex> {
    let mut units = 0;
    for (byte, c) in text.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += c.len_utf16();
        if units > offset {
            return None;
        }
    }
    (units == offset).then_some(text.len())
}
