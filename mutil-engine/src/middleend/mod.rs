//! Middle-end: layout output containers, interaction types and layout options
//! (spec §2.6, §5, §6).
//!
//! Phase 0 defines data only; line breaking, reordering and justification
//! arrive in later phases.

mod interaction;
mod options;
mod output;

pub use interaction::{
    BoxHeightStyle, CaretAffinity, CaretStop, CaretVisual, CursorPosition, HitResult, Motion,
    Selection, SelectionRect,
};
pub use options::{
    HangMode, HangingWhitespaceMode, LineBreakStrategy, LineHeight, SoftHyphenPolicy,
};
pub use output::{
    DecorationKind, DecorationLine, HangSide, LINE_GEOMETRY_EPSILON, LineInvariantError,
    PositionedGlyph, PositionedLine, PositionedParagraph, VisualGlyphRun,
};
