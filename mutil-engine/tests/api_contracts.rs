//! Public API contract probes. Shell behavior is intentionally still red.
use mutil_engine::*;

#[test]
fn empty_document_has_one_empty_paragraph() {
    assert_eq!(
        split_paragraphs(""),
        vec![ParagraphSlice {
            content: TextRange::empty_at(0),
            separator: TextRange::empty_at(0)
        }]
    );
}
#[test]
fn splitting_preserves_absolute_utf8_offsets_and_trailing_empty_paragraph() {
    assert_eq!(
        split_paragraphs("é\r\n中\n"),
        vec![
            ParagraphSlice {
                content: TextRange::new(0, 2),
                separator: TextRange::new(2, 4)
            },
            ParagraphSlice {
                content: TextRange::new(4, 7),
                separator: TextRange::new(7, 8)
            },
            ParagraphSlice {
                content: TextRange::empty_at(8),
                separator: TextRange::empty_at(8)
            },
        ]
    );
}
#[test]
fn line_separator_does_not_split_bidi_paragraphs() {
    let text = "a\u{2028}b";
    assert_eq!(
        split_paragraphs(text),
        vec![ParagraphSlice {
            content: TextRange::new(0, text.len()),
            separator: TextRange::empty_at(text.len())
        }]
    );
}
#[test]
fn every_paragraph_separator_is_retained() {
    for separator in ["\n", "\r", "\r\n", "\u{85}", "\u{b}", "\u{c}", "\u{2029}"] {
        let text = format!("{separator}{separator}");
        let slices = split_paragraphs(&text);
        assert_eq!(slices.len(), 3, "{separator:?}");
        assert_eq!(slices[0].separator, TextRange::new(0, separator.len()));
        assert_eq!(
            slices[1].separator,
            TextRange::new(separator.len(), text.len())
        );
        assert_eq!(slices[2].content, TextRange::empty_at(text.len()));
    }
}
#[test]
fn atomic_fragment_snaps_to_full_span_not_wrapped_fragment() {
    let mut f = AtomicFragment {
        atomic_id: 7,
        full_byte_range: TextRange::new(0, 20),
        fragment_byte_range: TextRange::new(10, 15),
        x_start: 0.,
        x_end: 10.,
        bidi_level: 0,
    };
    assert_eq!(f.hit_test(1.).byte_offset, 0);
    assert_eq!(f.hit_test(5.).byte_offset, 20);
    assert_eq!(f.hit_test(9.).affinity, CaretAffinity::Upstream);
    f.bidi_level = 1;
    assert_eq!(f.hit_test(1.).byte_offset, 20);
    assert_eq!(f.hit_test(5.).byte_offset, 0);
}
#[test]
fn absent_gdef_carets_fall_back_to_atom_widths() {
    assert_eq!(resolve_ligature_carets(30., 3, &[]), vec![10., 20.]);
}
#[test]
fn invalid_gdef_carets_fall_back_to_atom_widths() {
    for carets in [
        vec![10.],
        vec![20., 10.],
        vec![10., 10.],
        vec![0., 20.],
        vec![10., 30.],
        vec![f32::NAN, 20.],
        vec![10., f32::INFINITY],
    ] {
        assert_eq!(resolve_ligature_carets(30., 3, &carets), vec![10., 20.]);
    }
}
#[test]
fn valid_gdef_carets_are_preserved() {
    assert_eq!(resolve_ligature_carets(30., 3, &[8., 21.]), vec![8., 21.]);
}
#[test]
fn non_finite_or_non_positive_width_yields_no_carets() {
    for width in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.0, -1.0] {
        assert!(resolve_ligature_carets(width, 3, &[8., 21.]).is_empty());
    }
}
#[test]
fn huge_width_fallback_carets_stay_finite_and_ordered() {
    let carets = resolve_ligature_carets(f32::MAX, 4, &[]);
    assert!(carets.iter().all(|v| v.is_finite() && *v > 0.0));
    assert!(carets.windows(2).all(|w| w[0] < w[1]));
}
#[test]
fn information_separators_split_paragraphs() {
    for c in ['\u{1c}', '\u{1d}', '\u{1e}', '\u{b}', '\u{c}'] {
        let text = format!("a{c}b");
        assert_eq!(
            split_paragraphs(&text),
            vec![
                ParagraphSlice {
                    content: TextRange::new(0, 1),
                    separator: TextRange::new(1, 2)
                },
                ParagraphSlice {
                    content: TextRange::new(2, 3),
                    separator: TextRange::empty_at(3)
                },
            ],
            "{c:?}"
        );
    }
}
#[test]
fn tab_precision_exhaustion_returns_none() {
    assert_eq!(TabStops { step: 8. }.next_stop(1e10), None);
    assert_eq!(TabStops { step: 8. }.next_stop(f32::MAX), None);
}
#[test]
fn locale_heuristics_require_opt_in() {
    assert_eq!(
        FrontendOptions::default().locale_policy,
        LocalePolicy::ExplicitOnly
    );
}
#[test]
fn report_does_not_claim_uninstalled_unicode_components() {
    let r = conformance_report();
    assert_eq!(
        (r.grapheme, r.bidi, r.line_break, r.word),
        (None, None, None, None)
    );
}
#[test]
fn font_instance_queries_accept_variations_without_changing_face_identity() {
    let variations = [FontVariation::new(*b"wght", 650.)];
    let instance = FontInstanceRef {
        font: FontRef::unscaled(FontId(7)),
        font_size: 18.,
        variations: &variations,
    };
    let shaper: &dyn TextShaper = &NoopShaper;
    // Noop checks interface closure only; real backend measurements need fonts.
    let _: FontMetrics = shaper.font_metrics(instance);
    let _ = shaper.ligature_carets(instance, 1, Direction::Ltr);
}
#[test]
fn saturated_badness_prefers_smaller_excess_slack() {
    let capacities = TierCapacities::default();
    let near = line_fit(1e6, &capacities, 16.);
    let far = line_fit(2e6, &capacities, 16.);
    assert_eq!((near.badness, far.badness), (MAX_BADNESS, MAX_BADNESS));
    assert!(near.comparison_key() < far.comparison_key());
}

#[test]
fn tier_two_without_primary_capacity_starts_at_zero() {
    let capacities = TierCapacities {
        stretch: [0., 0., 10.],
        shrink: [0.; 2],
    };
    let near = line_fit(0.002, &capacities, 16.);
    assert!(near.badness > 0. && near.badness < 0.01);
    assert_eq!(line_fit(10., &capacities, 16.).badness, 400.);
}

#[test]
fn standard_tab_stops_remain_strictly_forward() {
    let stops = TabStops { step: 8. };
    assert_eq!(stops.next_stop(0.), Some(8.));
    assert_eq!(stops.next_stop(8.), Some(16.));
    for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(stops.next_stop(x), None);
    }
}
