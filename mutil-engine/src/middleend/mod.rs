//! Middle-end: layout output containers and interaction types (spec §2.6, §6).
//!
//! Phase 0 defines data only; line breaking, reordering and justification
//! arrive in later phases.

mod interaction;
mod output;

pub use interaction::{CaretAffinity, CaretStop, CursorPosition, SelectionRect};
pub use output::{
    DecorationKind, DecorationLine, PositionedGlyph, PositionedLine, PositionedParagraph,
    VisualGlyphRun,
};
