//! Run, atom and cluster consistency: bidi levels, safety flags, finiteness, spacing.

use crate::common::*;
use mutil_engine::*;

/// Atoms and clusters have no level of their own: the run's level is theirs,
/// so changing it changes the whole payload, and a level above
/// `BidiLevel::MAX` cannot be built.
#[test]
fn the_run_level_is_the_level_of_its_atoms() {
    let mut a = ast("abc");
    assert!(check(&a, "abc").is_ok());
    a.runs[0].bidi_level = BidiLevel::RTL;
    assert!(check(&a, "abc").is_ok());
    a.runs[0].bidi_level = BidiLevel::MAX;
    assert!(check(&a, "abc").is_ok());
    assert_eq!(BidiLevel::new(BIDI_MAX_DEPTH + 2), None);
}

#[test]
fn safety_flags_must_be_consistent() {
    let mut a = ast("abc");
    a.clusters[0].safe_to_break_before = false;
    assert!(!a.clusters[0].safety_flags_consistent());
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemNotSelfContained { item: 0 })
    );
}

fn font_scale_must_be_finite(v: f32) {
    let mut a = ast("abc");
    a.clusters[0].font.scale = v;
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

fn natural_width_must_be_finite(v: f32) {
    let mut a = ast("abc");
    a.clusters[0].natural_width = v;
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

macro_rules! nonfinite_cases {
    ($nan:ident, $pos:ident, $neg:ident, $probe:ident) => {
        #[test]
        fn $nan() {
            $probe(f32::NAN);
        }
        #[test]
        fn $pos() {
            $probe(f32::INFINITY);
        }
        #[test]
        fn $neg() {
            $probe(f32::NEG_INFINITY);
        }
    };
}

nonfinite_cases!(
    reject_nan_scale,
    reject_infinite_scale,
    reject_negative_infinite_scale,
    font_scale_must_be_finite
);

nonfinite_cases!(
    reject_nan_natural_width,
    reject_infinite_natural_width,
    reject_negative_infinite_natural_width,
    natural_width_must_be_finite
);

#[test]
fn concat_unsafety_alone_is_valid() {
    let mut a = ast("abc");
    a.clusters[0].safe_to_concat_before = false;
    assert!(a.clusters[0].safety_flags_consistent());
    assert!(check(&a, "abc").is_ok());
}

#[test]
fn nbsp_glue_must_not_be_collapsible() {
    let mut a = ast("\u{a0}");
    rebuild_items(&mut a, |_| {
        vec![
            WordNode::Glue(GlueNode {
                original_bidi_class: BidiClass::Cs,
                bidi_level: BidiLevel::LTR,
                is_collapsible: true,
                byte_range: TextRange::new(0, 2),
                kind: GlueKind::WordSpace,
                natural_width: 4.,
                max_stretch: 0.,
                max_shrink: 0.,
            }),
            end(2),
        ]
    });
    assert_eq!(
        check(&a, "\u{a0}"),
        Err(LayoutInputError::InvalidGlue { item: 0 })
    );
    if let WordNode::Glue(g) = &mut a.items[0] {
        g.is_collapsible = false;
    }
    assert!(check(&a, "\u{a0}").is_ok());
}

#[test]
fn non_finite_cluster_glyph_measurement_is_rejected() {
    let mut a = ast("abc");
    a.glyphs[0].x_advance = f32::NAN;
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

#[test]
fn explicit_spacing_and_discretionary_equations() {
    let mut a = ast("a");
    a.runs[0].letter_spacing = 1.0;
    a.atoms[0].flags |= AtomFlags::NO_LETTER_SPACING;
    a.clusters[0].flags = a.atoms[0].flags;
    assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut a = ast("a");
        a.runs[0].word_spacing = value;
        assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    }
    let mut a = ast("a");
    a.clusters[0].natural_width = -1.0;
    if let WordNode::Box(BoxNode::Text { natural_width, .. }) = &mut a.items[0] {
        *natural_width = -1.0;
    }
    assert_eq!(
        check(&a, "a"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
    for spacing in [-0.25, 0.25] {
        let mut a = ast("ab");
        let mut disc = DiscretionaryNode::direct(1);
        disc.no_break_width = spacing;
        rebuild_items(&mut a, |b| {
            vec![
                boxed(b, 0, 1),
                WordNode::Discretionary(disc),
                boxed(b, 1, 2),
                end(2),
            ]
        });
        assert!(check(&a, "ab").is_ok());
    }
    let mut a = ast("a");
    if let WordNode::Discretionary(d) = &mut a.items[1] {
        d.no_break_width = 1.0;
    }
    assert_eq!(
        check(&a, "a"),
        Err(LayoutInputError::InvalidDiscretionary { item: 1 })
    );
}

/// Every opportunity of a long single-run chain cuts back to the chain start
/// and forward to its end.
#[test]
fn a_long_hyphen_chain_accepts_cuts_across_the_whole_chain() {
    let n = 2000;
    let (t, a) = hyphen_chain_ast(n, |offset, n| {
        ((offset - 1) as u32, (n + 1 - offset) as u32)
    });
    assert!(check(&a, &t).is_ok());
}

#[test]
fn a_hyphen_cut_one_byte_past_the_chain_start_is_rejected() {
    let n = 2000;
    let middle = n / 2;
    let (t, a) = hyphen_chain_ast(n, |offset, n| {
        let past_start = if offset == middle { offset } else { offset - 1 };
        (past_start as u32, (n + 1 - offset) as u32)
    });
    let Err(LayoutInputError::InvalidHyphenOpportunity { item }) = check(&a, &t) else {
        panic!("expected InvalidHyphenOpportunity");
    };
    // Items: x, Direct, then box / Hyphenate pairs; the opportunity at offset
    // `middle` follows box `middle - 2`.
    assert_eq!(item, 2 + 2 * (middle - 1) - 1);
}

#[test]
fn a_hyphen_cut_one_byte_past_the_chain_end_is_rejected() {
    let n = 50;
    let (t, a) = hyphen_chain_ast(n, |offset, n| {
        let past_end = if offset == 10 {
            n + 2 - offset
        } else {
            n + 1 - offset
        };
        ((offset - 1) as u32, past_end as u32)
    });
    // The opportunity at offset 10 follows box 8 (see the test above).
    assert_eq!(
        check(&a, &t),
        Err(LayoutInputError::InvalidHyphenOpportunity {
            item: 2 + 2 * 9 - 1
        })
    );
}

#[test]
fn a_non_finite_variation_cannot_be_built() {
    for bad in [f32::NAN, f32::INFINITY] {
        assert_eq!(FontVariation::new(*b"wght", bad), None);
    }
}

#[test]
fn unresolved_run_script_is_an_invalid_shaping_run() {
    for script in [ScriptTag::COMMON, ScriptTag::INHERITED] {
        let mut a = ast("abc");
        a.runs[0].script = script;
        assert_eq!(
            check(&a, "abc"),
            Err(LayoutInputError::InvalidShapingRun { run: 0 })
        );
    }
}

#[test]
fn a_tab_owns_source_bytes() {
    let build = |tab: TextRange| {
        let mut a = ast("a\tb");
        rebuild_items(&mut a, |b| {
            vec![
                boxed(b, 0, 1),
                WordNode::Tab { byte_range: tab },
                boxed(b, 2, 3),
                end(3),
            ]
        });
        a
    };
    assert!(check(&build(TextRange::new(1, 2)), "a\tb").is_ok());
    // Zero bytes are not a tab, wherever they sit.
    for at in [1, 2] {
        assert_eq!(
            check(&build(TextRange::empty_at(at)), "a\tb"),
            Err(LayoutInputError::InvalidTab { item: 1 }),
            "empty tab at {at}"
        );
    }
}

#[test]
fn an_inline_object_is_at_a_representable_level() {
    // An object's level comes from its context: any level.
    let with_object = |bidi_level| {
        let mut a = ast("ab\u{fffc}");
        rebuild_items(&mut a, |b| {
            vec![
                boxed(b, 0, 2),
                WordNode::Box(BoxNode::InlineObject {
                    bidi_level,
                    baseline_align: BaselineAlignment::Auto,
                    byte_range: TextRange::new(2, 5),
                    box_id: 1,
                    width: 10.0,
                    ascent: 8.0,
                    descent: 2.0,
                    overflow_policy: BoxOverflowPolicy::Clip,
                }),
                end(5),
            ]
        });
        check(&a, "ab\u{fffc}")
    };
    assert_eq!(with_object(BidiLevel::MAX), Ok(()));
    assert_eq!(with_object(BidiLevel::RTL), Ok(()));
}

#[test]
fn a_zero_width_tab_stop_geometry_is_not_constrained() {
    // Tab widths are decided during layout; only the stops are validated.
    let positioned = PositionedTab {
        byte_range: TextRange::new(0, 1),
        x: 10.,
        width: 0.,
    };
    let mut l = line();
    l.visual_runs = vec![run(glyph(0., 100., false))];
    l.tabs = vec![positioned];
    assert_eq!(l.validate(&ctx()), Ok(()));
}

#[test]
fn every_text_box_and_inline_object_owns_source_bytes() {
    // A text box without atoms.
    let mut a = ast("abc");
    rebuild_items(&mut a, |b| vec![boxed(b, 0, 0), boxed(b, 0, 3), end(3)]);
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemNotSelfContained { item: 0 })
    );
    assert!(!a.is_self_contained(0));
    // An inline object without bytes: nothing for a caret to address.
    let object = |start, end| {
        WordNode::Box(BoxNode::InlineObject {
            bidi_level: BidiLevel::LTR,
            baseline_align: BaselineAlignment::Auto,
            byte_range: TextRange::new(start, end),
            box_id: 1,
            width: 10.0,
            ascent: 8.0,
            descent: 2.0,
            overflow_policy: BoxOverflowPolicy::Clip,
        })
    };
    let mut a = ast("ab\u{fffc}");
    rebuild_items(&mut a, |b| vec![boxed(b, 0, 2), object(2, 5), end(5)]);
    assert_eq!(check(&a, "ab\u{fffc}"), Ok(()));
    let mut a = ast("ab");
    rebuild_items(&mut a, |b| vec![boxed(b, 0, 2), object(2, 2), end(2)]);
    assert_eq!(
        check(&a, "ab"),
        Err(LayoutInputError::InvalidInlineBox { item: 1 })
    );
}
