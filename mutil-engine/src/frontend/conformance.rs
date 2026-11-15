//! The Unicode conformance suites of the installed segmenters (spec RM-1):
//! `GraphemeBreakTest.txt`, `WordBreakTest.txt` and `LineBreakTest.txt` of
//! the pinned data version, in the comment-stripped form under
//! `tests/assets/ucd/` (provenance in `tests/assets/README.md`).
//!
//! Where ICU4X's stable rule tables predate the data version, the differing
//! cases are listed in `tests/assets/ucd/known-differences.txt`, and each
//! suite asserts that exact set: a fix and a regression both fail it, and the
//! failure prints the set to record.

use super::segment::{grapheme_boundaries, line_opportunities, word_boundaries};
use super::{LineBreakStrictness, WordBreakOption};
use std::collections::BTreeSet;

/// One test case: the text and the byte offsets the file marks with `÷`.
struct Case {
    line: usize,
    text: String,
    breaks: Vec<usize>,
}

fn load(name: &str) -> (String, Vec<Case>) {
    let path = format!("{}/tests/assets/ucd/{name}", env!("CARGO_MANIFEST_DIR"));
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut lines = data.lines();
    let header = lines.next().unwrap_or_default().to_owned();
    let cases = lines
        .enumerate()
        .map(|(i, line)| {
            let mut text = String::new();
            let mut breaks = Vec::new();
            for token in line.split_whitespace() {
                match token {
                    "÷" => breaks.push(text.len()),
                    "×" => {}
                    hex => text.push(
                        u32::from_str_radix(hex, 16)
                            .ok()
                            .and_then(char::from_u32)
                            .unwrap_or_else(|| panic!("{name}:{}: bad code point {hex}", i + 2)),
                    ),
                }
            }
            Case {
                line: i + 2,
                text,
                breaks,
            }
        })
        .collect();
    (header, cases)
}

/// The lines of the cases whose computed breaks differ from the file's.
fn failures(cases: &[Case], segment: impl Fn(&str) -> Vec<usize>) -> BTreeSet<usize> {
    cases
        .iter()
        .filter(|case| segment(&case.text) != case.breaks)
        .map(|case| case.line)
        .collect()
}

/// The cases `known-differences.txt` records for `file`, with the count its
/// section header states.
fn known_differences(file: &str) -> BTreeSet<usize> {
    let path = format!(
        "{}/tests/assets/ucd/known-differences.txt",
        env!("CARGO_MANIFEST_DIR")
    );
    let data = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let header = format!("[{file}] ");
    let mut lines = data.lines().skip_while(|l| !l.starts_with(&header));
    let count: usize = lines
        .next()
        .and_then(|l| l[header.len()..].parse().ok())
        .unwrap_or_else(|| panic!("{path}: no section for {file}"));
    let known: BTreeSet<usize> = lines
        .take_while(|l| !l.is_empty())
        .flat_map(str::split_whitespace)
        .map(|n| {
            n.parse()
                .unwrap_or_else(|_| panic!("{path}: bad line number {n}"))
        })
        .collect();
    assert_eq!(known.len(), count, "{path}: {file} lists the wrong count");
    known
}

fn check(file: &str, version: &str, total: usize, segment: impl Fn(&str) -> Vec<usize>) {
    let (header, cases) = load(file);
    assert_eq!(
        header,
        format!("# {version}-{}.txt", crate::UNICODE_DATA_VERSION),
        "{file} is not the pinned data version"
    );
    assert_eq!(cases.len(), total, "{file} has the wrong number of cases");
    let failed = failures(&cases, segment);
    let known = if file == "WordBreakTest.txt" {
        BTreeSet::new()
    } else {
        known_differences(file)
    };
    assert!(
        failed == known,
        "{file}: {} cases differ, {} recorded; newly failing {:?}, newly passing {:?}",
        failed.len(),
        known.len(),
        failed.difference(&known).collect::<Vec<_>>(),
        known.difference(&failed).collect::<Vec<_>>(),
    );
}

#[test]
fn grapheme_break_test_passes_except_the_recorded_cases() {
    check(
        "GraphemeBreakTest.txt",
        "GraphemeBreakTest",
        853,
        grapheme_boundaries,
    );
}

#[test]
fn word_break_test_passes_in_full() {
    check("WordBreakTest.txt", "WordBreakTest", 1944, word_boundaries);
}

#[test]
fn line_break_test_passes_except_the_recorded_cases() {
    // UAX #14's default algorithm is ICU4X's `Strict` strictness (CJ
    // resolves to NS) with the `Normal` word option; a case's leading `×`
    // marks that 0 is no opportunity.
    check("LineBreakTest.txt", "LineBreakTest", 19346, |text| {
        let graphemes = grapheme_boundaries(text);
        line_opportunities(
            text,
            LineBreakStrictness::Strict,
            WordBreakOption::Normal,
            &graphemes,
        )
    });
}
