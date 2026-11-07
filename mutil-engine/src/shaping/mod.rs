//! Abstract shaping bridge and its empty implementation (spec §4).

mod noop;
mod traits;

pub use noop::NoopShaper;
pub use traits::{BoundaryEdit, TextShaper};
