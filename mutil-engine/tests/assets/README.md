# Test assets

## `JetBrainsMono-Subset.ttf`

A subset of JetBrains Mono Regular, version 2.304, used by the pipeline tests
(`tests/pipeline/`) as the one real face the HarfRust backend shapes with.

- **Source:** `JetBrainsMono-Regular.ttf` from the Debian package
  `fonts-jetbrains-mono` (upstream <https://github.com/JetBrains/JetBrainsMono>),
  SHA-256 `bb03b32b13f66bb799975d478d3725fbed9101728d46ffda995e87d7847fda2d`.
- **Licence:** SIL Open Font License 1.1, in `OFL-JetBrainsMono.txt` beside this file. The
  font declares no Reserved Font Name, so the subset keeps its name.
- **Subset:** made with fontTools 4.68.0:

  ```bash
  pyftsubset JetBrainsMono-Regular.ttf \
    --unicodes="U+0020-007E,U+00A0,U+00E9,U+0301,U+2026" \
    --layout-features='ccmp,locl,mark,mkmk,calt' \
    --no-hinting --name-IDs='*' --drop-tables+=DSIG \
    --output-file=JetBrainsMono-Subset.ttf
  ```

  SHA-256 of the result:
  `ba601e58ee9ae0980fb3c157f5ecbc8fee5da728059cff8b3f324c257ce30c19`.

What the tests rely on: 1000 units per em; every spacing glyph, the space
included, advances 600 units; U+0301 is a zero-advance mark that `mark`
positions over its base; OS/2 `fsSelection` bit 7 (`USE_TYPO_METRICS`) is set,
with typo ascender 1020, descender -300 and line gap 0; `post`
`underlinePosition` is -155 and OS/2 `yStrikeoutPosition` 320. There is no
`BASE`, `vhea` or GDEF ligature caret table, so those metrics take the
backend's documented fallbacks.


## Script fonts

Seven subsets of Noto fonts, registered by `SCRIPT_SHAPER` in
`tests/pipeline/common.rs`, give the acceptance tests (spec RM-4) a
proportional Latin face with ligatures and one face per script they exercise.

- **Source:** the variable fonts of the Google Fonts repository,
  `https://raw.githubusercontent.com/google/fonts/main/ofl/<dir>/<file>`,
  fetched on 2026-11-15, with these SHA-256 hashes:

  | Subset | Source file (`<dir>/<file>`) | SHA-256 of the source |
  | --- | --- | --- |
  | `NotoSerif-Subset.ttf` | `notoserif/NotoSerif[wdth,wght].ttf` | `4d8e6761424656867019081a1a01336f3cb086982682698714054fc33f782713` |
  | `NotoSansDevanagari-Subset.ttf` | `notosansdevanagari/NotoSansDevanagari[wdth,wght].ttf` | `14ec4af41f27482216d1c2229f417ff9b1425e1babb014e57d1d40d03229853e` |
  | `NotoSansJP-Subset.ttf` | `notosansjp/NotoSansJP[wght].ttf` | `c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f` |
  | `NotoSansThai-Subset.ttf` | `notosansthai/NotoSansThai[wdth,wght].ttf` | `5a1c559bb539583c8a1fd99d1c5b9491e5e14478c9cd2bd0970d5c3096cc9ef8` |
  | `NotoSansLao-Subset.ttf` | `notosanslao/NotoSansLao[wdth,wght].ttf` | `9608b94603a82d09a8038946f9775242f99e3b3459b7f1e4d5b335b578cd7ab3` |
  | `NotoSansKhmer-Subset.ttf` | `notosanskhmer/NotoSansKhmer[wdth,wght].ttf` | `f37a8431a0c5d5ed2f81a767417546aca576a81fb7eff9c924d46aecf828f2ca` |
  | `NotoSansMyanmar-Subset.ttf` | `notosansmyanmar/NotoSansMyanmar[wdth,wght].ttf` | `7abbbfbe2514105d7ce94937aee3feb2ba89b73a256c8b77b5866bd9b83e32ec` |

- **Licence:** SIL Open Font License 1.1, each in `OFL-<Family>.txt` beside
  this file (`OFL-NotoSerif.txt` and so on), copied from the same directory
  of the repository. Only Noto Sans JP declares a Reserved Font Name,
  “Source”; its subset is named Noto Sans JP, which does not use it.
- **Subset:** made with fontTools 4.68.0 in two steps. First a static
  Regular instance (`wght=400`, and `wdth=100` where the font has the axis):

  ```bash
  fonttools varLib.instancer SOURCE.ttf wght=400 wdth=100 --static -o static.ttf
  # Noto Sans JP, whose only axis is wght, also needs its names updated:
  fonttools varLib.instancer NotoSansJP[wght].ttf wght=400 --static \
    --update-name-table -o static.ttf
  ```

  Then the subset, with every layout feature kept:

  ```bash
  pyftsubset static.ttf --unicodes="RANGES" [--text="HAN"] \
    --no-hinting --name-IDs='*' --drop-tables+=DSIG --layout-features='*' \
    --output-file=SUBSET.ttf
  ```

  | Subset | `RANGES` | SHA-256 of the result |
  | --- | --- | --- |
  | `NotoSerif-Subset.ttf` | `U+0020-007E,U+00A0,U+00E9,U+0301,U+2013-2014,U+2018-2019,U+201C-201D,U+2026` | `47298b242832c75319cc38adf78503fbea9ce194e202489b14ce0b57d515bf74` |
  | `NotoSansDevanagari-Subset.ttf` | `U+0020,U+0900-097F,U+200C-200D,U+25CC` | `eb1c337f592065ddf0d5714d619f14c9c9630968311cb22da223146be0390435` |
  | `NotoSansJP-Subset.ttf` | regenerated: see *CJK routing fonts* below | see below |
  | `NotoSansThai-Subset.ttf` | `U+0020,U+0E01-0E5B,U+200B,U+25CC` | `6bbdc5eeea43cb103bb3dad01c721477b7840eba0ee36bfe8b1e38b3f6578ac4` |
  | `NotoSansLao-Subset.ttf` | `U+0020,U+0E81-0EDF,U+200B,U+25CC` | `0dff5f94bfbabfdb51c6769454cb157591a53a6f8e890e3da043c49fde192a0b` |
  | `NotoSansKhmer-Subset.ttf` | `U+0020,U+1780-17F9,U+19E0-19FF,U+200B,U+25CC` | `d0645e32e2d8c3f6db60ddf9e978ecba7cd9c78277bc2df27db4f2b2a2968a79` |
  | `NotoSansMyanmar-Subset.ttf` | `U+0020,U+1000-109F,U+200B,U+25CC` | `59f5afedc4204f64d056d1b91be27dfb7e8455f69abe19f6b2cb93c8425551a3` |

  Noto Sans JP also takes `--text` with these Han characters:
  `日本語中文漢字排版測試東京北京人大小上下山川水火木金土月年時分秒国言書読話見行来出入前後左右名前今何私彼女子学生先会社電車駅道`.

What the tests rely on: every face has 1000 units per em. Noto Serif forms
`fi`, `fl`, `ff`, `ffi` and `ffl` through `liga`, and its GDEF table gives
them ligature carets (`fi` at 332, `ffi` at 334 and 668); its space advances
260 units. Noto Sans JP has `BASE` (`ideo` at -120, `romn` at 0, `icfb` at
-78 and `icft` at 838 for every script) and `vhea` (ascent 500, descent
-500), and `vert`. The Devanagari, Thai, Lao, Khmer and Myanmar faces cover
their whole blocks and keep their shaping features.

## CJK routing fonts

Five subsets, registered by `CJK_SHAPER` in `tests/pipeline/cjk_routing.rs`
(with `NotoSerif-Subset.ttf` above), give the SHP-12, SHP-16 and SHP-25 tests
a Latin sans face that shares CJK punctuation, the three regional CJK faces
and one pan-CJK face. The Noto Sans JP subset is the one the script fonts
section lists, regenerated with these ranges.

- **Source:** fetched on 2026-11-15. The Google Fonts files come from
  `https://raw.githubusercontent.com/google/fonts/main/ofl/<dir>/<file>`;
  Noto Sans CJK SC comes from
  `https://raw.githubusercontent.com/notofonts/noto-cjk/main/Sans/OTF/SimplifiedChinese/NotoSansCJKsc-Regular.otf`.

  | Subset | Source file | SHA-256 of the source |
  | --- | --- | --- |
  | `NotoSans-Subset.ttf` | `notosans/NotoSans[wdth,wght].ttf` | `bfb7bb691513f12e734dc346c03a03f784912432d7e3fa8e56efcf906fe86b3d` |
  | `NotoSansSC-Subset.ttf` | `notosanssc/NotoSansSC[wght].ttf` | `a3041811a78c361b1de50f953c805e0244951c21c5bd412f7232ef0d899af0da` |
  | `NotoSansTC-Subset.ttf` | `notosanstc/NotoSansTC[wght].ttf` | `864727d210d54f2537bbe23b3a839436c3992af72de9322af5270897246bd44f` |
  | `NotoSansJP-Subset.ttf` | `notosansjp/NotoSansJP[wght].ttf` | `c2f3b4d463500a2ddcd3849cded1fceeb9fd6d1c32e6cbecd568453ba50fc68f` |
  | `NotoSansCJKsc-Subset.otf` | `NotoSansCJKsc-Regular.otf` (static) | `2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b` |

- **Licence:** SIL Open Font License 1.1, in `OFL-NotoSans.txt`,
  `OFL-NotoSansSC.txt`, `OFL-NotoSansTC.txt`, `OFL-NotoSansJP.txt` and
  `OFL-NotoSansCJK.txt`. The SC, TC and JP files declare the Reserved Font
  Name “Source”, which none of the subset names uses.
- **Subset:** fontTools 4.68.0, the same two steps as the script fonts (a
  static instance at `wght=400`, `wdth=100` for Noto Sans, with
  `--update-name-table` for the SC, TC and JP faces; the CJK OTF is already
  static), then `pyftsubset` with `--layout-features='*'`:

  | Subset | `RANGES` | `--text` | SHA-256 of the result |
  | --- | --- | --- | --- |
  | `NotoSans-Subset.ttf` | `U+0020-007E,U+00A0,U+00B7,U+00E9,U+0301,U+2013-2014,U+2018-2019,U+201C-201D,U+2026` | none | `d80764c0bd182cc5445bc52e9bea54149a552983b77f9e66e0c1d42d7eb6c1ab` |
  | `NotoSansJP-Subset.ttf` | `CJK` | `HAN` + `令你甘心骨肉分离離` | `b6a6ec6da91d77ced9f4e5470bb54dd7f5e624026bc82fc2d00a1860b3f48d9c` |
  | `NotoSansSC-Subset.ttf` | `CJK` | `HAN` + `令你甘心骨肉分离離们` | `620ab7f5e3ea4bcc8dc5bc33442098c75a4b9c286f19f62cc72d7e9632b8adb3` |
  | `NotoSansTC-Subset.ttf` | `CJK` | `HAN` + `令你甘心骨肉分离離` | `d452a43e8a05f4116e2e51ff6362493deebecf4f9eee5d1bdc82ee94dbac0325` |
  | `NotoSansCJKsc-Subset.otf` | `U+0020,U+3001-3002,U+FF0C` | `令骨中人東京` | `c0ca77edc8d03192e37cfcd0f5f8c7be423929ee56ba65dcf5baba88a05f29a3` |

  `CJK` is `U+0020,U+00B7,U+2014-2015,U+2018-2019,U+201C-201D,U+2026,U+2E3A-2E3B,U+3000-303F,U+3041-3096,U+309B-309F,U+30A0-30FF,U+FF01-FF5E`,
  the Noto Sans JP ranges of the script fonts plus U+00B7, and `HAN` the Han
  characters listed there. The result hashes depend on fontTools'
  timestamps, so a rebuild matches the glyphs, not the bytes.

What the tests rely on: 1000 units per em. Noto Sans maps “ ” … — · with
proportional advances (359, 359, 791, 1000 and 268 units) and no CJK
ideograph. Noto Sans SC and TC give “ ” ‘ ’ … · full-width 1000-unit glyphs;
Noto Sans JP gives … and the corner brackets 1000 units but “ ” and ·
proportional ones (about 474 and 561). Their `GSUB` language systems are
`ZHS`, `ZHT` and `JAN` alone; Noto Sans CJK SC lists `JAN`, `KOR`, `ZHH` and
`ZHT`, whose `locl` lookups change 令 and 骨. Only the SC subset maps 们.

## Right-to-left fonts

Two subsets give the Phase 2 tests (spec RM-6, RM-14) a Hebrew and an
Arabic face: `NotoSansHebrew-Subset.ttf` and `NotoNaskhArabic-Subset.ttf`.

- **Source:** the variable fonts of the Google Fonts repository,
  `https://raw.githubusercontent.com/google/fonts/main/ofl/<dir>/<file>`,
  fetched on 2026-11-16, with these SHA-256 hashes:

  | Subset | Source file (`<dir>/<file>`) | SHA-256 of the source |
  | --- | --- | --- |
  | `NotoSansHebrew-Subset.ttf` | `notosanshebrew/NotoSansHebrew[wdth,wght].ttf` | `7ef36a2c3593758cdb622e1bdef4f84523e92fbc3ccc667438dd80ff54c2de88` |
  | `NotoNaskhArabic-Subset.ttf` | `notonaskharabic/NotoNaskhArabic[wght].ttf` | `67b5a525a661b607971fbd3f96a81b89d3a768e74534fca84f18ac97e6fab72f` |

- **Licence:** SIL Open Font License 1.1, in `OFL-NotoSansHebrew.txt` and
  `OFL-NotoNaskhArabic.txt`, copied from the same directories. Neither
  declares a Reserved Font Name.
- **Subset:** fontTools 4.68.0, the same two steps as the script fonts: a
  static Regular instance with its names updated
  (`wght=400`, plus `wdth=100` for Hebrew, `--update-name-table`), then
  `pyftsubset` with `--layout-features='*'`, `--no-hinting`,
  `--name-IDs='*'` and `--drop-tables+=DSIG`:

  | Subset | `RANGES` | SHA-256 of the result |
  | --- | --- | --- |
  | `NotoSansHebrew-Subset.ttf` | `U+0020-007E,U+00A0,U+00AB,U+00BB,U+0591-05F4,U+200E-200F,U+25CC,U+FB1D-FB4F` | `82df632084ba813106f74205c8bc445a1fed981a94479ce112dabaee90c0ed8b` |
  | `NotoNaskhArabic-Subset.ttf` | `U+0020-007E,U+00A0,U+00AB,U+00BB,U+0600-06FF,U+0750-077F,U+200C-200F,U+25CC,U+FB50-FDFF,U+FE70-FEFF` | `2a80ab7f21a8aa8cfd01cb089188b58bee58286af2c63c8eb648a14642824adf` |

What the tests rely on: both faces have 1000 units per em and map the ASCII
brackets, digits, `.`, `,`, « and ». Noto Sans Hebrew's space advances 270
units and א 632. Noto Naskh Arabic's space advances 221 units; it joins
Arabic letters through `init`, `medi` and `fina`. Its only GDEF caret list
entry is U+FDF2, the ligature `liga` forms for اللّٰه, spelt with U+0651
SHADDA and U+0670 SUPERSCRIPT ALEF (plain الله shapes as four joined
glyphs). It is 1190 units wide, with carets at 453, 692 and 960, which are
not evenly spaced.

## `WisteriaVarTest.ttf`

A synthetic variable font for the variable-font tests, built by
`build_variable_test_font.py` beside this file. It has no upstream and draws
nothing real: each glyph is a rectangle plus a serif foot whose width grows
from 0, so the tests can predict every number it gives.

- **Licence:** written for this repository and covered by its licence.
- **Build:** `python3 -I build_variable_test_font.py` with fontTools 4.60.2
  (`fontTools.varLib` from seven masters, with the date pinned). SHA-256 of the
  result:
  `6283bc286483bd9c0e104821b74dfd2edbe84a40bdf38d3f9403d7a3d431089e`.

What the tests rely on (all in font units, 1000 per em):

- **Axes (`fvar`):** `wght` 100–400–900 and `wdth` 75–100–125 (registered),
  and `SERF` 0–0–100, a custom sans-to-serif axis. `avar` eases the upper
  half of `wght`: 650 is normalised to 0.25, not 0.5.
- **Glyphs:** `.notdef` 0, `space` 1, `H` 2, `a` 3, `a.serif` 4, `f` 5, `i` 6,
  `f_i` 7, `o` 8, `x` 9. `liga` forms `f_i` from `f i` at every location.
  From `SERF` 50 (normalised 0.5) a feature variation (`rvrn`) replaces `a`
  with `a.serif`, a different glyph with a wider advance: a structural change
  that no delta can make.
- **Metrics at the default location:** `OS/2` `fsSelection` bit 7
  (`USE_TYPO_METRICS`) is set. Typo and `hhea` ascender 800, descender −200,
  line gap 0. Win ascent 900, descent 300. Underline position −100,
  thickness 50. Strike-out position 300. x-height 500, cap height 700. No
  `vhea`, `BASE` or `STAT`.
- **`MVAR`** varies `hasc`, `hdsc`, `hlgp`, `undo`, `unds` and `xhgt`; the
  `hhea` and typo line metrics move together. The deltas at each master:

  | Location | ascender | descender | line gap | underline pos | underline thick | x-height |
  | --- | --- | --- | --- | --- | --- | --- |
  | `wght` 900 | +80 | −40 | +20 | 0 | +40 | +20 |
  | `wght` 100 | −20 | 0 | 0 | 0 | −20 | 0 |
  | `wdth` 75 or 125 | 0 | 0 | 0 | 0 | 0 | 0 |
  | `SERF` 100 | +30 | −30 | 0 | −30 | 0 | 0 |
  | `wght` 900 + `SERF` 100 | +150 | −70 | +20 | −30 | +40 | +20 |

  The corner is 40 units above the sum of its two axes' ascender deltas, so
  `MVAR` reaches it only through a two-axis region. Between masters the
  deltas interpolate linearly in normalised coordinates, unrounded: `SERF` 49
  gives ascender 814.7.
- **Advances (`HVAR`, `gvar`):** default `a` 500, `a.serif` 560, `f` 300,
  `i` 250, `f_i` 500, `H` 700, `space` 250. `wght` 900 adds 100 and `wght`
  100 subtracts 50 from each, the space excepted. `SERF` 100 adds 40, the
  space excepted. `wdth` scales each default advance by `wdth / 100`, the
  space included, and the axes' changes add up: at `wght` 900, `wdth` 125,
  `SERF` 100, `f_i` advances 765.
- **Ligature caret (`GDEF`):** `f_i` has one caret, format 3, at 250 by
  default and 300 at `wght` 900, varying with `wght` only.
- Values outside an axis's range clamp to its end; a tag the font lacks is
  ignored.

## Unicode conformance data (`ucd/`)

`GraphemeBreakTest.txt`, `WordBreakTest.txt`, `LineBreakTest.txt`,
`BidiTest.txt` and `BidiCharacterTest.txt` of Unicode 18.0.0, for the RM-1
suites in `src/frontend/conformance.rs`.

- **Source:** `https://www.unicode.org/Public/18.0.0/ucd/auxiliary/<file>`,
  fetched on 2026-11-15, under the Unicode terms of use
  (<https://www.unicode.org/terms_of_use.html>), with these SHA-256 hashes:
  `GraphemeBreakTest.txt`
  `b0cf047ee94485bbdc846de2b902f5f8a815f6b674f9d04223cddadd91c9df31`,
  `WordBreakTest.txt`
  `3dd70c071781276067c680d87303f60434adce7b067bd063194af374edad86a5`,
  `LineBreakTest.txt`
  `fd9ff9eca411080085b41471bba79ae19d9db2e16924f9770b8509f1affd3f95`.
  `BidiTest.txt` and `BidiCharacterTest.txt` come from
  `https://www.unicode.org/Public/18.0.0/ucd/<file>`, fetched on
  2026-11-16, with SHA-256 hashes
  `9af2f882a4ab50912e388f069a673b94eacd82fa6d07d20a3ff7f3c759e905aa` and
  `045b24d2c8ab066951bd32fe8c6b4de34647f72b5b1c7df0265f24ab53573e01`.
- **Form:** each file keeps its first line, which names the version, and its
  test lines without their trailing comments; comment-only and blank lines
  are dropped:

  ```bash
  { head -1 FILE; sed -E 's/[[:space:]]*#.*$//; /^$/d' FILE; } > ucd/FILE
  ```

  SHA-256 of the results: `GraphemeBreakTest.txt`
  `503a664752b6b4ff9e63730ca5c8bb06d3db4e16964c10bf32a74e484383b1c3`,
  `WordBreakTest.txt`
  `fc15cceefbccdbdc05a56fbf0f2e3068c6c7020c3c3d32504ee4c48127ce9ef7`,
  `LineBreakTest.txt`
  `fcbe49dbcc8caac9319e62d6657d49a9ed86ccb5ee4407bd2c797464639a3dc8`,
  `BidiTest.txt`
  `da141dc8c4d12332e9fb339b90eb7773a790cdb4c5e0eac6765314325a2bfb84`,
  `BidiCharacterTest.txt`
  `f064875576804aa4cfa28b281c2b08544301e97548b51c0faad6ba89188937ac`.
- **`known-differences.txt`** lists, by line number in these files, the cases
  where ICU4X 2.4's stable rule tables, which predate Unicode 18.0.0, break
  differently: 10 grapheme cases (the 18.0 form of GB9c) and 328 line-break
  cases (the stable line segmenter's Unicode 15.1 rules). The word suite
  passes in full.
