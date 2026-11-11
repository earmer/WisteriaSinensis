//! Middle-end: layout options, the validators of the layout inputs and
//! outputs, the output containers and the interaction types (spec §2.6, §5,
//! §6).
//!
//! Line breaking, reordering and justification are not implemented yet.

mod badness;
mod error_messages;
mod input_validation;
mod interaction;
mod layout;
mod options;
mod output;
mod output_validation;
mod tolerance;

pub use badness::{
    CAPACITY_EPSILON, FIT_EPSILON, LineFit, MAX_BADNESS, OVERFULL_BADNESS, OVERFULL_RATIO,
    TierCapacities, UNDERFULL_BADNESS_CAP, line_fit,
};
pub use input_validation::{LayoutInputError, check_layout_inputs};
pub use interaction::{
    BoxHeightStyle, CaretVisual, CursorPosition, FlowMotion, HitResult, Motion, Selection,
    SelectionRect, resolve_ligature_carets,
};
pub use layout::layout_paragraph;
pub use options::{
    HangEndMode, HangMode, HangStartMode, HangingWhitespaceMode, InlineSlotConstraint,
    LayoutConstraints, LayoutOptions, LineBreakStrategy, LineClampPolicy, LineGeometry,
    LineGeometryProvider, LineHeight, SoftHyphenPolicy, StrutMetrics, TabStops,
};
pub use output::{
    AtomicFragment, CaretAffinity, CaretStop, DecorationKind, DecorationLine, HangSide,
    LayoutPolicySnapshot, LineContext, PositionedAnnotationRun, PositionedGlyph,
    PositionedInlineBox, PositionedLine, PositionedParagraph, PositionedSlot,
    PositionedSpanFragment, PositionedTab, VisualGlyphRun,
};
pub use output_validation::{
    AnnotationErrorReason, LineInvariantError, ParagraphErrorReason, SlotErrorReason,
};
pub use tolerance::{LINE_GEOMETRY_ABS_EPSILON, LINE_GEOMETRY_REL_EPSILON};
