//! Abstract shaping bridge, its empty implementation and the font source
//! renderers resolve faces through (spec §4, §8).

mod font_source;
mod noop;
mod profile;
mod traits;

pub use font_source::{FontBytes, FontFace, FontSource};
pub use noop::NoopShaper;
pub use profile::ShaperProfile;
pub use traits::{LeadingEdit, TextShaper, TrailingEdit};
