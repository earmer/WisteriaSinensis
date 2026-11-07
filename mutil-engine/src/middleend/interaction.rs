//! Draft interactive types: caret and selection geometry (spec §6.1, §6.6).
//!
//! Only the three types the roadmap names, plus [`CaretAffinity`] and what they
//! need. `HitResult`, `CaretVisual`, `Motion`, `Selection` and `BoxHeightStyle`
//! arrive with the queries that use them.

use crate::types::{BidiLevel, ByteIndex, Rect};

/// Which side of an ambiguous boundary a caret belongs to (spec §6.1).
///
/// Flutter's definition: upstream leans toward the start of the string. At a
/// soft wrap, upstream is the end of the first line and downstream the start of
/// the second; affinity also resolves a bidi boundary. Explicit newlines are
/// unambiguous `[C1]`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretAffinity {
    /// Leans toward the following text.
    #[default]
    Downstream,
    /// Leans toward the preceding text.
    Upstream,
}

/// A caret location: offset plus affinity (spec §6.1).
///
/// Invariant: `byte_offset` is a grapheme atom boundary. Deletion ranges are
/// not so constrained (§6.5), but they are not carets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CursorPosition {
    /// Atom-aligned byte offset.
    pub byte_offset: ByteIndex,
    /// Affinity at wraps and bidi boundaries.
    pub affinity: CaretAffinity,
}

/// One visual boundary on a line, in visual order (spec §6.1). Two stops sit at
/// each direction boundary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaretStop {
    /// Line-local x position.
    pub x: f32,
    /// Logical byte offset at this stop.
    pub offset: ByteIndex,
    /// Affinity this stop represents.
    pub affinity: CaretAffinity,
    /// Bidi level of the slot.
    pub level: BidiLevel,
    /// Index into `PositionedParagraph::lines`.
    pub line: usize,
}

/// One visual piece of a selection (spec §6.6).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SelectionRect {
    /// Rectangle in paragraph coordinates.
    pub rect: Rect,
    /// Index into `PositionedParagraph::lines`.
    pub line: usize,
    /// Bidi level of the covered slots.
    pub level: BidiLevel,
    /// `true` for the optional extension drawn for a selected newline.
    pub is_line_end_extension: bool,
}
