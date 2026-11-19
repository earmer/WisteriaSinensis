//! In-memory font registry and CJK regional-design detection (spec SHP-25).

use super::metrics::{UnitMetrics, unit_metrics};
use crate::shaping::{FontFace, FontSource};
use crate::types::{
    FontFamily, FontId, FontSlant, FontStyle, GenericFamily, LocaleTag, ScriptTag, ShapingRun,
};
use core::fmt;
use read_fonts::TableProvider;
use read_fonts::model::Font;
use smallvec::SmallVec;
use std::sync::Arc;

/// The regional design a CJK face is built for (spec SHP-25).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CjkRegion {
    /// Simplified Chinese (`zh-Hans`, Mainland China and Singapore; OpenType `ZHS`).
    Simplified,
    /// Traditional Chinese as used in Taiwan (`zh-Hant`, `zh-TW`; OpenType `ZHT`).
    Traditional,
    /// Traditional Chinese as used in Hong Kong and Macao (`zh-HK`; OpenType `ZHH`).
    HongKong,
    /// Japanese (`ja`; OpenType `JAN`).
    Japanese,
    /// Korean (`ko`; OpenType `KOR`).
    Korean,
    /// Several regions in one face, told apart by `locl` under the run's
    /// language (spec SHP-16).
    Pan,
}

/// What a registered face is for, recorded once when it is registered
/// (spec SHP-25).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FaceProfile {
    /// The face is a CJK face: it maps core Han, kana, Hangul or Bopomofo, or
    /// was registered under a CJK locale. A Latin face that maps only shared
    /// punctuation is not.
    pub cjk: bool,
    /// The region a CJK face targets, when its tables or its registration
    /// state one; always `None` for a face that is not CJK.
    pub region: Option<CjkRegion>,
}

/// One registered face: the family and style it answers to, its bytes, the
/// parsed font at its default location and that location's metrics, in font
/// units, and its profile.
#[derive(Clone)]
pub(super) struct RegisteredFace {
    family: Arc<str>,
    style: FontStyle,
    face: FontFace,
    pub(super) font: Font,
    pub(super) metrics: UnitMetrics,
    pub(super) profile: FaceProfile,
}

impl fmt::Debug for RegisteredFace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredFace")
            .field("family", &self.family)
            .field("style", &self.style)
            .field("face", &self.face)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

/// An in-memory set of font faces: the id space of a
/// [`HarfRustShaper`](super::HarfRustShaper) and
/// the [`FontSource`] a renderer resolves its ids through (spec SHP-21,
/// HST-2).
///
/// Faces are added while the registry is being built; once it is shared it
/// no longer changes, so an id always names the same face. A `FontId` is the
/// index of its face in registration order and is never reused.
#[derive(Clone, Debug, Default)]
pub struct FontRegistry {
    faces: Vec<RegisteredFace>,
    pub(super) generics: Vec<(GenericFamily, Arc<str>)>,
}

impl FontRegistry {
    /// An empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `face` under `family` and `style` and returns its id, or
    /// `None`, registering nothing, when its bytes hold no font at its face
    /// index or the font's `units_per_em` is 0. The bytes are shared, not copied (spec HST-1); they are parsed
    /// here, once.
    ///
    /// The face's [`FaceProfile`] is read from its tables (spec SHP-25).
    pub fn add_face(
        &mut self,
        family: Arc<str>,
        style: FontStyle,
        face: FontFace,
    ) -> Option<FontId> {
        self.register(family, style, face, None)
    }

    /// Registers `face` like [`FontRegistry::add_face`], with the locale it
    /// is designed for stated by the host instead of read from its tables
    /// (spec SHP-25). A CJK locale (`zh-TW`, `ja`, `und-Hani` for a pan-CJK
    /// face, ...) makes it a CJK face of that region; any other locale makes
    /// it a non-CJK face; `und` reads the profile from the tables, as
    /// `add_face` does.
    pub fn add_face_with_locale(
        &mut self,
        family: Arc<str>,
        style: FontStyle,
        face: FontFace,
        locale: &LocaleTag,
    ) -> Option<FontId> {
        let profile = (locale.as_str() != "und").then(|| {
            let region = locale_region(locale);
            FaceProfile {
                cjk: region.is_some(),
                region,
            }
        });
        self.register(family, style, face, profile)
    }

    /// The profile recorded for `id` when it was registered, or `None` when
    /// the registry did not issue it (spec SHP-25).
    #[must_use]
    pub fn profile(&self, id: FontId) -> Option<FaceProfile> {
        self.face(id).map(|face| face.profile)
    }

