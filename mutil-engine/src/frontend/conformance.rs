//! The Unicode conformance suites of the installed segmenters and of the
//! bidi algorithm (spec RM-1): `GraphemeBreakTest.txt`, `WordBreakTest.txt`,
//! `LineBreakTest.txt`, `BidiTest.txt` and `BidiCharacterTest.txt` of the
//! pinned data version, in the comment-stripped form under
//! `tests/assets/ucd/` (provenance in `tests/assets/README.md`).
//!
//! Where ICU4X's stable rule tables predate the data version, the differing
//! cases are listed in `tests/assets/ucd/known-differences.txt`, and each
//! suite asserts that exact set: a fix and a regression both fail it, and the
//! failure prints the set to record. The bidi suites record no differences
//! (spec RM-12): they pass in full, and a failure prints its first cases.

use super::bidi::{self, Bracket};
use super::segment::{grapheme_boundaries, line_opportunities, word_boundaries};
use super::{LineBreakStrictness, WordBreakOption};
use crate::types::{BidiClass, BidiLevel, Direction};
use std::collections::BTreeSet;

/// One test case: the text and the byte offsets the file marks with `÷`.
struct Case {
    line: usize,
    text: String,
    breaks: Vec<usize>,
}

/// The contents of `tests/assets/ucd/{name}`.
fn read_ucd(name: &str) -> String {
    let path = format!("{}/tests/assets/ucd/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// Asserts that `header`, the first line of `file`, names the pinned data
/// version.
fn assert_pinned(file: &str, version: &str, header: &str) {
    assert_eq!(
        header,
        format!("# {version}-{}.txt", crate::UNICODE_DATA_VERSION),
        "{file} is not the pinned data version"
    );
}

fn load(name: &str) -> (String, Vec<Case>) {
    let data = read_ucd(name);
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
    let path = "known-differences.txt";
    let data = read_ucd(path);
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
    assert_pinned(file, version, &header);
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

/// What a bidi test file records for one case: the resolved paragraph level
/// (`None` where the file does not record it), the levels after L1 (`None`
/// for a character X9 removes, `x` in the files) and the visual order of the
/// characters X9 keeps.
#[derive(Debug, PartialEq, Eq)]
struct BidiOutcome {
    paragraph: Option<u8>,
    levels: Vec<Option<u8>>,
    order: Vec<usize>,
}

/// One bidi test case: the paragraph's classes and bracket roles, the base
/// direction it is resolved with (`None` = automatic) and its outcome.
struct BidiCase<'a> {
    line: usize,
    input: &'a str,
    classes: Vec<BidiClass>,
    brackets: Vec<Bracket>,
    base: Option<Direction>,
    expected: BidiOutcome,
}

/// UAX #9 on the paragraph as one line: resolution, L1 over every character
/// and L2 over the characters X9 keeps.
fn run_bidi(classes: &[BidiClass], brackets: &[Bracket], base: Option<Direction>) -> BidiOutcome {
    let bidi::Resolution {
        direction,
        mut levels,
    } = bidi::resolve(classes, brackets, base);
    let paragraph = direction.base_level();
    bidi::reset_line_levels(
        classes.iter().map(|&class| bidi::line_class(class)),
        &mut levels,
        paragraph,
    );
    let kept: Vec<usize> = classes
        .iter()
        .enumerate()
        .filter(|&(_, &class)| !bidi::is_removed(class))
        .map(|(index, _)| index)
        .collect();
    let kept_levels: Vec<BidiLevel> = kept.iter().map(|&index| levels[index]).collect();
    BidiOutcome {
        paragraph: Some(paragraph.get()),
        levels: classes
            .iter()
            .zip(&levels)
            .map(|(&class, level)| (!bidi::is_removed(class)).then_some(level.get()))
            .collect(),
        order: bidi::visual_order(&kept_levels)
            .into_iter()
            .map(|visual| kept[visual])
            .collect(),
    }
}

/// A space-separated level list, `x` marking a removed character.
fn parse_levels(field: &str) -> Vec<Option<u8>> {
    field
        .split_whitespace()
        .map(|level| match level {
            "x" => None,
            n => Some(n.parse().unwrap_or_else(|_| panic!("bad level {n}"))),
        })
        .collect()
}

/// A space-separated list of logical indices.
fn parse_order(field: &str) -> Vec<usize> {
    field
        .split_whitespace()
        .map(|index| {
            index
                .parse()
                .unwrap_or_else(|_| panic!("bad index {index}"))
        })
        .collect()
}

fn parse_class(name: &str) -> BidiClass {
    match name {
        "L" => BidiClass::L,
        "R" => BidiClass::R,
        "AL" => BidiClass::Al,
        "EN" => BidiClass::En,
        "ES" => BidiClass::Es,
        "ET" => BidiClass::Et,
        "AN" => BidiClass::An,
        "CS" => BidiClass::Cs,
        "NSM" => BidiClass::Nsm,
        "BN" => BidiClass::Bn,
        "B" => BidiClass::B,
        "S" => BidiClass::S,
        "WS" => BidiClass::Ws,
        "ON" => BidiClass::On,
        "LRE" => BidiClass::Lre,
        "LRO" => BidiClass::Lro,
        "RLE" => BidiClass::Rle,
        "RLO" => BidiClass::Rlo,
        "PDF" => BidiClass::Pdf,
        "LRI" => BidiClass::Lri,
        "RLI" => BidiClass::Rli,
        "FSI" => BidiClass::Fsi,
        "PDI" => BidiClass::Pdi,
        other => panic!("bad bidi class {other}"),
    }
}

/// The paragraph directions a `BidiTest.txt` bitset selects, one case each.
fn bitset_bases(bitset: u8) -> impl Iterator<Item = Option<Direction>> {
    [
        (1, None),
        (2, Some(Direction::Ltr)),
        (4, Some(Direction::Rtl)),
    ]
    .into_iter()
    .filter(move |&(bit, _)| bitset & bit != 0)
    .map(|(_, base)| base)
}

/// The cases of `BidiTest.txt` (after its header), in order: each data line
/// takes the levels and order of the `@Levels` and `@Reorder` lines above it.
/// The file has no brackets.
fn bidi_test_cases(data: &str, each: &mut dyn FnMut(BidiCase)) {
    let mut levels = Vec::new();
    let mut order = Vec::new();
    for (i, line) in data.lines().enumerate().skip(1) {
        if let Some(field) = line.strip_prefix("@Levels:") {
            levels = parse_levels(field);
        } else if let Some(field) = line.strip_prefix("@Reorder:") {
            order = parse_order(field);
        } else {
            let (names, bitset) = line
                .split_once(';')
                .unwrap_or_else(|| panic!("BidiTest.txt:{}: bad line", i + 1));
            let classes: Vec<BidiClass> = names.split_whitespace().map(parse_class).collect();
            let bitset = bitset
                .trim()
                .parse()
                .unwrap_or_else(|_| panic!("BidiTest.txt:{}: bad bitset {bitset}", i + 1));
            for base in bitset_bases(bitset) {
                each(BidiCase {
                    line: i + 1,
                    input: line,
                    brackets: vec![Bracket::None; classes.len()],
                    classes: classes.clone(),
                    base,
                    expected: BidiOutcome {
                        paragraph: None,
                        levels: levels.clone(),
                        order: order.clone(),
                    },
                });
            }
        }
    }
}

/// The cases of `BidiCharacterTest.txt` (after its header), one per line.
fn bidi_character_test_cases(data: &str, each: &mut dyn FnMut(BidiCase)) {
    for (i, line) in data.lines().enumerate().skip(1) {
        let fields: Vec<&str> = line.split(';').collect();
        let [text, direction, paragraph, levels, order] = fields[..] else {
            panic!("BidiCharacterTest.txt:{}: bad line", i + 1);
        };
        let text: Vec<char> = text
            .split_whitespace()
            .map(|hex| {
                u32::from_str_radix(hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .unwrap_or_else(|| {
                        panic!("BidiCharacterTest.txt:{}: bad code point {hex}", i + 1)
                    })
            })
            .collect();
        let base = match direction {
            "0" => Some(Direction::Ltr),
            "1" => Some(Direction::Rtl),
            "2" => None,
            other => panic!("BidiCharacterTest.txt:{}: bad direction {other}", i + 1),
        };
        each(BidiCase {
            line: i + 1,
            input: line,
            classes: text.iter().map(|&c| bidi::char_class(c)).collect(),
            brackets: text.iter().map(|&c| bidi::bracket(c)).collect(),
            base,
            expected: BidiOutcome {
                paragraph: Some(paragraph.parse().unwrap_or_else(|_| {
                    panic!("BidiCharacterTest.txt:{}: bad level {paragraph}", i + 1)
                })),
                levels: parse_levels(levels),
                order: parse_order(order),
            },
        });
    }
}

/// How many failures a bidi suite prints in full.
const SHOWN_BIDI_FAILURES: usize = 10;

/// Reads the cases of a bidi test file, handing each to the callback.
type BidiCases = fn(&str, &mut dyn FnMut(BidiCase));

/// Runs a bidi suite: the header names the pinned data version, the file has
/// `total` cases and every case passes (spec RM-12 records no differences).
fn check_bidi(file: &str, version: &str, total: usize, cases: BidiCases) {
    let data = read_ucd(file);
    assert_pinned(file, version, data.lines().next().unwrap_or_default());
    let mut count = 0;
    let mut failed = BTreeSet::new();
    let mut shown = Vec::new();
    cases(&data, &mut |case| {
        count += 1;
        let actual = run_bidi(&case.classes, &case.brackets, case.base);
        // A file without a paragraph-level field does not check it.
        let actual = BidiOutcome {
            paragraph: case.expected.paragraph.and(actual.paragraph),
            ..actual
        };
        if actual != case.expected {
            failed.insert(case.line);
            if shown.len() < SHOWN_BIDI_FAILURES {
                shown.push(format!(
                    "line {}: {:?} with base {:?}\n  expected {:?}\n  actual   {:?}",
                    case.line, case.input, case.base, case.expected, actual
                ));
            }
        }
    });
    assert_eq!(count, total, "{file} has the wrong number of cases");
    assert!(
        failed.is_empty(),
        "{file}: {} lines fail (expected none); first failing cases:\n{}\nall failing lines: {:?}",
        failed.len(),
        shown.join("\n"),
        failed,
    );
}

#[test]
fn bidi_test_passes_in_full() {
    check_bidi("BidiTest.txt", "BidiTest", 770_241, bidi_test_cases);
}

#[test]
fn bidi_character_test_passes_in_full() {
    check_bidi(
        "BidiCharacterTest.txt",
        "BidiCharacterTest",
        91_707,
        bidi_character_test_cases,
    );
}
