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

/// `v` when it is finite and not negative, else zero.
fn finite_nonnegative(v: f32) -> f32 {
    if v.is_finite() { v.max(0.0) } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::finite_nonnegative;

    #[test]
    fn finite_nonnegative_sanitizes_non_finite_values() {
        assert_eq!(finite_nonnegative(-2.5), 0.0);
        assert_eq!(finite_nonnegative(3.5), 3.5);
        assert_eq!(finite_nonnegative(f32::NEG_INFINITY), 0.0);
    }
}
