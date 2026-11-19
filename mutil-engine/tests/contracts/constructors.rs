//! Constructor naming and canonical construction: `try_new` returns `Option`
//! and refuses what the spec refuses (TYP-2, TYP-5, FNT-1, GEO-1, OPT-1), the
//! gates' `try_from_*` constructors return `Result` (AST-11, PAR-5), and
//! `FontFeatures` is canonical as built (FNT-2).

use super::common::*;
use mutil_engine::*;
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

fn hash_of(features: &FontFeatures) -> u64 {
    let mut hasher = DefaultHasher::new();
    features.hash(&mut hasher);
    hasher.finish()
}

fn variation(tag: [u8; 4], value: f32) -> FontVariation {
    FontVariation::try_new(tag, value).expect("a finite coordinate")
}

fn features_of(settings: &[([u8; 4], u32)]) -> FontFeatures {
    let mut features = FontFeatures::default();
    for &(tag, value) in settings {
        features.push(tag, value);
    }
    features
}

/// FNT-2: each list reads in ascending tag order whatever the push order.
#[test]
fn features_read_in_tag_order() {
    let mut features = FontFeatures::default();
    features.push(*b"liga", 1);
    features.push(*b"kern", 1);
    let tags: Vec<[u8; 4]> = features.as_slice().iter().map(|f| f.tag).collect();
    assert_eq!(tags, [*b"kern", *b"liga"]);

    let mut features = FontFeatures::default();
    features.push_variation(variation(*b"wght", 650.0));
    features.push_variation(variation(*b"opsz", 12.0));
    features.push_variation(variation(*b"wdth", 90.0));
    let tags: Vec<[u8; 4]> = features.variations().iter().map(|v| v.tag()).collect();
    assert_eq!(tags, [*b"opsz", *b"wdth", *b"wght"]);
}

/// FNT-2: the default value is the empty set.
#[test]
fn default_features_are_empty() {
    let features = FontFeatures::default();
    assert!(features.as_slice().is_empty());
    assert!(features.variations().is_empty());
}

/// FNT-2: equal effective settings are equal values and hash alike, however
/// they were pushed.
#[test]
fn equal_settings_compare_and_hash_equal_in_any_push_order() {
    let forward = features_of(&[(*b"kern", 1), (*b"liga", 0), (*b"chws", 1)]);
    let backward = features_of(&[(*b"chws", 1), (*b"liga", 0), (*b"kern", 1)]);
    let duplicated = features_of(&[
        (*b"liga", 1),
        (*b"kern", 1),
        (*b"chws", 1),
        (*b"liga", 0),
        (*b"kern", 1),
    ]);
    assert_eq!(forward, backward);
    assert_eq!(forward, duplicated);
    assert_eq!(hash_of(&forward), hash_of(&backward));
    assert_eq!(hash_of(&forward), hash_of(&duplicated));
    let set: HashSet<FontFeatures> = [forward, backward, duplicated].into_iter().collect();
    assert_eq!(set.len(), 1);

    let one = features_of(&[(*b"kern", 1)]);
    let other = features_of(&[(*b"kern", 0)]);
    assert_ne!(one, other);
}

/// FNT-2: the same holds for variation coordinates.
#[test]
fn equal_variations_compare_and_hash_equal_in_any_push_order() {
    let build = |axes: &[([u8; 4], f32)]| {
        let mut features = FontFeatures::default();
        for &(tag, value) in axes {
            features.push_variation(variation(tag, value));
        }
        features
    };
    let a = build(&[(*b"wght", 650.0), (*b"wdth", 90.0)]);
    let b = build(&[(*b"wdth", 90.0), (*b"wght", 650.0), (*b"wght", 650.0)]);
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));
}

/// FNT-1, FNT-2: `-0.0` and `0.0` are one coordinate, so they build one value.
#[test]
fn negative_zero_variation_equals_zero() {
    let negative = variation(*b"slnt", -0.0);
    assert_eq!(negative, variation(*b"slnt", 0.0));
    assert!(negative.value().is_sign_positive());
    let mut a = FontFeatures::default();
    a.push_variation(negative);
    let mut b = FontFeatures::default();
    b.push_variation(variation(*b"slnt", 0.0));
    assert_eq!(a, b);
    assert_eq!(hash_of(&a), hash_of(&b));
}

