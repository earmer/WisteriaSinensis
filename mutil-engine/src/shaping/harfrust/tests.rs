use super::registry::{
    code_page_region, family_region, locale_region, single_region, style_distance,
};
use super::*;
use crate::shaping::FontBytes;
use crate::types::{FontSlant, GenericFamily};

#[test]
fn the_instance_cache_is_bounded() {
    let mut cache: HashMap<u32, u32> = HashMap::new();
    for key in 0..1000 {
        let built = *bounded_entry(&mut cache, key, MAX_CACHED_INSTANCES, || key * 2);
        assert_eq!(built, key * 2);
        assert!(cache.len() <= MAX_CACHED_INSTANCES);
    }
    // A cached key is served without rebuilding, even when full.
    let full = cache.len();
    let key = *cache.keys().next().expect("an entry");
    let served = *bounded_entry(&mut cache, key, full, || unreachable!("cached"));
    assert_eq!(served, key * 2);
    assert_eq!(cache.len(), full);
}

#[test]
fn bytes_that_hold_no_font_are_not_registered() {
    let mut registry = FontRegistry::new();
    let face = FontFace::new(FontBytes::from_static(b"not a font"), 0);
    assert_eq!(
        registry.add_face("Test".into(), FontStyle::default(), face),
        None
    );
    assert!(registry.resolve_font(FontId(0)).is_none());
    let shaper = HarfRustShaper::new(Arc::new(registry));
    assert!(shaper.resolve_font(FontId(0)).is_none());
    assert!(shaper.registry().resolve_font(FontId(0)).is_none());
}

#[test]
fn a_generic_keyword_keeps_its_last_family() {
    let mut registry = FontRegistry::new();
    registry.set_generic(GenericFamily::Serif, "A".into());
    registry.set_generic(GenericFamily::Serif, "B".into());
    assert_eq!(registry.generics, [(GenericFamily::Serif, Arc::from("B"))]);
}

/// The backend shares parsed fonts between layout threads (spec SHP-1).
#[test]
fn parsed_fonts_and_the_backend_are_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Font>();
    check::<FontRegistry>();
    check::<HarfRustShaper>();
}

#[test]
fn an_empty_registry_answers_invalid_metrics_and_the_fallback_space() {
    let shaper = HarfRustShaper::new(Arc::new(FontRegistry::new()));
    let instance = FontInstanceRef {
        font: FontRef::unscaled(FontId(0)),
        font_size: 16.0,
        variations: &[],
    };
    assert!(!shaper.font_metrics(instance).is_valid());
    assert_eq!(
        shaper.space_advance(instance, GlyphOrientation::Horizontal),
        4.0
    );
}

#[test]
fn locales_name_their_cjk_region_from_subtags_alone() {
    use CjkRegion::*;
    for (tag, region) in [
        ("zh", Some(Simplified)),
        ("zh-CN", Some(Simplified)),
        ("zh-SG", Some(Simplified)),
        ("zh-Hans", Some(Simplified)),
        ("zh-Hans-TW", Some(Simplified)),
        ("zh-TW", Some(Traditional)),
        ("zh-Hant", Some(Traditional)),
        ("zh-Hant-TW", Some(Traditional)),
        ("zh-HK", Some(HongKong)),
        ("zh-MO", Some(HongKong)),
        ("zh-Hant-HK", Some(HongKong)),
        ("zh-Hans-HK", Some(Simplified)),
        ("yue", Some(HongKong)),
        ("ja", Some(Japanese)),
        ("ja-JP", Some(Japanese)),
        ("ko-KR", Some(Korean)),
        ("und-Hani", Some(Pan)),
        ("und", None),
        ("en", None),
        ("sr-Cyrl", None),
        ("zh-x-tw", Some(Simplified)),
    ] {
        let locale = LocaleTag::try_from_str(tag).expect("well formed");
        assert_eq!(locale_region(&locale), region, "{tag}");
    }
}

#[test]
fn one_vote_names_a_region_and_disagreement_names_the_fallback() {
    use CjkRegion::*;
    assert_eq!(single_region([], Some(Pan)), None);
    assert_eq!(
        single_region([Japanese, Japanese], Some(Pan)),
        Some(Japanese)
    );
    assert_eq!(single_region([Japanese, Korean], Some(Pan)), Some(Pan));
    assert_eq!(single_region([Japanese, Korean], None), None);
    // A generic typographic family name does not hide the legacy one.
    for (families, expected) in [
        (&["Foo Sans", "Foo Sans TC"][..], Some(Traditional)),
        (&["Foo Sans JP", "Foo Sans JP Thin"][..], Some(Japanese)),
        (&["Foo Sans SC", "Foo Sans TC"][..], None),
        (&["Foo Sans"][..], None),
        (&["Foo Sans TRADITIONALFOO"][..], None),
    ] {
        assert_eq!(family_region(families.iter().copied()), expected);
    }
    // Noto Sans JP sets JIS and GB 2312: code pages alone do not decide.
    assert_eq!(code_page_region(0b11 << 17), None);
    assert_eq!(code_page_region(1 << 20), Some(Traditional));
}

#[test]
fn a_slant_mismatch_outweighs_any_weight() {
    let regular = FontStyle::default();
    let italic = FontStyle {
        slant: FontSlant::Italic,
        ..regular
    };
    let black = FontStyle {
        weight: crate::types::FontWeight::try_new(900).unwrap(),
        ..regular
    };
    assert!(style_distance(black, regular) < style_distance(italic, regular));
    assert_eq!(style_distance(regular, regular), 0);
}
