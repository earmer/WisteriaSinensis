//! In-memory font registry and CJK regional-design detection (spec SHP-25).

use super::super::{FontFace, FontSource};
use super::metrics::{UnitMetrics, unit_metrics};
use crate::types::{
    FontFamily, FontId, FontSlant, FontStyle, GenericFamily, LocaleTag, ScriptTag, ShapingRun,
};
use core::fmt;
use read_fonts::TableProvider;
use read_fonts::model::Font;
use std::collections::HashSet;
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
    /// index. The bytes are shared, not copied (spec HST-1); they are parsed
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
        let bytes = face.bytes().shared_arc();
        let font = Font::new(bytes, face.face_index())?;
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

    /// The faces to try for `families` and `style`, best first: the best
    /// style match of each family of the stack in order, then every other
    /// face in registration order (the deterministic last resort, SHP-12).
    pub(super) fn candidates(&self, families: &[FontFamily], style: FontStyle) -> Vec<FontId> {
        let mut order: Vec<FontId> = Vec::new();
        for family in families {
            let name: Option<&str> = match family {
                FontFamily::Named(name) => Some(name.as_ref()),
                FontFamily::Generic(generic) => self
                    .generics
                    .iter()
                    .find(|(keyword, _)| keyword == generic)
                    .map(|(_, name)| name.as_ref()),
            };
            let Some(name) = name else { continue };
            let mut members: Vec<(u32, FontId)> = self
                .faces
                .iter()
                .enumerate()
                .filter(|(_, face)| face.family.eq_ignore_ascii_case(name))
                .map(|(index, face)| (style_distance(face.style, style), id_of(index)))
                .collect();
            members.sort_by_key(|&(distance, id)| (distance, id.0));
            order.extend(members.into_iter().map(|(_, id)| id));
        }
        order.extend((0..self.faces.len()).map(id_of));
        // Each face keeps its first position.
        let mut seen = HashSet::new();
        order.retain(|&id| seen.insert(id));
        order
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
    let mut found: Option<CjkRegion> = None;
    for record in list.script_records() {
        let Ok(script) = record.script(data) else {
            continue;
        };
        for lang in script.lang_sys_records() {
            let vote = match &lang.lang_sys_tag().to_be_bytes() {
                b"ZHS " => CjkRegion::Simplified,
                b"ZHT " => CjkRegion::Traditional,
                b"ZHH " | b"ZHTM" => CjkRegion::HongKong,
                b"JAN " => CjkRegion::Japanese,
                b"KOR " => CjkRegion::Korean,
                _ => continue,
            };
            match found {
                None => found = Some(vote),
                Some(region) if region == vote => {}
                Some(_) => return Some(CjkRegion::Pan),
            }
        }
    }
    found
}

fn name_region(font: &Font) -> Option<CjkRegion> {
    use read_fonts::model::name::NameId;
    let names = font.names();
    // Both the typographic and the legacy family name vote: a generic
    // typographic name must not hide a regional legacy one.
    let mut found = None;
    for id in [NameId::TYPOGRAPHIC_FAMILY_NAME, NameId::FAMILY_NAME] {
        let Some(name) = names.get(id).and_then(|entry| entry.english_or_first()) else {
            continue;
        };
        if !scan_chars_region(name.name().chars(), &mut found) {
            return None;
        }
    }
    found
}

pub(super) fn scan_chars_region(
    chars: impl IntoIterator<Item = char>,
    found: &mut Option<CjkRegion>,
) -> bool {
    let mut buf = [0u8; 12];
    let mut len = 0usize;
    let mut flush = |buf: &[u8; 12], len: &mut usize| -> bool {
        let word_len = *len;
        *len = 0;
        if word_len == 0 || word_len > buf.len() {
            return true;
        }
        let Ok(word) = core::str::from_utf8(&buf[..word_len]) else {
            return true;
        };
        if let Some(vote) = word_region(word) {
            match *found {
                None => *found = Some(vote),
                Some(region) if region == vote => {}
                Some(_) => return false,
            }
        }
        true
    };
    for c in chars {
        if c.is_ascii_alphanumeric() {
            if len < buf.len()
                && let Ok(byte) = u8::try_from(u32::from(c))
            {
                buf[len] = byte;
            }
            len += 1;
        } else if !flush(&buf, &mut len) {
            return false;
        }
    }
    flush(&buf, &mut len)
}

pub(super) fn word_region(word: &str) -> Option<CjkRegion> {
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
