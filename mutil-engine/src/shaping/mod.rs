//! Abstract shaping bridge, its empty implementation, the HarfRust backend
//! skeleton and the font source renderers resolve faces through (spec §4, §8).

mod font_source;
mod harfrust;
mod noop;
mod profile;
mod traits;

pub use font_source::{FontBytes, FontFace, FontSource};
pub use harfrust::{FontRegistry, HarfRustShaper};
pub use noop::NoopShaper;
pub use profile::ShaperProfile;
pub use traits::{LeadingEdit, TextShaper, TrailingEdit};
