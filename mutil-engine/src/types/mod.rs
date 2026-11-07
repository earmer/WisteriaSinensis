//! Pure-logic primitives, geometry and the shared data model.
//!
//! This layer depends on nothing else in the crate. The frontend, shaping and
//! middle-end layers all build on it, which keeps the module graph acyclic:
//!
//! ```text
//! types  <-  frontend  <-  shaping
//!   ^           ^
//!   +-----  middleend
//! ```

mod atom;
mod bidi;
mod cluster;
mod font;
mod geometry;
mod tag;
mod text;

pub use atom::{AtomFlags, GraphemeAtom, LineBreakClass};
pub use bidi::{BIDI_MAX_DEPTH, BidiDirection, BidiLevel, Direction};
pub use cluster::{RawShapedGlyph, ShapedCluster};
pub use font::{
    FamilyStack, FontFamily, FontFeature, FontFeatures, FontId, FontMetrics, FontRef, FontSlant,
    FontStyle, FontWeight, GenericFamily, VerticalMetrics,
};
pub use geometry::{Point, Rect, Size};
pub use tag::{LocaleTag, ScriptTag};
pub use text::{ByteIndex, TextRange};
