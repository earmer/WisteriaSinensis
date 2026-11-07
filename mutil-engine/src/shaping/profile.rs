//! Runtime configuration of the shaping backend (spec §4.6).

/// Which shaping contract a backend honours.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ShaperProfile {
    /// Full contract: HarfRust, CoreText or DirectWrite called directly (or a
    /// patched GPUI). Clusters, break-safety flags, `locl` and ligature carets
    /// are all available.
    #[default]
    FullContract,
    /// Degraded GPUI `PlatformTextSystem` adapter (§4.6 (a)): clusters are
    /// reconstructed from glyph byte indices, `safe_to_break_before` is
    /// reported `false` conservatively at ambiguous seams, `locl` cannot be
    /// honoured and `ligature_carets` returns nothing.
    LineOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_contract_is_the_default() {
        assert_eq!(ShaperProfile::default(), ShaperProfile::FullContract);
    }
}
