//! Layout options (spec §5 Stage 1.3 and Stage 4): the layout policy
//! ([`LayoutOptions`]) with its strategy enums, hanging punctuation, the
//! line-height policy, and the container geometry ([`LayoutConstraints`]).

mod geometry;
mod hanging;
mod line_height;
mod policy;

pub(super) use geometry::slots_are_valid;
pub use geometry::{
    InlineSlotConstraint, LayoutConstraints, LineGeometry, LineGeometryProvider, TabStops,
};
pub use hanging::{HangEndMode, HangMode, HangStartMode};
pub use line_height::{LineHeight, StrutMetrics};
pub use policy::{
    HangingWhitespaceMode, LayoutOptions, LineBreakStrategy, LineClampPolicy, SoftHyphenPolicy,
};