    fn register(
        &mut self,
        family: Arc<str>,
        style: FontStyle,
        face: FontFace,
        profile: Option<FaceProfile>,
    ) -> Option<FontId> {
        let bytes = Arc::clone(&face.bytes().0);
        let font = Font::new(bytes, face.face_index())?;
        // Every metric is scaled by `1 / units_per_em` (SHP-21). The stated
        // value: `Metrics::units_per_em` replaces a 0 with 1000.
        if font.metrics().raw_units_per_em == 0 {
            return None;
        }
        let id = FontId(u32::try_from(self.faces.len()).ok()?);
        let metrics = unit_metrics(&font);
        let profile = profile.unwrap_or_else(|| face_profile(&font));
        self.faces.push(RegisteredFace {
            family,
            style,
            face,
            font,
            metrics,
            profile,
        });
        Some(id)
    }

    /// Names the family a generic keyword resolves to; a later call for the
    /// same keyword replaces the earlier one.
    pub fn set_generic(&mut self, generic: GenericFamily, family: Arc<str>) {
        self.generics.retain(|(keyword, _)| *keyword != generic);
        self.generics.push((generic, family));
    }

    pub(super) fn face(&self, id: FontId) -> Option<&RegisteredFace> {
        self.faces.get(usize::try_from(id.0).ok()?)
    }

    fn resolve_family_name<'a>(&'a self, family: &'a FontFamily) -> Option<&'a str> {
        match family {
            FontFamily::Named(name) => Some(name.as_ref()),
            FontFamily::Generic(generic) => self
                .generics
                .iter()
                .find(|(keyword, _)| keyword == generic)
                .map(|(_, name)| name.as_ref()),
        }
    }

    /// The first candidate face for `families` and `style`, without allocating
    /// the full fallback order.
    pub(super) fn primary_candidate(
        &self,
        families: &[FontFamily],
        style: FontStyle,
    ) -> Option<FontId> {
        for family in families {
            let Some(name) = self.resolve_family_name(family) else {
                continue;
            };
            if let Some(best) = self
                .faces
                .iter()
                .enumerate()
                .filter(|(_, face)| face.family.eq_ignore_ascii_case(name))
                .min_by_key(|&(index, face)| (style_distance(face.style, style), id_of(index).0))
                .map(|(index, _)| id_of(index))
            {
                return Some(best);
            }
        }
        (!self.faces.is_empty()).then_some(FontId(0))
    }

    /// The pre-resolved faces to try for `families` and `style`, best first:
    /// the best style match of each family of the stack in order, then every
    /// other face in registration order (the deterministic last resort, SHP-12).
    pub(super) fn resolve_candidates(
        &self,
        families: &[FontFamily],
        style: FontStyle,
    ) -> SmallVec<[ResolvedCandidate<'_>; 8]> {
        let mut order: SmallVec<[ResolvedCandidate<'_>; 8]> = SmallVec::new();
        let mut seen: SmallVec<[bool; 64]> = SmallVec::from_elem(false, self.faces.len());
        for family in families {
            let Some(name) = self.resolve_family_name(family) else {
                continue;
            };
            let start = order.len();
            for (index, face) in self.faces.iter().enumerate() {
                if !seen[index] && face.family.eq_ignore_ascii_case(name) {
                    seen[index] = true;
                    order.push(ResolvedCandidate {
                        id: id_of(index),
                        face,
                    });
                }
            }
            order[start..].sort_by_key(|cand| (style_distance(cand.face.style, style), cand.id.0));
        }
        for (index, face) in self.faces.iter().enumerate() {
            if !seen[index] {
                seen[index] = true;
                order.push(ResolvedCandidate {
                    id: id_of(index),
                    face,
                });
            }
        }
        order
    }
}

/// One candidate face resolved once per run so face selection works directly
/// on in-memory references with zero registry lookups inside the per-atom loop.
#[derive(Clone, Copy)]
pub(super) struct ResolvedCandidate<'a> {
    pub(super) id: FontId,
    pub(super) face: &'a RegisteredFace,
}

impl ResolvedCandidate<'_> {
    pub(super) fn profile(self) -> FaceProfile {
        self.face.profile
    }
}

fn id_of(index: usize) -> FontId {
    // `add_face` refuses an index that does not fit.
    FontId(u32::try_from(index).unwrap_or(u32::MAX))
}

/// How far `face` is from the requested `style`: a slant mismatch outweighs
/// any weight difference.
pub(super) fn style_distance(face: FontStyle, wanted: FontStyle) -> u32 {
    let slant = match (face.slant, wanted.slant) {
        (a, b) if a == b => 0,
        (FontSlant::Italic, FontSlant::Oblique) | (FontSlant::Oblique, FontSlant::Italic) => 1,
        _ => 2,
    };
    slant * 10_000 + u32::from(face.weight.get().abs_diff(wanted.weight.get()))
}

