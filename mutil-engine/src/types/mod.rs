//! Pure-logic primitives, geometry and the shared data model.
//!
//! This layer depends on nothing else in the crate. The shaping, frontend and
//! middle-end layers all build on it, which keeps the module graph acyclic
//! (see the layer diagram at the crate root).

mod atom;
mod bidi;
mod cluster;
mod font;
mod geometry;
mod hyphen;
mod run;
mod tag;
mod text;
mod writing;

pub use atom::{AtomFlags, GraphemeAtom, LineBreakClass};
pub use bidi::{BIDI_MAX_DEPTH, BidiClass, BidiDirection, BidiLevel, Direction};
pub use cluster::{RawShapedGlyph, ShapedCluster};
pub use font::{
    FamilyStack, FontFamily, FontFeature, FontFeatures, FontId, FontInstanceRef, FontMetrics,
    FontRef, FontSlant, FontStyle, FontVariation, FontWeight, GenericFamily,
};
pub use geometry::{MAX_MEASUREMENT, Point, Rect, Size, is_extent, is_measurement};
pub use hyphen::{HyphenOpportunity, ReplacementText};
pub use run::{ShapingRun, TextDecoration};
pub use tag::{InvalidLocaleTag, InvalidScriptTag, LocaleTag, ScriptTag};
pub use text::{ByteIndex, TextRange};
pub use writing::{BaselineAlignment, GlyphOrientation, TextOrientation, WritingMode};
