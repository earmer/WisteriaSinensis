//! Abstract shaping bridge and its empty implementation (spec §4).

mod noop;
mod profile;
mod traits;

pub use noop::NoopShaper;
pub use profile::ShaperProfile;
pub use traits::{LeadingEdit, TextShaper, TrailingEdit};
