# Technical Specification for a Multilingual Universal Typesetting and Interactive Layout Engine (revised draft, research date 2026-11-07)

**Status of this document.** This is a revision of the user's draft specification, rebuilt against the reference documents and implementations catalogued in the research notes. The draft's section structure (1 to 7), three-tier architecture, data-structure names and phased roadmap are retained wherever they survive scrutiny. Every requirement carries one of three labels:

- **[Sourced]**: backed by a reference listed in the References section. Reference tags such as `[U3]` point to that list.
- **[Design decision]**: a choice made by this specification. It may be informed by sources but is not stated by them.
- **[Unverified]**: asserted by the draft or by background knowledge, but not confirmed by the research. It is kept because it is plausible and useful, and it must be validated before it is relied upon.

Version-specific facts carry their dates. Where a source page is itself marked draft, proposed or preliminary, that is said so. Non-official sources are flagged in the References.

**Implementation status.** The reference crate `mutil-engine` provides the public data model of section 2, coordinate transforms, input and output validators, checked geometry queries and a non-rendering `NoopShaper`, all covered by contract tests. It does not yet implement Unicode analysis (section 3), line fitting, boundary re-shaping, justification, ruby placement, clamping or the interactive queries of section 6: `layout_paragraph` validates its input and returns an empty paragraph. Sections 3, 5 and 6 therefore state what an implementation must do, marked *future* where the crate has no code; the signatures and validation rules of sections 2 and 4 are those the crate enforces.

**Summary of what changed.** The draft's architecture stands. Its main errors fall into six groups. First, break safety: `is_safe_to_break_after` inverts the HarfBuzz flag's polarity and omits `UNSAFE_TO_CONCAT`, which is the flag that matters for hyphenation `[B1]`. Second, Unicode conformance: the bracket table cannot be extended with quotation marks without breaking UAX #9 conformance, L1 and L2 are mis-stated, and grapheme segmentation needs the Indic conjunct rule GB9c `[U2][U3]`. Third, line breaking: "unified punctuation binding" is already UAX #14 behaviour, the "emergency break between punctuation marks" contradicts UAX #14 and CJK practice, the Knuth–Plass demerits are incomplete, and the 5,000-byte switch conflicts with the 500,000-byte target `[U7][L1][L2]`. Fourth, CJK typography: the autospace numbers, the `1.5x` justification guard and the absence of a compression pool are not supported by JLReq, CLREQ or CSS Text 4 `[W2][W4][W5]`. Fifth, interaction: caret affinity, RTL line ends, Shift+arrow behaviour, Backspace granularity and selection rectangles all need rewriting `[C1][C2][C3]`. Sixth, platform claims: the GPUI adapter cannot carry the contract as written, and `hypher` cannot supply spelling-change hyphenation `[B6][H1]`.

---

## 1. System scope and architecture boundaries

This specification defines a multilingual text typesetting and interactive layout engine decoupled from any rendering backend or windowing system. It covers the pipeline from UTF-8 text and styling spans to glyph coordinates, caret geometry and selection rectangles. It stops before rasterisation: it manages no GPU atlas, shaders or platform event loop.

**1.1 Conformance baseline.** The engine targets Unicode 18.0.0, announced 2026-09-16 `[U1]`. Component revisions are:

| Standard | Revision | Date | Note |
|---|---|---|---|
| UAX #29 Text Segmentation | rev 49 | 2026-09-01 | Carries a "draft document" banner. GB9c was reformulated in this revision. `[U2]` |
| UAX #9 Bidirectional Algorithm | rev 52 | 2026-09-01 | `[U3]` |
| UAX #14 Line Breaking | rev 57 | 2026-09-01 | `[U7]` |
| UAX #24 Script Property | rev 41 | 2026-08-28 | `[U4]` |
| UTS #51 Unicode Emoji | v18.0, rev 31 | 2026-08-28 | `[U5]` |
| UTS #37 Ideographic Variation Database | v7.0, rev 16 | 2026-04-30 | The page read is labelled "Proposed Update". `[U6]` |

Each installed component pins and reports its actual Unicode data version through `conformance_report()`, which names the data in use rather than an aspirational target; a component that is not implemented reports None. The crate currently installs no segmentation components, so every entry is None and no segmentation dependency is present. Production integration chooses stable segmenter interfaces at integration time, without the unstable neo feature gate and without claiming target-version conformance from constructor names; acceptance data must match the reported component version. The Unicode 18 references above describe the research target, not a claim that all components implement that version.

**1.2 Determinism and language tags.** Explicit BCP 47 tags win. `LocalePolicy::ExplicitOnly` is default: untagged text uses the host default locale. The script-based rules in Step 4 require `ExperimentalHeuristics` opt-in. Output is deterministic for the same text, options, fonts and data versions.

**1.3 Three-tier architecture.** The draft's three tiers are retained.

1. **Frontend (semantic analysis and paragraph AST).** Ingests UTF-8 and style spans. Produces grapheme atoms, bidi levels, resolved scripts, derived locales, shaping runs, and a box-and-glue sequence. Outputs are independent of font metrics and column width. Dictionary or LSTM segmentation, where used, is a model-dependent step, not a fixed-cost one `[S6]`.
2. **Shaping bridge.** Dispatches homogeneous shaping runs to a backend (HarfRust, CoreText, DirectWrite, a test mock) through a trait. Performs contextual shaping, font fallback and measurement, and aggregates glyphs into clusters. It also provides re-shaping at break points and ligature caret data.
3. **Middle-end.** Line breaking over box-and-glue, boundary re-shaping, per-line bidi reordering, justification, vertical metrics, and the interaction queries (caret, hit test, selection).

**1.4 Backend targets.** HarfRust 0.15.0 (2026-10-22) is the maintained Rust port of HarfBuzz; its README states it matches HarfBuzz 15.0.0 `[B2]`. rustybuzz (0.20.1, 2024-11-12) is archived and its README recommends HarfRust `[B2]`. HarfBuzz itself reached 15.1.0 on 2026-11-04 `[B1]`. Parley 0.12.0 and fontique 0.12.0 (2026-10-09) are built on HarfRust and are the closest Rust analogue to the bridge plus middle-end `[S4]`. CoreText and DirectWrite remain valid platform backends; both accept a language for locale-specific glyph selection `[B5]`. GPUI is treated separately in section 4.6.

---

## 2. Core data structures

All phases share these structures. Signatures below are the revised contract. Fields changed from the draft are marked in comments.

### 2.1 Identifiers and span types

```rust
pub type ByteIndex = usize;            // byte offset into the paragraph's UTF-8 text
pub struct TextRange { pub start: ByteIndex, pub end: ByteIndex } // half-open, code point aligned; TextRange::EMPTY
pub struct ScriptTag([u8; 4]);          // ISO 15924, four ASCII letters by construction; Zsye is an engine pseudo-script (see 3, Step 1)
pub struct LocaleTag(Arc<str>);         // BCP 47, well formed by construction; LocaleTag::try_from_str, und, as_str
pub struct InvalidScriptTag;            // the parse errors of the two tags
pub struct InvalidLocaleTag;
pub type BidiLevel = u8;                // even = LTR, odd = RTL (UAX #9)
pub const BIDI_MAX_DEPTH: BidiLevel = 125;
pub enum Direction { Ltr, Rtl }         // Direction::from_level, base_level, is_rtl
pub type BidiDirection = Direction;
pub enum BidiClass { L, R, Al, En, Es, Et, An, Cs, Nsm, Bn, B, S, Ws, On,
                     Lre, Lro, Rle, Rlo, Pdf, Lri, Rli, Fsi, Pdi } // UAX #9 bidirectional character types

pub struct Point { pub x: f32, pub y: f32 }                              // y grows downwards
pub struct Size { pub width: f32, pub height: f32 }
pub struct Rect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 } // Rect::right, Rect::bottom
```

**[Design decision]** All internal offsets are UTF-8 byte offsets and synthetic elements never displace them. Platform text-input and accessibility layers commonly use other units (Chromium's editing state machines work in UTF-16 code units `[C3]`), so the engine will provide `utf8_to_utf16(offset)` and `utf16_to_utf8(offset)` and reject or round offsets that fall inside a code point (section 6.7; *future*, the crate has no conversion yet).

`ScriptTag` has constants for the scripts the engine names (`COMMON`, `INHERITED`, `UNKNOWN`, `EMOJI`, `LATIN`, `CYRILLIC`, `GREEK`, `ARABIC`, `HEBREW`, `DEVANAGARI`, `THAI`, `HAN`, `HIRAGANA`, `KATAKANA`, `HANGUL`, `BOPOMOFO`), the parsers `try_from_bytes` and `try_from_str` (also `FromStr`), `as_bytes` and `is_unresolved` (true for `Zyyy` and `Zinh`).

**Tags are parsed, not validated [Design decision].** Neither tag has a public field or an unchecked constructor, so a value that exists is well formed and no later stage re-checks its syntax. `ScriptTag::try_from_bytes` accepts exactly four ASCII letters in any case and stores them in the ISO 15924 form (`Latn`); the constants are built through the same parser at compile time. `LocaleTag::try_from_str` accepts exactly the `Language-Tag` grammar of RFC 5646 §2.1 (a `langtag`, a private-use tag, or a grandfathered tag) and stores the tag in the case of §2.1.1 (`ja-JP`, `sr-Cyrl`), so equal tags compare and hash equal. That is the RFC's *well-formed* class: nothing is looked up in the ISO 15924 or IANA registries, which would need data tables. The geometry types are plain `f32` values in layout units, unit-agnostic.

`ScriptTag`: after frontend resolution no atom may remain `Zyyy` (Common) or `Zinh` (Inherited) `[U4]`. `Zsye` is not a Unicode `Script` property value: emoji code points are `Script=Common` in Scripts.txt (18.0). `Zsye` is an ISO 15924 code that UTS #51 §4.3 describes as "prefer emoji style", and UTS #51 warns that it is unsuitable for general language tags `[U5][U8]`. Its use here is an **[Design decision]**: an engine-internal font-fallback key.

### 2.2 Logical atom: `GraphemeAtom`

An atom is one **extended** grapheme cluster (EGC) per UAX #29, including GB9c `[U2]`.

```rust
pub struct GraphemeAtom {
    pub orientation: GlyphOrientation, // resolved, see 2.7
    pub byte_range: TextRange,
    pub script: ScriptTag,
    pub bidi_level: BidiLevel,        // derived from code point levels, see 3 Step 2
    pub linebreak_class: LineBreakClass, // class of the base code point [Design decision]
    pub flags: AtomFlags,
}

#[non_exhaustive]
pub enum LineBreakClass { Bk, Cr, Lf, Cm, Nl, Sg, Wj, Zw, Gl, Sp, Zwj, B2, Ba, Bb, Hy, Cb, Cl, Cp, Ex, In, Ns,
                          Op, Qu, Is, Nu, Po, Pr, Sy, Ai, Ak, Al, Ap, As, Cj, Eb, Em, H2, H3, Hl, Id,
                          Jl, Jt, Jv, Ri, Sa, Vf, Vi, Xx } // one variant per UAX #14 class (data through Unicode 15.1); Default = Xx

pub struct AtomFlags(u16);            // bit set; ten of sixteen bits assigned, the rest masked off
```

`linebreak_class` is a per-code-point property in UAX #14. The draft did not define the atom-level rule; **[Design decision]**: the atom takes the class of its base (first) code point, while the segmenter itself always runs over the original text so that LB9 and LB10 see the real sequence `[U7]`.

`AtomFlags` is a hand-written bit set (`EMPTY`, `ALL`, `bits`, `from_bits_truncate`, `is_empty`, `contains`, `intersects`, `union`, `insert`, `remove`, `set`); `FIRST_ATOM_ONLY` and `LAST_ATOM_ONLY` name the per-flag aggregation masks below. The flags:

| Flag | Meaning |
|---|---|
| `IS_CJK_IDEOGRAPH` | Han ideographs and full-width Kana. |
| `ALLOW_CJK_JUSTIFY_AFTER` | The boundary after this atom is an inter-character justification opportunity (replaces explicit zero-width glue nodes). |
| **`NO_LETTER_SPACING`** | Cursive scripts. CSS Text 4 Appendix D: Arabic, Hanifi Rohingya, Mandaic, Mongolian, N'Ko, Phags Pa, Syriac "do not admit gaps between their letters for either justification or letter-spacing" `[W2]`. |
| **`KASHIDA_CAPABLE`** | Arabic-script atoms eligible for elongation. See Open question 12. |
| `IS_EMOJI_SEQUENCE` | A cluster that is a UTS #51 emoji sequence (core, ZWJ, keycap, flag, tag, modifier). |
| `IS_OPEN_PUNCT` / `IS_CLOSE_PUNCT` | Opening or closing punctuation. |
| `HAS_VARIATION_SELECTOR` | Cluster contains U+FE0E, U+FE0F or an IVS selector (U+E0100..U+E01EF); set after segmentation. |
| `IS_APOSTROPHE` | Resolved intra-word apostrophe (new, see Step 2). |
| **`IN_ATOMIC_EDIT_SPAN`** | The atom lies inside an `AtomicEditSpan` (mention, chip, code span). Caret stops strictly inside the span are suppressed and Backspace / Delete remove the whole span (section 6). `check_layout_inputs` keeps the flag in step with `atomic_spans`: every atom inside one carries it, no atom outside carries it (`AtomicFlagMismatch`). |

**Removed:** `IS_LATIN_LIKE_LIGATURE_BASE`. The restriction of caret subdivision to Latin, Cyrillic and Greek had no source (section 6.2). **Split:** `IS_CURSIVE_CONNECTED` becomes `NO_LETTER_SPACING` and `KASHIDA_CAPABLE`. The draft's inclusion of Devanagari in the cursive set is not supported by the CSS list. Whether Indic scripts should also forbid letter-spacing because of the headline (shirorekha) is a **[Design decision]** pending the Indic layout requirements (Open question 13).

**Cluster flag aggregation [Design decision].** A `ShapedCluster` (2.3) takes its `flags` from the atoms in its `atom_range` per flag, not as a plain union: `IS_OPEN_PUNCT` from the **first** atom only; `ALLOW_CJK_JUSTIFY_AFTER` and `IS_CLOSE_PUNCT` from the **last** atom only (the boundary after a ligature is the one after its last component; an opportunity inside the ligature does not exist); every other flag, `IN_ATOMIC_EDIT_SPAN` included, is the bitwise OR of all atoms. An empty atom list gives no flags. The reference implementation is `AtomFlags::for_cluster`; its masks are `AtomFlags::FIRST_ATOM_ONLY` and `AtomFlags::LAST_ATOM_ONLY`.

**Rich inline span inputs [Design decision].** Notion-grade inline elements (long breakable user mentions, page-reference chips, inline `code`, inline maths) must not be locked into rigid unbreakable boxes. The frontend input contract therefore carries two span streams beside the style spans. They are **orthogonal**: the first governs geometry, the second editing. A mention is usually both, an inline code span often only the first.

```rust
pub struct InlineDecorationSpan {
    pub span_id: u64,                     // unique among decoration spans
    pub byte_range: TextRange,            // non-empty
    pub start_padding: f32,               // horizontal advance of left border + padding; finite, >= 0
    pub end_padding: f32,                 // horizontal advance of right border + padding; finite, >= 0
    pub break_mode: BoxDecorationBreak,   // aligned with CSS Fragmentation 3 box-decoration-break
}
pub enum BoxDecorationBreak {
    Slice, // padding applies only at the true logical start and end of the span (default)
    Clone, // every broken line fragment receives the full start_padding and end_padding
}
pub struct AtomicEditSpan {
    pub span_id: u64,                     // unique among atomic spans
    pub byte_range: TextRange,            // non-empty
}
pub struct CompositionSpan { pub byte_range: TextRange } // IME preedit styling, see below
pub const MAX_DECORATION_NESTING_DEPTH: usize = 8;
```

`InlineDecorationSpan::is_valid` checks the padding and range rules above; `AtomicEditSpan::is_interior(offset)` is true strictly inside the range (6.1).

Both streams are carried by `ParagraphAst` (2.5). **Frontend rule for CJK autospace near spans (3, Step 6.6).** When an `InlineDecorationSpan` has `start_padding > 0.0`, no `CjkAutospace` glue may be inserted at `byte_range.start`, neither immediately outside nor immediately inside the boundary; the symmetric rule holds at `byte_range.end` when `end_padding > 0.0`. This prevents double spacing between a padded chip border and an adjacent Han ideograph. (Reference: `InlineDecorationSpan::forbids_cjk_autospace_at`.)

**Composition styling [Design decision].** A third, independent range, `CompositionSpan`, carries IME preedit styling. It is not stored in `ParagraphAst`. It styles normal EGC atoms: its edge may lie at a code-point boundary inside an EGC, and it never changes segmentation or makes an interior caret stop legal (6.7).

### 2.3 Shaped unit: `ShapedCluster`

```rust
pub struct FontId(pub u32);            // opaque, backend-assigned; identifies a physical face
pub struct FontRef { pub font_id: FontId, pub scale: f32 } // scale: DirectWrite MapCharacters returns one; 1.0 = as requested
                                                           // FontRef::unscaled(font_id)

pub struct RawShapedGlyph {              // belongs to the cluster whose glyph_range contains it
    pub glyph_id: u32,                  // backend glyph index within the cluster's FontRef
    pub x_advance: f32,                 // advance along the active shaping axis (internal horizontal advance for TCY)
    pub x_offset: f32,                  // horizontal offset from the pen position
    pub y_offset: f32,                  // vertical offset from the baseline
    pub is_synthesized: bool,           // engine-generated (the hyphen of TrailingEdit); carried to PositionedGlyph
}

pub struct ShapedCluster {
    pub orientation: GlyphOrientation,  // resolved, see 2.7
    pub byte_range: TextRange,
    pub atom_range: Range<usize>,       // K >= 1 atoms; a cluster never splits an atom; run-local
    pub font: FontRef,
    pub bidi_level: BidiLevel,
    pub flags: AtomFlags,
    pub glyph_range: Range<usize>,      // into the glyph buffer stored beside the cluster; logical order, see 4.2
    pub natural_width: f32,
    pub safe_to_break_before: bool,     // true = no UNSAFE_TO_BREAK
    pub safe_to_concat_before: bool,    // true = no UNSAFE_TO_CONCAT
}
// ShapedCluster::atom_count, is_ligature (K > 1), safety_flags_consistent

pub enum GenericFamily { Serif, SansSerif, Monospace, Cursive, Fantasy, SystemUi, Emoji }
pub enum FontFamily { Named(Arc<str>), Generic(GenericFamily) }
pub type FamilyStack = Arc<[FontFamily]>;              // shared between runs by reference count
pub struct FontWeight(pub u16);                        // CSS scale 1..=1000; NORMAL = 400, BOLD = 700
pub enum FontSlant { Normal, Italic, Oblique }
pub struct FontStyle { pub weight: FontWeight, pub slant: FontSlant }
pub struct FontFeature { pub tag: [u8; 4], pub value: u32 }
pub struct FontVariation { /* tag: [u8; 4], value: f32 (private) */ } // FontVariation::new, tag(), value(); equality and hash by bit pattern
pub struct FontFeatures { pub features: SmallVec<[FontFeature; 4]>, pub variations: SmallVec<[FontVariation; 2]> }
// FontFeatures::new, push, push_variation, as_slice, variations, normalize
pub struct FontInstanceRef<'a> { pub font: FontRef, pub font_size: f32, pub variations: &'a [FontVariation] }
```

**Polarity.** HarfBuzz defines `HB_GLYPH_FLAG_UNSAFE_TO_BREAK` as: if text is broken at the beginning of the cluster the glyph belongs to, both sides need re-shaping. The flag is set on every glyph of the cluster. When it is absent it is safe to break at the beginning of that cluster `[B1]`. Hence "break after cluster *i*" is "before cluster *i+1*", and the end of text is always safe because HarfBuzz has no end-of-text flag position `[B1]`. Blink names its accessor `IsSafeToBreakBefore` for the same reason `[B4]`.

**`UNSAFE_TO_CONCAT`.** Defined as: if text on one side of the start of the cluster changes, the other side's shaping might change. Its absence does not by itself prove that concatenation is safe: only two pieces both clear of the flag can be joined. `UNSAFE_TO_BREAK` always implies `UNSAFE_TO_CONCAT`. The flag is produced only if the buffer flag `HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT` is set, which was added in HarfBuzz 4.0.0 (2022-03-01) `[B1]`. `UNSAFE_TO_BREAK` itself dates from HarfBuzz 1.5.0 (2017-08-23) `[B1]`.

**Cluster notes [Design decision].** (1) In terms of the two `safe_*` booleans, "`UNSAFE_TO_BREAK` implies `UNSAFE_TO_CONCAT`" reads `safe_to_concat_before ⇒ safe_to_break_before`. The combination `safe_to_break_before = true, safe_to_concat_before = false` is legal (only concatenation is unsafe); the converse is a contradiction (`ShapedCluster::safety_flags_consistent`). (2) `atom_range` is **run-local**: it indexes the run's atom slice `&ast.atoms[run.atom_range]`, which is the `atoms` argument of `shape_run` and `reshape_window`; shaper output is stored as is and never re-based. `glyph_range` indexes the glyph buffer the cluster is stored beside (`ParagraphAst::glyphs`, an annotation's `glyphs`, or the `output_glyphs` of the shaper call that produced it); a shaper writes absolute positions of that buffer, and in cluster order the ranges tile it. A cluster may have no glyphs. The paragraph-level ranges of `BoxNode::Text` and `ShapingRun` are described in 2.5. (3) A cluster belongs to exactly one text box (`BoxNode::Text::cluster_range`): split points are chosen before clusters are assigned, and a split inside a multi-atom cluster is resolved by re-shaping that window. The first cluster after such a split carries `safe_to_concat_before = false`, which marks the seam for the rejoin rule of 5, Stage 1.6.5: if a `Hyphenate` break at that seam is not taken, both sides are re-shaped together so the unbroken ligature and kerning are restored (`ParagraphAst::rejoin_windows`). (4) Within a box the clusters partition its atoms in logical order, and each `byte_range` runs from its first atom's start to its last atom's end. (5) `flags` is the per-flag aggregate of 2.2; `ParagraphAst::is_self_contained` checks it together with (2) to (4). (6) A cluster is single-font: `RawShapedGlyph` carries no font id, so every glyph of a cluster belongs to `font` (4.2 req. 3). (7) A cluster carries one `natural_width`, the final inline layout advance including explicit letter spacing, word spacing and ruby expansion; raw shaper advances remain `RawShapedGlyph.x_advance`, and no equality between the two is assumed (tate-chu-yoko in particular, 2.7). There is no stored chws-off width and no unconditional second shaping pass: chws context is repaired locally at the breaks actually chosen (4.2 req. 6). (8) `FontFeatures::normalize` sorts by tag and keeps the last setting per tag, features and variations alike. A run's `letter_spacing`, `word_spacing` and explicit baseline offset must be finite; negative spacing increments are allowed provided every final text and glue width stays finite and non-negative. Non-zero letter spacing is rejected on a run that contains an atom flagged `NO_LETTER_SPACING` (`InvalidSpacing`).

### 2.4 Box-and-glue sequence: `WordNode`

```rust
pub enum WordNode {                       // four variants
    Box(BoxNode),
    Glue(GlueNode),
    Discretionary(DiscretionaryNode),
    Tab { byte_range: TextRange, bidi_level: BidiLevel },   // level == paragraph level (UAX #9 L1)
}

pub enum BoxNode {
    Text { run_index: usize, byte_range: TextRange,
           atom_range: Range<usize>,      // into ParagraphAst::atoms (paragraph-level)
           cluster_range: Range<usize>,   // into ParagraphAst::clusters (paragraph-level)
           natural_width: f32 },
    InlineObject { baseline_align: BaselineAlignment, byte_range: TextRange, box_id: u64,
                   width: f32, ascent: f32, descent: f32, overflow_policy: BoxOverflowPolicy,
                   bidi_level: BidiLevel },   // resolved level of the object's U+FFFC
    SpanBoundary { byte_range: TextRange,   // empty range anchored at the span's start or end
                   span_id: u64, edge: SpanEdge /* Leading | Trailing */,
                   width: f32,              // start_padding or end_padding
                   bidi_level: BidiLevel }, // Leading: level of the first atom in the span; Trailing: of the last
}

pub enum SpanEdge { Leading, Trailing }   // logical start or end of a decoration span

pub enum BoxOverflowPolicy {                 // default Clip
    Clip,          // clamp visual rendering to wrap_width
    AllowOverflow, // permit visual_width > wrap_width when isolated on an empty line
    ScaleToFit,    // uniformly scale width, ascent and descent by wrap_width / width
}

pub struct GlueNode {
    pub original_bidi_class: BidiClass, pub bidi_level: BidiLevel, pub is_collapsible: bool,
    pub byte_range: TextRange, pub kind: GlueKind, // the kind fixes the tier: GlueKind::tier()
    pub natural_width: f32, pub max_stretch: f32, pub max_shrink: f32,
}
pub enum GlueTier {
    Tier0 = 0, // punctuation compression pool and Western WordSpace
    Tier1 = 1, // CJK-Latin autospace (CjkAutospace)
    Tier2 = 2, // inter-ideographic expansion (ALLOW_CJK_JUSTIFY_AFTER); a tier of the prefix tables only,
               // no GlueKind has it
}                // GlueTier is #[repr(u8)], default Tier0
pub enum GlueKind { WordSpace, CjkAutospace, CjkPunctCompression } // the last: Tier0, shrink only
// GlueKind::tier() -> GlueTier; GlueKind::is_synthetic() is true for the two kinds that own no bytes

pub struct DiscretionaryNode {
    pub byte_range: TextRange, // empty at the break for a soft opportunity; the newline bytes for a HardBreak
    pub penalty: i32,          // -10000 = mandatory HardBreak; 0 = natural; > 0 = penalised
    pub flagged: bool,         // true for hyphenation breaks; triggers the Knuth-Plass alpha demerits
    pub pre_break_width: f32,  // width added to the end of line k if broken here
    pub post_break_width: f32, // width added to the start of line k+1 if broken here
    pub no_break_width: f32,   // width contributed to the current line if NOT broken here
    pub action: BreakAction,
    pub cloned_spans: SmallVec<[u64; 2]>, // Clone-mode spans this break falls strictly inside
}
impl DiscretionaryNode {
    pub const MANDATORY_BREAK_PENALTY: i32 = -10_000;
    pub const NATURAL_PENALTY: i32 = 0;
    pub const URL_PENALTY_MIN: i32 = 25;
    pub const URL_PENALTY_MAX: i32 = 50;
    pub const HYPHEN_PENALTY: i32 = 135;
    pub const SPELLING_CHANGE_HYPHEN_PENALTY: i32 = 250;
    pub const CLONED_SPAN_PENALTY: i32 = 300;
    pub const MATH_RELATION_PENALTY: i32 = 500;
    pub const MATH_BINARY_OPERATOR_PENALTY: i32 = 700;
    pub const FORBIDDEN_BREAK_PENALTY: i32 = 10_000;
    // direct(byte_offset), hard_break(newline, is_paragraph_end), byte_offset(), is_mandatory()
}
// WordNode::byte_span() returns the node's source range; is_break_opportunity() is true for Discretionary.

pub enum BreakAction {
    Direct,                                  // zero-cost break between words, Han ideographs, Thai words
    HardBreak { is_paragraph_end: bool },    // mandatory line termination (penalty = -10000)
    Hyphenate(HyphenOpportunity),            // TextShaper::reshape_window at materialisation
    CjkPunctAdjust,                          // unit: the deltas are pre_break_width / post_break_width
    MathOperator { repeat: Option<RepeatedOperator> }, // break after a top-level relation or binary operator
}
pub struct RepeatedOperator { pub glyph_id: u32, pub advance: f32 } // the operator repeated at the start of line k+1

pub type ReplacementText = Box<str>;

pub struct HyphenOpportunity {            // mirrors libhyphen's (rep, pos, cut) [H3]; the break position is the node's
    pub prefix_replacement: Option<ReplacementText>, // e.g. Dutch "omaatje" -> "oma-tje"
    pub suffix_replacement: Option<ReplacementText>,
    pub prefix_cut_bytes: u32,             // original bytes removed immediately before the break
    pub suffix_cut_bytes: u32,             // original bytes removed immediately after the break
}
```