/// FNT-2: the last setting for a tag replaces an earlier one.
#[test]
fn the_last_setting_for_a_tag_wins() {
    let mut features = FontFeatures::default();
    features.push(*b"kern", 1);
    features.push(*b"kern", 0);
    let kept: Vec<([u8; 4], u32)> = features
        .as_slice()
        .iter()
        .map(|f| (f.tag, f.value))
        .collect();
    assert_eq!(kept, [(*b"kern", 0)]);

    let mut features = FontFeatures::default();
    features.push_variation(variation(*b"wght", 400.0));
    features.push_variation(variation(*b"wght", 700.0));
    assert_eq!(features.variations(), [variation(*b"wght", 700.0)]);
}

/// FNT-2: a run built with unsorted, repeated pushes needs no normalising to
/// pass the input gate.
#[test]
fn a_run_with_unsorted_pushes_passes_the_input_gate() {
    let mut a = ast("a");
    let features = &mut a.runs[0].features;
    features.push(*b"liga", 1);
    features.push(*b"kern", 1);
    features.push(*b"liga", 0);
    features.push_variation(variation(*b"wght", 650.0));
    features.push_variation(variation(*b"opsz", 12.0));
    assert_eq!(check(&a, "a"), Ok(()));
}

/// TYP-2: `try_new` is `None` exactly for an inverted range.
#[test]
fn text_range_try_new_refuses_an_inverted_range() {
    assert_eq!(TextRange::try_new(5, 3), None);
    let range = TextRange::try_new(3, 5).expect("start <= end");
    assert_eq!((range.start(), range.end()), (3, 5));
    assert_eq!(TextRange::try_new(4, 4), Some(TextRange::empty_at(4)));
}

/// TYP-2: `new` panics on `start > end`.
#[test]
#[should_panic]
fn text_range_new_panics_on_an_inverted_range() {
    let _ = TextRange::new(5, 3);
}

/// TYP-2: `len` and `is_empty` agree.
#[test]
fn text_range_len_and_is_empty_agree() {
    assert_eq!(TextRange::EMPTY.len(), 0);
    assert!(TextRange::EMPTY.is_empty());
    let empty = TextRange::empty_at(7);
    assert_eq!(empty.len(), 0);
    assert!(empty.is_empty());
    let range = TextRange::new(2, 6);
    assert_eq!(range.len(), 4);
    assert!(!range.is_empty());
}

/// TYP-5: levels up to `BidiLevel::MAX` exist; none above it.
#[test]
fn bidi_level_try_new_stops_at_the_maximum_depth() {
    assert_eq!(BIDI_MAX_DEPTH, 125);
    assert_eq!(BidiLevel::try_new(0), Some(BidiLevel::LTR));
    assert_eq!(BidiLevel::try_new(1), Some(BidiLevel::RTL));
    assert_eq!(BidiLevel::try_new(BIDI_MAX_DEPTH + 1), Some(BidiLevel::MAX));
    assert_eq!(BidiLevel::try_new(BIDI_MAX_DEPTH + 2), None);
    assert_eq!(BidiLevel::try_new(u8::MAX), None);
}

/// FNT-1: weights are `1..=1000`.
#[test]
fn font_weight_try_new_accepts_one_to_one_thousand() {
    assert_eq!(FontWeight::try_new(0), None);
    assert_eq!(FontWeight::try_new(1001), None);
    assert_eq!(FontWeight::try_new(u16::MAX), None);
    assert_eq!(FontWeight::try_new(1).map(FontWeight::get), Some(1));
    assert_eq!(FontWeight::try_new(1000).map(FontWeight::get), Some(1000));
    assert_eq!(FontWeight::try_new(400), Some(FontWeight::NORMAL));
    assert_eq!(FontWeight::try_new(700), Some(FontWeight::BOLD));
}

/// FNT-1: a variation coordinate must be finite.
#[test]
fn font_variation_try_new_requires_a_finite_coordinate() {
    assert_eq!(FontVariation::try_new(*b"wght", f32::NAN), None);
    assert_eq!(FontVariation::try_new(*b"wght", f32::INFINITY), None);
    assert_eq!(FontVariation::try_new(*b"wght", f32::NEG_INFINITY), None);
    let v = variation(*b"wght", 650.0);
    assert_eq!((v.tag(), v.value()), (*b"wght", 650.0));
}

