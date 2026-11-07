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
pub use options::{HangMode, LineHeight};
pub use output::{
    DecorationKind, DecorationLine, PositionedGlyph, PositionedLine, PositionedParagraph,
    VisualGlyphRun,
};
