//! Run, atom and cluster consistency: bidi levels, safety flags, finiteness, spacing.

use crate::common::*;
use mutil_engine::*;

fn reject_level_mismatch(target: usize) {
    let mut a = ast("abc");
    assert!(check(&a, "abc").is_ok());
    if target == 0 {
        a.runs[0].bidi_level = 1;
    }
    if let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = &mut a.items[0]
    {
        if target == 1 {
            atoms[0].bidi_level = 1;
        }
        if target == 2 {
            clusters[0].bidi_level = 1;
        }
    }
    assert!(check(&a, "abc").is_err());
}

#[test]
fn reject_run_level_mismatch() {
    reject_level_mismatch(0);
}

#[test]
fn reject_atom_level_mismatch() {
    reject_level_mismatch(1);
}

#[test]
fn reject_cluster_level_mismatch() {
    reject_level_mismatch(2);
}

#[test]
fn safety_flags_must_be_consistent() {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].safe_to_break_before = false;
        assert!(!clusters[0].safety_flags_consistent());
    }
    assert!(check(&a, "abc").is_err());
}

fn font_scale_must_be_finite(v: f32) {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].font.scale = v;
    }
    assert!(check(&a, "abc").is_err());
}

fn natural_width_must_be_finite(v: f32) {
    let mut a = ast("abc");
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].natural_width = v;
    }
    assert!(check(&a, "abc").is_err());
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
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].safe_to_concat_before = false;
        assert!(clusters[0].safety_flags_consistent());
    }
    assert!(check(&a, "abc").is_ok());
}

#[test]
fn nbsp_glue_must_not_be_collapsible() {
    let mut a = ast("\u{a0}");
    a.items = vec![
        WordNode::Glue(GlueNode {
            original_bidi_class: BidiClass::Cs,
            bidi_level: 0,
            is_collapsible: true,
            byte_range: TextRange::new(0, 2),
            kind: GlueKind::WordSpace,
            tier: GlueTier::Tier0,
            natural_width: 4.,
            max_stretch: 0.,
            max_shrink: 0.,
        }),
        end(2),
    ];
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
    if let WordNode::Box(BoxNode::Text { clusters, .. }) = &mut a.items[0] {
        clusters[0].glyphs[0].x_advance = f32::NAN;
    }
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidTextWidth { item: 0 })
    );
}

#[test]
fn non_finite_variation_and_impossible_bidi_level_are_rejected() {
    let mut a = ast("abc");
    a.runs[0]
        .features
        .variations
        .push(FontVariation::new(*b"wght", f32::INFINITY));
    // `FontVariation::new` canonicalises NaN but not infinity.
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
    let mut a = ast("abc");
    a.runs[0].bidi_level = BIDI_MAX_DEPTH + 2;
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::InvalidShapingRun { run: 0 })
    );
}

#[test]
fn explicit_spacing_and_discretionary_equations() {
    let mut a = ast("a");
    a.runs[0].letter_spacing = 1.0;
    if let WordNode::Box(BoxNode::Text {
        atoms, clusters, ..
    }) = &mut a.items[0]
    {
        atoms[0].flags |= AtomFlags::NO_LETTER_SPACING;
        clusters[0].flags = atoms[0].flags;
    }
    assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let mut a = ast("a");
        a.runs[0].word_spacing = value;
        assert_eq!(check(&a, "a"), Err(LayoutInputError::InvalidSpacing));
    }
    let mut a = ast("a");
    if let WordNode::Box(BoxNode::Text {
        clusters,
        natural_width,
        ..
    }) = &mut a.items[0]
    {
        clusters[0].natural_width = -1.0;
        *natural_width = -1.0;
    }
    assert!(check(&a, "a").is_err());
    for spacing in [-0.25, 0.25] {
        let mut a = ast("ab");
        let mut disc = DiscretionaryNode::direct(1);
        disc.no_break_width = spacing;
        disc.no_break_spacing = spacing;
        a.items = vec![
            boxed(0, 1, false),
            WordNode::Discretionary(disc),
            boxed(1, 2, false),
            end(2),
        ];
        assert!(check(&a, "ab").is_ok());
        if let WordNode::Discretionary(d) = &mut a.items[1] {
            d.no_break_spacing = 0.0;
        }
        assert!(check(&a, "ab").is_err());
    }
    let mut a = ast("a");
    if let WordNode::Discretionary(d) = &mut a.items[1] {
        d.no_break_width = 1.0;
        d.no_break_spacing = 1.0;
    }
    assert!(check(&a, "a").is_err());
}