**Cut semantics [Design decision].** The break position is the discretionary's own (`DiscretionaryNode::byte_offset()`, the start of its empty `byte_range`), in the **original** text; the opportunity does not repeat it. A replacement that straddles the break (libhyphen's `cut` can, for example `ck` to `k-k` `[H3]`) is split between `prefix_cut_bytes` (bytes removed from the end of the prefix) and `suffix_cut_bytes` (bytes removed from the start of the suffix); each count must end on an atom boundary, so a cut never splits an atom, and each stays inside the **contiguous same-run hyphen chain** of the word: the maximal run of `BoxNode::Text` items sharing one `run_index`, joined only by `Hyphenate` discretionaries that meet the boxes end to end, and stopped by glue, a tab, an inline object, a `SpanBoundary`, a run change or any other discretionary (`reshape_window` takes one `ShapingRun`, and a box cannot straddle a span boundary). A two-byte spelling-change cut may therefore cross an unchosen hyphenation point one byte away within the same word. `prefix_replacement` replaces the prefix-side cut and `suffix_replacement` the suffix-side cut. A cut with no replacement text is a pure deletion. Section 4.1 maps each side to its own edit: `TrailingEdit` for the prefix side and `LeadingEdit` for the suffix side. `PositionedLine` validation does not apply to these; `check_layout_inputs` checks that the break lies between two text boxes of one run that meet at it, and that both cuts stay inside the chain and end on atom boundaries of it. `ParagraphAst::break_window(item)` returns the `ReshapeWindow { run_index, atom_range, byte_range }` to re-shape when the break is taken: the box on each side of it, extended along the chain as far as a cut reaches; line *k* re-shapes the part before the break with the `TrailingEdit`, line *k+1* the part after it with the `LeadingEdit`. For a hyphenation, `pre_break_width - clone_pre` and `post_break_width - clone_post` (rule 5 below) are the net prefix and suffix shaping deltas. `HyphenOpportunity` has no penalty of its own: `DiscretionaryNode.penalty` is the only authority.

**Algebraic model [Design decision, Sourced: L1, L2, W4, W5].** The six ad-hoc variants of the earlier draft (`Word`, `PunctuationRun`, `Glue`, `Tab`, `HardBreak`, `InlineBox`) are replaced by the four-variant algebra of Omega / LuaTeX. `HardBreak`, `PunctuationRun` (and with it `PunctBind`) and the nested `hyphen_breaks` vector are absorbed into `Discretionary`. The frontend is a **compiler from linguistic rules to algebraic primitives**; the middle-end is a language-agnostic solver.

**Fundamental invariant.** The line breaker may end a line **if and only if** it is at a `WordNode::Discretionary` (the single exception is the emergency cluster break on an overflowing empty line, 5 Stage 1.5). `Glue` is never an implicit break point. If a space is breakable the frontend emits the `Discretionary` **after** the `Glue` run: `[Box, Glue(WordSpace), Discretionary]`, and for several consecutive spaces one `Discretionary` at the end, `[Box, Glue, Glue, Discretionary, Box]`. When the line breaks there, the spaces belong to the trailing edge of line *k* (`PositionedLine::byte_range` includes them, `visible_byte_end` stops at the last visible box, 2.6) and line *k+1* starts at the discretionary's `byte_range.end`. A non-breaking space is `[Box, Glue]`.

**Three-state discretionary.** `pre_break_width`, `post_break_width` and `no_break_width` are finite signed deltas; actions are materialized only at committed breaks. A soft opportunity has an empty range at a valid node boundary. A real `HardBreak { is_paragraph_end: false }` owns exactly U+2028 (three UTF-8 bytes). Every `ParagraphAst` ends with exactly one empty `HardBreak { is_paragraph_end: true }` sentinel at `text.len()`. All other paragraph separators are rejected before layout. Nonempty content ranges concatenate exactly to `0..text.len()`; empty nodes sit at valid boundaries in monotone order and contribute no coverage. The line ending at a break includes its owned bytes; the next line starts at that range's end. `visible_byte_end` includes all non-collapsible spaces. A hard break, real or sentinel, has `no_break_width == 0.0`. `layout_paragraph` propagates validation errors in its Result.

**Linguistic-to-algebraic compilation rules (Frontend Step 6).**