impl FontSource for FontRegistry {
    fn resolve_font(&self, font: FontId) -> Option<FontFace> {
        self.face(font).map(|registered| registered.face.clone())
    }
}

/// Taiwanese), else none, which leaves stack order (spec SHP-25).
pub(super) fn preferred_regions(run: &ShapingRun) -> &'static [CjkRegion] {
    use CjkRegion::{HongKong, Japanese, Korean, Simplified, Traditional};
    match locale_region(&run.locale) {
        Some(Simplified) => &[Simplified],
        Some(Traditional) => &[Traditional, HongKong],
        Some(HongKong) => &[HongKong, Traditional],
        Some(Japanese) => &[Japanese],
        Some(Korean) => &[Korean],
        Some(CjkRegion::Pan) => &[],
        None => match run.script {
            s if s == ScriptTag::HIRAGANA || s == ScriptTag::KATAKANA => &[Japanese],
            s if s == ScriptTag::HANGUL => &[Korean],
            s if s == ScriptTag::BOPOMOFO => &[Traditional],
            _ => &[],
        },
    }
}

/// The CJK region a locale names, read from its subtags alone (spec
/// SHP-25): Chinese by script, then region (`Hant` is Traditional, and Hong
/// Kong's under `HK` or `MO`; `Hans` Simplified; `TW` Traditional; `HK` and
/// `MO` Hong Kong; any other or none Simplified), Cantonese Hong Kong unless
/// `Hans`, `ja` Japanese, `ko` Korean, and `und-Hani` pan-CJK. Any other
/// locale names none.
pub(super) fn locale_region(locale: &LocaleTag) -> Option<CjkRegion> {
    let mut subtags = locale.as_str().split('-');
    let language = subtags.next()?;
    let mut script: Option<&str> = None;
    let mut region: Option<&str> = None;
    for subtag in subtags {
        if subtag.len() == 1 {
            break; // an extension or private use: nothing further is a region
        }
        if subtag.len() == 4 && subtag.bytes().all(|b| b.is_ascii_alphabetic()) {
            script = script.or(Some(subtag));
        } else if subtag.len() == 2
            || (subtag.len() == 3 && subtag.bytes().all(|b| b.is_ascii_digit()))
        {
            region = region.or(Some(subtag));
        }
    }
    let eq = |opt: Option<&str>, target: &str| opt.is_some_and(|s| s.eq_ignore_ascii_case(target));
    let hong_kong = eq(region, "HK") || eq(region, "MO");
    if language.eq_ignore_ascii_case("zh") || language.eq_ignore_ascii_case("cmn") {
        Some(if eq(script, "hant") {
            if hong_kong {
                CjkRegion::HongKong
            } else {
                CjkRegion::Traditional
            }
        } else if eq(script, "hans") {
            CjkRegion::Simplified
        } else if eq(region, "TW") {
            CjkRegion::Traditional
        } else if hong_kong {
            CjkRegion::HongKong
        } else {
            CjkRegion::Simplified
        })
    } else if language.eq_ignore_ascii_case("yue") {
        Some(if eq(script, "hans") {
            CjkRegion::Simplified
        } else {
            CjkRegion::HongKong
        })
    } else if language.eq_ignore_ascii_case("ja") {
        Some(CjkRegion::Japanese)
    } else if language.eq_ignore_ascii_case("ko") {
        Some(CjkRegion::Korean)
    } else if language.eq_ignore_ascii_case("und") {
        match script?.to_ascii_lowercase().as_str() {
            "hani" => Some(CjkRegion::Pan),
            "hans" => Some(CjkRegion::Simplified),
            "hant" => Some(CjkRegion::Traditional),
            "jpan" | "hira" | "kana" => Some(CjkRegion::Japanese),
            "kore" | "hang" => Some(CjkRegion::Korean),
            _ => None,
        }
    } else {
        None
    }
}

/// Code points only a CJK face maps: core Han, kana, Hangul and Bopomofo.
const CJK_PROBES: [char; 12] = [
    '一', '人', '中', '大', '日', '国', 'あ', 'の', 'ア', '가', '한', 'ㄅ',
];

