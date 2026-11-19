//! Public policy defaults.
use mutil_engine::*;

// These defaults are externally specified policy, not representation checks.
#[test]
fn public_policy_defaults_match_the_contract() {
    assert_eq!(AutospaceProfile::default(), AutospaceProfile::Jlreq);
    assert_eq!(
        HangingWhitespaceMode::default(),
        HangingWhitespaceMode::Collapsible
    );
    assert_eq!(SoftHyphenPolicy::default(), SoftHyphenPolicy::Keep);
    assert_eq!(ShaperProfile::default(), ShaperProfile::FullContract);
    assert_eq!(LineBreakStrategy::default(), LineBreakStrategy::Balanced);
}
