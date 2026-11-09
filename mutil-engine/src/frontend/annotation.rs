//! Explicit directional spans and independently shaped interlinear annotations.
use super::ShapingRun;
use crate::types::{BidiDirection, CompactString, GraphemeAtom, ShapedCluster, TextRange};
/// Maximum nesting of source-level directional spans.
pub const MAX_BIDI_SPAN_DEPTH: usize = 63;
/// Source-level directional control pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BidiControlKind {
    /// FSI/LRI/RLI through PDI.
    Isolate(Option<BidiDirection>),
    /// LRO/RLO through PDF.
    Override(BidiDirection),
}
/// Nonempty, properly nested directional-control span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitBidiSpan {
    /// Unique identifier.
    pub span_id: u64,
    /// Source byte range aligned to concrete content boundaries.
    pub byte_range: TextRange,
    /// Control pair.
    pub control: BidiControlKind,
}
/// Physical side of an interlinear annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnnotationSide {
    /// Above horizontal text, right of either vertical mode.
    #[default]
    Over,
    /// Below horizontal text, left of either vertical mode.
    Under,
}
/// Distribution of ruby glyphs over their base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AnnotationAlignment {
    /// Center the annotation.
    #[default]
    Center,
    /// Equal surrounding spaces.
    SpaceAround,
    /// Equal internal spaces.
    SpaceBetween,
    /// Align to inline start.
    Start,
}
/// Annotation payload policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AnnotationKind {
    /// One indivisible ruby group.
    Ruby {
        /// Glyph distribution.
        alignment: AnnotationAlignment,
        /// Maximum finite overhang beyond the base, within its slot.
        max_overhang: f32,
    },
    /// Repeat one shaped mark over eligible atoms.
    Emphasis,
}
/// Independently shaped annotation with local byte and atom coordinates.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationSpan {
    /// Unique identifier.
    pub annotation_id: u64,
    /// Base source range, aligned to atoms and clusters.
    pub base_byte_range: TextRange,
    /// Line-over or line-under side.
    pub side: AnnotationSide,
    /// Ruby or emphasis policy.
    pub kind: AnnotationKind,
    /// Additional block extent, including the gap.
    pub block_extent: f32,
    /// Nonempty annotation source without line/paragraph separators.
    pub annotation_text: CompactString,
    /// Shaping settings in annotation-local coordinates.
    pub run: ShapingRun,
    /// Local atoms; `run.atom_range` is `0..atoms.len()`.
    pub atoms: Vec<GraphemeAtom>,
    /// Local clusters partitioning the atoms, with `atom_range`s into `atoms`;
    /// `run.cluster_range` is `0..clusters.len()`.
    pub clusters: Vec<ShapedCluster>,
}