/// OPT-1: a tab step is finite and positive.
#[test]
fn tab_stops_try_new_requires_a_finite_positive_step() {
    for step in [0.0, -0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(TabStops::try_new(step), None, "{step}");
    }
    assert_eq!(TabStops::try_new(40.0).map(|t| t.step()), Some(40.0));
    assert_eq!(TabStops::DEFAULT.step(), 8.0);
}

/// GEO-1: a slot is a finite start and an end strictly beyond it, with a
/// finite width, or an infinite end.
#[test]
fn inline_slot_constraint_try_new_refuses_a_bad_interval() {
    assert!(InlineSlotConstraint::try_new(0.0, 10.0).is_some());
    assert!(InlineSlotConstraint::try_new(-5.0, 5.0).is_some());
    assert!(InlineSlotConstraint::try_new(0.0, f32::INFINITY).is_some());

    assert!(
        InlineSlotConstraint::try_new(10.0, 3.0).is_none(),
        "inverted"
    );
    assert!(InlineSlotConstraint::try_new(5.0, 5.0).is_none(), "empty");
    assert!(InlineSlotConstraint::try_new(f32::NAN, 5.0).is_none());
    assert!(InlineSlotConstraint::try_new(0.0, f32::NAN).is_none());
    assert!(InlineSlotConstraint::try_new(f32::INFINITY, f32::INFINITY).is_none());
    assert!(InlineSlotConstraint::try_new(f32::NEG_INFINITY, 5.0).is_none());
    assert!(InlineSlotConstraint::try_new(0.0, f32::NEG_INFINITY).is_none());
    assert!(
        InlineSlotConstraint::try_new(-f32::MAX, f32::MAX).is_none(),
        "the width is not finite"
    );
    let slot = InlineSlotConstraint::try_new(2.0, 10.0).unwrap();
    assert_eq!((slot.start(), slot.end(), slot.width()), (2.0, 10.0, 8.0));
}

/// AST-11: the input gate returns the inputs for a valid paragraph.
#[test]
fn try_from_ast_accepts_valid_inputs() {
    let a = ast("ab");
    let options = LayoutOptions::default();
    assert!(ValidatedInputs::try_from_ast(&a, "ab", &constraints(), &options).is_ok());
}

/// AST-11: of two violations the gate reports the first in its order (shaping
/// runs before items), which is also what `check_layout_inputs` reports.
#[test]
fn try_from_ast_returns_the_first_violation() {
    let mut a = ast("ab");
    a.glyphs[0].x_offset = 2.0 * MAX_MEASUREMENT;
    a.runs[0].font_size = f32::NAN;
    let options = LayoutOptions::default();
    let error = ValidatedInputs::try_from_ast(&a, "ab", &constraints(), &options)
        .map(drop)
        .unwrap_err();
    assert_eq!(error, LayoutInputError::InvalidShapingRun { run: 0 });
    assert_eq!(
        check_layout_inputs(&a, "ab", &constraints(), &options),
        Err(error)
    );
}

/// PAR-5: the output gate returns the paragraph's first violation, as
/// `PositionedParagraph::validate` does, and the layout for a valid one.
#[test]
fn try_from_paragraph_returns_the_first_violation_or_the_layout() {
    let ctx = ctx();
    let text = "a";
    let mut l = line();
    l.byte_range = TextRange::new(0, 1);
    l.visible_byte_end = 1;
    let mut g = glyph(0.0, 10.0);
    g.byte_range = Some(l.byte_range);
    l.visual_runs = vec![run(g)];
    set_visual_width(&mut l, 10.0);
    let mut good = paragraph(&ctx, vec![l]);
    good.total_block_size = good.lines[0].line_height;
    assert_eq!(good.validate(text), Ok(()));
    let layout = ValidatedLayout::try_from_paragraph(&good, text).expect("valid output");
    assert_eq!(layout.text(), text);

    let empty = paragraph(&ctx, vec![]);
    let violation = empty.validate(text).expect_err("no lines");
    assert_eq!(
        ValidatedLayout::try_from_paragraph(&empty, text).map(drop),
        Err(violation)
    );
}
