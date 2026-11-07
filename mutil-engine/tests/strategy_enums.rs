//! The five strategy/profile enums derive the full set of standard traits and
//! carry the defaults the spec names.

use mutil_engine::*;
use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;

fn assert_value_type<T>(variants: &[T], default: T)
where
    T: Debug + Clone + Copy + PartialEq + Eq + Hash + Default + Send + Sync + 'static,
{
    assert_eq!(T::default(), default);
    // Hash + Eq: all variants are distinct set members; Copy: usable after move.
    let set: HashSet<T> = variants.iter().copied().collect();
    assert_eq!(set.len(), variants.len());
    for &v in variants {
        let copy = v;
        assert_eq!(copy.clone(), v);
        assert!(!format!("{v:?}").is_empty());
    }
    // One byte: fieldless enums with at most 256 variants.
    assert_eq!(core::mem::size_of::<T>(), 1);
}

#[test]
fn autospace_profile() {
    assert_value_type(
        &[AutospaceProfile::Jlreq, AutospaceProfile::CssCompat],
        AutospaceProfile::Jlreq,
    );
}

#[test]
fn hanging_whitespace_mode() {
    assert_value_type(
        &[
            HangingWhitespaceMode::Collapsible,
            HangingWhitespaceMode::Preserved,
        ],
        HangingWhitespaceMode::Collapsible,
    );
}

#[test]
fn line_break_strategy() {
    assert_value_type(
        &[
            LineBreakStrategy::Greedy,
            LineBreakStrategy::Balanced,
            LineBreakStrategy::Publication,
        ],
        LineBreakStrategy::Balanced,
    );
}

#[test]
fn soft_hyphen_policy() {
    assert_value_type(
        &[SoftHyphenPolicy::Keep, SoftHyphenPolicy::Strip],
        SoftHyphenPolicy::Keep,
    );
}

#[test]
fn shaper_profile() {
    assert_value_type(
        &[ShaperProfile::FullContract, ShaperProfile::LineOnly],
        ShaperProfile::FullContract,
    );
}
