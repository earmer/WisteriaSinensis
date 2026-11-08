//! Strategy parameters affect the solver's bounded search budget.
use mutil_engine::*;

#[test]
fn search_budgets_increase_with_quality() {
    let strategies = [
        LineBreakStrategy::Greedy,
        LineBreakStrategy::Balanced,
        LineBreakStrategy::Publication,
    ];
    assert_eq!(
        strategies.map(|s| (s.beam_width(), s.window_depth())),
        [(1, 1), (2, 2), (8, 6)]
    );
    for pair in strategies.windows(2) {
        assert!(pair[0].beam_width() < pair[1].beam_width());
        assert!(pair[0].window_depth() < pair[1].window_depth());
    }
}

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
