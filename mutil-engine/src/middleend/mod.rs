//! Middle-end: options, validators, layout, and interaction (spec §2.6, §5, §6).

mod badness;
mod error_messages;
mod input_validation;
mod interaction;
mod layout;
mod line_geometry;
mod options;
mod output;
mod output_validation;
mod shaper_gate;
mod tolerance;
mod validated;

pub use badness::{
    CAPACITY_EPSILON, FIT_EPSILON, LineFit, MAX_BADNESS, OVERFULL_BADNESS, OVERFULL_RATIO,
    TierCapacities, UNDERFULL_BADNESS_CAP, line_fit,
};
pub use input_validation::{LayoutInputError, check_layout_inputs};
pub use interaction::{
    CaretVisual, CursorMove, CursorPosition, FlowMotion, HitResult, Motion, ParagraphEdge,
    Selection, SelectionRect, resolve_ligature_carets, utf8_to_utf16, utf16_to_utf8,
};
pub use layout::{break_lines_parameterized, layout_paragraph};
pub use line_geometry::GlyphRole;
pub use options::{
    CjkPunctuationTrim, HangEndMode, HangMode, HangStartMode, HangingWhitespaceMode,
    InlineSlotConstraint, LayoutConstraints, LayoutOptions, LineBreakStrategy, LineClampPolicy,
    LineGeometry, LineGeometryProvider, LineHeight, SoftHyphenPolicy, StrutMetrics, TabStops,
};
pub use output::{
    AtomicFragment, CaretAffinity, CaretStop, DecorationKind, DecorationLine, HangSide,
    LayoutPolicySnapshot, LineContext, PositionedAnnotationRun, PositionedGlyph,
    PositionedInlineBox, PositionedLine, PositionedParagraph, PositionedSlot,
    PositionedSpanFragment, PositionedTab, SlotSource, SpanEdgeCut, SpanFragmentEdges,
    VisualGlyphRun,
};
pub use output_validation::{
    AnnotationErrorReason, LineInvariantError, ParagraphErrorReason, SlotErrorReason,
};
pub use tolerance::{LINE_GEOMETRY_ABS_EPSILON, LINE_GEOMETRY_REL_EPSILON};
pub use validated::{
    Clearance, GeometryAnswer, OpenGeometry, PlacedGlyph, PlacedRun, PlacedRunSource,
    ValidatedInputs, ValidatedLayout,
};
