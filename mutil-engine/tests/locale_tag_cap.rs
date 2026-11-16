//! The 128-byte cap on locale tags (spec TYP-4, TYP-7): a tag of exactly
//! 128 bytes is accepted, a longer one is rejected, and an oversized host
//! string is rejected before anything is allocated.
//!
//! This file is its own test binary so that it can count allocations with a
//! global allocator without affecting the other suites.

use mutil_engine::{InvalidLocaleTag, LocaleTag};
use stats_alloc::{INSTRUMENTED_SYSTEM, Region, StatsAlloc};
use std::alloc::System;
use std::sync::{Mutex, MutexGuard};

#[global_allocator]
static GLOBAL: &StatsAlloc<System> = &INSTRUMENTED_SYSTEM;

/// Serialises the tests of this binary: the allocation counters are global,
/// so no other test may allocate while one is measuring.
static SERIAL: Mutex<()> = Mutex::new(());

/// Holds `SERIAL` for the rest of a test, even after another test panicked.
fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// TYP-4's cap, in bytes.
const CAP: usize = 128;

/// The size of the oversized inputs of the allocation test.
const HUGE: usize = 4 * 1024 * 1024;

/// The most bytes a rejection of an oversized input may allocate: far below
/// the input's size.
const ALLOCATION_ALLOWANCE: usize = 1024;

/// A well-formed private-use tag (`x-` then `-`-separated alphanumeric
/// subtags of 1 to 8 characters) of exactly `len` bytes.
fn private_use_tag(len: usize) -> String {
    assert!(
        len >= 3,
        "a private-use tag needs at least `x-` and one subtag"
    );
    let mut tag = String::from("x");
    while tag.len() < len {
        let remaining = len - tag.len() - 1;
        // Never leave a single byte, which could not hold `-` and a subtag.
        let subtag = match remaining {
            9 => 7,
            r => r.min(8),
        };
        tag.push('-');
        tag.extend((0..subtag).map(|i| char::from(b'a' + (i % 26) as u8)));
    }
    assert_eq!(tag.len(), len, "the generated tag has the requested length");
    tag
}

/// TYP-4: a well-formed tag of exactly 128 bytes is accepted by
/// `try_from_str`, `FromStr` and `parse_lenient`, and kept as given.
#[test]
fn a_tag_of_exactly_128_bytes_is_accepted() {
    let _serial = serial();
    let tag = private_use_tag(CAP);
    let strict = LocaleTag::try_from_str(&tag).expect("a 128-byte tag is accepted");
    assert_eq!(strict.as_str(), tag, "the tag is stored as given");
    let parsed: LocaleTag = tag.parse().expect("`FromStr` accepts a 128-byte tag");
    assert_eq!(parsed, strict, "`FromStr` agrees with `try_from_str`");
    let lenient = LocaleTag::parse_lenient(&tag).expect("a 128-byte tag is accepted leniently");
    assert_eq!(
        lenient, strict,
        "`parse_lenient` agrees with `try_from_str`"
    );
}

/// TYP-4, TYP-7: a well-formed-shaped tag of 129 bytes is rejected by both
/// constructors.
#[test]
fn a_tag_of_129_bytes_is_rejected() {
    let _serial = serial();
    let tag = private_use_tag(CAP + 1);
    assert_eq!(
        LocaleTag::try_from_str(&tag),
        Err(InvalidLocaleTag),
        "`try_from_str` rejects a 129-byte tag"
    );
    assert_eq!(
        tag.parse::<LocaleTag>(),
        Err(InvalidLocaleTag),
        "`FromStr` rejects a 129-byte tag"
    );
    assert_eq!(
        LocaleTag::parse_lenient(&tag),
        Err(InvalidLocaleTag),
        "`parse_lenient` rejects a 129-byte tag"
    );
}

/// TYP-7: `parse_lenient` measures the length after trimming, so surrounding
/// white space does not count towards the cap.
#[test]
fn parse_lenient_measures_the_trimmed_input() {
    let _serial = serial();
    let tag = private_use_tag(CAP);
    let padded = format!("   {tag}  \t");
    assert!(padded.len() > CAP, "the padded input exceeds the cap");
    let lenient = LocaleTag::parse_lenient(&padded).expect("the trimmed tag fits the cap");
    assert_eq!(lenient.as_str(), tag, "the white space is trimmed");
}

/// TYP-7: an oversized input holding `_` is rejected by `parse_lenient`.
#[test]
fn parse_lenient_rejects_an_oversized_input_with_underscores() {
    let _serial = serial();
    let input = "ja_JP_".repeat(CAP / 6 + 2);
    assert!(input.len() > CAP, "the input exceeds the cap");
    assert_eq!(
        LocaleTag::parse_lenient(&input),
        Err(InvalidLocaleTag),
        "an oversized input with `_` is rejected"
    );
}

/// A locale-tag constructor under test.
type Constructor = fn(&str) -> Result<LocaleTag, InvalidLocaleTag>;

/// The bytes `parse` allocates while rejecting `input`, which must be
/// rejected.
fn bytes_allocated_rejecting(input: &str, parse: Constructor) -> usize {
    let region = Region::new(GLOBAL);
    let result = parse(input);
    let allocated = region.change().bytes_allocated;
    assert_eq!(
        result,
        Err(InvalidLocaleTag),
        "the oversized input is rejected"
    );
    allocated
}

/// TYP-4, TYP-7: rejecting a multi-megabyte input allocates next to nothing,
/// with or without `_`, through either constructor. Every measurement is in
/// this one test because the allocation counters are global.
#[test]
fn an_oversized_input_is_rejected_before_anything_is_allocated() {
    let _serial = serial();
    let hyphenated: String = "abcdefgh-".repeat(HUGE / 9);
    let hyphenated = format!("x-{hyphenated}a");
    let underscored = hyphenated.replace('-', "_");
    assert!(hyphenated.len() >= HUGE / 2, "the input is multi-megabyte");
    let constructors: [(&str, Constructor); 2] = [
        ("try_from_str", LocaleTag::try_from_str),
        ("parse_lenient", LocaleTag::parse_lenient),
    ];
    for (name, parse) in constructors {
        for (shape, input) in [("`-`", &hyphenated), ("`_`", &underscored)] {
            let allocated = bytes_allocated_rejecting(input, parse);
            assert!(
                allocated < ALLOCATION_ALLOWANCE,
                "`{name}` allocated {allocated} bytes rejecting a {}-byte input with {shape}",
                input.len()
            );
        }
    }
}
