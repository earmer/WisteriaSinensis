//! Middle-end: layout output containers, interaction types and layout options
//! (spec §2.6, §5, §6).
//!
//! Phase 0 defines data only; line breaking, reordering and justification
//! arrive in later phases.

mod badness;
mod interaction;
mod layout;
mod options;
mod output;

pub use badness::{
    CAPACITY_EPSILON, FIT_EPSILON, LineFit, MAX_BADNESS, OVERFULL_BADNESS, OVERFULL_RATIO,
    TierCapacities, UNDERFULL_BADNESS_CAP, line_fit,
};
pub use interaction::{
    BoxHeightStyle, CaretVisual, CursorPosition, FlowMotion, HitResult, Motion, Selection,
    SelectionRect, resolve_ligature_carets,
};
pub use layout::{LayoutInputError, check_layout_inputs, layout_paragraph};
pub use options::{
    HangEndMode, HangMode, HangStartMode, HangingWhitespaceMode, InlineSlotConstraint,
    LayoutConstraints, LayoutOptions, LineBreakStrategy, LineClampPolicy, LineGeometry,
    LineGeometryProvider, LineHeight, SoftHyphenPolicy, TabStops,
};
pub use output::{
    AtomicFragment, CaretAffinity, CaretStop, DecorationKind, DecorationLine, HangSide,
    LINE_GEOMETRY_ABS_EPSILON, LINE_GEOMETRY_REL_EPSILON, LineContext, LineInvariantError,
    PositionedAnnotationRun, PositionedGlyph, PositionedInlineBox, PositionedLine,
    PositionedParagraph, PositionedSlot, PositionedSpanFragment, PositionedTab, VisualGlyphRun,
};
