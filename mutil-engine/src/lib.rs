//! `mutil-engine`: a headless, synchronous, GUI-free multilingual typesetting and
//! interactive layout engine.
//!
//! The crate contains the data model (§2), the `TextShaper` contract and
//! backends (`HarfRustShaper`, `LineOnlyShaper`, `NoopShaper`, §4), the
//! Phase 1 frontend (`compile_paragraph`, §3), input and output validators
//! (§2.6), greedy line breaking with per-line L1/L2 bidi reordering and
//! vertical stacking (`layout_paragraph`, §5), and the Phase 1 interaction
//! queries (§6). Paragraph-level bidi analysis, justification, hyphenation,
//! annotations, clamping and the optimising breakers are not implemented yet.
//!
//! Layers (dependencies point left or up; the frontend calls the shaping
//! bridge through the `TextShaper` trait):
//!
//! ```text
//! types  <-  shaping  <-  frontend
//!   ^           ^           ^
//!   +-----  middleend  -----+
//! ```
//!
//! Everything public is re-exported at the crate root.
//!
//! The normative contract is `SPECIFICATION.md` at the repository root;
//! `§` references in this crate's documentation are to its sections, and
//! `RESEARCH.md` holds the evidence behind them under the same numbers.
//! `scripts/check_spec.py` checks in CI that every type, field, variant and
//! member the specification names exists here under that name.

pub mod frontend;
pub mod middleend;
pub mod shaping;
pub mod types;

pub use frontend::*;
pub use middleend::*;
pub use shaping::*;
pub use types::*;

/// The Unicode version of the data every Unicode component of the engine is
/// built on (spec SCO-8): the compiled data of `icu_segmenter` and
/// `icu_properties`, which the lock file pins to one release.
pub const UNICODE_DATA_VERSION: &str = "18.0.0";

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
    /// Script and script-extension (UAX #24) Unicode data version.
    pub script: Option<&'static str>,
    /// Emoji property (UTS #51) data version.
    pub emoji: Option<&'static str>,
}

/// Reports installed components rather than claiming target-version
/// conformance: graphemes, line-break opportunities, scripts, emoji
/// sequences and the word boundaries of the word motions use ICU4X data of
/// [`UNICODE_DATA_VERSION`]; bidi resolution is not wired in yet.
pub fn conformance_report() -> ConformanceReport {
    ConformanceReport {
        grapheme: Some(UNICODE_DATA_VERSION),
        bidi: None,
        line_break: Some(UNICODE_DATA_VERSION),
        word: Some(UNICODE_DATA_VERSION),
        script: Some(UNICODE_DATA_VERSION),
        emoji: Some(UNICODE_DATA_VERSION),
    }
}
