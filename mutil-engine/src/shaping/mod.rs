//! Abstract shaping bridge, the HarfRust backend, the `LineOnly` adapter,
//! the empty test double and the font source renderers resolve faces through
//! (spec §4, §8).

mod clusters;
mod font_source;
mod harfrust;
mod line_only;
mod noop;
mod profile;
mod traits;

pub use font_source::{FontBytes, FontFace, FontSource};
pub use harfrust::{CjkRegion, FaceProfile, FontRegistry, HarfRustShaper};
pub use line_only::{LineGlyph, LineOnlyShaper, LineShaper};
pub use noop::NoopShaper;
pub use profile::ShaperProfile;
pub use traits::{LeadingEdit, TextShaper, TrailingEdit};