1. *Elimination of `PunctuationRun` and kinsoku (UAX #14 LB13, LB15d, LB22).* Closing punctuation (CL, CP, EX, IS, SY) and inseparable characters (IN) are emitted as ordinary `BoxNode::Text`. Because the frontend emits **no** `Discretionary` between the preceding box and the punctuation box, the two are algebraically fused; the same holds after opening punctuation (OP), which binds to the following box. The segmenter's output is never contradicted `[U7]`.
2. *Elimination of `no_line_end_words` special cases.* For Czech one-letter prepositions (`AIiVvOoUuSsZzKk`) and U+00A0 the frontend emits `[Box("v"), Glue(WordSpace)]` with no intervening `Discretionary`. A normal breakable space is `[Box("word"), Glue(WordSpace), Discretionary(penalty = 0, widths = 0, Direct)]`.
3. *Elimination of nested `hyphen_breaks`.* Where hyphenation opportunities exist inside a word, the frontend **splits** the word's clusters at each opportunity boundary into adjacent `BoxNode::Text` items and interleaves `DiscretionaryNode`s with `flagged = true`, `penalty = 135` (or the profile value), `pre_break_width = estimated hyphen width`, `post_break_width = 0.0`, `no_break_width = 0.0` and `action = Hyphenate(..)`. The break offset is the node's own (`byte_range` is the empty range there; the `HyphenOpportunity` does not repeat it); `check_layout_inputs` requires a text box on each side meeting at that offset, the two boxes to share one `run_index` (a word is split within a single `ShapingRun`, and the hyphen glyph and `TextShaper::reshape_window` are scoped to one), and each cut to end on an atom boundary within the contiguous same-run hyphen-connected `BoxNode::Text` sequence. (The alternative of keeping the word whole and anchoring the offset is not representable in a linear sequence.)
4. *CJK `chws` punctuation seams.* Where two adjacent CJK punctuation clusters are compressed by `chws` (combined advance `1.5 em` instead of `2.0 em`) and a break between them is permitted (for example a closing quote followed by an opening quote), the frontend emits each cluster at its compressed width and places a `Discretionary` between them with `no_break_width = 0.0`, `pre_break_width` the delta that trims or restores the line-end mark, and `post_break_width` the delta that restores the line-start opening mark using the width measured by on-demand boundary reshaping; `action = CjkPunctAdjust`, a **unit variant** (the earlier cluster indices and the advance overrides both duplicated what the node's widths already say). `pre_break_width` is the signed delta added to the `natural_width` of the last cluster of the immediately preceding `BoxNode::Text` when the break is taken (the end of line *k*); `post_break_width` the delta added to the first cluster of the immediately following one (the start of line *k+1*). Cloned padding (rule 5) rides in the same widths, so `check_layout_inputs` works on the net deltas `net_pre = pre_break_width - clone_pre` and `net_post = post_break_width - clone_post`: at least one must be non-zero (within eps), and each non-zero one requires the neighbouring text box to exist and the cluster's `natural_width` plus the net delta to be non-negative.
5. *Breakable rich spans.* Inside a span with `BoxDecorationBreak::Clone`, **every** internal `Discretionary`, a mandatory one included, has `pre_break_width` incremented by `end_padding`, `post_break_width` incremented by `start_padding`, and the span's id pushed onto `cloned_spans`; only a **non-mandatory** one also has its penalty increased by `+300` (so the breaker prefers keeping the chip intact unless that makes the line too bad; a mandatory break keeps `-10000`, it is never penalised); the list is **complete**, every `Clone` span the break is strictly inside appears in it. Cloning is **orthogonal to the action**: a hyphenation break inside a chip is `Hyphenate(..)` *and* lists the chip, and keeps its own action. When the line breaks there, each listed span receives its cloned `end_padding` on line *k* (already in `pre_break_width`) and its cloned `start_padding` on line *k+1* (in `post_break_width`). `check_layout_inputs` requires every listed id to name a `Clone`-mode decoration span that contains the break **strictly inside** (`start < offset < end`: a break on the edge has nothing to clone) and ids not to repeat. **Widths.** With `clone_pre = Σ end_padding` and `clone_post = Σ start_padding` over `cloned_spans`, a `Direct` or `HardBreak { is_paragraph_end: false }` break must have `pre_break_width == clone_pre` and `post_break_width == clone_post` (within eps); a `MathOperator` break must have `pre_break_width == clone_pre` and `post_break_width == clone_post + repeat.advance`, the advance of the operator repeated at the start of line *k+1* (finite and non-negative; zero without a repeat); the sentinel lists no span, so both widths are `0.0`. `CjkPunctAdjust` and `Hyphenate` carry their own deltas on top of the clone sums (rule 4 above; 2.4 cut semantics).
6. *Flattened inline maths.* An external maths engine decomposes a top-level list (`$a + b = c$`) into `BoxNode::InlineObject` items separated by `Discretionary` items with `penalty = 500` after relations and `penalty = 700` after binary operators (TeX `\relpenalty`, `\binoppenalty`). Sub-formulas in braces, fractions or radicals are one `InlineObject` with no internal `Discretionary`.

**Canonical penalty scale.**

| Penalty | Meaning |
|---|---|
| `-10000` (`-INF_PENALTY`) | Mandatory `HardBreak` (U+2028 inside a paragraph, or the end of paragraph; `\n` and `\r\n` are paragraph separators handled by `split_paragraphs`). |
| `0` | Natural break: before a breakable `WordSpace`, between CJK ideographs, between Thai, Lao, Khmer and Myanmar segmented words, after an explicit hyphen U+002D. |
| `25..50` | URL interior slash and punctuation breaks `[L3]`. |
| `135`, `flagged` | Standard Liang-pattern hyphenation (`hypher`). |
| `250`, `flagged` | Spelling-change hyphenation (libhyphen: Dutch, Catalan, Hungarian). |
| `300` | Added to every break inside a cloned `InlineDecorationSpan` (long mention, inline `code`). |
| `500` | Break after a top-level inline-maths relation. |
| `700` | Break after a top-level inline-maths binary operator. |
| `>= 10000` | Forbidden break: represented by **omitting** the node; a node carrying it is rejected by `check_layout_inputs`. |

`check_layout_inputs` also requires `action == HardBreak` exactly when `penalty <= -10000`, and finite widths. The 25..50 URL band and the 135 / 250 hyphen values restate Typst's and libhyphen's figures `[L3][H3]` and remain starting points to tune.

`GlueKind` is `WordSpace`, `CjkAutospace` or `CjkPunctCompression`. **`GlueTier`** names the levels of the JLReq / CLREQ priority ladder: `Tier0` for the punctuation compression pool and Western word spaces, `Tier1` for CJK–Latin autospace, `Tier2` for the zero-width inter-ideographic expansion (`ALLOW_CJK_JUSTIFY_AFTER`). `GlueKind::tier` maps `WordSpace` and `CjkPunctCompression` to `Tier0` and `CjkAutospace` to `Tier1`; the kind is the only authority, a `GlueNode` stores no tier.

**Whitespace classification [Design decision].** `GlueNode` keeps `original_bidi_class: BidiClass`, the resolved `bidi_level` and `is_collapsible`. UAX #9 L1 reads the original classes, not classes modified by earlier phases (5, Stage 2). NBSP U+00A0, Figure Space U+2007 and Narrow NBSP U+202F are never collapsible (`check_layout_inputs` rejects a `WordSpace` over them with `is_collapsible == true`): their advances stay in `visual_width` and they advance `visible_byte_end`. Only collapsible trailing whitespace may occupy the trailing excluded range (5, Stage 1.7). Synthetic glue (`CjkAutospace`, `CjkPunctCompression`; `GlueKind::is_synthetic`) has an empty source range and owns no source bytes; only `WordSpace` glue owns bytes. A `WordSpace` owns a non-empty run of word-space characters and nothing else: `is_word_space` is true for the Unicode space separators (`General_Category = Zs`: U+0020, U+00A0, U+1680, U+2000 to U+200A, U+202F and U+205F), and `check_layout_inputs` compares every glue node with its source bytes (`InvalidGlue`). The ideographic space U+3000 is excluded: CJK typography treats it as a fixed full-width character (JLReq class cl-14) that justification does not stretch and collapsing does not remove, so the frontend shapes it as ordinary text in a `BoxNode::Text`, followed by a break opportunity (UAX #14 class BA, break after), and a `WordSpace` over it is rejected.

**Tier 2 is derived, not emitted.** The frontend emits **no** zero-width `Tier2` glue between ideographs, which would inflate the node count of a CJK paragraph by a third. `ALLOW_CJK_JUSTIFY_AFTER` stays on `ShapedCluster.flags`; when the middle-end builds the prefix tables over `BoxNode::Text` clusters, each cluster carrying the flag adds `cjk_inter_char_max_stretch` (for example `0.25 · font_size`) to `Stretch[Tier2]` at its position, except the last visible cluster at `b_vis`. No `GlueKind` maps to `Tier2`, so a `GlueNode` of that tier cannot be built. **`CjkPunctCompression`** carries the compressible margin of CJK full-width punctuation that `chws` has not already squeezed, or that JLReq / CLREQ mid-line compression frees: `Tier0`, `natural_width = 0`, `max_stretch = 0`, `max_shrink` up to `0.5 · ic`; `check_layout_inputs` enforces that shape. `CjkAutospace` values depend on the active profile:

| Profile | `natural_width` | `max_shrink` to | `max_stretch` to | Source |
|---|---|---|---|---|
| `Jlreq` (default for ja) | 1/4 em | 1/8 em | 1/2 em (1/3 em in some styles) | JLReq 3.8.3 and 3.8.4 `[W4]` |
| `CssCompat` | 0.125 ic (1/8 of the CJK advance) | not specified | not specified | CSS Text 4 `text-autospace` `[W2]` |

CLREQ 6.3.3 gives "up to 1/4 em" between Han and Western characters, with no space at the line start or end and none beside Chinese pause marks or CJK brackets `[W5]`. **[Design decision]** Sizes are expressed in `ic` (the CJK advance, usually 1 em). Whether the glue also applies to Kana, Cyrillic and Greek neighbours is a design choice; CSS applies `ideograph-alpha` and `ideograph-numeric` to ideographs `[W2]`. The draft's three numbers (0.125, 0.125, 0.0625 em) have no source and are withdrawn.

`Tab`: `TabStops::next_stop(x) -> Option<f32>` returns Some only for finite `next > x`; exhausted precision, invalid inputs or overflow return None. None means consume the tab with zero advance and force a line termination at that position (5, Stage 3.1). Candidate scoring freezes all elasticity before the final tab, exactly as justification does. A `Discretionary` with `action = HardBreak` (U+2028; `\n` and `\r\n` split paragraphs instead) terminates the line and disables justification. `BoxNode::InlineObject` is a rigid object that owns its replacement bytes, usually one U+FFFC, so that carets and selections can address it: like a text box, which owns at least one atom, it never has an empty `byte_range` (a host widget without source text puts U+FFFC in its text). It has a `baseline_align` (2.7) and an `overflow_policy`: **`BoxOverflowPolicy`** says what happens to a rigid object wider than the line. `Clip` clamps the rendering to `wrap_width`; `AllowOverflow` lets an object that sits alone on a line exceed `wrap_width`, in which case the line's `visual_width` equals the object's width, so containment `[0, visual_width]` still holds; `ScaleToFit` scales width, ascent and descent uniformly by `wrap_width / width`. `BoxNode::SpanBoundary` carries the padding advance of one end of an `InlineDecorationSpan`; with `Clone` the middle-end re-materialises it at each break listed in `cloned_spans`. Its `bidi_level` is that of the first atom inside the span for a `Leading` edge and of the last for a `Trailing` edge; `check_layout_inputs` compares it with the atoms.

### 2.5 Paragraph AST

```rust
pub struct ParagraphAst {
    pub writing_mode: WritingMode,    // 2.7
    pub bidi_spans: Vec<ExplicitBidiSpan>,           // 2.8
    pub annotations: Vec<AnnotationSpan>,            // 2.9
    pub base_direction: BidiDirection,
    pub strut_metrics: FontMetrics,   // raw, as returned by the shaper; never policy-adjusted
    pub strut_font_size: f32,         // size the metrics were requested at
    pub runs: Vec<ShapingRun>,        // same family stack, style, script, locale, bidi level, decoration
    pub atoms: Vec<GraphemeAtom>,     // all body atoms, run order then logical order
    pub clusters: Vec<ShapedCluster>, // all body clusters, run order then logical order
    pub glyphs: Vec<RawShapedGlyph>,  // all body glyphs, cluster order then logical order
    pub items: Vec<WordNode>,
    pub decoration_spans: Vec<InlineDecorationSpan>, // padding and fragmentation (2.2)
    pub atomic_spans: Vec<AtomicEditSpan>,           // caret and deletion units (2.2)
}
// ParagraphAst::run_atoms(run), run_clusters(run), cluster_glyphs(cluster), text_box(item), is_self_contained(item),
//               break_window(item), rejoin_windows(untaken_items)

pub struct TextBoxView<'a> {          // resolved view of a BoxNode::Text
    pub run_index: usize, pub byte_range: TextRange,
    pub atom_start: usize,            // paragraph-level index of atoms[0]
    pub atoms: &'a [GraphemeAtom], pub clusters: &'a [ShapedCluster], pub natural_width: f32,
}
pub struct ReshapeWindow {            // a stretch of one run to hand to TextShaper::reshape_window
    pub run_index: usize,
    pub atom_range: Range<usize>,     // paragraph-level
    pub byte_range: TextRange,
}

pub struct ShapingRun {
    pub baseline_align: BaselineAlignment,
    pub letter_spacing: f32, pub word_spacing: f32,   // finite
    pub orientation: GlyphOrientation,
    pub features: FontFeatures,       // single source of truth for features and variations
    pub byte_range: TextRange,
    pub families: FamilyStack,
    pub font_size: f32,               // read by shaping and reshaping
    pub style: FontStyle,
    pub script: ScriptTag,
    pub locale: LocaleTag,
    pub bidi_level: BidiLevel,        // ShapingRun::direction() is RTL iff odd
    pub decoration: TextDecoration,
    pub atom_range: Range<usize>,     // paragraph-level slice of atoms (annotation runs: 0..atoms.len())
    pub cluster_range: Range<usize>,  // paragraph-level slice of clusters
}
pub struct TextDecoration { pub underline: bool, pub strikethrough: bool } // colour is applied at paint time
// Span invariants (enforced by check_layout_inputs):
//  atomic_spans   : start-ordered, non-empty, strictly disjoint (a.end <= b.start || b.end <= a.start), span_id unique.
//                   Both edges coincide with the start or end of a GraphemeAtom of a BoxNode::Text or of a
//                   BoxNode::InlineObject, and the span wraps at least one of them (a span of whitespace
//                   alone is invalid): an edge inside a GlueNode, a Tab or a line terminator, or on
//                   an unowned offset, is rejected. IN_ATOMIC_EDIT_SPAN is set on exactly the atoms
//                   inside an atomic span (AtomicFlagMismatch).
//  decoration_spans: sorted by start ascending, end descending; equal ranges retain input order.
//                   Reject disorder in O(N); use one bounded stack plus indexed id lookup. span_id unique; proper nesting only (disjoint, or one contains the other); partial
//                   overlap is rejected; depth <= MAX_DECORATION_NESTING_DEPTH = 8 (locked). Equal ranges are
//                   allowed: the lower index is the OUTER span.
//  SpanBoundary pair: every decoration span has exactly one SpanBoundary(Leading) at
//                   byte_range.start and one SpanBoundary(Trailing) at byte_range.end in items, even when the
//                   padding is zero; they mark is_first_fragment / is_last_fragment after bidi reordering
//                   (5 Stage 2) without cross-referencing atom offsets. Missing nodes: MissingSpanBoundary.
//  Seam spacing: a Glue(CjkAutospace) at s..s has index < idx_lead, one at e..e index > idx_trail.
//                   Adjacent spans a.end == b.start: [A Trailing, Glue(CjkAutospace), Discretionary, B Leading]; at any
//                   offset a CjkAutospace glue precedes every zero-width break opportunity (ItemsOutOfOrder).
//  Item order and enclosure (ItemsOutOfOrder, SpanEnclosure):
//                   items are monotone by byte range (prev.end <= next.start; zero-width nodes may share an
//                   offset). For a span [s, e) with boundary indices idx_lead < idx_trail: every item inside
//                   [s, e] sits strictly between them, every item outside does not; a Discretionary at s..s has
//                   index < idx_lead, one at e..e index > idx_trail. Frontends therefore split boxes at
//                   span edges; a box that straddles an edge fails the monotone check.
//  SpanBoundary order: strict LIFO. At byte_range.start the outer span's Leading precedes the inner's; at
//                   byte_range.end the inner's Trailing precedes the outer's. check_layout_inputs rejects a
//                   Trailing that closes a span other than the innermost open one, a repeated Leading, and
//                   an equal-range pair in the wrong order.
//  the two streams are independent id spaces (atomic identity is never inferred from decoration ids); neither
//                   stores colours or radii, only padding and break mode.

pub struct FontMetrics {         // field set is a design decision; all 15 floats finite
    pub vertical_ascent: f32, pub vertical_descent: f32, pub vertical_line_gap: f32,
    pub ideographic_baseline_offset: f32, pub central_baseline_offset: f32,
    pub hanging_baseline_offset: f32, pub mathematical_baseline_offset: f32,
    pub ascent: f32, pub descent: f32, pub line_gap: f32,
    pub underline_position: f32, pub underline_thickness: f32,
    pub strikeout_position: f32, pub x_height: f32, pub cap_height: f32,
    pub use_typo_metrics: bool,  // OS/2 fsSelection bit 7 [B7]
}   // FontMetrics::is_valid, natural_line_height (ascent + descent + line_gap)

pub struct StrutMetrics { pub ascent: f32, pub descent: f32, pub line_gap: f32 } // the RESOLVED strut (5, Stage 4.1)
// StrutMetrics::line_height (ascent + descent + line_gap), is_valid
```

OpenType OS/2 `fsSelection` bit 7 (`USE_TYPO_METRICS`) strongly recommends that applications use `sTypoAscender - sTypoDescender + sTypoLineGap` as default line spacing when it is set; vendors are encouraged to set it in version 4 or later tables `[B7]`. GPUI's metrics expose ascent, descent, cap height, x-height and bounding box, but no underline, strikeout or line gap, so an adapter needs another source for them `[B6]`.

**Metric validity [Design decision].** Raw `FontMetrics` require all 15 floating-point fields finite. Ascent, descent, line gap, their vertical equivalents, underline thickness, x-height and cap-height are non-negative depths, not signed offsets; the horizontal ascent+descent sum and the vertical one are each positive, and every derived height stays finite (`FontMetrics::is_valid`). The four baseline offsets (ideographic, central, hanging, mathematical) are scaled layout units, positive toward line-over; font bridges supply missing vertical and BASE metrics. `resolve_primary_font_metrics` takes the requested variation coordinates, so `ParagraphAst::strut_metrics` holds the metrics of the actual variable-font instance, not its default axes. Font sizes and fixed line-height multipliers must be finite and strictly positive and the resolved total line height positive; negative half-leading may make the effective ascent or descent negative, which alone is valid (5, Stage 4.1). The two contracts have two types: raw font data is a `FontMetrics`, whose depths are never negative, and the strut resolved from it is a `StrutMetrics { ascent, descent, line_gap }`, which carries only what a line box reads, may have a negative ascent or descent, and is valid when its fields are finite and its `line_height()` is finite and positive. `NoopShaper` is a deliberately non-rendering zero-metric test double.

**Paragraph and source ownership.** One `ParagraphAst` represents exactly one UAX #9 bidi paragraph. Validation rejects every UAX #9 class-B paragraph separator (LF, CR including CRLF, NEL U+0085, PS U+2029, the information separators U+001C–U+001E) and the application-policy terminators VT U+000B and FF U+000C anywhere in the text, independently of which nodes the AST emits (`ParagraphSeparatorInText`); `split_paragraphs` (3) is how a caller gets there. Only U+2028 terminates an internal line without resetting paragraph bidi state: a real `HardBreak` owns exactly that U+2028 and a unique final sentinel owns `text.len()..text.len()`; no other hard-break terminator is valid. Offsets inside the AST are paragraph-local. Item ranges concatenate exactly to `0..text.len()` (2.4).

**Storage model [Design decision].** Shaped text is stored once per paragraph, in three flat arrays: `ParagraphAst::atoms`, the `clusters` that group them and the `glyphs` that render the clusters, each in run order and then logical order. The clusters tile the glyphs (`ShapedCluster::glyph_range`, in cluster order; a gap, an overlap or a leftover glyph is `InvalidGlyphStorage`). The runs tile the atoms and the clusters (`ShapingRun::atom_range`, `cluster_range`) and each `BoxNode::Text` names a contiguous slice of each by paragraph-level `atom_range` and `cluster_range` lying inside its run's ranges; the text boxes of one run take consecutive ranges and cover them exactly. `ShapedCluster::atom_range` alone is run-local (2.3). `ParagraphAst::run_atoms`, `run_clusters`, `cluster_glyphs` and `text_box` (a `TextBoxView`) resolve the views; an annotation owns its atoms, clusters and glyphs locally, with run ranges starting at 0 (2.9). A text box owns at least one atom, and its cached `natural_width` is checked against the sum of its clusters' (`InvalidTextWidth`): it is kept because the line fitter reads it for every candidate. Violations are `InvalidRunStorage` and `ItemNotSelfContained`.

**Input validation.** The AST is untrusted. `check_layout_inputs` validates source coverage, run, atom and cluster levels, scripts, orientation, safety-flag consistency, finite positive font sizes and fallback scales, and finite measurements; invalid input is never silently repaired downstream. Accepting an AST does not certify that it was produced by a conforming UAX implementation: the validator neither re-segments nor reruns UAX #9, and glyph rotation and placement are rendering responsibilities. Public structs are not `#[non_exhaustive]`, and source compatibility across field additions is not promised. Cluster boundaries are indivisible layout boundaries; editing atomicity stays independent of them through `AtomicEditSpan`.

**`LayoutInputError` [Design decision].** `#[non_exhaustive]`; `check_layout_inputs` returns the first violation. Variants and triggers:

| Variant | Trigger |
|---|---|
| `OrientationMismatch` | glyph orientation disagrees with mode, run or level parity |
| `InvalidFontMetrics` | invalid raw metrics or baseline offsets |
| `InvalidSpacing` | non-finite or forbidden explicit spacing |
| `InvalidBidiSpan { span }` | an explicit bidi span breaks the rules of 2.8 |
| `InvalidAnnotationSpan { annotation }` | invalid annotation base or payload (2.9) |
| `InvalidLayoutOptions` | invalid clamp or other layout policy |
| `InvalidLineGeometry` | malformed geometry, a slot outside the wrap envelope `[0, wrap_width]`, or exhausted clearance progress (5) |
| `NonMonotonicGeometry` | an expanded block envelope introduced inline space |
| `InvalidConstraints` | invalid width, indentation (without a provider each indent must lie in `[0, wrap_width)`) or tab stops |
| `ParagraphSeparatorInText` | `text` contains a paragraph separator |
| `InvalidStrut` | raw strut metrics, strut font size or fixed multiplier non-finite (or negative size) |
| `RunOutsideText { run }` | a run's range exceeds the text or splits UTF-8 |
| `InvalidShapingRun { run }` | level above the UAX #9 maximum, bad font size, unresolved script or non-finite variation, or a body run whose bytes start before the previous body run ends |
| `InvalidRunStorage { run }` | run atom or cluster ranges do not tile the paragraph storage, or its text boxes do not cover them |
| `InvalidGlyphStorage { cluster }` | the clusters' glyph ranges do not tile the paragraph's glyphs |
| `RunIndexOutOfRange { item }` | an item's `run_index` is invalid |
| `ItemOutsideText { item }` | an item's bytes exceed the text or split UTF-8 |
| `UncoveredSource { byte }` | the items leave source unowned; `byte` is the first byte no item owns |
| `ScriptMismatch { item }` | an atom's script differs from its run's |
| `ItemOutsideRun { item }` | a text box lies outside its run's bytes, or one of its atoms or clusters is at another bidi level |
| `InvalidTextWidth { item }` | a box width or cluster measurement is non-finite, or the box width is not the sum of its clusters |
| `ItemNotSelfContained { item }` | a text box has no atoms or otherwise violates `is_self_contained`, has inconsistent break-safety flags, or its ranges leave the run's or are out of order |
| `InvalidInlineBox { item }` | an inline object has a non-finite or negative size, an invalid baseline alignment or an unrepresentable bidi level, or no source bytes |
| `InvalidGlue { item }` | non-finite or negative glue values, an unrepresentable level, a `CjkPunctCompression` that is not shrink only, or source bytes that do not match the kind (a `WordSpace` owns word-space characters and is not collapsible over a no-break space; synthetic glue owns none) |
| `InvalidTab { item }` | a tab owns no source bytes (empty `byte_range`) or is not at the paragraph level (UAX #9 L1); its geometric width is not constrained |
| `InvalidDiscretionary { item }` | non-finite widths, forbidden penalty, action/penalty disagreement, ownership-rule violation, bad `cloned_spans`, or break widths that disagree with the clone sums, the repeated operator's advance or the `CjkPunctAdjust` deltas |
| `InvalidHyphenOpportunity { item }` | a `Hyphenate` break is not flagged, not between two same-run text boxes at its offset, or has a cut off an atom boundary or outside its chain |
| `InvalidSpanBoundary { item }` | a boundary is non-empty, has a bad width, position or `bidi_level`, or breaks LIFO order |
| `UnknownSpan { item }` | an item names a span that `decoration_spans` lacks (or a non-`Clone` or non-enclosing span in `cloned_spans`) |
| `InvalidDecorationSpan { span }` | invalid padding or range, duplicate id, out of order (ascending start, then descending end), partial overlap, or nesting deeper than `MAX_DECORATION_NESTING_DEPTH` |
| `MissingParagraphEnd` | items do not end with the paragraph-end sentinel |
| `MissingSpanBoundary { span }` | a decoration span lacks its `Leading` or `Trailing` boundary |
| `ItemsOutOfOrder { item }` | an item starts before the previous one ends |
| `SpanEnclosure { span, item }` | an item is on the wrong side of a span's boundaries, or straddles an edge |
| `AtomicFlagMismatch { item }` | `IN_ATOMIC_EDIT_SPAN` disagrees with `atomic_spans` |
| `InvalidAtomicSpan { span }` | empty, outside the text, duplicate, out of order (ascending start), overlapping or nested, bad edge, or wraps no atom or object |

**Validation cost [Design decision].** `check_layout_inputs` makes constant linear passes with boundary indexes, bounded stacks and indexed id lookup. With the standard randomized hash maps and sets its time is expected O(total input records and bytes), with O(input boundaries + spans) extra memory. It neither sorts incoming spans (disorder is rejected in O(N)) nor copies annotation payloads to validate a temporary AST, and it does not rescan all items for every span or atom: the end-to-end validator, not merely `check_spans` in isolation, must avoid quadratic scans. A release-mode benchmark over 1250 and 5000 spans (`#[ignore]`d, run explicitly) provides scaling evidence; establishing a one-pass bound would need deterministic work counting.

### 2.6 Middle-end output

```rust
pub struct PositionedLine {
    pub slots: SmallVec<[PositionedSlot; 1]>, // every available interval of the line and its content; never empty
    pub is_ellipsized: bool,          // the suffix represents clamped source
    pub annotations: Vec<PositionedAnnotationRun>,
    pub atomic_fragments: Vec<AtomicFragment>, // independent atomic identity and full logical range
    pub byte_range: TextRange,        // includes trailing whitespace
    pub visible_byte_end: ByteIndex,  // includes every non-collapsible space
    pub baseline_y: f32, pub ascent: f32, pub descent: f32, pub line_height: f32,
    pub visual_width: f32,            // sum of the slots' content advances; hanging glyphs excluded
    pub hanging_left: f32,            // overflow beyond the left edge of the first occupied slot's content
    pub hanging_right: f32,           // overflow beyond the right edge of the last occupied slot's content
    pub is_justified: bool,
    pub visual_runs: Vec<VisualGlyphRun>,   // screen left to right
    pub visual_boxes: Vec<PositionedInlineBox>, // inline objects, see below
    pub tabs: Vec<PositionedTab>,           // tabs, see below
    pub span_fragments: Vec<PositionedSpanFragment>, // decoration fragments
    pub caret_stops: Vec<CaretStop>,        // visual-order stops, see 6.1
    pub decorations: Vec<DecorationLine>,
}
// PositionedLine::validate(&LineContext), is_valid(&LineContext), extreme_caret_stop(HangSide)
pub struct VisualGlyphRun {
    pub orientation: GlyphOrientation,
    pub font_size: f32,
    pub variations: SmallVec<[FontVariation; 2]>,
    pub font: FontRef,
    pub bidi_level: BidiLevel,
    pub byte_range: TextRange,        // source bytes covered by the run
    pub glyphs: Vec<PositionedGlyph>, // screen left to right: pen positions never decrease
}
pub enum DecorationKind { Underline, Strikethrough }
pub struct DecorationLine {           // merged across adjacent clusters and glue; colour is looked up by byte_range
    pub kind: DecorationKind, pub byte_range: TextRange,
    pub x: f32, pub width: f32,       // line-local left end; non-negative extent
    pub y_offset: f32, pub thickness: f32,
}
pub struct PositionedSlot {
    pub constraint: InlineSlotConstraint, // the available interval, as resolved at layout; never requeried
    pub content_start: f32,           // left edge of the content block, inside the constraint
    pub visual_width: f32,            // advance of the content block
}                                     // PositionedSlot::content_end() = content_start + visual_width
pub enum HangSide { Left, Right }     // HangSide::is_line_start(direction)
pub struct LayoutPolicySnapshot {     // the policy a paragraph was laid out under, recorded in its output
    pub writing_mode: WritingMode,
    pub strut: StrutMetrics,          // resolved strut, half-leading folded in
    pub line_height: LineHeight, pub whitespace: HangingWhitespaceMode, pub hang_mode: HangMode,
    pub wrap_width: f32,              // LayoutConstraints::wrap_width
    pub slot_source: SlotSource,      // where the lines' slots came from
    pub clamp: Option<LineClampPolicy>,
}
pub enum SlotSource {                 // standard_slot(line_index, wrap_width) -> Option<InlineSlotConstraint>
    Standard { first_line_indent: f32, hanging_indent: f32 },  // no provider: one slot [indent, wrap_width]
    Provider,                         // a LineGeometryProvider answered; only the slots' shape is checkable
}
// LayoutPolicySnapshot::resolve(ast, constraints, options) -> Result<Self, LayoutInputError>, is_valid(),
//                       line_context(base_direction, line_index)
pub struct LineContext {              // Copy; what one line is validated against. No Default.
    pub writing_mode: WritingMode,
    pub base_direction: BidiDirection,
    pub strut: StrutMetrics,
    pub line_height: LineHeight, pub whitespace: HangingWhitespaceMode, pub hang_mode: HangMode,
    pub wrap_width: f32,
    pub line_index: usize,
}
// LineContext::for_paragraph(ast, constraints, options, line_index) -> Result<Self, LayoutInputError>
pub const LINE_GEOMETRY_ABS_EPSILON: f32 = 1e-3;
pub const LINE_GEOMETRY_REL_EPSILON: f32 = 1e-5;
pub struct AtomicFragment {
    pub atomic_id: u64,
    pub full_byte_range: TextRange,     // the whole atomic span; the same on every fragment of it
    pub fragment_byte_range: TextRange, // non-empty; the fragments of a span on one line tile its part of the line
    pub x: f32, pub width: f32,         // physical left edge, line-local; width >= 0
    pub bidi_level: BidiLevel,
}
pub struct PositionedGlyph {
    pub glyph_id: u32,
    pub x: f32,          // STRICT PEN ADVANCE POSITION, not the drawing origin
    pub x_offset: f32,   // shaper horizontal ink offset; the drawing origin is x + x_offset
    pub y_offset: f32, pub advance: f32,
    pub byte_range: Option<TextRange>,  // None if and only if synthesized; otherwise non-empty and inside the line
    pub is_synthesized: bool,           // generated hyphen; never copied
    pub is_hanging: bool,               // hanging punctuation with actual overflow; may straddle a body edge
}
pub struct PositionedInlineBox {
    pub baseline_align: BaselineAlignment, // from BoxNode::InlineObject
    pub box_id: u64, pub byte_range: TextRange,
    pub x: f32,                         // left edge, line-local; inside the content of one slot
    pub y: f32,                         // box baseline relative to the line baseline, y down
    pub width: f32, pub ascent: f32, pub descent: f32,   // non-negative
    pub overflow_policy: BoxOverflowPolicy,              // from BoxNode::InlineObject
    pub bidi_level: BidiLevel,                           // from BoxNode::InlineObject
}
pub struct PositionedTab {
    pub byte_range: TextRange,
    pub x: f32,                         // left edge, line-local
    pub width: f32,                     // non-negative; next_tab_stop(x) - x
    pub bidi_level: BidiLevel,          // the paragraph level (UAX #9 L1)
}
pub struct PositionedSpanFragment {     // post-bidi visual background box
    pub span_id: u64,
    pub span_range: TextRange,          // the whole span; the same on every fragment of it
    pub byte_range: TextRange,          // the part of the span on this fragment: inside span_range and the line
    pub x: f32,                         // left visual edge, line-local
    pub width: f32,                     // non-negative; includes the padding that applies
    pub ascent: f32, pub descent: f32,  // non-negative
    pub is_first_fragment: bool,        // true iff byte_range.start == span_range.start
    pub is_last_fragment: bool,         // true iff byte_range.end == span_range.end
    pub bidi_level: BidiLevel,          // level of the text in the fragment; its parity orients hit tests
}
pub struct PositionedAnnotationRun {    // see "Annotation output" below
    pub annotation_id: u64, pub base_byte_range: TextRange, pub side: AnnotationSide,
    pub font: FontRef, pub font_size: f32, pub variations: SmallVec<[FontVariation; 2]>,
    pub orientation: GlyphOrientation,
    pub baseline_offset: f32,           // toward line-over
    pub max_overhang: f32, pub block_extent: f32,
    pub base_over: f32, pub base_under: f32,
    pub glyphs: Vec<PositionedGlyph>,   // synthesized, no body byte range
}
pub struct PositionedParagraph {
    pub policy: LayoutPolicySnapshot,   // what the lines were laid out under, writing mode included
    pub base_direction: BidiDirection,  // copied from the AST
    pub truncated_range: Option<TextRange>, // unrendered logical tail, when clamped
    pub total_block_size: f32,          // block extent including clearance
    pub lines: Vec<PositionedLine>,     // top to bottom
}
// PositionedParagraph::validate_geometry(), validate(text)
```

**Geometry conventions [Design decision].** `x` is line-local and is the strict **pen advance position** along the line, not the drawing origin; the rasteriser draws the glyph at `(line_origin_x + glyph.x + glyph.x_offset, baseline_y - glyph.y_offset)`. A zero-advance combining mark with a negative `x_offset` at the start of a line therefore has `glyph.x == 0.0` and never trips the containment check; `x` may still be negative for left-hanging punctuation. Containment is judged on the pen interval `[x, x + advance]`. `visual_width` is the body width: the advance envelope of the main content, excluding only the overflow of hanging punctuation and preserved collapsible trailing whitespace; the body is `[0, visual_width]`. `ParagraphAst::strut_metrics` is the **raw** primary-font metrics. **Policy snapshot.** `LayoutPolicySnapshot::resolve(ast, constraints, options)` resolves the policy of a paragraph once: it validates the constraints, the clamp and the strut inputs and calls `LineHeight::resolve_strut(raw, strut_font_size, writing_mode)` to obtain the active `StrutMetrics`, with half-leading folded into ascent and descent (5, Stage 4.1); `strut_font_size` is the size recorded in the AST, since `LayoutOptions` carries no font size. A `PositionedParagraph` records that snapshot, so validating it takes nothing from the caller and it cannot be checked against a more permissive policy than the one that produced it. A line is validated against a `LineContext`, the snapshot's line-relevant fields plus the base direction and the line's index: `policy.line_context(base_direction, i)`, or `LineContext::for_paragraph(ast, constraints, options, i)` from the layout inputs. There is no default context, because an all-zero strut validates no line. `PositionedLine::validate` checks a line against its context in the passes below and reports the first violation as a `LineInvariantError`; the variants and their triggers are listed at the end of this section.

*Pass 1, finiteness.* Every number of every entity is finite: line metrics and extents, each glyph's `x`, `x_offset`, `advance`, `y_offset`, each inline box's coordinates and sizes, each tab's `x` and `width`, each span fragment's `x`, `width`, `ascent` and `descent`; extents, box sizes, tab widths and fragment sizes are also non-negative. (A glyph `advance` may be negative, since kerning can produce one.) *Pass 2, envelope.* (a) `visible_byte_end` lies within `byte_range` and is a boundary: the line's start or end, or the start or end of a non-synthesized glyph, an inline box or a tab of the line (glyph ranges are whole clusters and clusters whole graphemes); (b) an empty line (`byte_range` empty, not ellipsized) has no visual runs, boxes, tabs, span fragments, atomic fragments or annotations, zero `visual_width` and hanging extents, and a `line_height` equal to the strut's (under `LineHeight::Fixed` also the strut's `ascent` and `descent`); (c) under `LineHeight::Fixed` every line's `line_height`, `ascent` and `descent` equal the strut's (a fixed line box never grows, for fallback glyphs or for annotations, which overflow into the inter-line space), under `LineHeight::Normal` `line_height` is at least the strut's (fallback glyphs and annotations may enlarge it). *Pass 3, containment and hanging.* (d) every non-hanging, non-trailing glyph, inline box and tab lies inside the content block of one slot (see "Slots" below; on a line with one slot whose content starts at 0 the block is `[0, visual_width]`, which the rest of this paragraph writes for brevity) (a glyph by its pen interval; a zero-advance glyph by its pen position, unless it is flagged `is_hanging` and so rides on a hanging base); every `is_hanging` glyph has actual overflow beyond tolerance, but may straddle a body edge; normalize `lo = min(x, x + advance)`, `hi = max(x, x + advance)`; (e) `hanging_left` and `hanging_right` equal respectively the maximum overflow of the hanging glyphs beyond the left edge of the first occupied slot's content and beyond the right edge of the last one's (`max(0, -lo)` and `max(0, hi - visual_width)` for one slot from 0), so are positive exactly when such glyphs exist, and an edge that `HangMode::allows(side, base_direction, line_index == 0)` forbids has none; under `HangEndMode::Allow` a mark hangs at the line end only if it would otherwise wrap, so the slot it hangs from must be too narrow for it: `slot.visual_width + hanging extent > slot capacity` (`HangingWithoutOverflow` otherwise; an unbounded slot never hangs anything), while `Force` hangs regardless; (f) trailing whitespace (glyphs and tabs whose `byte_range` starts at or after `visible_byte_end`) has zero width under `Collapsible`; under `Preserved` it is exempt from (d) but, by UAX #9 L1 (6.3, Stage 3), lies on the far edge of the base direction, right of the body in an LTR paragraph and left of it in an RTL one, and never counts towards the hanging extents. Comparisons use `max(1e-3, 1e-5 · max(|a|, |b|))`, because `f32` sums at large coordinates drift by more than a fixed epsilon. (g) every `PositionedSpanFragment` lies inside the content of one slot, covers bytes inside its `span_range` and inside the line, and has `is_first_fragment` exactly when its bytes start where the span does and `is_last_fragment` exactly when they end where it does (`SpanFragmentEdgeMismatch`), which a painter joining borders can therefore trust on the line alone; every glyph (hanging glyphs, trailing whitespace and synthesized glyphs excepted) or inline box whose `byte_range` lies within the `byte_range` of a fragment of span *S* has its interval contained in **at least one** fragment of *S*. A line that consists of a single `InlineObject` with `AllowOverflow` wider than `wrap_width` sets `visual_width` to the object's width, so (d) and (g) hold unchanged. (h) every `Clip` or `ScaleToFit` inline box satisfies `x + width <= ctx.wrap_width + eps`; and `visual_width <= ctx.wrap_width + eps`, with two single-entity exemptions. **A:** the line has no visual runs, no tabs and exactly one inline box, whose policy is `AllowOverflow`. **B:** the line has no inline boxes and no tabs, and its glyphs belong to one indivisible cluster (the emergency cluster break on an empty line; they share one non-empty `byte_range`). Let *entity_range* be the box's `byte_range` (A) or the cluster's, and *entity_start_x*, *entity_end_x* the box's `x` and `x + width` (A) or the start of the first and end of the last glyph (B), with *entity_width* = `width` (A) or the sum of the glyph advances, which must equal `entity_end_x - entity_start_x` (B). The exemption applies only to an entity that cannot fit: the line has one slot as wide as `wrap_width`, the content starts at the slot's start (no alignment offset) and is wider than the slot. If `span_fragments` is empty: `entity_start_x == 0` and `visual_width == entity_width` (within eps). If it is non-empty (the entity sits inside padded decoration spans): every fragment starts at `entity_range.start` and ends at or after `entity_range.end`, `entity_range.end >= visible_byte_end` (so any bytes a fragment owns past the entity lie in the trailing whitespace, since (g) keeps fragments inside the line), `0 - eps <= entity_start_x` and `entity_end_x <= visual_width + eps`, and the fragments reach both edges of the body, `min(frag.x) == 0` and `max(frag.x + frag.width) == visual_width` (within eps). A 26-wide cluster with 3 units of padding on each side thus passes at `wrap_width = 30`, with no new field on `PositionedSpanFragment`, while an unpadded entity cannot inflate `visual_width`. A fragment's `bidi_level` must not exceed `BIDI_MAX_DEPTH + 1`. The check is containment, not tiling: glue yields no output entity, so entities need not cover the body without gaps.

**Slots [Design decision].** A line reports one `PositionedSlot { constraint, content_start, visual_width }` per available interval of its geometry (5, "Line geometry and slots"). The slot carries the `InlineSlotConstraint` it was laid out in, so a line describes its own geometry and validation never queries a provider again; there is no separate snapshot that could disagree with the slots, and a line always has at least one slot. The constraints are well formed, ascending and disjoint, and when `wrap_width` is finite every one lies in the line's inline envelope `[0, wrap_width]` (`0 <= start < end <= wrap_width`, within the geometry tolerance); with an infinite `wrap_width` only `start >= 0` applies. The slots' content widths sum to the line's `visual_width`.

**The content block [Design decision].** A slot's content occupies `[content_start, content_start + visual_width]`, which lies inside its constraint. The constraint is thereby divided exactly: `[constraint.start, content_start)` is alignment offset, the block is content, and what follows it is slack (unjustified and last lines keep theirs), so a centred or end-aligned line and a line that starts with preserved spaces are distinguishable. Inside the block the **line-end** edge is exact and the **line-start** edge may be preceded by preserved leading glue, which has no output entity of its own. In an LTR paragraph the last item ends at the block's right edge, `visual_width == max(right edge) - content_start`, and the gap `[content_start, first item)` is leading glue (a hard break followed by spaces, or a paragraph that opens with NBSP); in an RTL paragraph the mirror image holds: the leftmost item starts at `content_start` and leading glue lies between the last item and the block's right edge. Content is the body glyphs (pen `x` to `x + advance`), inline boxes, tabs and decoration-span fragments (whose padding is advance no glyph covers); trailing whitespace, zero-advance marks and hanging punctuation are not content, except that the part of a hanging glyph inside the block counts for the slot it hangs from. A slot without content has `visual_width == 0`. Body glyph advances, objects, tabs, decoration fragments and atomic fragments each fit wholly inside one slot's block. A block may end beyond its constraint only for a single slot at full unobstructed width holding, from its start, one `AllowOverflow` object or one indivisible cluster wider than the slot (exemptions A and B of Pass 3 (h)), optionally with a synthesized clamp suffix; `Clip` and `ScaleToFit` objects do not receive that exemption (a block that ends beyond its constraint otherwise is `SlotOverflow`, and a line wider than `ctx.wrap_width` is `LineWiderThanWrapWidth`), and a synthesized-only ellipsis receives the narrow-column exemption. Hanging extents equal the actual outer overflow of the first and last occupied slots and must be permitted by `HangMode`; unused slots and internal obstacles cannot hold hanging glyphs or preserved trailing whitespace. Zero-advance marks keep the advance-geometry exemption; no ink extents are available in this API.

**Source ownership [Design decision].** A glyph is synthesized if and only if its `byte_range` is absent; a present range is non-empty and lies inside the line's `byte_range`. Body source ranges belong to the line, and so do the bytes of tabs, boxes (never empty), decorations and caret stops. The glyphs of a visual run are in screen order: pen positions never decrease, `glyphs[i].x <= glyphs[i + 1].x` (`GlyphsOutOfVisualOrder`); a mark shares its base's position and keeps its drawing offset in `x_offset`, and a negative advance does not disturb the order, since only `x` is compared. A caret stop carries the bidi level of the content it belongs to: its owners are the glyphs, inline boxes and tabs whose bytes contain or touch the stop's `offset` (a glyph lends its visual run's `bidi_level`), at a boundary either neighbour qualifies, and the stop's level is the level of one of them (`InvalidCaretStop`). A stop with no owner is legal only on a line with no glyphs, boxes or tabs (an empty line), where it carries the paragraph level. A tab is always at the paragraph level, to which UAX #9 L1 resets segment separators (`InvalidTab`). **Atomic fragments.** Each fragment owns a non-empty `fragment_byte_range` inside its `full_byte_range` and inside the line; bidi reordering can split one span into several fragments on a line, and those agree on `full_byte_range`, are disjoint and together cover exactly the part of the span that lies on the line (`InvalidAtomicFragment`).

**Annotation output [Design decision].** A `PositionedAnnotationRun` (2.9) carries `annotation_id`, `base_byte_range`, `side`, its font instance (`font`, `font_size`, `variations`), `orientation`, `baseline_offset`, `max_overhang`, `block_extent`, the producer-measured `base_over` and `base_under` (the largest extents of the base's glyphs and inline boxes toward line-over and line-under) and its glyphs, which are synthesized, finite and carry no body byte range. It attaches to a non-empty visible base (`base_byte_range` inside `byte_range.start..visible_byte_end`) in exactly one slot. Every non-synthesized body glyph intersecting the base lies wholly inside it and in that slot, and at least one such glyph exists. Glyph-less source nodes (notably glue and zero-width source bytes) may sit inside the base, so contiguous byte coverage is not required: the base envelope is the union of the intersecting body glyph intervals. Annotation advance intervals stay within both the base-relative overhang allowance (`max_overhang` beyond the envelope) and the active slot. The annotation sits no closer to the baseline than its base: `baseline_offset >= base_over` for `Over`, `baseline_offset <= -base_under` for `Under`. Because the output carries `block_extent`, a standalone validator can verify the declared height: under `LineHeight::Normal` the line's ascent (descent) is at least the larger of the strut's and `base_over + block_extent` (`base_under + block_extent`); under `Fixed` the line box stays locked to the strut and the annotation overflows. Annotation ids are unique on a line and in the paragraph, and two annotations on one side never share base bytes (opposite sides may overlap).

**Empty and clamped lines [Design decision].** An ordinary empty line has zero content and no visual entities or ellipsis. The explicit clamp exception permits an empty `byte_range` with only synthesized ellipsis output, or no glyphs for an empty suffix; this avoids falsely consuming an invisible source atom and preserves `truncated_range = 0..text.len()` when a clamp removes every base cluster.

**`LineInvariantError` [Design decision].** `#[non_exhaustive]`; `PositionedParagraph::validate_geometry` wraps it as `(line_index, error)`. Variants, grouped by pass:

| Variant | Trigger |
|---|---|
| `InvalidSlots { reason }` | the slots are malformed or disagree with the content; `SlotErrorReason`: `InvalidWrapWidth`, `NoSlots`, `InvalidConstraint` (malformed, outside the wrap envelope, or out of order), `InvalidContent` (a block that is not finite, is negative or starts outside its constraint), `WidthNotConserved`, `WidthNotContentSpan`, `ContentOutsideSlots` |
| `OrientationMismatch` | a run's orientation does not match the writing mode or level parity |
| `InvalidGlyphSource { run, glyph }` | synthesized flag and source range disagree, or the range is empty, inverted or outside the line |
| `InvalidAnnotation { index, reason }` | an annotation is malformed or misplaced; `AnnotationErrorReason`: `OnEmptyLine`, `InvalidMetrics`, `OrientationMismatch`, `BaseOutsideLine`, `NoGlyphs`, `InvalidGlyph`, `DuplicateId`, `OverlapsOnSide`, `GlyphStraddlesBase`, `NoBaseGlyphs`, `BaseSpansSlots`, `OutsideOverhang`, `InsideBase` (closer to the baseline than its base), `DoesNotFitLineBox` |
| `InvalidParagraph { reason }` | a paragraph-level invariant, reported with the line where it was detected; `ParagraphErrorReason`: `InvalidPolicy`, `NoLines`, `InvalidBlockSize`, `LineBoxesOverlap`, `LineBoxExceedsLineHeight`, `NotStandardSlots`, `SourceNotCovered`, `InvalidTruncation`, `DuplicateAnnotation`, `InconsistentSpanFragments`, `InconsistentAtomicFragments` |
| `NonFiniteOrNegativeExtent` | `visual_width`, `hanging_left` or `hanging_right` negative or non-finite |
| `NonFiniteLineMetrics` | `baseline_y`, `ascent`, `descent` or `line_height` non-finite |
| `NonPositiveLineHeight { expected, actual }` | a line height or the resolved strut height is not positive |
| `VisibleEndOutsideRange` | `visible_byte_end` outside `byte_range` |
| `VisibleEndNotOnBoundary` | `visible_byte_end` is neither the line's start or end nor the start or end of a non-synthesized glyph, box or tab |
| `EmptyLineHasVisualRuns`, `EmptyLineHasBoxes`, `EmptyLineHasTabs`, `EmptyLineHasSpanFragments`, `EmptyLineHasAtomicFragments` | an empty line carries that entity kind |
| `EmptyLineHeightMismatch { expected, actual }` | an empty line's height differs from the strut's |
| `EmptyLineHasExtent` | an empty line has non-zero `visual_width` or hanging extent |
| `LineHeightMismatch { expected, actual }` | under `Fixed`, `line_height` differs from the strut's |
| `LineMetricsMismatch { expected_ascent, expected_descent, actual_ascent, actual_descent }` | under `Fixed`, ascent or descent differs from the strut's (empty lines included) |
| `LineHeightBelowStrut { minimum, actual }` | under `Normal`, `line_height` is below the strut's |
| `InvalidVisualRun { run }` | a run has no glyphs, bytes outside the line, or a non-finite or non-positive font size, fallback scale or variation |
| `GlyphsOutOfVisualOrder { run, glyph }` | a glyph's pen position lies left of the previous glyph's in its run |
| `InvalidAtomicFragment { index }` | an atomic fragment has invalid geometry or bidi level, bytes that are empty, outside the line or outside its span, or the fragments of its span on the line disagree about the span, overlap, or do not cover its part of the line |
| `NonFiniteGlyph { run, glyph }` | a glyph `x`, `x_offset`, `advance` or `y_offset` is non-finite |
| `TrailingWhitespaceNotCollapsed { run, glyph }` / `TrailingTabNotCollapsed { index }` | trailing whitespace or tab has width under `Collapsible` |
| `TrailingWhitespaceOnWrongSide { run, glyph }` / `TrailingTabOnWrongSide { index }` | preserved trailing whitespace or tab is not on the far edge of the base direction (UAX #9 L1) |
| `TrailingWhitespaceMarkedHanging { run, glyph }` | a trailing-whitespace glyph has `is_hanging` |
| `HangingGlyphInsideBlock { run, glyph }` | an `is_hanging` glyph does not overflow the body |
| `BodyGlyphOutsideBlock { run, glyph }` | a body glyph lies outside the content of every slot |
| `HangingExtentMismatch { side, expected, actual }` | a hanging extent differs from what the hanging glyphs occupy |
| `HangingNotAllowed { side }` | punctuation hangs at an edge the `HangMode` forbids |
| `HangingWithoutOverflow { side }` | under `HangEndMode::Allow`, a mark hangs at the line end although it would have fitted its slot |
| `InvalidInlineBox { index }` | non-finite coordinate, negative size, invalid baseline alignment or unrepresentable bidi level, or bytes that are empty or outside the line |
| `InlineBoxOutsideBlock { index }` | an inline box outside the content of every slot |
| `InlineBoxWiderThanWrapWidth { index }` | a `Clip` or `ScaleToFit` box ends beyond `ctx.wrap_width` |
| `LineWiderThanWrapWidth { visual_width, wrap_width }` | line wider than `ctx.wrap_width` (reported) and not exemption A or B |
| `SlotOverflow { slot, extent, capacity }` | a slot's content ends beyond its constraint (`extent` is measured from the constraint's start), the line still within `ctx.wrap_width`, and not exemption A or B |
| `InvalidTab { index }` / `TabOutsideBlock { index }` | tab non-finite, negative, not at the paragraph level, empty, outside the line, or outside the body |
| `InvalidSpanFragment { index }` / `SpanFragmentOutsideBlock { index }` / `SpanFragmentOutsideLine { index }` | fragment non-finite or negative, outside the body, or outside the line's bytes |
| `SpanFragmentEdgeMismatch { index }` | a fragment's bytes lie outside its `span_range`, or a first / last flag disagrees with where the fragment starts / ends |
| `GlyphOutsideSpanFragment { span_id, run, glyph }` / `InlineBoxOutsideSpanFragment { span_id, index }` | a glyph or box of a span lies outside every fragment of that span |
| `InvalidCaretStop { index }` | non-finite `x`, out of visual order, outside the line, other line, or a level that is unrepresentable or that no glyph, box or tab owning the stop's offset has (on an empty line: not the paragraph level) |
| `InvalidDecoration { index }` | non-finite coordinate, negative width or thickness, or bytes outside the line |

**Paragraph validation [Design decision].** `PositionedParagraph::validate(text)` extends `validate_geometry()`; both read the paragraph's own `policy` and take no context from the caller. Geometry: a policy that `LayoutPolicySnapshot::resolve` could have produced (`is_valid`: a valid strut, line height and clamp, a positive wrap width, and standard indents inside `[0, wrap_width)`); at least one line, each line valid under `policy.line_context(base_direction, line_index)`; under `SlotSource::Standard`, each line has exactly the one slot `[indent, wrap_width]`, `first_line_indent` on line 0 and `hanging_indent` after it (`NotStandardSlots`; a provider's answers cannot be recomputed after layout, so under `SlotSource::Provider` only their shape is checked); forward, non-overlapping block bounds from `WritingMode::line_bounds(baseline_y, ascent, descent)`; `ascent + descent <= line_height`; a finite `total_block_size` no smaller than the last line's bottom. Coverage: UTF-8 character-boundary ranges starting at zero, adjacent range continuity and the exact remaining tail. Unclamped output ends at `text.len()` with no ellipsized line; clamped output requires a recorded clamp policy, a non-empty `truncated_range` starting at the final line's end and ending at `text.len()`, and exactly the final line ellipsized. **Identity across lines.** Annotation ids are unique in the paragraph. The fragments of one decoration span carry one `span_range`, and exactly one of them is the first and exactly one the last; each line already ties the flags to the fragment's bytes (Pass 3 (g)), so the fragments are in logical order, and a span whose end lies in the truncated tail has no last fragment. The fragments of one atomic span carry one `full_byte_range` and cover every rendered byte of it, so a line that crosses the span without a fragment is an error. `validate_geometry` serves geometry-only fixtures and does not certify source coverage. `total_block_size` is the authority for coordinate transforms (2.7) and includes clearance; `base_direction` is copied from the AST and is the direction every line is validated with. The skeleton `layout_paragraph` returns records the policy and has no lines, so it cannot pass `PositionedParagraph::validate`.

The draft required a mandatory `byte_range` on every glyph while 6.6 said synthesized glyphs have none. **[Design decision]** The field is `Option<TextRange>`. Because `CjkAutospace` has `start == end`, it cannot be a distinct item inside `line.byte_range`; this is harmless but must be handled in the line builder `[C1]`.

### 2.7 Writing modes and orientation

```rust
pub enum WritingMode { HorizontalTb, VerticalRl, VerticalLr }
pub enum TextOrientation { Mixed, Upright, Sideways }      // frontend policy
pub enum GlyphOrientation { Horizontal, VerticalUpright, VerticalSideways, HorizontalInVertical }
pub enum BaselineAlignment { Auto, Alphabetic, Ideographic, Central, Hanging, Mathematical,
                             LineTop, LineBottom, Offset(f32) }
```

`WritingMode` provides `is_vertical`, `flow_to_physical_point`, `physical_to_flow_point`, `flow_to_physical_rect`, `physical_to_flow_rect`, `over_to_block` and `line_bounds`; `GlyphOrientation::is_valid(mode, level)` and `BaselineAlignment::is_valid` check the rules stated below.

**Flow space [Design decision].** Layout runs in canonical flow coordinates: inline `x` and block-progressing `y`. `WritingMode` maps a flow point to paragraph-local physical coordinates: `HorizontalTb` to `(x, y)`, `VerticalLr` to `(y, x)`, `VerticalRl` to `(total_block_size - y, x)`, where `PositionedParagraph::total_block_size` is the block extent used by every transform. Physical coordinates are paragraph-local; placing the paragraph in a container is a host translation. Rectangle transforms normalise their transformed opposite corners.

**Line-over mapping [Design decision].** Offsets toward line-over are not block coordinates. A positive over offset maps to a negative block delta in `HorizontalTb` and `VerticalRl` and to a positive one in `VerticalLr` (`WritingMode::over_to_block`). A line's block bounds are `[baseline - ascent, baseline + descent]`, except `VerticalLr`, which uses `[baseline - descent, baseline + ascent]` (`WritingMode::line_bounds`); both vertical modes therefore place over annotations physically to the right. Validators use the same helper. Baseline offsets in `FontMetrics` and `BaselineAlignment::Offset` are positive toward line-over, and an explicit offset must be finite.

**Orientation [Design decision].** `TextOrientation` is the frontend policy: `Mixed` resolves each character from the vertical-orientation properties, `Upright` forces upright glyphs and an LTR paragraph, `Sideways` shapes horizontally and rotates clockwise. Each atom, cluster and run carries a resolved `GlyphOrientation`. `Horizontal` is valid only in `HorizontalTb`; the three vertical orientations only in a vertical mode; `VerticalUpright` and `HorizontalInVertical` additionally require an even resolved bidi level (`GlyphOrientation::is_valid`). Atoms and clusters must match their owning run's orientation, and `VisualGlyphRun` and annotation runs carry theirs; any violation is `OrientationMismatch`.

**Tate-chu-yoko [Design decision].** A `HorizontalInVertical` run is one indivisible horizontal group. It shapes to exactly one cluster covering all of the run's atoms (the backend merges per-character shaping clusters itself, 4.1) and is referenced by exactly one text box that spans the whole run, so it has no internal break opportunity: a soft break between two boxes of the same TCY run is rejected. Its `natural_width` is the group's advance along the vertical line. TCY text is atomic for editing only when independently covered by an `AtomicEditSpan`.

**Vertical metrics.** In a vertical mode the strut and line extents come from the vertical metrics: `LineHeight::resolve_strut` reads `vertical_ascent`, `vertical_descent` and `vertical_line_gap` as ascent, descent and gap (5, Stage 4.1).

### 2.8 Explicit bidirectional spans

```rust
pub struct ExplicitBidiSpan { pub span_id: u64, pub byte_range: TextRange, pub control: BidiControlKind }
pub enum BidiControlKind { Isolate(Option<BidiDirection>) /* FSI/LRI/RLI .. PDI */,
                           Override(BidiDirection)        /* LRO/RLO .. PDF */ }
pub const MAX_BIDI_SPAN_DEPTH: usize = 63;
```

Source-level directional controls travel out of band as `ParagraphAst::bidi_spans`, a stream independent of decoration and atomic spans (its `span_id` is a caller handle that the engine does not interpret). `check_layout_inputs` enforces (`InvalidBidiSpan`): unique ids; non-empty ranges that fit the text; both ends on concrete content boundaries (a text atom, glue, tab, inline object or discretionary edge); spans ordered by ascending start and descending end and properly nested, so a child never ends after its parent; equal ranges rejected; and nesting at most `MAX_BIDI_SPAN_DEPTH = 63`, counted on the source spans independently of resolved levels, which can reach `BIDI_MAX_DEPTH + 1 = 126`. The validator does not rerun UAX #9.

### 2.9 Annotations

```rust
pub struct AnnotationSpan {
    pub annotation_id: u64,            // unique
    pub base_byte_range: TextRange,    // atom- and cluster-aligned, text only
    pub side: AnnotationSide,          // Over | Under
    pub kind: AnnotationKind,          // Ruby { alignment, max_overhang } | Emphasis
    pub block_extent: f32,             // additional block extent including the gap; finite, >= 0
    pub annotation_text: Box<str>,     // non-empty, no line or paragraph separator
    pub run: ShapingRun,               // annotation-local coordinates
    pub atoms: Vec<GraphemeAtom>,      // local; run.atom_range == 0..atoms.len()
    pub clusters: Vec<ShapedCluster>,  // local; run.cluster_range == 0..clusters.len()
    pub glyphs: Vec<RawShapedGlyph>,   // local; the clusters' glyph_ranges tile it
}
pub enum AnnotationSide { Over, Under }
pub enum AnnotationAlignment { Center, SpaceAround, SpaceBetween, Start }
pub enum AnnotationKind { Ruby { alignment: AnnotationAlignment, max_overhang: f32 }, Emphasis }
```

`Over` is above horizontal text and right of either vertical mode (2.7); `Under` is below horizontal text and left of vertical text. **Validation [Design decision]** (`InvalidAnnotationSpan`): ids are unique; the base is non-empty, fits the text and has both ends on atom and cluster edges of one text-only stretch, so discretionaries, glue, tabs and inline objects cannot occur inside a base; on each side the bases are ordered by start and do not overlap, while bases on opposite sides may overlap. `annotation_text` is partitioned exactly by the local atoms and clusters, with `run.byte_range == 0..annotation_text.len()`. The annotation's run, font, scale, flag, orientation, level and safety checks share the body validation routines (2.5), and its atoms never carry `IN_ATOMIC_EDIT_SPAN`. A ruby `max_overhang` is finite and non-negative; an `Emphasis` mark is exactly one atom (one source character) and one cluster.

`block_extent` is the room the annotation needs beyond its base on its side, gap included. Under `LineHeight::Normal` the line box grows to hold `base_over + block_extent` (`Over`) or `base_under + block_extent` (`Under`) when that exceeds the strut (5, Stage 4.4); under `LineHeight::Fixed` it does not grow and the annotation overflows into the inter-line space.

**Ruby and emphasis compilation (future) [Design decision].** A ruby is an indivisible grouped span, expanded during shaping and spacing, before break compilation. Its base capacity is `max(base_width, ruby_width - 2 * max_overhang)`: only the part of a wide ruby that overhang cannot absorb widens the base. Expansion is distributed over the base clusters in proportion to their raw widths, equally when the raw total is zero. When both sides carry annotations over the same base, the width requirement is the maximum of the two, not their sum. Placement keeps the ruby inside its slot even where base-relative overhang would allow more. Emphasis repeats one shaped mark over each eligible atom of the base, excluding spaces and opening or closing punctuation. Truncation and editing never split an annotated group (5, "Line clamping and truncation"; 6.8).

---

## 3. Frontend semantic analysis pipeline

The frontend runs six passes. **Complexity (revised).** The draft's claim of "strict O(N)" is withdrawn as written. Passes 1, 2, 4 and 5 are plausibly linear. Pass 3 as drafted could be quadratic: the interior scan for each pair costs O(pair length), so nested pairs of depth D cost O(N·D), and a rightward search for each unclosed quote costs O(N) on text with no strong character. Real itemisers stay linear because their stacks are bounded: Blink keeps 32 brackets and drops the oldest, Pango 128 and empties the stack when full, and UAX #9 BD16 uses 63 and stops `[S1][S2][U3]`. This specification therefore requires: a bounded stack in passes 2 and 3 **[Design decision: depth 63 by default]**, memoised next-strong look-ahead computed by one reverse sweep **[Design decision]**, and no stated bound for pass 6, because dictionary and LSTM models are documented only qualitatively ("faster than the LSTM model but requires more data"; LSTM "more expensive during segmentation") `[S6]`.

**Paragraph splitting.** The frontend analyses one paragraph at a time. `split_paragraphs(text: &str) -> Vec<ParagraphSlice>` returns `ParagraphSlice { content: TextRange, separator: TextRange }` in absolute source-document UTF-8 bytes. It splits on UAX #9 class-B separators (LF, CR, CRLF as one separator, NEL U+0085, PS U+2029, U+001C–U+001E) and on the application-policy terminators VT U+000B and FF U+000C (`is_paragraph_separator`). Empty input returns one empty paragraph; a final separator produces a final empty paragraph (`"abc\n"` yields `(0..3, 3..4)` and `(4..4, 4..4)`); U+2028 stays in the content. Callers pass each content slice to the engine and keep the absolute range for document mapping; offsets inside the AST are paragraph-local (2.5).

```rust
pub struct FrontendOptions {
    pub writing_mode: WritingMode, pub text_orientation: TextOrientation,
    pub locale_policy: LocalePolicy, pub autospace_profile: AutospaceProfile,
    pub quote_policy: QuotePolicy,
    pub default_locale: LocaleTag,     // for runs that carry none (1.2)
}
pub enum LocalePolicy { ExplicitOnly /* default */, ExperimentalHeuristics /* no effect yet */ }
pub enum AutospaceProfile { Jlreq /* default */, CssCompat }   // 2.4 table
pub enum QuotePolicy { Standard /* default: 3, Step 3.1 */, CjkAware /* 3, Step 3.2 */ }
pub struct ParagraphSlice { pub content: TextRange, pub separator: TextRange }
// is_paragraph_separator(char), is_line_or_paragraph_separator(char), split_paragraphs(&str)
```

The frontend records raw strut metrics and never applies `LineHeight`; `FrontendOptions` therefore has no line-height field. `FrontendOptions::is_valid` states that every numeric field of the options and their profiles is within range; every field is an enum or a locale tag today, so it returns `true`, and the frontend entry point will call it.

**Analysis order.** Grapheme atoms (Step 1), glyph orientation in vertical modes (2.7), bidi resolution including explicit spans (Step 2, 2.8), script and locale (Steps 3 and 4), run splitting (Step 5), then shaping, spacing and ruby expansion (2.9) and break compilation (Step 6). Run splits include orientation, bidi level, style, script and locale. A tate-chu-yoko group is one run, one cluster and one text box (2.7).

### Step 1: Extended grapheme clusters and emoji classification

1. Segment the text by UAX #29 EGC rules `[U2]`. The rules relevant to the draft are GB9 (`× (Extend|ZWJ)`), GB9a, GB9b, GB11, GB12/GB13 and GB9c. Rev 49 (Unicode 18.0.0, 2026-09-01) states GB9c as `\p{InCB=Linker} \p{InCB=Extend}* × \p{InCB=Consonant}`. Rev 47 (Unicode 17.0, 2025-08-17) had a longer form; the change is logged in rev 49 `[U2]`. GB9c was introduced with Unicode 15.1.0 (rev 43, dated 2023-08-16) `[U2]`. The implementation must read `InCB` from `DerivedCoreProperties.txt`, not only `Grapheme_Cluster_Break`. Devanagari U+0915..U+0939 and U+0958..U+095F are `InCB=Consonant`; U+094D is `InCB=Linker` and GCB Extend `[U8]`. A Devanagari conjunct such as क्ष is then one atom.
2. Classify each cluster after segmentation (no second UTS #51 state machine, so caret movement cannot disagree with segmentation): `IS_EMOJI_SEQUENCE` is set when the cluster is a core emoji sequence, a ZWJ sequence (`emoji_zwj_element ( ZWJ emoji_zwj_element )+`), a keycap sequence (`[0-9#*] FE0F 20E3`; legacy forms without U+FE0F are treated separately), a flag sequence (two Regional Indicators), a tag sequence (tag base, tag characters U+E0020..U+E007E, tag end U+E007F), or a modifier sequence `[U5]`. The draft omitted keycap and tag sequences. A ZWJ inside Indic or Arabic text is `Script=Inherited` and must not become `Zsye`; only a ZWJ between pictographics qualifies `[U8]`. A lone Regional Indicator is not a valid flag but is still its own cluster `[U5]`.
3. An emoji modifier sequence does not require U+FE0F to request emoji presentation. Otherwise a cluster whose base has `Emoji_Presentation=No` and which is not followed by U+FE0F is text style (ED-7) and is not assigned `Zsye`. A cluster followed by U+FE0E is text style and is likewise excluded `[U5]`. UTS #51 states that selectors "should not be added to existing text without an explicit directive from the user" `[U5]`.
4. Set `HAS_VARIATION_SELECTOR` afterwards. U+FE0E, U+FE0F, the IVS selectors U+E0100..E01EF, tag characters and U+20E3 are all GCB=Extend and attach by GB9 `[U8]`. U+FE0E is VS15; IVS selectors are VS17 to VS256 and apply only to ideographs `[U6]`. Standardized Variation Sequences (VS1..VS16 generally) and Mongolian free variation selectors also exist; neither was verified in the research **[Unverified]**.
5. Known source conflict: UTS #51 rev 31 states that every emoji sequence is a single grapheme cluster, while UAX #29 rev 49 corrects its own note to say that only RGI sequences are, and that some valid non-RGI sequences consist of several clusters `[U2][U5]`. **[Design decision]** The engine follows UAX #29, which is what the segmenter produces.

### Step 2: Bidirectional resolution and pairing

1. **Base direction.** Use the caller's direction if given (HL1 permits overriding P2 and P3). Otherwise apply P2 and P3: find the first L, AL or R character, skipping characters between an isolate initiator and its matching PDI (or to the end of the paragraph if unmatched), ignoring embedding initiators but not their content; level 1 for AL or R, otherwise level 0 `[U3]`.
2. **Two pairing tables.** Quotation marks are not Bidi_Paired_Bracket characters: `BidiBrackets.txt` derives pairs from `gc=Ps/Pe`, `bc=ON` and `Bidi_M=Y`, lists 130 pairs, and has no entry for U+2018, U+2019, U+201C, U+201D, U+00AB, U+00BB, U+201E, U+2039 or U+203A `[U8]`. Adding them to the table used for N0 would change visual order against every conforming implementation (the research's inference: an Arabic paragraph containing «English» would pair the guillemets under N0 instead of resolving them by N1/N2).
   - **Table B (bidi)** is exactly `BidiBrackets.txt`. BD16 requires: opener's `Bidi_Paired_Bracket` (or canonical equivalent) equals the closer (or its canonical equivalent); U+2329/U+232A and U+3008/U+3009 are equivalent and no more will be added; a 63-element stack per isolating run sequence; if an opener is found while the stack is full, stop and return an empty list `[U3]`. N0 operates inside isolating run sequences after rules X1 to X10, using the character's current type after X6, so the pairing cannot be a single paragraph-wide `paired_map` computed on raw atoms. N0 treats EN and AN as R, applies N0b, N0c1 and N0c2, and leaves pairs with no strong content unset (N0d). A non-spacing mark following a changed bracket takes its new type `[U3]`.
   - **Table S (script pairing)** is a superset: brackets, guillemets (U+00AB/U+00BB, U+2039/U+203A), curly quotes (U+2018/U+2019, U+201C/U+201D), CJK corner brackets. Pango and the ICU sample script-run code do this `[S2][S3]`; Blink pairs only real brackets `[S1]`. Table S is used by Step 3 only and never influences bidi.
   - CLDR delimiters show that quote direction is language-dependent: en `“ ”`; de `„ “`; fr `« »`; ru `« »`; pl `„ ”`; ja `「 」`; sv, fi and he use U+201D for both start and end; ar uses `” “` `[U9]`. Pairing by code point alone is therefore a heuristic. **[Design decision]** Table S pairs by code point by default, with a locale-aware override table sourced from CLDR when a locale is known. The draft listed U+201E without a partner; under CLDR German it opens and U+201C closes.
3. **Atom-level levels.** UAX #9 is defined on code points, and the cluster-level layout needs one level per atom. **[Design decision]** An atom takes the level of its first code point, and non-spacing marks follow their base. The bridge must guarantee that no `ShapedCluster` straddles two levels.
4. **Intra-word apostrophe filter (revised).** The draft's rule applied to U+2019 between Latin, Cyrillic or Greek letters, and its example `rock 'n' roll` has a space on one side of each apostrophe, so it does not satisfy the rule. ASCII U+0027 is also absent from its table. UAX #29 defines this by Word_Break class, not script: WB6/WB7 bind `AHLetter (MidLetter | MidNumLetQ) × AHLetter`, where `MidNumLetQ` includes both U+2019 (MidNumLet) and U+0027 (Single_Quote); U+02BC is already ALetter `[U2][U8]`. **[Design decision]** U+0027, U+2019 and U+02BC are classed "ambiguous quote or apostrophe". The filter marks the character `IS_APOSTROPHE` and bypasses the Table S stack only if a Word_Break AHLetter lies on both sides. Leading and trailing apostrophes (’tis, dogs’, Afrikaans ’n) are out of the scope of the UAX #29 default word definition (§4.2.2) and need locale tailoring, which is optional `[U2]`. An apostrophe left on the stack can wrongly close a real quote (for example ‘That is ’tis dogs’ bone’), so a closer matches only if an opener of the same pair type is on the stack `[S2][S1]`.
5. **Explicit directional spans.** `ExplicitBidiSpan`s (2.8) are honoured as UAX #9 isolate and override controls at their concrete boundaries. Source nesting is capped at 63 independently of the resolved levels, which can still reach 126.

### Step 3: Script resolution (UAX #24)

**3.1 Default algorithm [Sourced]**:

- Explicit scripts stand. Combining marks (Mc, Mn, Me) inherit the script of their base and are never separated from it. Most marks are already absorbed because atoms are clusters; a mark at atom 0 or after a Common character needs a forward-scan fallback `[U4]`.
- Use `Script_Extensions`, not only `Script`. Blink intersects the current run's script set with each new character's set, so a Common character with extensions keeps the run alive if they intersect `[S1]`. Examples from ScriptExtensions.txt: U+0964 danda lists Deva, Beng, Guru and others; U+30FB lists Bopo, Hang, Hani, Hira, Kana, Yiii; U+300C lists Bopo, Hang, Hani, Hira, Kana, Yiii; U+00B7 lists Hani, Latn, Grek, Geor and others `[U8]`. Pango and the ICU sample use `Script` only `[S2][S3]`.
- Paired punctuation "belongs to the enclosing or outer level of the text and should therefore match the script of the enclosing text", and both members resolve to the same script where possible `[U4]`. Implement it with a bounded stack: push an opener that is Common; on a closer, scan back to the matching opener, pop to it, and give the closer the opener's script; leave the stack untouched for an unmatched closer; when full, apply the overflow policy (drop the oldest, as Blink does) `[S1]`. Pango resolves deferred entries when the run script becomes known (`start_sp`); this gives linear time with no look-ahead `[S2]`.
- Fullwidth forms: for Common characters with East Asian Width W, F or H and no Script_Extensions, Blink substitutes the Han ideographic script list taken from U+300C, to avoid splitting runs in the middle of CJK punctuation `[S1]`.
- Other Common characters take the preceding strong script; at paragraph start they take the first following strong script. This is the UAX #24 "simple heuristic" and Parley's behaviour `[U4][S4]`.

**3.2 Optional CJK-aware quote policy (`QuotePolicy::CjkAware`) [Design decision]**: the draft's rules 2 and 3 are kept as an opt-in policy, because they target a real goal (quotation glyphs taken from a CJK font) that no surveyed engine pursues by this means. They are made implementable by defining the terms:

- "CJK" means East Asian Width W, F or H, or script in {Hani, Hira, Kana, Hang, Bopo}. Hangul is not an ideograph; `IS_CJK_IDEOGRAPH` is not the test.
- Anchors a and b are the nearest non-whitespace atoms whose script has already been resolved by 3.1; a nested identical mark is not an anchor.
- Precedence: a pair is CJK if anchor a is CJK, else if anchor b is CJK, else if the interior contains CJK and the opener is not preceded by an ASCII space. The "ASCII space" test deviates from UAX #24 §5.1 and treats U+3000 and NBSP as not-a-space; it turns `He said “你好”` into Han quotes when no space precedes the opener. This is acknowledged and intended.
- The chosen CJK script is the Han ideographic script common to the neighbours if one exists, otherwise `Hani`.
- Unclosed openers: keep the opener on the bounded stack and resolve it by fix-up at stack overflow or paragraph end, using the left neighbour's script, else the next strong script from the memoised reverse sweep, else the paragraph default. This replaces the draft's Phase A and B. **The draft's "jitter-free" claim is withdrawn**: while typing, an unclosed opener resolves one way, and once the closer is typed the pair may re-resolve (research inference). The policy guarantees determinism for a given text, not stability under edits.

**3.3 Dashes, ellipses and middle dots.** A maximal run of U+2014, U+2026 or U+00B7 is resolved as one unit. The set is extensible (U+2015, U+2E3A, U+22EF, U+30FB are candidates) **[Design decision]**. If exactly one neighbouring non-punctuation anchor is CJK, the run takes that script; if both are CJK, Han; if both differ, the left neighbour wins **[Design decision]**; if neither is CJK, the host script, not the undefined label "Western". Only U+00B7 has Script_Extensions (including Hani); for U+2014 and U+2026 treating the run as Han is a font-selection convention, not a Unicode property `[U8]`. CLREQ requires that a Chinese ellipsis (two U+2026) and the two-em dash (two U+2014, or U+2E3A) must not be separated across lines, so the run is also an unbreakable unit in section 5 `[W5]`. CLREQ notes that Unicode 17.0 suggests U+22EF for the ellipsis `[W5]`.

### Step 4: Locale derivation

Explicit locale tags win; untagged text uses the host default under `LocalePolicy::ExplicitOnly`. The following heuristics run only under `ExperimentalHeuristics`. Parley passes the script and the language into font fallback, so locale affects font choice as well as `locl` `[S4]`. When explicitly enabled, the experimental rules are:

1. **Kana present**: Han atoms default to `ja-JP`. **[Design decision]** The scope is a run or sentence, not the whole paragraph. A Chinese sentence quoting kana, or a Japanese sentence of kanji only, defeats the paragraph-wide rule (research inference). U+30FB is not kana by Script property.
2. **Hangul present**: Han atoms default to `ko-KR`. Same scope rule; risk is low because Hanja in Korean text is correct.
3. **Serbian Cyrillic**: of the six letters U+0452, U+0458, U+0459, U+045A, U+045B, U+045F, only **U+0452 (ђ) and U+045B (ћ)** are absent from Macedonian's CLDR main exemplar set. U+0458, U+0459, U+045A and U+045F are all in Macedonian (ј љ њ џ). U+0458 also occurs in `az_Cyrl` and U+045F in Abkhaz. The Bosnian Cyrillic (`bs_Cyrl`) main set contains all six `[U9]`. The discriminators are therefore U+0452 and U+045B; the result is `sr-Cyrl` with the caveat that Bosnian Cyrillic matches too. For Macedonian, ѓ (U+0453), ќ (U+045C) and ѕ (U+0455) are the safe discriminators **[Unverified]** (not checked in CLDR). Whether Montenegrin Cyrillic adds letters is **[Unverified]**; CLDR has no `cnr` file at the paths tried.
4. Otherwise the host default locale. The engine does not toggle locales based on the frequency of Chinese ideographs. This is a **[Design decision]** with no cited source. Detecting `zh-Hant` from traditional-only characters would need Unihan variant data, which was not examined.

No primary source was found for how Chrome, Android, Gecko, WebKit, Windows or macOS derive a Han locale for unmarked text; no browser behaviour is claimed here.

### Step 5: Rich text splitting into shaping runs

Intersect contiguous (`ScriptTag`, `LocaleTag`, `BidiLevel`, `GlyphOrientation`) spans with caller style spans (family, size, weight, slant). Style variation within a cursive script (for example colour changes across an Arabic word) must not split the `ShapingRun`; colour is applied at paint time over a single shaping context **[Design decision]**. Atoms never straddle runs (this also prevents dotted circles in Indic text, which come from splitting a syllable across runs or fonts `[U2]` research inference).

### Step 6: Segmentation and box-and-glue assembly

1. **Segmenter integration.** Use stable constructors for line, grapheme and word segmentation, pin the installed data, and report actual per-component versions. Do not require unstable neo constructors or claim target-version conformance from constructor names. The crate installs none of these dependencies yet.
2. **Options.** `LineBreakOptions` offers `strictness` (Loose, Normal, Strict (default), Anywhere), `word_option` (Normal, BreakAll, KeepAll) and `content_locale` `[S6]`. There are no per-call overrides, so the engine caches one segmenter per option set. Strict resolves CJ to NS; treating CJ as ID gives CSS "normal" breaking `[U7]`.
3. **Complex scripts.** `LineSegmenter` handles Khmer, Lao, Myanmar and Thai only. `new_auto` uses the LSTM model when available; `new_dictionary` uses word lists; `new_for_non_complex_scripts` disables both `[S6]`. `LineSegmenter` does not segment Chinese or Japanese; CJ dictionary segmentation exists only in `WordSegmenter` `[S6]`. Default UAX #14 gives a break opportunity between ideographs (class ID), so no dictionary is needed for line breaking. Phrase-based breaking (CSS `word-break: auto-phrase`, shipped in Chrome 119 for Japanese using a BudouX port, page dated 2023-12-04) requires a separate model and is out of scope `[S6][W2]`.
4. **Hyphens.** An explicit hyphen (U+002D) is a UAX #14 opportunity. UAX #29 word boundaries deliberately keep hyphens out of word definitions `[U2]`; word-level navigation therefore uses a separate UAX #29 pass (section 6.5), not these nodes.
5. **Punctuation fusion.** Any CL, CP, EX, IS or SY character following a non-space, and any IN, becomes an ordinary `BoxNode::Text` with **no** `Discretionary` between it and the preceding box (2.4, rule 1); there is no `PunctuationRun`. IS carries numeric context (LB15c, LB25), so fusion applies only where the UAX #14 result already forbids the break; the segmenter's output is never contradicted `[U7]`.
6. **Glue and break nodes.** A breakable space becomes `[Glue(WordSpace, Tier0), Discretionary(penalty 0)]`; a non-breaking one only the glue (2.4, rule 2). A UAX #14 opportunity between boxes becomes a `Discretionary` and a mandatory break one with `HardBreak`. Spaces other than U+3000 become `WordSpace`; U+3000 is text (2.4). Insert `CjkAutospace` where an ideograph meets a Latin, Greek, Cyrillic or digit atom directly, with no space and no punctuation between, excluding line start and end and the neighbourhood of CJK brackets and pause marks `[W5]`. `CjkAutospace` is further suppressed on both sides of a decoration-span edge that has non-zero padding (2.2). Tabs become `Tab`; newlines a `Discretionary` with `HardBreak`. Span edges become `BoxNode::SpanBoundary`.
7. **URLs [Design decision].** UAX #14 yields few opportunities inside URLs. Typst overrides it with a link-specific rule: it detects `scheme://` or `www.`, breaks after `://`, at other-to-other and alphabetic/digit transitions, never beside `(` or `[`, and anywhere in pieces of 16 bytes or more `[L3]`. The engine offers a similar tailoring hook; the numbers are Typst's, and any adopted values must be validated.
8. **No-line-end words.** A locale-keyed table `no_line_end_words` binds listed one-letter words to the next word by emitting no `Discretionary` after them (2.4, rule 2). For Czech the set of one-letter prepositions and conjunctions is `AIiVvOoUuSsZzKk`, following the Czech Language Institute guideline as summarised by the third-party Typst package `vlna` 0.4.1 (2026-10-30) `[L8]`. It is empty for English. The draft's "A, I, a" had no support. Whether Polish follows the same convention is **[Unverified]**. The table is exported as U+00A0 semantics (GL, LB12) for CSS interoperability `[U7]`.

---

## 4. Abstract shaping bridge

### 4.1 Trait

```rust
pub trait TextShaper: Send + Sync {
    /// Unadjusted metrics of a font instance.
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics;

    fn resolve_primary_font_metrics(
        &self, families: &[FontFamily], style: FontStyle,
        script: ScriptTag, locale: &LocaleTag, font_size: f32,
        variations: &[FontVariation],
    ) -> (FontId, FontMetrics);

    /// Shapes one complete run. Output is in logical order (see 4.2).
    /// `text` is the whole paragraph text; `run.byte_range` selects the run, so
    /// cluster byte offsets are absolute paragraph indices.
    fn shape_run(
        &self, text: &str, run: &ShapingRun, atoms: &[GraphemeAtom],
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,   // appended; glyph_range indexes it absolutely
    );

    /// Re-shape a window around a break, hyphenation point or word split (5.1).
    /// `atoms` is the run-local slice given to `shape_run`; all settings come from `run`.
    fn reshape_window(
        &self, text: &str, run: &ShapingRun, atoms: &[GraphemeAtom],
        window: TextRange,
        leading_edit: LeadingEdit,   // acts on the window start only
        trailing_edit: TrailingEdit, // acts on the window end only
        output_clusters: &mut Vec<ShapedCluster>,
        output_glyphs: &mut Vec<RawShapedGlyph>,   // appended; glyph_range indexes it absolutely
    );

    /// GDEF ligature caret positions (n-1 for n components); empty if absent.
    /// Relative to the glyph origin, in layout units (the unit of `font_size`).
    fn ligature_carets(&self, instance: FontInstanceRef<'_>, glyph_id: u32, direction: Direction) -> SmallVec<[f32; 4]>;
}

pub enum LeadingEdit { None, ReplaceSuffix { replacement: Box<str>, cut_bytes: u32 } }
pub enum TrailingEdit { None, AppendHyphen { hyphen: char },
                        ReplacePrefix { replacement: Box<str>, cut_bytes: u32, hyphen: char } }
// LeadingEdit::for_opportunity(&HyphenOpportunity), TrailingEdit::for_opportunity(&HyphenOpportunity, hyphen)
```

`NoopShaper` is the non-rendering zero-metric implementation. `reshape_window` and `ligature_carets` are **[Design decisions]** that make the break-safety and caret requirements implementable. The HarfBuzz primitive behind the latter is `hb_ot_layout_get_ligature_carets`, which returns GDEF positions that may need adjustment for kerning `[B1][B7]`. **`ligature_carets` units [Design decision].** The return value is in layout coordinates, never font design units: the backend has already applied the font size and the fallback scale (effective size `font_size · font.scale`). Positions are x offsets relative to the glyph origin, increasing, in the unit of `font_size`. The engine is unit-agnostic and does not know the device pixel ratio, so "physical pixels" means "the units of `font_size`". Doing the conversion in the backend removes design-unit arithmetic from callers. **`shape_run` input [Design decision].** `text` is the whole paragraph text and `run.byte_range` selects the run, so the backend returns absolute paragraph offsets and no layer translates coordinates; the text outside the run may serve as shaping context (HarfBuzz pre- and post-context). **Single source of configuration.** `ShapingRun.features: FontFeatures` owns the OpenType features and variation axes. Both `shape_run` and `reshape_window` read family stack, size, features and variations only from `run`, with no duplicate configuration arguments, and ignore `run.atom_range` and `run.cluster_range`, which only locate the run's slices in the AST. **Cross-font limit.** OpenType contextual lookups, kerning and `chws` act only inside one physical font file. When two adjacent clusters are routed to different fonts, passing the full text does not let the backend kern or squeeze across the seam; full-width punctuation squeezing across fonts is a middle-end fallback, computed in the metrics stage (5). **Font instances.** `FontId` identifies a physical face, not a size or variation instance. `FontInstanceRef<'a> { font: FontRef, font_size: f32, variations: &'a [FontVariation] }` supplies the complete query context: `font_metrics(instance)` returns the unadjusted metrics of that instance and `ligature_carets(instance, glyph_id, direction)` returns layout-unit positions. `resolve_primary_font_metrics(families, style, script, locale, font_size, variations)` takes the same variation coordinates (4.5). `VisualGlyphRun` owns `font`, `font_size` and its variation coordinates, so a painter has the whole instance (2.6). **Tate-chu-yoko.** For a run whose orientation is `HorizontalInVertical`, `shape_run` returns exactly one cluster covering all of the run's atoms, and `reshape_window` one cluster covering the whole window, with glyphs in logical order, `natural_width` the group's advance along the vertical line, and both safety flags as for the start of the run (2.7).

**`reshape_window` contract [Design decision].** `atoms` is the original run-local atom slice; features, variations, families and size come exclusively from `run`. `window` is in paragraph text coordinates, lies within `run.byte_range`, and starts and ends on atom boundaries. Output `atom_range`s index `atoms` directly, in the same space as `shape_run` output, and the output clusters partition exactly the atoms inside the window. Ranges always describe the original text (6.6). The edit is two orthogonal one-sided enums: `LeadingEdit` is `None` or `ReplaceSuffix { replacement, cut_bytes }` and acts on the window's start only; `TrailingEdit` is `None`, `AppendHyphen { hyphen }` or `ReplacePrefix { replacement, cut_bytes, hyphen }` and acts on the window's end only. A regular line break passes one edit and `None` for the other edge; the middle fragment of a word split twice ("pro-gram-ming" on a narrow column) passes both; both `None` is a plain, symmetric, text-preserving re-shape. `ReplacePrefix` removes `cut_bytes` immediately before the break point, `ReplaceSuffix` immediately after it, and neither adds the other's hyphen. The two cuts must not overlap (their sum does not exceed the window length). A window carrying a hyphen contains at least one atom, since the hyphen attaches to its last cluster. With `cut_bytes == 0` and a non-empty replacement no cluster covers removed bytes, so the replacement glyphs belong to the window's last cluster (`ReplacePrefix`) or first cluster (`ReplaceSuffix`). A generated hyphen is appended as glyphs of the last cluster, flagged `RawShapedGlyph::is_synthesized`. The two replacing edits mirror `HyphenOpportunity` (2.4): `TrailingEdit::for_opportunity(opportunity, hyphen)` reads only `prefix_replacement` and `prefix_cut_bytes` (giving `ReplacePrefix`, or `AppendHyphen` when the prefix side is unchanged), `LeadingEdit::for_opportunity(opportunity)` reads only the suffix side (giving `ReplaceSuffix`, or `None`). Each `cut_bytes` ends on an atom boundary. `TextShaper` is `Send + Sync`: the methods take `&self`, one backend can serve several layout threads, and a backend with interior caches synchronises them itself.

### 4.2 Implementation requirements

1. **Complete run shaping, with font-run boundaries acknowledged.** `shape_run` receives the whole `ShapingRun` to preserve cross-word kerning, `calt` and `chws`. Context is lost at every font switch inside a run ; whole-run caching introduces no chunk seams (requirement 5). The draft did not acknowledge either tension.
2. **RTL handling.** Set the buffer direction to RTL when the run's level is odd. HarfBuzz mirrors during shaping: `hb_ot_rotate_chars` substitutes `unicode->mirroring(cp)` if the font has the glyph, otherwise it sets the `rtlm` mask so the feature supplies mirrored glyphs `[B1]`. UAX #9 L4 mirrors a character if and only if its resolved direction is R and `Bidi_Mirrored=Yes`; U+FD3E and U+FD3F are not mirrored `[U3]`. HarfBuzz also reverses the buffer at the end of positioning for backward directions, so an RTL result arrives in **visual** order `[B1]`; Pango compensates with explicit reversals `[B3]`. The bridge therefore reverses to **logical** order before returning, including the glyph order within a cluster. **[Design decision]** The shaper alone owns mirroring; the frontend must never substitute mirrored code points, because mirroring twice flips the glyph back. Verify with a test of an Arabic base plus mark that `x_offset` values survive the double reversal. For DirectWrite and CoreText, mirroring behaviour was not verified from documentation **[Unverified]**.
3. **Per-atom fallback.** No surveyed engine falls back inside a single shaping call. Blink detects `.notdef` per cluster and retries the next font `[B4]`; cosmic-text shapes the whole run, collects clusters with glyph 0, shapes again with the next font and keeps only matching starts `[S5]`; Parley selects a font per character cluster using whole-cluster coverage (`calculate_coverage`) `[S4]`; DirectWrite's `MapCharacters` returns a mapped length, a font (or NULL) and a scale factor `[B5]`. Requirement: select the font per atom using script and locale as the fallback key (as Parley's `FallbackKey(script, language)` does `[S4]`), require that all code points of an atom use one font (this avoids split base/mark and split ZWJ sequences), split the run into font runs, shape each, and carry `FontRef.scale`. If whole-atom coverage is exhausted, choose the best face covering the first code point and shape the entire atom with it; unsupported marks use that face's glyph 0. If even that code point is unsupported, select a deterministic final missing-glyph face. The cluster and all its glyphs remain single-font. `Hani` skips Latin-only fonts; `Zsye` routes to colour emoji fonts (design decision on the pseudo-script, see 2.1). `RawShapedGlyph` carries no font id, and `NoopShaper` only closes the interface: fallback behaviour needs tests against a real backend.
4. **Many-to-many cluster merging.** Merge glyphs by cluster value, then force merging by `atom_range`. HarfBuzz's default `MONOTONE_GRAPHEMES` level already merges marks into base clusters, but the header recommends `MONOTONE_CHARACTERS` for new clients, where marks keep their own cluster value; merging only by cluster value would then allow carets or breaks between base and mark `[B1]`. A cluster value is the start index of a group; its end comes from the next distinct value in logical order. EGC and shaped-cluster boundaries are not universally identical to typographic syllables, and backends differ in whether they merge every orthographic syllable into one cluster, so complex-script cluster behaviour must be tested per backend rather than assumed.
5. **Caching.** Cache whole ShapingRuns using text and shaping context, family stack, style, features, variations, script, locale, bidi level, direction and size. No 256-byte whitespace chunks or incremental splices in Phase 1/2. Edits trigger full single-paragraph relayout.
6. **`chws` boundary repair.** Do not store a chws-off width or perform unconditional second shaping passes. For a font with chws, reshape locally when an actual chosen break splits punctuation context; use the resulting edge widths. Fonts without chws use the metrics-stage punctuation adjustment policy. Unsafe break seams and hyphenation still require their own reshape handling. Reshape only when needed at actual boundaries, including unsafe breaks and chws punctuation seams; no fixed number of calls is required where safe seam repair needs retries.

### 4.3 Backend notes

HarfBuzz has one font per buffer and no fallback of its own (well known, not separately cited in the notes). Cluster levels (`MONOTONE_GRAPHEMES` 0, `MONOTONE_CHARACTERS` 1, `CHARACTERS` 2, `GRAPHEMES` 3) are defined in `hb-buffer.h`; the manual states that HarfBuzz is concerned with clusters, not graphemes `[B1]`. Pango sets `MONOTONE_CHARACTERS` and a language per buffer but does not request `UNSAFE_TO_CONCAT`; it re-shapes the run at a break `[B3]`. Blink uses `UNSAFE_TO_BREAK` only `[B4]`. cosmic-text and Parley treat re-shaping coarsely `[S5][S4]`. The engine can do better by following the header recipe (5.1).

### 4.4 Locale and `locl`

HarfBuzz takes the language per buffer (`hb_buffer_set_language`, used by Blink and Pango `[B3][B4]`); DirectWrite takes `localeName` in `GetGlyphs`, with the example that the same character may map to different glyphs for ja-jp versus zh-chs `[B5]`; CoreText uses `kCTLanguageAttributeName` to select localised glyphs and locale-specific line-breaking rules `[B5]`. The details of BCP 47 to OpenType language-system tag mapping were not checked **[Unverified]**.

### 4.5 Vertical metrics contract

See 5, Stage 4. `resolve_primary_font_metrics` must return a complete `FontMetrics` (2.5, "Metric validity"), vertical and baseline metrics included, for the requested variation instance: `variations` change a variable font's metrics, and the frontend stores the result unchanged as the raw strut.

### 4.6 GPUI adapter and its limits

GPUI (crate 0.2.2, 2026-10-30; source read from `main`, signatures may differ) exposes `PlatformTextSystem::layout_line(text, font_size, runs: &[FontRun]) -> LineLayout`, where `ShapedGlyph` carries `id`, `position`, `index` (a byte index) and `is_emoji`. It has no cluster spans, no break-safety flags, no bidi, no locale or language field in `Font`, `TextRun`, `FontRun` or `PlatformTextSystem`, and its comments describe RTL as a workaround for "the lack of complete UAX #9 support". Its `LineWrapper` takes break candidates from `LineSegmenter::new_for_non_complex_scripts`, so Thai, Lao, Khmer and Burmese get no dictionary segmentation. `shape_line` asserts there is no `\n` `[B6]`. GPUI also has a missing-glyph sink for clusters that exhausted fallback and tests that its emergency wrapping never splits graphemes, but there is no re-shape after the break `[B6]`.

Consequences, **[Design decision]**: (a) the adapter is a **degraded profile** (`pub enum ShaperProfile { FullContract /* default */, LineOnly }`): it reconstructs clusters from glyph byte indices, reports `safe_to_break_before = false` conservatively at ambiguous seams, cannot honour `locl`, and cannot provide ligature carets; (b) a full-contract profile bypasses `PlatformTextSystem` and calls HarfRust, CoreText or DirectWrite directly, or patches GPUI. The draft's Phase 3 plan to "pass locale tags through the GPUI bridge" is unsupported by the current API.

---

## 5. Middle-end geometric layout and alignment pipeline

`layout_paragraph(ast, text, shaper, constraints, options) -> Result<PositionedParagraph, LayoutInputError>` takes a `ParagraphAst`, the paragraph text, a `TextShaper`, the container geometry (`LayoutConstraints`) and the layout policy (`LayoutOptions`). It first runs `check_layout_inputs` (2.5) and propagates its error. The pipeline has five stages, preceded by the geometry contract below.

**Current implementation.** `layout_paragraph` is a validation-only skeleton: it returns an empty `PositionedParagraph` carrying the AST's writing mode and base direction. With a geometry provider it also runs a provider contract check: a strut probe, a conservative probe of the input that can begin at block start (up to the first break opportunity: the strut, inline objects, text-cluster heights and, under `LineHeight::Normal`, the `block_extent` of annotations whose base starts before that break), and bounded clearance progress. It never treats a later item's height as a first-line requirement. This is not the line-fitting solver, which the stages below specify but the crate does not yet implement.

### Line geometry and slots

```rust
pub struct LayoutConstraints { pub first_line_indent: f32, pub hanging_indent: f32,
    pub geometry_provider: Option<Arc<dyn LineGeometryProvider>>, pub wrap_width: f32, pub tab_stops: TabStops }
pub struct InlineSlotConstraint { pub start: f32, pub end: f32 }   // only `end` may be +infinity
pub struct LineGeometry { pub slots: SmallVec<[InlineSlotConstraint; 2]>,
    pub next_feature_block_offset: Option<f32> }
pub trait LineGeometryProvider: Debug + Send + Sync {
    fn query_line_geometry(&self, line_index: usize, block_start: f32, block_end: f32) -> LineGeometry;
}
```

**Slots [Design decision].** The inline space available to one line is a set of slots in inline-ascending order. A slot has a finite start and an end strictly beyond it; a positive-infinite end (and capacity) explicitly means no wrapping, and finite endpoint subtraction must not overflow. Slots are ascending and disjoint. Visual content coordinates, advances and dimensions always remain finite. Finite and infinite capacities never compare approximately equal.

**Default geometry.** The unobstructed width is the container `wrap_width`, before indentation (positive infinity is allowed). Both indents must be finite; without a custom provider each indent must lie in `[0, wrap_width)`, because the standard slot starts at the indent and every slot lies in the line's inline envelope `[0, wrap_width]` (negative indentation is not representable). A provider's slots are held to the same envelope: `0 <= start < end <= wrap_width` for a finite `wrap_width`, only `start >= 0` and `start < end` for an infinite one (`InvalidLineGeometry`). The default geometry is one indented slot: `first_line_indent` on line 0, `hanging_indent` afterwards.

**Provider contract.** A provider answers in canonical flow coordinates for a line index and a block envelope `[block_start, block_end)`. Every response is validated at query time (`InvalidLineGeometry`). For a fixed line index and block start, an expanded envelope may only remove space: each returned slot must lie inside a slot of the smaller envelope's response (`NonMonotonicGeometry`). An empty response to the strut probe requires a finite forward `next_feature_block_offset`; clearance progress below 1e-4 layout units, or more than 1024 attempts, fails with `InvalidLineGeometry`. An expanded probe that happens to be empty is a contract datapoint, not a rejection of the initial position, because the taller envelope is not known to occur there.

**Fitting (future).** The fitter keeps a non-decreasing envelope and never shrinks a committed line height. **Monotone available space alone does not prove that two iterations suffice**: strategy changes, slot fragmentation and reshaping can change the chosen entity set. A bounded implementation must return an explicit failure or use a specified fallback; it must not silently clip source to force convergence, and no two-iteration convergence claim is made. Multi-slot fitting must establish legal break boundaries at obstacle crossings. Slot storage is always inline-ascending while logical consumption follows the base direction. Justification fits each occupied slot independently. Whole-line bidi reordering followed by slot assignment must still prove that the reordered indivisible clusters fit their assigned slots; a logical capacity partition is not by itself such a proof. Tabs use the flow-space origin as the stop reference, and the consumption of a tab by an obstacle edge and slot transitions belong to the fitter.

**Query helpers.** `LayoutConstraints::resolve_line_geometry(line_index, block_start, block_end)` returns the provider's answer, or the default one-slot geometry; `query_checked_geometry(line_index, block_start, block_end, previous)` additionally validates it (`InvalidLineGeometry`) and, given the smaller envelope's response, checks `LineGeometry::is_subset_of(previous)` (`NonMonotonicGeometry`). `InlineSlotConstraint::width`, `is_valid` and `lies_in_envelope(wrap_width)`, and `LineGeometry::is_valid(block_start, wrap_width)`, state the slot rules above.

### Layout options

```rust
pub struct LayoutOptions {
    pub justify: bool, pub clamp: Option<LineClampPolicy>, pub strategy: LineBreakStrategy,
    pub hang_mode: HangMode, pub line_height: LineHeight,
    pub whitespace: HangingWhitespaceMode, pub soft_hyphen: SoftHyphenPolicy,
}
pub struct TabStops { pub step: f32 }                 // finite and > 0; next_stop(x), is_valid
pub enum LineBreakStrategy { Greedy, Balanced /* default */, Publication } // beam_width 1/2/8, window_depth 1/2/6
pub enum LineHeight { Normal /* default */, Fixed(f32) }   // resolve_strut(font, font_size, mode) -> StrutMetrics
pub enum HangingWhitespaceMode { Collapsible /* default */, Preserved }
pub enum SoftHyphenPolicy { Keep /* default */, Strip }
pub struct HangMode { pub start: HangStartMode, pub end: HangEndMode }   // HangMode::NONE, allows(side, dir, first)
pub enum HangStartMode { None, FirstLine, AllLines }
pub enum HangEndMode { None, Allow, Force }
pub struct LineClampPolicy { pub max_lines: Option<usize>, pub max_block_size: Option<f32>, pub ellipsis: Box<str> }
// LineClampPolicy::max_lines(n), max_block_size(h), is_valid; the default ellipsis is U+2026
```

`LayoutConstraints::is_valid` is the `InvalidConstraints` rule: `wrap_width > 0` and not NaN, finite indents, each indent in `[0, wrap_width)` unless a provider is set, valid tab stops. `LayoutConstraints::default()` has infinite `wrap_width`, zero indents and `TabStops { step: 8.0 }`.

### Stage 1: Line breaking and boundary re-shaping

**1. Overflow ladder.** Break opportunities are used in this order: (i) UAX #14 opportunities as returned by the segmenter, with the CL/CP/EX/IS/SY/IN prohibitions intact; (ii) tailored opportunities (URLs); (iii) only for a unit wider than the whole line, an emergency break. The CSS equivalents are `overflow-wrap: break-word | anywhere` and `line-break: anywhere`, which break "at otherwise disallowed points" to prevent overflow `[W1]`. If the last text box plus the boxes fused to it (closing punctuation has no `Discretionary` before it, 2.4 rule 1) does not fit on the line, the entire unit moves to the next line. **There are no emergency break points between fused boxes while the unit still fits.** Every break other than the emergency cluster break (5 below) is at a `Discretionary` (2.4). The draft's acceptance test "consecutive exclamation marks do not empty previous lines" is restated in section 7.

**2. Kinsoku and compression [Sourced].** JLReq 2.5.3: closing brackets, full stops and commas must not begin a line; opening brackets must not end a line `[W4]`. The draft covered only the first rule; the line-end prohibition for opening punctuation (UAX #14 class OP) binds an opener to the following cluster. Consecutive punctuation is normally half-width at line start and end; CLREQ 6.3.2.3 says an opening bracket at line start may have half a character trimmed before it and a full-width mark at line end shall be trimmed to half-width `[W5]`. Taiwan publications often do not adjust punctuation width, while most Mainland China and Hong Kong publications do `[W5]`. **[Design decision]** The width-adjustment behaviour is profile-selected (`zh-Hans`, `zh-Hant`, `ja`).

**3. Hanging punctuation [Sourced, new].** JLReq 2.5.1 defines hanging for full stops and commas that would otherwise have to move to a line start, placed just outside the text block `[W4]`. CSS Text 4 defines `hanging-punctuation: none | [first || [force-end | allow-end] || last]` with at most one hanging character at each edge `[W2]`. The engine offers `HangMode { start: HangStartMode, end: HangEndMode }` with `HangStartMode::{None, FirstLine, AllLines}` and `HangEndMode::{None, Allow, Force}`. `FirstLine` lets an opening mark hang only on line index 0 of the paragraph (CSS `first`; a line after a hard break is not a first line), `AllLines` on every line. The edges are logical (start and end of the line in the base direction); `PositionedLine::hanging_left` and `hanging_right` are physical. `HangMode::allows(side, base_direction, is_first_line)` is the mapping: the start edge is the left one for an LTR paragraph and the right one for an RTL paragraph, the end edge the opposite; the end edge ignores the line index. `Allow` hangs a stop or comma only if it would otherwise move to the next line; `Force` hangs it even if the line could have fitted it. Hanging extents are the maxima of `max(0, -lo)` and `max(0, hi - visual_width)` over the normalised glyph advance intervals (2.6), not full glyph advances: a hanging glyph may straddle the body edge, and a hanging flag with no overflow beyond tolerance is invalid. Inline padding or border between the punctuation and the line edge (a `SpanBoundary` of non-zero width) prevents hanging.

**4. Boundary re-shaping.**
- *Hyphenation.* A hyphenation break extracts the prefix, applies `prefix_replacement` and `prefix_cut_bytes` (`TrailingEdit::for_opportunity`), appends U+002D (or the locale's hyphenation character; CSS has `hyphenate-character` `[W2]`), and calls `reshape_window`. Following the HarfBuzz header recipe: at the end of a line walk back from the break to the first cluster start that is not unsafe-to-concat, shape from there to the end of the line, check that the result is also clear at its start, splice; otherwise step back and repeat. At the start of the next line apply the mirrored forward algorithm, with `suffix_replacement` and `suffix_cut_bytes` (`LeadingEdit::for_opportunity`) `[B1]`. The generated hyphen has `is_synthesized = true`. Pango's approach, re-shaping the whole run, is the unoptimised fallback `[B3]`.
- *Compression recovery.* Where a line break separates two punctuation clusters that were compressed by `chws`, the line-end cluster is **trimmed** (half-width) when it does not fit and the line-start opening mark is **restored** to full width, as CSS `text-spacing-trim: normal` does (full-width opening punctuation at line start; closing punctuation at line end trimmed only if it does not otherwise fit, before justification) `[W2]`. The draft's rule restored only the line-end cluster.

**5. Emergency cluster break.** If a single unit on an empty line exceeds `wrap_width`, split at positions that are **both** UAX #29 grapheme boundaries and `ShapedCluster` boundaries, because shaper clusters may be finer than graphemes at some cluster levels. If `safe_to_break_before` is false at the split, re-shape the fragments independently. The CSS "typographic character unit" is indivisible for the operation (for example Myanmar syllable clusters) `[W1]`. UAX #14 keeps ZWJ emoji sequences together, which the segmenter's example confirms `[S6]`. Emergency wrapping never splits a bridge-produced `ShapedCluster`, and an unsafe seam requires re-shaping both sides. One indivisible cluster wider than the column occupies its own overflowing line (2.6, Pass 3 (h), exemption B).

**6. One parameterised beam-lookahead line breaker [Design decision, Sourced: L1, L2, L4, W4].** The pair of strategies is the standard dichotomy (Typst `Simple`/`Optimized`, Android `SIMPLE`/`HIGH_QUALITY`, Chromium greedy plus `ScoreLineBreaker`) `[L3][L6][L4]`. This engine treats the two as the ends of a continuum.

*6.1 Single state-machine law.* The middle-end has no separate code paths for greedy and optimal breaking. One function, `break_lines_parameterized(ast, constraints, options, beam_width: usize, window_depth: usize)`, executes every strategy. `LineBreakStrategy` maps onto its parameters:

| Strategy | `beam_width` *K* | `window_depth` *W* | Allocation | Intended use |
|---|---|---|---|---|
| `Greedy` | 1 | 1 | stack only, O(1) | monospace code blocks, terminal logs, Phase 1 UAX #14 oracle tests |
| `Balanced` (**default**) | 2 | 2 | stack only, O(1) | interactive rich-text editing of long paragraphs |
| `Publication` | 8 | 6 | small bounded buffer | static reading view, print and PDF export |

(`LineBreakStrategy::beam_width` and `window_depth` implement the table. The numbers are **[Design decision]**; the 500 KB at 120 FPS target remains the restated criterion of section 7.)

*Line-emission rule.* A `PositionedLine` corresponds to one step `(a, b]` between consecutive **chosen** `Discretionary` breakpoints; every mandatory breakpoint (`penalty == -INF_PENALTY`: both kinds of `HardBreak`) is always chosen. The sentinel therefore ends the last line; a final U+2028 leaves a trailing blank line (Stage 4.2 gives the traces).

*6.2 Prefix sums and three-state width evaluation.* Before scanning, the breaker builds a prefix-sum array over the `WordNode` sequence (or evaluates incrementally in one pass), recording cumulative box and glue natural widths and cumulative per-tier stretch and shrink: `PrefixWidth[i]`, `Stretch[tier][i]`, `Shrink[tier][i]`. For a candidate line from `Discretionary` index *a* (exclusive) to *b* (inclusive):

1. `b_vis` is found by walking back from *b* over only immediately preceding glue with `is_collapsible == true` (the order is `[Box, Glue.., Discretionary]`). Under `Collapsible`, natural width and stretch / shrink are integrated over `(a, b_vis]` only; under `Preserved`, the glue in `(b_vis, b)` keeps its natural width as trailing hanging whitespace on the far edge of the base direction (UAX #9 L1) and takes no part in justification. The `Stretch[Tier2]` table gains `cjk_inter_char_max_stretch` per cluster flagged `ALLOW_CJK_JUSTIFY_AFTER`, the last visible cluster excepted; `CjkPunctCompression` glue contributes to `Shrink[Tier0]`.
2. `W(a, b) = post_break_width(a) + (PrefixWidth[b_vis] − PrefixWidth[a]) + pre_break_width(b)`. The `no_break_width` of the interior discretionaries is already in `PrefixWidth`; the boundary nodes contribute their `post_break_width` and `pre_break_width` instead of their `no_break_width`.

*6.3 Multi-tier piecewise badness.* The pure function `line_fit(slack, &TierCapacities, font_size) -> LineFit` computes it (`TierCapacities { stretch: [f32; 3], shrink: [f32; 2] }` with `primary_stretch`, `total_stretch`, `total_shrink`; `LineFit { ratio, badness, overfull, excess_slack_bits }` with `comparison_key`). Its constants are `FIT_EPSILON = 1e-3`, `CAPACITY_EPSILON = 1e-5`, `OVERFULL_BADNESS = 1_000_000`, `OVERFULL_RATIO = -2`, `UNDERFULL_BADNESS_CAP = MAX_BADNESS = 100_000`; non-finite or negative inputs are overfull. This puts the JLReq / CLREQ priority ladder (Stage 3.2) into the cost function instead of leaving it to a post-hoc justification patch. For a candidate line `(a, b)` with visible content ending at `b_vis`, the net capacities over the visible suffix after the candidate's final Tab (or `(a, b_vis]` when there is no Tab) are `Y_t = Stretch[t][b_vis] − Stretch[t][a]` for `t ∈ {0, 1, 2}` and `Z_t = Shrink[t][b_vis] − Shrink[t][a]` for `t ∈ {0, 1}` (`Z_2 = 0`: ideographs never shrink below natural width). Put `Z = Z_0 + Z_1`, `Y_01 = Y_0 + Y_1`, `Y_total = Y_01 + Y_2`, `Δ = wrap_width − W(a, b)` and `ε = 1e-3`. Five exhaustive cases (implemented as `middleend::line_fit`):

- **A. Exact fit (`|Δ| ≤ ε`).** `r = 0`, `b = 0`.
- **B. Compression (`Δ < −ε`).** If `Z ≤ 1e-5` or `−Δ > Z`, the line is overfull: `r = −2`, `b = 1 000 000` (infeasible in the standard passes, allowed only in the emergency pass). Otherwise `r = Δ / Z ∈ [−1, 0)` and `b = 100·|r|³`.
- **C. Primary expansion (`ε < Δ ≤ Y_01`).** `r = Δ / Y_01 ∈ (0, 1]` and `b = 100·r³`. Tier 2 gaps stay at zero width.
- **D. Tier 2 spill (`Y_01 < Δ ≤ Y_total`, so `Y_2 > 0`).** `Δ_2 = Δ − Y_01`, `r_2 = Δ_2 / Y_2 ∈ (0, 1]`, `r = 1 + r_2`, `b = base_D + (400 - base_D)·r_2³`, with `base_D = 100` when `Y_01 > 0`, otherwise 0; the endpoint is 400.
- **E. Underfull beyond all stretch (`Δ > Y_total`).** Let `base = 400` if `Y_2 > 0`, otherwise `100` if `Y_01 > 0`, otherwise `0`. With `r_excess = (Δ − Y_total) / max(font_size, 1)`, continue `b = min(MAX_BADNESS, base + 400·r_excess³)`, where MAX_BADNESS = 100 000. This meets the endpoint of the last available segment. At saturation compare `LineFit::comparison_key()`, the pair `(badness, excess_slack_bits)` over the bits of the finite non-negative excess: the smaller excess strictly wins. The exact-fit epsilon is an explicit numerical tolerance, not a claim of continuity at its cutoff.

Non-finite inputs give the overfull fit, never a NaN.

The step (`+100` base, `3×` the cubic slope) makes the optimiser prefer breaks that absorb slack with Western spaces and CJK autospace alone, and use inter-ideographic spacing only when unavoidable. Demerits `D(a, b)` follow from `b` and the penalty of node *b* with the Knuth–Plass equations of 6.6, including α (`double_hyphen_demerits` when both ends are `flagged`), γ (`adj_demerits` across incompatible fitness classes) and `runt_penalty` when *b* ends the paragraph and the visible line holds one word or fewer than 4 characters.

*6.4 `Balanced` execution (*K* = 2, *W* = 2).* At each line boundary the state machine keeps at most two candidate breakpoints near the right margin: `B_greedy`, the furthest feasible one (`W ≤ wrap_width + Z`, or the last low-penalty one before overflow), and `B_yield`, the immediately preceding natural (`penalty == 0`) or low-penalty one. Both live in a fixed `[State; 2]` on the stack. While the scanner crosses the next line (*W* = 2) it compares the two-line cumulative demerits `D(line_i) + D(line_{i+1})` of the paths through `B_greedy` and `B_yield`; the winner for `line_i` is committed and materialised at once.

*6.5 Dominator pruning and lazy reshape (`Balanced` and `Publication`).*

1. *Dominator commit.* When all *k ≤ K* live paths trace their ancestors at depth *d ≤ W* to the same `Discretionary` *B\**, every line up to *B\** is committed immediately.
2. *Lazy reshape.* `TextShaper::reshape_window` is **never** called while candidate edges are evaluated. Only when a line `(a*, b*)` is committed does the middle-end inspect `a*.action` and `b*.action`; if either is `Hyphenate`, or a boundary has `!safe_to_concat_before`, `reshape_window` runs once for that boundary. **Rejoin rule.** Every `Hyphenate` discretionary *inside* the committed line (not taken) whose following cluster has `!safe_to_concat_before` marks a seam that split a cluster early (2.3 note 3); the middle-end re-shapes the two neighbouring text boxes as one window with `LeadingEdit::None` and `TrailingEdit::None`, restoring the unbroken shaping. `ParagraphAst::rejoin_windows(untaken_items)` returns these windows with those of consecutive untaken seams already merged, so each atom is shaped once; `ParagraphAst::break_window(item)` gives the window for a taken break. If the exact width then overflows by more than `Z` (a rare micro-overflow), the breaker falls back to the runner-up state of the beam for that one line.
3. *Relayout scope.* Phase 1/2 always reflow the entire independent paragraph. No cached-line resynchronisation is permitted before a frontend dirty-range invalidation model exists.

*6.6 Knuth–Plass demerits (the `Publication` profile in full; the others truncate the same recurrence).* Boxes are text boxes and inline objects; glue is the tiered glue of 2.4; penalties are the `Discretionary` nodes. From Knuth and Plass (1981):

- adjustment ratio `r`; badness `b = ∞` if `r < -1` (or undefined), else `100·|r|³`;
- demerits `D = (1 + b + n)² + a` for penalty `n ≥ 0`; `D = (1 + b)² − n² + a` for `−∞ < n < 0`; `D = (1 + b)² + a` for a forced break (`n = −∞`);
- `a = α` when this break and the previous one are both flagged (hyphen); plus `γ` when adjacent lines fall in incompatible fitness classes;
- feasibility threshold `ρ` (a break is feasible only if `r ≤ ρ`); looseness `q` optional `[L1]`.

The draft's expression `(1 + 100·|r|³ + p)²` equals the paper's case `n ≥ 0` but is not the full definition. The TeX program (tex.web 3.141592653, January 2021) differs: `d = line_penalty + b`; if `|d| ≥ 10000` then `d = 100000000` else `d = d²`; `if pi > 0 then d += pi²` (outside the square) `else if pi > eject_penalty then d −= pi²`; `+ double_hyphen_demerits` (or `final_hyphen_demerits`) for consecutive flagged breaks; `+ adj_demerits` if fitness classes differ by more than 1; `inf_bad = 10000` `[L2]`. Fitness classes in TeX: very loose (`b > 99`), loose (`b > 12`), decent, tight. TeX makes up to three passes: pretolerance without hyphenation; then tolerance with hyphenation; then `emergency_stretch` `[L2]`. **[Design decision]** The engine implements the paper's form with the threshold and emergency pass, so a feasible solution always exists; badness is the tiered piecewise function of 6.3 rather than the single cubic. Default numeric parameters are **not** specified here, because plain.tex/LaTeX defaults were not retrieved **[Unverified]**. As a precedent, Typst scores `(1 + badness + penalty)²` with `badness = 100·|ratio|³`, ratio clamped to [-2, 10], an overfull line scoring 1,000,000, a runt cost of 100 and a hyphen cost of 135 with a 15% per-character edge adjustment; its source comment records that the paper's 50 over-hyphenates in Typst's model `[L3]`. These are starting points to tune, not recommendations.

*Penalties to include:* hyphen penalty and consecutive-hyphen term; last-line short-word penalty (Chromium: `kOrphansPenalty = 10000` for a one-word last line; hyphenated second-to-last lines penalised, `kNumLastHyphenatedLines = 2` `[L4]`); locale `no_line_end_words` as bonds (3, Step 6.8). The draft's single-letter Latin penalty is removed.

*Cost model and complexity.* Knuth–Plass running time is bounded by (maximum words per line) × (number of potential breakpoints); active nodes with `r < -1` are removed `[L1]`. Typst's optimised version still loops over active predecessors, pruned by an upper bound from an approximate pass and by overfull/underfull rules `[L3]`. The linear-time `textwrap` optimal-fit works because its cost `gap²` admits SMAWK; TeX's penalty, hyphen and fitness terms do not obviously satisfy the same property (research inference; not verified) `[L7]`. **[Design decision]** The engine caps the number of active nodes (Open question 6 asks for the value).

*Long paragraphs.* Bounded beam search limits candidate work per line; it does not establish a dirty-region or frame-time guarantee. Phase 1/2 use full-paragraph relayout with whole-run shaping caches. Measure full-paragraph work before making performance claims.

**7. Hanging whitespace.** Two modes **[Design decision]**: `Collapsible` (CSS `normal`-like): NBSP, Figure Space and Narrow NBSP always remain in visual_width and advance visible_byte_end; only trailing glue with `is_collapsible == true` collapses to zero visual width, are recorded in `line.byte_range`, and `visible_byte_end` marks the last visible cluster; `Preserved` (CSS `pre-wrap`-like): trailing spaces hang outside the line width but keep selectable, caret-reachable slots. CSS Text 3 says preserved trailing spaces at a soft wrap "must (unconditionally) hang" so the user can see them when "selecting or editing", while `break-spaces` does not hang `[W1]`. An editor will normally use `Preserved` on the line containing the caret. Trailing glue is the glue run immediately before the chosen `Discretionary`. **Output [Design decision].** Trailing whitespace glyphs (those whose `byte_range` starts at or after `visible_byte_end`) have zero advance under `Collapsible`. Under `Preserved` they may lie outside `[0, visual_width]`; they are exempt from the body-bounds rule, are not `is_hanging` and never count towards `hanging_left` or `hanging_right`, which describe hanging punctuation only.

### Stage 2: Per-line bidi reordering

After line breaks are fixed, reorder each line independently (UAX #9 L1 to L2):

1. Collect per-cluster levels (clusters never straddle levels; 3, Step 2.3).
2. **L1 (corrected).** On each line, reset to the paragraph embedding level: (a) segment separators (U+0009, class S) and paragraph separators (class B); (b) any sequence of whitespace characters and/or isolate formatting characters (FSI, LRI, RLI, PDI) **preceding** a segment or paragraph separator; (c) any such sequence at the end of the line. Use the **original** bidi classes, not those modified by earlier phases. U+2028 LINE SEPARATOR is class WS, not B `[U3]`. Consequence for `WordNode::Tab`: its level is always the paragraph level, and whitespace before a tab mid-line also takes the paragraph level.
3. **L2 (corrected).** "From the highest level found in the text to the lowest odd level on each line, including intermediate levels not actually present in the text, reverse any contiguous sequence of characters that are at that level or higher." The draft started at the maximum odd level. The first reversal is performed even when the highest level is even (for example level 2 inside an RTL paragraph). A level-2 cluster inside a level-1 span is not reversed internally but moves as a unit when the level-1 span is reversed `[U3]`. Glyph arrays within a cluster are reversed whenever the cluster's final reversal count is odd.
4. N0 and BD16 are not re-run per line; they were resolved per isolating run sequence in the frontend.
5. **Span fragment assembly [Design decision, Sourced: U3, C1].** `BoxNode::SpanBoundary` items take part in L2 with the `BidiLevel` of their adjacent inner atom (a leading boundary takes the first atom's level, a trailing one the last atom's). After L2 has put runs, inline boxes and boundaries in screen order, the middle-end sweeps left to right: contiguous visual items with the same `span_id` merge into one `PositionedSpanFragment` spanning from their leftmost to their rightmost edge; if L2 splits one span into two non-contiguous groups on the line, two fragments are emitted (matching the visual discontinuity of `SelectionRect`, 6.6). `is_first_fragment` marks the fragment that holds the logical start of the span, `is_last_fragment` the one that holds the logical end.

### Stage 3: Justification and tab stops

For lines other than the last or those ending in `HardBreak`, compute `slack = wrap_width − line_natural_width`.

1. **Tab stops [Design decision].** If the line contains a `Tab`, glue left of the final tab is frozen at natural width and slack is distributed only to the right of it. `TabStops::next_stop(x)` returns `Some(next)` only for finite `next > x`; exhausted precision, invalid inputs or overflow return `None`, which the breaker consumes as zero tab advance plus a forced line termination at that position, always consuming the tab so that a retry cannot loop. Candidate scoring (6.3) and materialisation freeze exactly the same capacities before the candidate's final tab, and tab advances are measured from the candidate's start, not as a paragraph-global additive width. Stops are multiples of `TabStops::step` from the flow-space origin.
2. **Priority ladder (replaces the draft's primary/secondary pools and the `1.5x` guard).** CSS Text 4 says a justification algorithm "may divide justification opportunities into different priority levels"; opportunities at one level expand or compress equally, and `text-justify` offers `auto | none | inter-word | inter-character | ruby` plus `no-compress` `[W2]`. JLReq 3.8.4 gives the expansion order for Japanese: (1) Western word spaces up to 1/2 em each; (2) the 1/4 em Japanese–Latin gap up to 1/2 em (1/3 em in some styles); (3) other non-prohibited places equally up to 1/4 em; (4) if still short, remaining non-inseparable gaps equally `[W4]`. The draft's `0.25 × font_size` constant matches step (3) but applies only after steps (1) and (2) are exhausted. **[Design decision]** The ladder is `Level0 = GlueTier::Tier0` (`WordSpace` and the punctuation pool), `Level1 = Tier1` (`CjkAutospace`), `Level2 = Tier2` (`ALLOW_CJK_JUSTIFY_AFTER` gaps), `Level3 = residual equal expansion`, with per-level capacities taken from the active profile. The same ladder drives the badness of Stage 1.6.3, so the line breaker already prefers breaks that need only the lower tiers.
3. **Compression pool (new).** JLReq 3.8.3 orders compression in six steps: Western word spaces down to 1/4 em; then the half-em after closing brackets, commas and full stops at line end is removed; then the 1/4 em around middle dots at line end; then the middle-dot 1/4 em mid-line; then the half-em before opening brackets and after closing brackets and commas mid-line; last, the 1/4 em Japanese–Latin gap to a minimum of 1/8 em. A JIS X 4051 variant reorders the middle-dot and bracket steps `[W4]`. That summary came through a summarising fetch tool, so the wording should be checked against the page. Without a compression pool, kinsoku lines are systematically over-loose; the engine breaks optimistically, then compresses. CLREQ 6.3.2.2: two adjacent marks that each take 2 em shrink to 1.5 em, and some styles to 1 em `[W5]`.
4. **Tolerance.** The `slack > capacity × 1.5` guard is removed. Use the adjustment ratio `r = slack / total_stretch` (capacity from the ladder) with a tolerance: lines with `r` above the tolerance are rejected by the `Publication` and `Balanced` profiles when an alternative exists and, under `Greedy` (or when none does), justified as far as the ladder allows and then left ragged **[Design decision]**. JLReq's fall-through (step 4) expands the remaining gaps equally instead of aborting.
5. **Cursive scripts.** `NO_LETTER_SPACING` atoms receive no inter-letter expansion `[W2]`. **Gap in the draft:** with no flags, Arabic lines can only expand at word spaces and will be loose; real Arabic typography elongates (kashida) but the research did not cover alreq, JSTF or `curs` **[Unverified]** (Open question 12). The draft's "Devanagari headlines never break under justification" remains a design requirement **[Design decision]**, not a CSS rule.
6. **Thai, Lao, Khmer, Burmese.** Keeping these in the word-space pool only is permitted by CSS `auto`, which requires only word separators and boundaries between script units as opportunities `[W2]`. Whether Thai tradition prefers inter-cluster expansion is **[Unverified]**.

### Stage 4: Strut metrics, blank lines, vertical clipping

1. **Strut.** The frontend records the raw primary-font metrics (`ParagraphAst::strut_metrics`, `strut_font_size`) and never reads the line-height policy; the middle-end resolves the active strut from `LayoutOptions::line_height` with `LineHeight::resolve_strut` (`Normal`: the font's own ascent, descent and line gap; `Fixed(m)`: height `m · font_size`, half-leading folded in, gap 0). CSS Inline Layout 3: when `line-height` is `normal`, an inline box's layout bounds enclose all its glyphs, even from different fonts. When it is not `normal`, the bounds derive solely from the metrics of the first available font, ignoring fallback glyphs, and leading `L = line-height − (A + D)` splits as half-leading above and below. A box with only fallback glyphs behaves as if it contained a strut with the first font's metrics. The font's line gap enters only when `line-height` is `normal` `[B7]` (CSS Inline 3 §5.3). The draft's fixed-strut model matches CSS for non-`normal` line-height; for the default case the engine offers `LineHeight::Normal` (union of glyph bounds) and `LineHeight::Fixed(multiplier)`. Fixed-strut layouts can let fallback glyphs overflow the line box vertically; this is standard CSS behaviour, not a bug. **Resolved strut [Design decision].** `ParagraphAst::strut_metrics` stores the **raw** primary-font metrics. The active strut is the output of `LineHeight::resolve_strut` applied by the middle-end: `LineHeight::Normal` keeps the font's ascent, descent and line gap; `LineHeight::Fixed(m)` gives total height `m · font_size`, splits the leading `total − (ascent + descent)` (possibly negative) equally above and below, and sets the line gap to 0 (`LineHeight::resolve_strut`). An empty line and every line under `Fixed` has `strut.ascent + strut.descent + strut.line_gap` as its height; under `Normal` a line may be taller but not shorter. In a vertical mode `LineHeight::resolve_strut` reads the vertical raw metrics (`vertical_ascent`, `vertical_descent`, `vertical_line_gap`) as ascent, descent and gap. A fixed height splits the leading equally, negative leading included, and never mutates the raw metrics. Font sizes and fixed multipliers must be finite and strictly positive and the resolved total line height positive; negative half-leading may yield a negative effective ascent or descent, which alone is valid.
2. **Blank lines.** Hard breaks or consecutive newlines create empty `PositionedLine`s with an empty visible range (`visible_byte_end == byte_range.start`; `byte_range` is empty at the paragraph-end sentinel and covers the newline terminator for an interior blank line), the resolved strut as height and no visual runs, so caret placement and hit-testing work. This follows from the **line-emission rule** (5 Stage 1.6): every `PositionedLine` is one step `(a, b]` between consecutive chosen breakpoints, the first line starting at the paragraph start and line *k* at `b_{k-1}.byte_range.end`; every mandatory breakpoint, the explicit `HardBreak` and the final sentinel alike, must be chosen as a line's terminating breakpoint. *N* newlines therefore yield exactly *N* + 1 lines and a trailing newline yields one trailing blank line, never two. Traces: `""` gives one line `0..0`; `"abc"` one line `0..3` with `visible_byte_end = 3`; `"abc\n"` the line `0..4` (`visible_byte_end = 3`) and the blank line `4..4`; `"a\n\nb"` the lines `0..2` (visible 1), `2..3` (empty, visible 2) and `3..4`.
3. **Mark positioning.** Preserve shaper `y_offset` exactly, with no ±1.5-line-height clamp. Rendering may clip overflow without changing glyph attachment positions.
4. **Annotation room [Design decision].** An annotation's room is measured from its base, not from the strut: a side needs `base_over + block_extent` (`Over`) or `base_under + block_extent` (`Under`), where `base_over` and `base_under` are the largest extents of the annotated base's glyphs and inline boxes toward line-over and line-under. Under `LineHeight::Normal` the line grows its ascent (descent) to the larger of the strut's and that requirement, and the annotation sits at `baseline_offset >= base_over` (`<= -base_under`), so it never overlaps its base. Under `LineHeight::Fixed` the line box stays locked to the strut, with ascent, descent and height unchanged, and the annotation overflows into the inter-line space.

### Stage 5: Decorations

Underline and strike-through merge across adjacent clusters and synthetic glue. **[Design decision, unverified against CSS Text Decoration]** Underlines use the lowest `underline_position` among intersecting fonts so lines do not cut CJK stroke bases. The CSS Text Decoration properties (`text-underline-position`, `text-decoration-skip-ink`, `text-underline-offset`) were not examined.

### Line clamping and truncation

`LayoutOptions::clamp` (`LineClampPolicy { max_lines, max_block_size, ellipsis }`) truncates a paragraph. When enabled, at least one limit is set, `max_lines` is positive, `max_block_size` is positive and finite, and the ellipsis contains no line or paragraph separator; an empty ellipsis means silent truncation. Otherwise validation fails with `InvalidLayoutOptions`. **Future behaviour [Design decision].** Truncation respects cluster, tate-chu-yoko, atomic-edit and ruby boundaries; a multi-glyph suffix is one synthesized sequence (every glyph `is_synthesized`, without a byte range). At least one line is retained even when the block limit is smaller than that line. The line count includes the empty lines produced by hard breaks but not clearance jumps. In `VerticalLr` the block-limit measurement uses the same mode-correct line bounds as validation (2.7). A clamped paragraph reports the unrendered logical tail as `truncated_range`, ends with exactly one ellipsized line (the last), and may need the output exceptions of 2.6 (synthesized-only ellipsis, empty base).

---

## 6. Caret navigation, hit testing and selection geometry

### 6.1 Caret representation

```rust
pub struct CursorPosition { pub byte_offset: ByteIndex, pub affinity: CaretAffinity }
pub enum CaretAffinity { Downstream, Upstream }

pub struct CaretStop {            // one per visual boundary on a line, in visual order
    pub x: f32, pub offset: ByteIndex, pub affinity: CaretAffinity,
    pub level: BidiLevel, pub line: usize,
}
```

The (offset, affinity) pair is the Flutter and Blink shape; Parley and cosmic-text use the same pair with different names `[C1]`. Flutter's definition: upstream leans toward the start of the string; at a soft wrap, upstream is at the end of the first line and downstream at the start of the second; affinity also resolves a bidi boundary; explicit newlines are unambiguous `[C1]`.

**Invariant (revised).** A caret position's `byte_offset` aligns with a grapheme atom boundary. **Deletion ranges** do not: `delete_backward_range(offset) -> TextRange` may land on a code point boundary inside an atom (6.5). The draft's single invariant was right for carets only.

**Visual stop table.** One `CaretStop` per visual boundary, with two stops at each direction boundary, lets `hit_test`, `caret_geometry`, `move_cursor` and `selection_geometry` share one structure. This is an inference from the Pango `(index, trailing)` and Parley cluster-boundary APIs, not a documented implementation `[C1]`.

**Atomic edit spans [Design decision].** When `PositionedLine::caret_stops` is built, any atom boundary strictly **inside** an `AtomicEditSpan` (`span.byte_range.start < offset < span.byte_range.end`) produces no `CaretStop`, even if the span wraps across lines or holds several words or ligatures. Only `span.byte_range.start` and `span.byte_range.end` generate stops. Consequently no caret can sit at the soft wrap inside a wrapped chip. (`AtomicEditSpan::is_interior` is the predicate.)

### 6.2 Visual slots and ligature carets

1. A cluster with one atom, or a cluster flagged `IS_EMOJI_SEQUENCE`, forms one slot. A Devanagari conjunct is one atom under GB9c and so one slot automatically; U+0E33 SARA AM is one code point `[U2]`.
2. A cluster with K > 1 atoms (a ligature) divides into K sub-slots. **Source of the divisions:** accept exactly K−1 finite, strictly increasing positions inside `(0, cluster.natural_width)`; otherwise divide the width equally into K slots. `resolve_ligature_carets(width, atom_count, carets)` exposes this operation for tests. Query the font instance's GDEF `LigCaretList` through `TextShaper::ligature_carets` (`caretCount` is components minus one, positions increasing; the example "ffi" has 2 carets `[B7]`); otherwise equal division, as Pango does when several characters share a cluster `[C1]`. The positions are unshaped and may need kerning adjustment `[B1]`. The draft's restriction to Latin-like bases is removed: no source supports it. In RTL the sub-slots are laid out right to left in logical order. The consequence of the draft's exclusion for Arabic lam-alef (two atoms, one ligature cluster) would be an unreachable caret between lam and alef; whether engines subdivide Arabic ligatures was not verified **[Unverified]**.
3. Hanging whitespace follows the Stage 1 mode. `Collapsible` trailing whitespace has no slot of its own (zero width); a click in the region past the last visible content is handled by 6.3 rule 3. `Preserved` spaces keep slots, placed on the far edge of the base direction (UAX #9 L1).

### 6.3 Hit testing

```rust
pub struct HitResult { pub position: CursorPosition, pub inside: bool }
fn hit_test(&self, point: Point) -> HitResult;
```

1. Clamp `point.y` to the first or last line; `inside = false` if clamped. Binary search lines by `y`.
2. Within a line, find the nearest caret stop: for a slot of width `w` the midpoint rule gives the leading or trailing side by level (even: left half to `start`, right half to `end`; odd: left half to `end`, right half to `start`), using half-glue margins at slot edges (a draft-specific choice, harmless). Android's `getOffsetForHorizontal` returns the closest offset in the same way `[C1]`.
3. **Beyond a line end (corrected).** The draft returned `(visible_byte_end, Upstream)` for points past the right edge. For RTL paragraphs the logical end is at the left edge, and on mixed lines the visual end edge maps to the extreme caret stop, not necessarily `visible_byte_end` (an LTR paragraph ending in an RTL run has its right edge at the start of that run). The rule is: a point past the visual end edge returns the extreme visual stop; past the visual start edge, the opposite extreme. **L1 geometry [Sourced].** UAX #9 L1 resets trailing whitespace to the paragraph embedding level, so it is laid out at the far edge of the *base* direction: rightmost in an LTR paragraph, leftmost in an RTL one. The "visual end edge" is therefore the edge of the last visible content on that side (for `Collapsible`, where the whitespace has no width), or of the preserved whitespace (for `Preserved`). Only when the content at that edge has the paragraph's base direction does the extreme stop coincide with `(visible_byte_end, Upstream)`; a line ending in an opposite-direction run does not, because that run's logical end lies at its far side. **Convergence principle [Design decision].** A click beyond the physical boundary of the line's visual content converges on the extreme stop in that physical direction among all caret stops of the line (`PositionedLine::extreme_caret_stop`; ties between equal-`x` stops resolve by visual order). An unconditional jump to `visible_byte_end`, which would move the caret to the middle of the line, is forbidden. DirectWrite also clamps to the nearest edge with `isInside = FALSE` `[C1]`.
4. A click on the second half of the last visible grapheme of a wrapped line yields `(visible_byte_end, Upstream)`; a click before the first grapheme of the next line yields `(offset, Downstream)` (the Flutter convention) `[C1]`.
5. **Atomic spans.** Hit testing reads `PositionedLine.atomic_fragments` directly, never decoration ids. Each `AtomicFragment` stores `atomic_id`, `full_byte_range`, `fragment_byte_range`, the physical left edge `x`, `width`, and `bidi_level`. Split at the clicked fragment midpoint: even levels map physical left to full start (Downstream), right to full end (Upstream); odd levels swap. A midpoint tie belongs to the physical right half. Wrapped fragment boundaries never become interior atomic caret stops.

### 6.4 Caret geometry

```rust
pub struct CaretVisual { pub primary_rect: Rect, pub secondary_rect: Option<Rect>, pub slant_angle: f32 }
```

**Corrected definition.** Pango and Android define the two carets by insertion direction relative to the paragraph's base direction: the strong (primary) caret is where a character of the base direction would be inserted, the weak (secondary) where a character of the opposite direction would `[C1]`. Parley and Flutter instead use affinity to pick one location `[C1]`. **[Design decision]** This engine takes `primary_rect` as the base-direction insertion point; affinity selects the line and side at a soft wrap or bidi boundary; `secondary_rect` is `Some` only if its x position **differs** from the primary's, because adjacent slots at levels 0 and 2 inside an LTR paragraph differ in level but coincide visually (research inference).

### 6.5 Movement and editing

```rust
pub enum Motion {
    Left, Right,                 // physical, visual
    Previous, Next,              // logical
    WordLeft, WordRight, WordPrevious, WordNext,
    LineStart, LineEnd, Up, Down,
}
pub struct Selection { pub anchor: CursorPosition, pub focus: CursorPosition }
```

1. **Backspace and Delete are logical, and their granularity is not the atom's.** Chromium's backspace state machine deletes one code point by default, with no special case for combining marks or Indic vowel signs; it removes CRLF as a unit, keycap sequences whole, emoji modifier, variation selector and ZWJ sequences together, one flag (a regional-indicator pair) at a time, and tag sequences with their base `[C3]` (summarised by a fetch tool; verify line-level detail). **[Design decision]** The engine follows this: delete the last code point of the atom, except for atoms flagged `IS_EMOJI_SEQUENCE`, `HAS_VARIATION_SELECTOR`, CRLF and tag sequences, which are removed whole; the policy is a flag for platform variance. Forward Delete needs its own forward rule. The draft covered only Backspace and treated it as atom stepping. **Atomic spans:** `delete_backward_range(offset)` (Backspace) and `delete_forward_range(offset)` (Delete) return the entire `AtomicEditSpan::byte_range` when the atom immediately before (Backspace) or after (Delete) `offset` carries `AtomFlags::IN_ATOMIC_EDIT_SPAN`; an atomic span joins the whole-unit removal list above.
2. **Left and Right are visual.** They move across the line's caret stops (so one keystroke crosses a whole `AtomicEditSpan`, 6.1); stepping past a line end skips collapsed hanging whitespace to the start of the neighbouring visual line. The cursor "may jump when it moves off the end of a run" in bidi text `[C1]`.
3. **Shift+arrow (corrected).** Firefox's default (`bidi.edit.caret_movement_style = 2`: visual movement, logical while selecting) produced reports of Ctrl+Shift+Right selecting words to the left (Bug 1198600, status unconfirmed, metadata 2022-10-11); Bug 1138419 shows Mozilla moving to visual selection (default changed in Nightly in 2020 except on Linux; fixed in Firefox 152 for GTK; macOS in bug 1425483) `[C2]`. One commenter claimed Safari always uses visual selection; that is hearsay. **[Design decision]** Shift+arrow extends the **focus** visually in the direction pressed; the selection is stored logically as anchor and focus; Backspace and Delete remain logical. The `Motion` enum separates physical `Left/Right` from logical `Previous/Next` because Firefox's word commands mapped to the wrong direction in RTL `[C2]`.
4. **Word movement uses UAX #29 word boundaries** (a separate pass, `WordSegmenter` semantics), not `BoxNode::Text`. `WordNode`s are line-breaking units: hyphen-split compounds, CJK per-ideograph breaks and fused punctuation boxes would produce wrong stops `[U7][U2]`. `WordSegmenter` is compatible with UAX #29 version 18.0.0 `[S6]`.
5. **Vertical movement.** Preserve `preferred_x` and sample the target line through `hit_test`, as Android's `getLineForVertical` plus `getOffsetForHorizontal` allow `[C1]`. Parley's `Cursor` stores no preferred x, so the caller holds it `[C1]`. Rules **[Design decision]**: reset `preferred_x` after any horizontal move or edit; `Up` on the first line goes to the text start and `Down` on the last line to the text end; `preferred_x` lives in `hit_test`'s coordinate space and is clamped by its edge semantics.

### 6.6 Selection geometry

```rust
pub struct SelectionRect { pub rect: Rect, pub line: usize, pub level: BidiLevel,
                           pub is_line_end_extension: bool }
fn selection_geometry(&self, range: TextRange, height: BoxHeightStyle) -> Vec<SelectionRect>;
pub enum BoxHeightStyle { Tight, Max, Strut }   // after Flutter's BoxHeightStyle [C1]
```

Selection is stored logically and rendered as visual pieces; Android's `getSelectionPath` returns "a potentially discontinuous set of rectangles" for a logical range and an empty path for an empty range `[C1]`. The merge step runs over **visual order** and joins only visually adjacent selected slots (and the glue margins between two selected slots); it never bridges a gap of unselected slots. A margin at a selection edge is split at the slot midpoint or caret stop. A selected newline optionally adds an extension rectangle (`is_line_end_extension`); whether engines do so was not verified **[Unverified]**. `Tight` is the union of selected actual font-instance ascent/descent queried through `font_metrics`, excluding line_gap and half-leading. `Max` includes line_gap and maximum inline-box height. `Strut` uses the resolved paragraph strut. No glyph ink bounds query is required.

**Clipboard.** Copying reads the raw UTF-8 text, so the generated hyphen never appears; CSS Text 3 states that hyphenation "is a rendering effect only" and must not change content, selection or search results `[W1]`. An actual U+00AD in the source is **not** synthesised and would be copied; WebKit once converted it to a space in extracted text and stopped (Bug 27523, noted 2010-11-10), and Firefox kept 0xAD (2009) `[C4]`. **[Design decision]** The engine states a policy flag `copy_soft_hyphen: Keep | Strip`, default `Keep`. A synthesised hyphen slot has no caret stop; caret and hit-test slots inside a rebuilt prefix (spelling replacements) still map to the original bytes.

### 6.7 Platform text-input and accessibility interfaces (new)

The draft exposed only four queries. **[Design decision]** Add: `range_rects(range) -> Vec<Rect>` (IME candidate placement; `NSTextInputClient.firstRect(forCharacterRange:actualRange:)` returns the first logical boundary rectangle `[C5]`), `rect_for_offset`, `offset_for_point` (= `hit_test`), an independent `CompositionSpan` (2.2) that does not change EGC segmentation, UTF-16 conversion (2.1), and AccessKit-style per-line exports of `character_lengths`, `character_positions`, `character_widths` and `word_starts`, with `TextPosition { node, character_index }` `[C5]`. Whether AccessKit lengths are UTF-8 bytes was not retrieved **[Unverified]**; Windows TSF, Android `InputConnection` and iOS `UITextInput` were not examined.

### 6.8 Writing modes, slots and truncation

The crate defines the data types of this section (`CursorPosition`, `HitResult`, `CaretVisual`, `Motion`, `Selection`, `SelectionRect`, `BoxHeightStyle`, `FlowMotion`) and three pure helpers (`AtomicFragment::hit_test`, `Motion::flow_direction`, `resolve_ligature_carets`); the queries of 6.3 to 6.6 are future work. **Contract for the queries [Design decision].**

- Physical arrow requests resolve once, at the interaction boundary, through `Motion::flow_direction(mode)` to inline previous or next, or block previous or next (`pub enum FlowMotion { InlinePrev, InlineNext, BlockPrev, BlockNext }`). Logical (`Previous`, `Next`) and word requests stay distinct and resolve to None there.
- Queries transform at their physical boundary (2.7) and then work in flow space over non-overlapping flow line boxes.
- A hit on an obstacle between slots snaps to the nearer caret stop, ties toward positive inline.
- Selections are split by slot (6.6).
- The preferred inline position is carried across block motion (6.5 rule 5).
- Ruby hits map to the base text; an annotation is never a caret target.
- Ellipsis hits, and caret requests into the hidden tail, map upstream to the truncation boundary (`truncated_range.start`).
- Tate-chu-yoko text is atomic for editing only when independently covered by an `AtomicEditSpan` (2.7).

---

## 7. Phased implementation roadmap and acceptance criteria

All phases share the data structures above. Every phase must also pass the Unicode conformance data for its components, run against the pinned data version (1.1): `GraphemeBreakTest.txt`, `WordBreakTest.txt`, `LineBreakTest.txt`, `BidiTest.txt` and `BidiCharacterTest.txt` **[Design decision; the test files themselves were not fetched]**.

### Phase 0 to Phase 1 transition checklist

The structural updates below were applied to the Phase 0 workspace **before** the Phase 1 segmenter and shaper are written; they are implemented and tested. The data model, validators and contract tests of section 2 are the Phase 0 deliverable; no segmenter, real shaper, line breaker or editor query exists yet, and source compatibility across field additions is not promised (2.5).

1. `x_offset: f32` added to `PositionedGlyph`; `PositionedLine::validate` checks the pen interval `[x, x + advance]`.
2. `WordNode` refactored into `Box`, `Glue`, `Discretionary`, `Tab` with `GlueTier`, `BoxOverflowPolicy` and `BreakAction`; `PunctBind` removed.
3. `InlineDecorationSpan`, `AtomicEditSpan` and `PositionedSpanFragment` added; the fragment containment check is part of `PositionedLine::validate`; `check_layout_inputs` validates both span streams and every new node.
4. `AtomFlags::IN_ATOMIC_EDIT_SPAN` added, aggregated by OR in `AtomFlags::for_cluster`.
5. `LineBreakStrategy` is `Greedy | Balanced | Publication`, `Balanced` the `Default`, with `beam_width()` and `window_depth()`.

**Verification.** The contract tests (`tests/contracts/`, one test binary) cover writing modes and orientation, font metrics, explicit spacing, bidi spans, annotations, slots, line geometry, clamping and truncation, storage and error messages, plus empty and synthesized-only clamping, infinity mismatch, malformed provider intervals, absent clearance progress, glyph-less annotation bases, late tall input outside the initial probe, and asymmetric `VerticalLr` paragraph bounds (including an under-side annotation). The manual release-mode span-scaling benchmarks are `#[ignore]`d and run explicitly.

### Phase 1: Foundational core and correctness closure

**Scope.** Data structures (section 2); Frontend Steps 1 to 6 (EGC with GB9c/InCB, two pairing tables, apostrophe filter, script resolution with a bounded stack and Script_Extensions, optional CJK quote policy, conservative locale derivation, stable `icu_segmenter` integration with actual-version reporting and offset mapping, punctuation fusion, URL tailoring hook, span boundaries and padding-aware autospace suppression); `TextShaper` trait with the HarfRust backend and the degraded GPUI profile (4.6); many-to-many cluster aggregation using `atom_range`; the `Greedy` profile of the parameterised breaker (6.1) with the overflow ladder, kinsoku and emergency cluster splitting; atomic-span caret suppression, hit-test snapping and whole-span deletion; strut metrics; unidirectional caret, hit-test and selection queries including `HitResult`.

**Acceptance criteria (revised).**
- Devanagari conjuncts (for example क्ष) are one atom under Unicode 15.1 or later data and are shaped within one run and one font, so no dotted circle appears. A negative control with pre-15.1 data splits them.
- Thai, Lao, Khmer and Burmese break at model boundaries (LSTM via `new_auto`, or dictionary via `new_dictionary`); the test records which was used.
- `high-quality` breaks after the hyphen only.
- A run of N `!` wider than the column wraps at cluster boundaries only after it reaches an empty line, and no non-empty line begins with a CL, CP, EX, IS or SY character unless produced by the overflow fallback. (This replaces the ill-defined "do not empty previous lines".)
- CJK quotation marks, ellipses and dashes use the intended script and are not split across lines; centring or trailing-edge placement follows the locale profile (Taiwan and Hong Kong centre punctuation; Mainland China places most marks toward the trailing edge `[W5]`), not a script tag.
- Emoji sequences (ZWJ, keycap, flag, tag, modifier) move the caret atomically; Backspace removes them whole while a combining mark is removed alone.
- Long URLs wrap without overflowing; an unbreakable unit wider than the line splits only at grapheme and cluster boundaries.
- `conformance_report()` returns `ConformanceReport { grapheme, bidi, line_break, word: Option<&'static str> }` naming actual per-component data versions; Phase 0 reports None. Production builds do not require unstable constructors.
- A mention chip (`AtomicEditSpan`) yields no caret stop inside it, a click inside it snaps to its start or end, and Backspace and Delete remove all of it, also when it wraps across lines.

### Phase 2: Bidirectional text, autospacing and justification

**Scope.** Bridge RTL shaping with single-owner mirroring and reversal to logical order; Stage 2 with corrected L1 and L2; split carets and the caret-stop table; visual arrow navigation; Shift+arrow visual extension with anchor/focus; `CjkAutospace` with the `Jlreq` and `CssCompat` profiles, suppressed beside padded span edges; `PositionedSpanFragment` assembly after L2 with bidi-split spans; `ALLOW_CJK_JUSTIFY_AFTER`; Stage 3 with the priority ladder, compression pool, tolerance and tab freezing; hanging punctuation modes; inline object layout with `BoxOverflowPolicy`; continuous underline decoration.

**Acceptance criteria.**
- Arabic and Hebrew lines order correctly across line wraps, including a level-2 run inside an RTL paragraph (L2 test), whitespace before a mid-line tab (L1 test) and trailing whitespace.
- Nested brackets mirror correctly under BD16/N0; guillemets in an Arabic paragraph resolve as neutrals; a 64-deep opener nest behaves as BD16 specifies.
- CJK and Latin mixed runs autospace per the active profile and justify flush with the margins using the ladder; closing marks never start a line and opening marks never end one.
- Over-loose lines are handled by the tolerance rule, not by the removed `1.5x` guard.
- A `BoxDecorationBreak::Clone` chip wrapped over two lines has full padding on both fragments, `Slice` only at the true ends, and a span split by L2 yields two fragments.
- Cursive scripts receive no letter-spacing; Devanagari headline continuity is checked as a design requirement.
- Clicking beyond either end of an RTL line, a mixed line, and above and below the text returns the specified stops with the right `inside` flag.

### Phase 3: Global optimisation, hyphenation and performance hardening

**Scope.** Hyphenation with a `HyphenOpportunity` source behind an adapter: `hypher` 0.1.8 (2026-09-17) for standard Liang-pattern hyphenation (embedded automata; `hyphenate_bounded` for minimum chars each side; no allocations for words up to roughly 41 to 45 bytes, as the README and docs.rs disagree; 48 languages per README versus "more than 30" per docs.rs `[H1]`), and libhyphen-style or `hyphenation` crate extended dictionaries for non-standard hyphenation (Catalan, Dutch, Hungarian, Swedish, pre-reform German). The `hyphenation` crate 0.8.4 (2021-08-19) has had no release since `[H2]`. The libhyphen API returns `rep`, `pos` and `cut` arrays (for example `rep[5]="ff=f", pos[5]=1, cut[5]=2`) and its dictionary syntax is `pattern/change,start,cut` `[H3]`. **German ck → k-k** is the pre-1996 rule and Dutch, Catalan, Hungarian and Swedish give valid modern cases (`omaatje` → `oma-tje`, `paral·lel` → `paral-lel`, `asszonnyal` → `asz-szony-nyal`, `tillata` → `till-lata`) `[H3]`. UAX #14 §5.4 notes that with a soft hyphen the word's spelling is the unhyphenated one (for example tug<SHY>gummi) and gives pre-reform "Drucker" → "Druk- / ker" as an example `[U7]`. Also: prefix re-shaping through `reshape_window`; synthesised hyphens; the `Balanced` and `Publication` profiles (section 5, Stage 1.6) with thresholds, passes and the active-node cap; lazy `reshape_window` at commit time; dominator pruning and full-paragraph relayout (Stage 1.6.5); Latin ligature carets via GDEF; whole-ShapingRun caches; locale propagation for `locl` through the full-contract shaper profile (not the GPUI adapter).

**Acceptance criteria (revised).**
- Narrow-column English text and German text hyphenate accurately with balanced line densities, honouring minimum characters before and after the hyphen (CSS `hyphenate-limit-chars` and `hyphenate-limit-lines` are the reference vocabulary `[W2]`), and nothing is hyphenated in a word that contains U+00AD.
- Clipboard copy contains no synthesised hyphen; U+00AD follows the `copy_soft_hyphen` policy.
- CJK localised variants (`zh-Hans`, `zh-Hant`, `ja`, `ko`) resolve correctly via `locl` **under the full-contract shaper profile**.
- Spelling-change hyphenation produces the replacement forms in a Dutch, a Catalan and a Hungarian test.
- Candidate evaluation never calls `reshape_window`; it is called once per committed hyphenated boundary, and a micro-overflow falls back to the runner-up path.
- **Performance:** benchmark full single-paragraph relayout. No dirty-region or 120 FPS guarantee is made before an invalidation model and measurements exist.

---

## Open questions and unverified items

Drawn from the Gaps sections of the research notes. Question numbers are stable; resolved questions are folded into the body, so the list has gaps.

1. **Segmenter data.** Choose stable interfaces during integration, pin their data versions, and run matching conformance suites. No unstable neo requirement remains.
2. **Variation selectors.** Standardized Variation Sequences (`StandardizedVariants.txt`) and Mongolian free variation selectors were not fetched. `emoji-sequences.txt` was not read, so RGI counts per category are unknown.
3. **UAX #9 details not read.** The X1 to X10 depth limit text (maximum depth 125 is background knowledge), the `BidiCharacterTest.txt` data, and the Unicode version that introduced BD14 to BD16. The conformance version of the `unicode-bidi` crate (0.3.18, 2024-12-16) was not checked.
4. **Mirroring in DirectWrite and CoreText**, and CoreText `kCTRunStatusRightToLeft` semantics, were not verified. Whether `hb_buffer_reverse_clusters` is the right way to restore logical order was not researched.
5. **Itemiser behaviour.** ICU's `PAREN_STACK_DEPTH` value was not found; Skia, WebKit, Gecko and swash itemisation were not read. No engine source describes apostrophe-versus-quote disambiguation in script itemisers.
6. **Active-node cap** for Knuth–Plass, default penalties, tolerance values and fitness-class constants: plain.tex/LaTeX defaults and the Liang 1983 dissertation were not retrieved, so no numeric defaults are given. Time complexity of ICU4X dictionary and LSTM segmentation has no primary statement.
7. **Incremental relayout. Deferred.** Phase 1/2 reflow full paragraphs. A frontend invalidation model is required before revisiting local resynchronisation.
8. **Hyphenation.** Primary source for the pre- and post-1996 German `ck` rules (Rat für deutsche Rechtschreibung) was not read. `hypher`'s `Lang` variants and German pattern edition, and its handling of Dutch or Hungarian cases, were not tested. Soft-hyphen copy behaviour in current Chrome is unverified. `hyphenate-character` and `hyphens: manual` text were not extracted.
9. **Czech, Slovak and Polish line-end rules.** The primary Czech Language Institute text was not read; Polish is unconfirmed.
10. **Locale derivation.** No primary source for how browsers or operating systems derive a Han locale for unmarked text; Macedonian discriminators (ѓ, ќ, ѕ) and Montenegrin Cyrillic are unchecked; `locl` language-system mappings (JAN, KOR, ZHS, ZHT, ZHH) are background knowledge.
11. **CJK typography.** JLReq Appendix A class lists, CLREQ 6.1.1 prohibition tables, KLREQ, iclreq, alreq and thai-lreq were not reached; whether Hangul participates in autospace is unchecked; the CSS Text 4 character classes for `ideograph-alpha` were not read; browser support figures for `text-autospace` came from MDN's compatibility data, and the Chrome 140 entry looked inconsistent with Chrome lacking `ideograph-alpha`, so re-check on chromestatus.com `[W7]`. Typst's stretch and shrink limits and `cjk-latin-spacing` behaviour in the `typst-layout` source were not read for this topic (the line-breaking file was).
12. **Arabic justification.** Kashida rules (alreq), `JSTF` and `curs` were not researched.
13. **Indic letter-spacing** (iclreq) and the Devanagari headline requirement are unverified; Thai inter-cluster expansion tradition is unverified.
14. **`chws`.** The HarfBuzz release introducing `chws` support, font coverage (Noto CJK, Source Han) and Chromium's implementation (restore versus re-shape) were not verified; the 66k characters of HarfBuzz NEWS beyond the first 100k were unread for this feature.
15. **Fallback and metrics.** CoreText cascade lists, Skia `SkShaper::FontRunIterator`, Pango fallback, Blink `FontFallbackIterator`, CSS `unicode-range`, `fontique`'s fallback API, OpenType hhea versus OS/2 win metric precedence per platform, and CSS Text Decoration rules were not retrieved. The Blink word-cache claim could not be re-verified because the older file paths returned 404.
16. **Carets and selection.** Blink/WebKit `TextAffinity` source semantics, Parley's `Affinity` variants and `Selection::geometry`, CoreText `CTLineGetOffsetForStringIndex`, `NSTextView` affinity, DirectWrite `HitTestTextRange`, CSSOM `getClientRects`, Chromium forward/backward grapheme state machines, Android and Apple editing granularity, Chrome/Safari/Word Shift+arrow behaviour, and Arabic lam-alef subdivision in real engines were not fetched.
17. **Platform input and accessibility.** Windows TSF, Android `InputConnection`, iOS `UITextInput`, `NSTextInputClient.characterIndexForPoint` and AccessKit's offset units are unexamined; the AccessKit version appears as 0.25.2 on one page and 0.25.0 on another.
18. **GPUI.** Per-platform fallback implementation (CoreText, DirectWrite or cosmic-text on Linux) is unconfirmed; the crate release (0.2.2, 2026-10-30) may differ from `main`. The Servo, Blitz, Vello and Xilem text stacks were not researched.
19. **Status banners.** Unicode 18.0 pages carry draft or preliminary banners (UAX #29, the versions page); UTS #37 rev 16 is a proposed update. Cite them as "Unicode 18.0 reports, revision N, date D".
23. **`Preserved` trailing spaces versus hanging. Resolved for the output** (5, Stage 1.7): they are exempt trailing whitespace, not hanging glyphs. 6.2.3 and 6.3.3 are reconciled through the L1 geometry (6.3 rule 3): the extreme visual stop of the visible content; it equals `(visible_byte_end, Upstream)` only when that content has the base direction. **[Design decision, to be confirmed]**: the reviewer's wording ("return the visible end byte" for any click past the rightmost visible glyph of an LTR line) differs for lines ending in an opposite-direction run.
27. **Chunk-join wording (4.2 req. 5). Reworded** to a conservative two-sided rule. Still **[Unverified]**: whether the end-of-chunk check is needed at all.
28. **`ligature_carets` units, `shape_run` inputs. Resolved** (4.1): layout units relative to the glyph origin, `font_size · font.scale`; `text` is the whole paragraph. Unverified: whether backends return positions in logical or visual component order for RTL (6.2 says increasing).
31. **Two-sided windows. Resolved** by `LeadingEdit` and `TrailingEdit` (4.1). Still unverified against a real backend: how replacement glyphs are attributed to clusters when the two cuts leave a window with no cluster boundary between them.

---

## References

Dates are those recorded in the research notes (research date 2026-11-07). Entries marked **[non-official]** are secondary or third-party.

**Unicode and CLDR**

- `[U1]` Unicode 18.0.0, announced 2026-09-16 (13,007 new characters, total 172,808; page carries a stale preliminary-draft banner). https://www.unicode.org/versions/latest/
- `[U2]` UAX #29 Unicode Text Segmentation, Unicode 18.0.0, revision 49, 2026-09-01 (draft banner). https://www.unicode.org/reports/tr29/tr29-49.html. Previous: rev 47 (Unicode 17.0, 2025-08-17) https://www.unicode.org/reports/tr29/tr29-47.html; rev 43 (2023-08-16) https://www.unicode.org/reports/tr29/tr29-43.html
- `[U3]` UAX #9 Unicode Bidirectional Algorithm, Unicode 18.0.0, revision 52, 2026-09-01. https://www.unicode.org/reports/tr9/tr9-52.html
- `[U4]` UAX #24 Unicode Script Property, Unicode 18.0.0, revision 41, 2026-08-28. https://www.unicode.org/reports/tr24/tr24-41.html
- `[U5]` UTS #51 Unicode Emoji, version 18.0, revision 31, 2026-08-28. https://www.unicode.org/reports/tr51/tr51-31.html
- `[U6]` UTS #37 Unicode Ideographic Variation Database, version 7.0, revision 16, 2026-04-30 (labelled "Proposed Update"). https://www.unicode.org/reports/tr37/
- `[U7]` UAX #14 Unicode Line Breaking Algorithm, Unicode 18.0.0, revision 57, 2026-09-01 (previous listed as tr14-55). https://www.unicode.org/reports/tr14/
- `[U8]` UCD 18.0.0 data files (fetched from `latest`): BidiBrackets (2026-02-03) https://www.unicode.org/Public/UCD/latest/ucd/BidiBrackets.txt; WordBreakProperty (2026-06-29) https://www.unicode.org/Public/UCD/latest/ucd/auxiliary/WordBreakProperty.txt; emoji-variation-sequences (2026-01-09) https://www.unicode.org/Public/UCD/latest/ucd/emoji/emoji-variation-sequences.txt; Scripts https://www.unicode.org/Public/UCD/latest/ucd/Scripts.txt; ScriptExtensions https://www.unicode.org/Public/UCD/latest/ucd/ScriptExtensions.txt; DerivedCoreProperties https://www.unicode.org/Public/UCD/latest/ucd/DerivedCoreProperties.txt; GraphemeBreakProperty https://www.unicode.org/Public/UCD/latest/ucd/auxiliary/GraphemeBreakProperty.txt; LineBreak https://www.unicode.org/Public/UCD/latest/ucd/LineBreak.txt
- `[U9]` CLDR `common/main` (main branch, fetched 2026-11-07): https://raw.githubusercontent.com/unicode-org/cldr/main/common/main/sr.xml (also `mk.xml`, `bs_Cyrl.xml`, `az_Cyrl.xml`, `ab.xml`, `en.xml`, `de.xml`, `fr.xml`, `sv.xml`, `pl.xml`, `ru.xml`, `ja.xml`, `ar.xml` in the same directory)

**Script-run itemisers and Rust text stacks**

- `[S1]` Chromium Blink `script_run_iterator.cc` (main). https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/platform/fonts/script_run_iterator.cc
- `[S2]` Pango `pango-script.c` and `pango-script-private.h` (main). https://gitlab.gnome.org/GNOME/pango/-/raw/main/pango/pango-script.c
- `[S3]` ICU sample `scrptrun.cpp` (Copyright 1999-2016 IBM). https://raw.githubusercontent.com/unicode-org/icu/main/icu4c/source/extra/scrptrun/scrptrun.cpp
- `[S4]` Parley 0.12.0 (2026-10-09), fontique 0.12.0: `itemize.rs` https://raw.githubusercontent.com/linebender/parley/main/parley_engine/src/itemize.rs; `shape/mod.rs` https://raw.githubusercontent.com/linebender/parley/main/parley/src/shape/mod.rs; `layout/cluster.rs`, `editing/cursor.rs`, `layout/line_break.rs` in the same repository.
- `[S5]` cosmic-text 0.19.0 (2026-04-22): https://raw.githubusercontent.com/pop-os/cosmic-text/main/src/shape.rs and `src/cursor.rs`.
- `[S6]` icu_segmenter 2.4.0 (2026-10-22; 2.3.0 on 2026-08-13): https://docs.rs/icu_segmenter/2.4.0/icu_segmenter/struct.LineSegmenter.html (also `LineSegmenterBorrowed`, `WordSegmenter`, `options::LineBreakOptions`); crate metadata https://crates.io/api/v1/crates/icu_segmenter

**CSS and typography standards**

- `[W1]` CSS Text Module Level 3, Candidate Recommendation Draft, 14 August 2026. https://www.w3.org/TR/css-text-3/
- `[W2]` CSS Text Module Level 4, Working Draft, 14 August 2026 (previous WD 8 June 2026). https://www.w3.org/TR/css-text-4/
- `[W3]` CSS Inline Layout Module Level 3 §5.3. https://www.w3.org/TR/css-inline-3/
- `[W4]` JLReq, Requirements for Japanese Text Layout, W3C Group Note, 11 August 2020. https://www.w3.org/TR/jlreq/ (read through a summarising fetch; verify wording)
- `[W5]` CLREQ, Requirements for Chinese Text Layout, Group Note Draft DNOTE-clreq-20261104 (4 November 2026). https://www.w3.org/TR/clreq/
- `[W6]` Adobe CCJK Type Blog, "Contextual spacing GPOS features redux", 2019-04-22 (vendor blog). https://ccjktype.fonts.adobe.com/2019/04/contextual-spacing-gpos-features-redux.html
- `[W7]` blink-dev Intent to Ship, `text-spacing-trim` (Chrome 123 estimated; intent 2024-01-31). https://groups.google.com/a/chromium.org/g/blink-dev/c/jVUR2ebE3e0. MDN browser-compat-data (Nov 2026 state) https://raw.githubusercontent.com/mdn/browser-compat-data/main/css/properties/text-spacing-trim.json and `text-autospace.json`. MDN pages (modified 2026-11-03 and 2026-04-20) **[non-official secondary]**

**Line breaking, hyphenation and long paragraphs**

- `[L1]` Knuth and Plass, "Breaking Paragraphs into Lines", Software: Practice and Experience 11(11), 1981. Copy hosted at https://Www.Gwern.net/doc/design/typography/tex/1981-knuth.pdf **[non-official host of a primary paper]**
- `[L2]` Knuth, `tex.web`, version 3.141592653 (January 2021). https://mirrors.ctan.org/systems/knuth/dist/tex/tex.web
- `[L3]` Typst `linebreak.rs` https://raw.githubusercontent.com/typst/typst/main/crates/typst-layout/src/inline/linebreak.rs; Typst text docs https://typst.app/docs/reference/text/text/
- `[L4]` Chromium `score_line_breaker.{h,cc}` and `score_line_break_context.h` (main), under https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/layout/inline/
- `[L5]` Chrome for Developers, `text-wrap: pretty` (Chrome 117; page updated 2023-10-23) https://developer.chrome.com/blog/css-text-wrap-pretty; WebKit blog https://webkit.org/blog/16547/better-typography-with-text-wrap-pretty/; date 2025-04-18 for the WebKit post and 2025-04-08 for Safari Technology Preview 216 from https://ppc.land/safaris-text-wrap-pretty-brings-superior-web-typography/ **[non-official]**; Chrome i18n features (word-break auto-phrase, page last updated 2023-12-04) https://developer.chrome.com/blog/css-i18n-features
- `[L6]` Android `LineBreaker` (API 29). https://developer.android.com/reference/android/graphics/text/LineBreaker
- `[L7]` `textwrap` 0.16.4 (2026-09-13): https://raw.githubusercontent.com/mgeisler/textwrap/master/src/wrap_algorithms/optimal_fit.rs
- `[L8]` Typst Universe package `vlna` 0.4.1 (2026-10-30) **[non-official third-party]**. https://typst.app/universe/package/vlna
- `[H1]` hypher 0.1.8 (2026-09-17; 0.1.7 on 2026-04-07): https://raw.githubusercontent.com/typst/hypher/main/README.md; https://docs.rs/hypher/0.1.8/hypher/. The README (main) and docs.rs 0.1.8 disagree on language count and allocation threshold.
- `[H2]` `hyphenation` (tapeinosyne) 0.8.4 (2021-08-19). https://raw.githubusercontent.com/tapeinosyne/hyphenation/master/README.md
- `[H3]` libhyphen `hyphen.h` https://raw.githubusercontent.com/hunspell/hyphen/master/hyphen.h; README.nonstandard (Android mirror) https://android.googlesource.com/platform/external/hyphenation/+/refs/heads/main/README.nonstandard; PyHyphen docs https://pyhyphen.readthedocs.io/en/latest/nonstandard.html **[non-official]**

**Shaping and platform APIs**

- `[B1]` HarfBuzz `hb-buffer.h` https://raw.githubusercontent.com/harfbuzz/harfbuzz/main/src/hb-buffer.h; `hb-ot-shape.cc` https://raw.githubusercontent.com/harfbuzz/harfbuzz/main/src/hb-ot-shape.cc; NEWS (top entry 15.1.0, 2026-11-04; 4.0.0, 2022-03-01; 1.5.0, 2017-08-23) https://raw.githubusercontent.com/harfbuzz/harfbuzz/main/NEWS; manual https://harfbuzz.github.io/clusters.html; `hb-ot-layout` https://harfbuzz.github.io/harfbuzz-hb-ot-layout.html
- `[B2]` HarfRust 0.15.0 (2026-10-22; first release 0.1.0 on 2025-06-10) https://raw.githubusercontent.com/harfbuzz/harfrust/main/README.md; rustybuzz 0.20.1 (2024-11-12, archived) https://raw.githubusercontent.com/harfbuzz/rustybuzz/main/README.md; versions https://crates.io/api/v1/crates/harfrust/versions
- `[B3]` Pango `shape.c` https://gitlab.gnome.org/GNOME/pango/-/raw/main/pango/shape.c and `pango-layout.c` (same directory).
- `[B4]` Blink `harfbuzz_shaper.cc` and `shape_result.cc` under https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/platform/fonts/shaping/
- `[B5]` DirectWrite `IDWriteTextAnalyzer::GetGlyphs` https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nf-dwrite-idwritetextanalyzer-getglyphs; `IDWriteFontFallback::MapCharacters` https://learn.microsoft.com/en-us/windows/win32/api/dwrite_2/nf-dwrite_2-idwritefontfallback-mapcharacters; Apple `kCTLanguageAttributeName` https://developer.apple.com/documentation/coretext/kctlanguageattributename
- `[B6]` GPUI (crate 0.2.2, 2026-10-30; sources read from `main`): https://raw.githubusercontent.com/zed-industries/zed/main/crates/gpui/src/platform.rs, `text_system.rs`, `text_system/line_layout.rs`, `text_system/line_wrapper.rs`
- `[B7]` OpenType OS/2 https://learn.microsoft.com/en-us/typography/opentype/spec/os2; OpenType GDEF https://learn.microsoft.com/en-us/typography/opentype/spec/gdef

**Caret, selection and editing**

- `[C1]` Pango `get_cursor_pos` https://docs.gtk.org/Pango/method.Layout.get_cursor_pos.html and `move_cursor_visually` https://docs.gtk.org/Pango/method.Layout.move_cursor_visually.html; Pango `GlyphString.get_logical_widths` https://docs.gtk.org/Pango/method.GlyphString.get_logical_widths.html; Android `Layout` https://developer.android.com/reference/android/text/Layout; Flutter `TextAffinity` https://api.flutter.dev/flutter/dart-ui/TextAffinity.html and `BoxHeightStyle` https://api.flutter.dev/flutter/dart-ui/BoxHeightStyle.html; DirectWrite `HitTestPoint` https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nf-dwrite-idwritetextlayout-hittestpoint and `HitTestTextPosition` https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nf-dwrite-idwritetextlayout-hittesttextposition; Parley `cursor.rs`; cosmic-text `cursor.rs`
- `[C2]` Mozilla Bug 1198600 https://bugzilla.mozilla.org/show_bug.cgi?id=1198600; Bug 1138419 https://bugzilla.mozilla.org/show_bug.cgi?id=1138419 (bug-tracker comments are user reports, not specifications)
- `[C3]` Chromium `backspace_state_machine.cc` and `.h` (summarised by a fetch tool). https://raw.githubusercontent.com/chromium/chromium/main/third_party/blink/renderer/core/editing/state_machines/backspace_state_machine.cc
- `[C4]` WebKit Bug 27523. https://bugs.webkit.org/show_bug.cgi?id=27523
- `[C5]` AccessKit `TextPosition` https://docs.rs/accesskit/latest/accesskit/struct.TextPosition.html and `Node` https://docs.rs/accesskit/latest/accesskit/struct.Node.html; Apple `NSTextInputClient.firstRect(forCharacterRange:actualRange:)` https://developer.apple.com/documentation/appkit/nstextinputclient/firstrect(forcharacterrange:actualrange:)

**Crate version metadata** (crates.io API, 2026-11-07): unicode-script 0.5.9 (2026-11-02); swash 0.2.10 (2026-07-17); skrifa 0.49.0 and read-fonts 0.46.0 (2026-10-21); unicode-segmentation 1.13.3 (2026-06-01); unicode-linebreak 0.1.5 (2023-07-25); icu_properties 2.4.0 (2026-10-22); unicode-bidi 0.3.18 (2024-12-16). https://crates.io/api/v1/crates/
