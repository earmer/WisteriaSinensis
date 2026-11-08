//! `mutil-engine`: a headless, synchronous, GUI-free multilingual typesetting and
//! interactive layout engine.
//!
//! **Phase 0** (this state of the crate) contains the strongly typed data model
//! of the specification (§2), the `TextShaper` contract (§4) and a `NoopShaper`
//! that proves the types close over the whole pipeline. It implements **no**
//! segmentation, shaping, line breaking or justification.
//!
//! Layers (dependencies point left):
//!
//! ```text
//! types  <-  frontend  <-  shaping
//!   ^           ^           ^
//!   +-----  middleend  -----+
//! ```
//!
//! Everything public is re-exported at the crate root.

pub mod frontend;
pub mod middleend;
pub mod shaping;
pub mod types;

pub use frontend::*;
pub use middleend::*;
pub use shaping::*;
pub use types::*;

/// Active component data version. None means the component is not implemented.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConformanceReport {
    /// Grapheme segmentation Unicode data version.
    pub grapheme: Option<&'static str>,
    /// Bidirectional algorithm Unicode data version.
    pub bidi: Option<&'static str>,
    /// Line segmentation Unicode data version.
    pub line_break: Option<&'static str>,
    /// Word segmentation Unicode data version.
    pub word: Option<&'static str>,
}
/// Reports installed components rather than claiming target-version conformance.
pub fn conformance_report() -> ConformanceReport {
    ConformanceReport {
        grapheme: None,
        bidi: None,
        line_break: None,
        word: None,
    }
}
