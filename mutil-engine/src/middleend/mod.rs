//! Middle-end: layout output containers, interaction types and layout options
//! (spec §2.6, §5, §6).
//!
//! Phase 0 defines data only; line breaking, reordering and justification
//! arrive in later phases.

mod interaction;
mod layout;
mod options;
mod output;

pub use interaction::{
    BoxHeightStyle, CaretAffinity, CaretStop, CaretVisual, CursorPosition, HitResult, Motion,
    Selection, SelectionRect,
};
pub use layout::{LayoutInputError, check_layout_inputs, layout_paragraph};
pub use options::{
    HangEndMode, HangMode, HangStartMode, HangingWhitespaceMode, LayoutConstraints, LayoutOptions,
    LineBreakStrategy, LineHeight, SoftHyphenPolicy, TabStops,
};
pub use output::{
    DecorationKind, DecorationLine, HangSide, LINE_GEOMETRY_ABS_EPSILON, LINE_GEOMETRY_REL_EPSILON,
    LineContext, LineInvariantError, PositionedGlyph, PositionedInlineBox, PositionedLine,
    PositionedParagraph, PositionedSpanFragment, PositionedTab, VisualGlyphRun,
};
