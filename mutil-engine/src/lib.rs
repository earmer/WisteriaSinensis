//! `mutil-engine`: headless, synchronous multilingual typesetting and layout engine (spec `SPECIFICATION.md`).
//!
//! Includes the data model (§2), `TextShaper` backends (§4), frontend (`compile_paragraph`, §3),
//! validators (§2.6), parameterised layout (`layout_paragraph`, `break_lines_parameterized`, §5),
//! clamping (`LineClampPolicy`), and interaction queries (§6). Hyphenation and annotations are pending.

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
/// sequences, bidi classes and paired brackets, and the word boundaries of
/// the word motions use ICU4X data of [`UNICODE_DATA_VERSION`].
pub fn conformance_report() -> ConformanceReport {
    ConformanceReport {
        grapheme: Some(UNICODE_DATA_VERSION),
        bidi: Some(UNICODE_DATA_VERSION),
        line_break: Some(UNICODE_DATA_VERSION),
        word: Some(UNICODE_DATA_VERSION),
        script: Some(UNICODE_DATA_VERSION),
        emoji: Some(UNICODE_DATA_VERSION),
    }
}