/// The profile of `font`, read from its tables (spec SHP-25). The face is
/// CJK when its character map holds a probe, or when its `OS/2` table
/// claims both a CJK Unicode range and a CJK code page. Its region comes
/// from the `GSUB` language systems first (one of `ZHS`, `ZHT`, `ZHH`,
/// `ZHTM`, `JAN`, `KOR` names it; several make it pan-CJK), else from its
/// typographic and legacy family names (a word `SC`, `TC`, `HK`, `JP`, `KR` or their spelled-out
/// forms), else from its `OS/2` code pages when they name exactly one
/// region; otherwise it is unknown.
fn face_profile(font: &Font) -> FaceProfile {
    let charmap = font.charmap();
    let tables = font.tables();
    let os2 = tables.os2().ok();
    let pages = os2
        .as_ref()
        .and_then(|os2| os2.ul_code_page_range_1())
        .unwrap_or(0);
    let claims_cjk = os2.as_ref().is_some_and(|os2| {
        // Bits 48-59 cover CJK symbols, kana, Bopomofo, Hangul and Han
        // (ulUnicodeRange2 bits 16-27).
        os2.ul_unicode_range_2() & 0x0FFF_0000 != 0
    }) && pages & CJK_CODE_PAGES != 0;
    let cjk = CJK_PROBES.iter().any(|&c| charmap.map_unicode(c).is_some()) || claims_cjk;
    if !cjk {
        return FaceProfile::default();
    }
    let region = gsub_region(font)
        .or_else(|| name_region(font))
        .or_else(|| code_page_region(pages));
    FaceProfile { cjk, region }
}

/// `OS/2` code page bits 17 (JIS), 18 (GB 2312), 19 (Wansung), 20 (Big5)
/// and 21 (Johab).
const CJK_CODE_PAGES: u32 = 0b1_1111 << 17;

/// The single region several region votes agree on, pan-CJK when they
/// disagree, none without a vote.
pub(super) fn single_region(
    votes: impl IntoIterator<Item = CjkRegion>,
    many: Option<CjkRegion>,
) -> Option<CjkRegion> {
    let mut found: Option<CjkRegion> = None;
    for vote in votes {
        match found {
            None => found = Some(vote),
            Some(region) if region == vote => {}
            Some(_) => return many,
        }
    }
    found
}

fn gsub_region(font: &Font) -> Option<CjkRegion> {
    let gsub = font.tables().gsub().ok()?;
    let list = gsub.script_list().ok()?;
    let data = list.offset_data();
    let votes = list
        .script_records()
        .iter()
        .filter_map(|record| record.script(data).ok())
        .flat_map(|script| {
            script
                .lang_sys_records()
                .iter()
                .filter_map(|lang| match &lang.lang_sys_tag().to_be_bytes() {
                    b"ZHS " => Some(CjkRegion::Simplified),
                    b"ZHT " => Some(CjkRegion::Traditional),
                    b"ZHH " | b"ZHTM" => Some(CjkRegion::HongKong),
                    b"JAN " => Some(CjkRegion::Japanese),
                    b"KOR " => Some(CjkRegion::Korean),
                    _ => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    single_region(votes, Some(CjkRegion::Pan))
}

fn name_region(font: &Font) -> Option<CjkRegion> {
    use read_fonts::model::name::NameId;
    let names = font.names();
    let families: Vec<String> = [NameId::TYPOGRAPHIC_FAMILY_NAME, NameId::FAMILY_NAME]
        .into_iter()
        .filter_map(|id| Some(names.get(id)?.english_or_first()?.name().chars().collect()))
        .collect();
    family_region(families.iter().map(String::as_str))
}

/// The region the words of `families` vote for. Both the typographic and the
/// legacy family name vote: a generic typographic name must not hide a
/// regional legacy one.
pub(super) fn family_region<'a>(families: impl IntoIterator<Item = &'a str>) -> Option<CjkRegion> {
    let votes = families.into_iter().flat_map(|family| {
        family
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter_map(word_region)
    });
    single_region(votes, None)
}

fn word_region(word: &str) -> Option<CjkRegion> {
    match word.to_ascii_uppercase().as_str() {
        "SC" | "CN" | "GB" | "HANS" | "SIMPLIFIED" => Some(CjkRegion::Simplified),
        "TC" | "TW" | "HANT" | "TRADITIONAL" => Some(CjkRegion::Traditional),
        "HK" | "HKSCS" => Some(CjkRegion::HongKong),
        "JP" | "JA" | "JAPANESE" => Some(CjkRegion::Japanese),
        "KR" | "KO" | "KOREAN" => Some(CjkRegion::Korean),
        _ => None,
    }
}

pub(super) fn code_page_region(pages: u32) -> Option<CjkRegion> {
    let votes = [
        (17, CjkRegion::Japanese),
        (18, CjkRegion::Simplified),
        (19, CjkRegion::Korean),
        (20, CjkRegion::Traditional),
        (21, CjkRegion::Korean),
    ]
    .into_iter()
    .filter(|(bit, _)| pages >> bit & 1 == 1)
    .map(|(_, region)| region);
    single_region(votes, None)
}
