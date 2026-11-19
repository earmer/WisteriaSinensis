# Multilingual Typesetting and Interactive Layout Engine — Specification

This document is the normative contract of the engine and of its reference crate `mutil-engine`. Background, sources, design history and open questions are in [RESEARCH.md](RESEARCH.md), whose sections carry the same numbers as the sections here: the evidence for §*n* is in RESEARCH §*n*.

## 0. How to read this document

**[DOC-1]** The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are to be read as described in RFC 2119 and RFC 8174 when, and only when, they appear in capitals.

**[DOC-2]** Every requirement carries a bracketed identifier such as `[AST-12]`. Identifiers are stable: a requirement that is withdrawn keeps its number retired, and a new one takes the next free number of its prefix. Error tables cite the requirement each error enforces.

**[DOC-3]** Every type, field, variant, method and constant this document names in code type exists in the crate under exactly that name. A ```` ```rust ```` block states the crate's definition: its public fields or its variants are exactly the crate's, and a struct written `{ .. }` has no public field (its invariant is kept by its constructors). A ```` ```rust planned ```` block states an interface that is specified but not implemented yet. `scripts/check_spec.py` enforces this in CI; a disagreement is a defect of either the document or the crate.

**[DOC-4]** *Future* marks behaviour that the crate does not implement yet. Its contract is normative for the implementation that will provide it.

**[DOC-5]** Geometry is in layout units, the unit of `font_size`. The engine does not know the device pixel ratio.

---

## 1. Scope and architecture

**[SCO-1]** The engine maps UTF-8 text and styling spans to glyph positions, caret geometry and selection rectangles. It stops before rasterisation: it manages no GPU atlas, shader or platform event loop.

**[SCO-2]** Output is deterministic for the same text, options, fonts and Unicode data versions.

### 1.1 Conformance baseline

**[SCO-3]** The engine targets Unicode 18.0.0: UAX #29 rev 49, UAX #9 rev 52, UAX #14 rev 57, UAX #24 rev 41, UTS #51 v18.0 rev 31, UTS #37 v7.0 rev 16.

**[SCO-4]** Each installed Unicode component reports the data version it actually uses through `conformance_report()`; a component that is not installed reports `None`. A component never claims a target version from its constructor's name.

```rust
pub struct ConformanceReport {
    pub grapheme: Option<&'static str>,
    pub bidi: Option<&'static str>,
    pub line_break: Option<&'static str>,
    pub word: Option<&'static str>,
    pub script: Option<&'static str>,
    pub emoji: Option<&'static str>,
}
pub const UNICODE_DATA_VERSION: &str = "18.0.0";
```

**[SCO-8] One data source.** Every Unicode component of the engine (segmentation, line breaking, script, emoji, bidi properties and canonical equivalence) reads the compiled data of `icu_segmenter`, `icu_properties` and `icu_normalizer`, all of one ICU4X release, and no other Unicode data crate is a dependency. The HarfRust backend is built with its `icu` feature, so its own character properties, mirroring and normalisation read the same ICU4X data (SHP-11). `UNICODE_DATA_VERSION` is the Unicode version of that data, and it is what an installed component reports (SCO-4). A contract test reads `Cargo.lock` and fails when the ICU4X data crates (`icu_segmenter_data`, `icu_properties_data`, `icu_normalizer_data`) leave the release line whose data is that version, so the reported version cannot drift from the data.

### 1.2 Language tags

**[SCO-5]** Explicit BCP 47 tags win. Under `LocalePolicy::ExplicitOnly` (the default) untagged text takes `FrontendOptions::default_locale`. Script-based locale heuristics (§3 Step 4) run only under `LocalePolicy::ExperimentalHeuristics`.

### 1.3 Three tiers

**[SCO-6]** The engine has three tiers, and dependencies point only as shown:

```text
types  <-  shaping  <-  frontend
  ^           ^           ^
  +-----  middleend  -----+
```

The frontend calls the shaping bridge while it compiles a paragraph (FE-3), and only through the `TextShaper` trait, never through a backend. The bridge depends on `types` alone: the data it exchanges (`ShapingRun`, `GraphemeAtom`, `ShapedCluster`, `RawShapedGlyph`, `HyphenOpportunity`) is defined there.

1. **Frontend**: grapheme atoms, bidi levels, scripts, locales, shaping runs and the box-and-glue item sequence of one paragraph (`ParagraphAst`). Its output is independent of column width.
2. **Shaping bridge**: the `TextShaper` trait. It shapes whole runs, re-shapes windows at breaks and reports ligature carets.
3. **Middle-end**: line breaking, boundary re-shaping, per-line bidi reordering, justification, vertical metrics (`layout_paragraph`), and the interaction queries.

### 1.4 Backend targets

**[SCO-7]** A conforming shaping backend implements the full `TextShaper` contract (§4.1). HarfRust, CoreText and DirectWrite can; a backend that cannot carry the contract runs behind the degraded `LineOnlyShaper` adapter (§4.6).

### 1.5 Architectural axioms

Every data-model, pipeline and tie-breaking decision in this specification follows from these axioms. Where two axioms conflict, the lower-numbered axiom prevails.

- **[AX-1] Source coordinates are the positional identity.** UTF-8 byte offsets into the paragraph text serve as the sole positional anchor across the pipeline. Entity identifiers (`box_id`, `span_id`, `annotation_id`, `atomic_id`) name structural entities; they never stand in for positions. A glyph whose bytes exist in the source maps directly to them, including glyphs generated by boundary re-shaping: they inherit the source range of the atoms they replace. A glyph without a source counterpart (an inserted hyphen, a repeated math operator, an ellipsis) carries `byte_range: None`, and that absence is the exclusive marker of synthesis. Ruby and emphasis glyphs operate within their annotation's local coordinate space and never claim body bytes.
- **[AX-2] One fact, one home.** A fact is stored exactly once. A secondary copy is permitted strictly as a *validated cache*, and only when all three conditions hold: a hot path reads it frequently; recomputing it is costly, requiring data outside the entity (such as neighbouring nodes, cluster lists or span streams) and more than amortised O(1) work; and the trust-boundary validator checks it for exact equality. A value derivable in O(1) solely from fields on self (without indexing external slices or sibling arenas) must never be stored; cross-table summaries (`byte_range`, `flags`, `natural_width`, `bidi_level` on clusters and boxes) are valid caches when they preserve slice self-containment or hot-loop locality. Parallel vectors and redundant error variants for a single root cause are prohibited.
- **[AX-3] Parse where types enforce; validate once at trust boundaries.** Types that can make invalid values unconstructible must do so (tags, levels, ranges, enums, `Option` in place of sentinels). Plain-data aggregates with public fields (the paragraph AST, constraints, options, positioned output) are validated precisely once where they cross a trust boundary. There are four gates, each the only place its checks happen: **G1** the input gate (`ValidatedInputs::try_from_ast`, also run by `check_layout_inputs`) for the AST, text, constraints and options; **G2** the shaper gate for every answer a `TextShaper` gives at layout time (metrics, and ligature carets through `resolve_ligature_carets`); **G3** the provider gate (`LayoutConstraints::query_checked_geometry`) for geometry provider answers; **G4** the output gate (`ValidatedLayout::try_from_paragraph`, also run by `PositionedParagraph::validate`) for positioned output. Code behind a gate receives `ValidatedInputs` or `ValidatedLayout`, relies on what they prove and does not re-check: their lookups index directly, and a broken invariant there is a bug, not an input error. Functions whose arguments only gated code computes state those arguments as preconditions (`line_fit`, ST1-8; `resolve_ligature_carets`, INT-5).
- **[AX-4] Orthogonal streams; a solver that never classifies characters.** The frontend compiles linguistic and styling rules into independent streams: `Discretionary` nodes determine where lines may break, `InlineDecorationSpan` dictates box slicing, `AtomicEditSpan` governs caret and deletion units, and `ExplicitBidiSpan` controls bidi isolation. Any interaction between streams must be compiled in explicitly by the frontend (for example, cloned padding is folded directly into discretionary widths); the middle-end never infers it. The middle-end operates exclusively on node kinds, flags and spans, never on code points. Validation inspects source bytes solely to verify the frontend's classification. Classification follows physical layout behaviour rather than Unicode naming (for example, U+3000 is a rigid text box, not adjustable glue).
- **[AX-5] Every delta is booked where it materialises.** Each geometric contribution (`pre_break_width`, `post_break_width`, cloned padding, leading and trailing glue, hanging extents) is charged strictly to the line fragment and edge where it is physically drawn. Every mapping from logical to physical edges must be parameterised explicitly: start/end by `base_direction`, and over/under and block-direction offsets by `WritingMode`.
- **[AX-6] Lazy materialisation, bounded progress.** Evaluating a candidate line never invokes the shaper; it operates in O(1) over precomputed prefix sums, or O(k) for a line containing k tabs. `reshape_window` and `rejoin_windows` execute exclusively for committed lines, with one exception: an emergency break (ST1-5) re-shapes the seam of each candidate it tries, because only shaping gives an unsafe seam's width (lost kerning or ligatures can widen or narrow it), so the break cannot be chosen from prefix sums. That path runs only when a single unit overflows its line, and each try moves the seam back by at least one unit. Geometry queries are monotonic: an expanded envelope never introduces new inline space. Every layout step either consumes input or advances the block position by at least the minimum clearance step; a tab lacking a reachable stop is consumed with zero advance and terminates the line.
- **[AX-7] Total entities, self-contained output.** Every inline entity exposes, from itself, its owning run or its paragraph, the complete property set required by downstream invariants (source range, bidi level, metrics) without searching adjacent nodes. `PositionedParagraph` records every resolved input its validation requires (the policy, the slot source with its indents, the strut and the base direction), enabling `validate(text)` to run in O(n log n) in the size of the output, requiring nothing beyond the source text.

**[AX-8]** The validated caches the crate keeps under AX-2 are exactly these: `BoxNode::Text`'s `byte_range` and `atom_range` (from its clusters' atoms) and `natural_width` (the sum of its clusters'); `ShapedCluster`'s `byte_range` (its atoms' bytes), `flags` (`AtomFlags::for_cluster` of its atoms), `bidi_level` and `orientation` (its atoms'); `ShapingRun`'s `script`, `bidi_level` and `orientation`, which repeat those of its atoms (the frontend resolves them per atom in §3 Steps 1–4, before runs exist, and Step 5 partitions runs by them); the cloned padding and the repeated operator's advance folded into `DiscretionaryNode`'s `pre_break_width` and `post_break_width`; `SpanBoundary`'s `width` (its span's padding) and `bidi_level` (the level of the first or last inline entity inside its span, else the base level, WN-8). Output entities copy what their validation needs from the input (AX-7), such as `PositionedSpanFragment::span_range`; those copies are the output's own facts, since the output carries no AST.

---

## 2. Core data structures

### 2.1 Identifiers, ranges, tags and geometry

```rust
pub type ByteIndex = usize;
pub struct TextRange { .. }           // [start, end), start <= end by construction
pub struct ScriptTag(..);             // ISO 15924, by construction
pub struct InvalidScriptTag;
pub struct LocaleTag(..);             // BCP 47 well formed, by construction
pub struct InvalidLocaleTag;
pub struct BidiLevel(..);             // 0..=BidiLevel::MAX, by construction
pub enum Direction { Ltr, Rtl }
pub type BidiDirection = Direction;
pub enum BidiClass { L, R, Al, En, Es, Et, An, Cs, Nsm, Bn, B, S, Ws, On,
                     Lre, Lro, Rle, Rlo, Pdf, Lri, Rli, Fsi, Pdi }
pub struct Point { pub x: f32, pub y: f32 }
pub struct Size { pub width: f32, pub height: f32 }
pub struct Rect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }
pub const MAX_MEASUREMENT: f32 = 1.0e15;
pub fn is_measurement(value: f32) -> bool;
pub fn is_extent(value: f32) -> bool;
```

**[TYP-1]** Every offset is a UTF-8 byte offset (`ByteIndex`) into the text of one paragraph. Synthetic elements never displace source offsets.

**[TYP-2]** A `TextRange` is half-open and its fields are private, so `start <= end` holds for every value: it is the coordinate primitive of the engine, parsed rather than validated. `TextRange::new` panics on `start > end` (for ranges the caller computes); `TextRange::try_new` returns `None` for it (for offsets read from outside, which host paths use). `TextRange::start`, `TextRange::end`, `TextRange::len`, `TextRange::is_empty`, `TextRange::contains`, `TextRange::contains_range`, `TextRange::cover`, `TextRange::slice` and `TextRange::as_range` are the queries. `TextRange::EMPTY` is `0..0` and `TextRange::empty_at` an empty range at an offset. Whether a range fits a text and falls on its code point boundaries is checked where the text is known.

**[TYP-3]** A `ScriptTag` is four ASCII letters stored in ISO 15924 case (`Latn`); `ScriptTag::try_from_bytes` and `ScriptTag::try_from_str` (also `FromStr`) are its only constructors besides the constants (`ScriptTag::COMMON`, `ScriptTag::INHERITED`, `ScriptTag::UNKNOWN`, `ScriptTag::EMOJI`, `ScriptTag::LATIN`, `ScriptTag::CYRILLIC`, `ScriptTag::GREEK`, `ScriptTag::ARABIC`, `ScriptTag::HEBREW`, `ScriptTag::DEVANAGARI`, `ScriptTag::THAI`, `ScriptTag::HAN`, `ScriptTag::HIRAGANA`, `ScriptTag::KATAKANA`, `ScriptTag::HANGUL`, `ScriptTag::BOPOMOFO`). `ScriptTag::is_unresolved` is true for `Zyyy` and `Zinh`. `Zsye` (`ScriptTag::EMOJI`) is an engine-internal font-fallback key for emoji presentation, not a Unicode `Script` value.

**[TYP-4]** A `LocaleTag` matches the RFC 5646 §2.1 `Language-Tag` grammar (a `langtag`, a private-use tag or a grandfathered tag) and is stored in the case of RFC 5646 §2.1.1, so equal tags compare and hash equal. `LocaleTag::try_from_str` (also `FromStr`) and `LocaleTag::und` construct it; nothing is looked up in a registry. RFC 5646 sets no maximum length, so a tag longer than 128 bytes is rejected with `InvalidLocaleTag` before anything is allocated: the cap bounds the memory an untrusted host string can claim.

**[TYP-5]** A `BidiLevel` is a UAX #9 embedding level no higher than `BidiLevel::MAX` (`BIDI_MAX_DEPTH + 1`, with `BIDI_MAX_DEPTH = 125`). `BidiLevel::try_new` returns `None` above it; `BidiLevel::LTR` and `BidiLevel::RTL` are levels 0 and 1; `BidiLevel::direction` and `Direction::from_level` give the parity (odd is RTL); `Direction::base_level` is the paragraph level of a direction.

**[TYP-6]** Geometry types are plain `f32` values with `y` growing downwards; `Rect::new`, `Rect::right` and `Rect::bottom` are helpers.

**[TYP-7] Lenient locale input.** `LocaleTag::try_from_str_lenient` reads host input that may use common shorthand. It trims white space, rejects a trimmed input longer than TYP-4's 128 bytes before anything is allocated, reads `_` as `-` (copying the input only when it holds a `_`) and, in any letter case, maps `jp` to `ja-JP`; `cn`, `zh-cn`, `zh-sc` and `zh-chs` to `zh-CN`; `tw`, `zh-tw`, `zh-tc` and `zh-cht` to `zh-TW`; `hk` and `zh-hk` to `zh-HK`; and `kr` to `ko-KR`, then parses the result as `LocaleTag::try_from_str` does. Without the mapping these inputs are well-formed tags with other meanings or none (`kr` is Kanuri, `zh-sc` Chinese in the Seychelles, `jp` and `cn` unassigned subtags), so `LocaleTag::try_from_str` keeps them as written and only a host that asks for the mapping gets it.

**[MSR-1] Measurement bound.** Every primary measurement the gates accept (widths, extents, capacities, offsets, spacings, font sizes, raw font metrics, resolved line boxes, a provider's clearance jump) is a measurement: finite with magnitude at most `MAX_MEASUREMENT` = 10¹⁵ layout units (`is_measurement`; `is_extent` adds non-negativity). Caches that sum an unbounded number of measurements, such as a text box's `natural_width`, are exempt. A paragraph holds fewer than 2⁶⁴ records, so every sum the engine forms stays below 2⁶⁴ · 10¹⁵ · 10⁴ ≈ 1.8 · 10³⁸ < `f32::MAX`: no layout arithmetic overflows, and nothing behind the gates checks for it.

### 2.2 Grapheme atoms and span streams

```rust
pub struct GraphemeAtom {
    pub orientation: GlyphOrientation,
    pub byte_range: TextRange,
    pub script: ScriptTag,
    pub bidi_level: BidiLevel,
    pub linebreak_class: LineBreakClass,
    pub flags: AtomFlags,
}
pub enum LineBreakClass { Bk, Cr, Lf, Cm, Nl, Sg, Wj, Zw, Gl, Sp, Zwj, B2, Ba, Bb, Hy, Hh, Cb, Cl, Cp,
                          Ex, In, Ns, Op, Qu, Is, Nu, Po, Pr, Sy, Ai, Ak, Al, Ap, As, Cj, Eb, Em,
                          H2, H3, Hl, Id, Jl, Jt, Jv, Ri, Sa, Vf, Vi, Xx }
pub struct AtomFlags(..);
```

**[ATM-1]** An atom is one extended grapheme cluster of UAX #29, GB9c included. It stores its bytes, script, bidi level (of its first code point; marks follow their base), glyph orientation, line-break class and flags. Script, level and orientation are primary data of the atom: the frontend resolves them in §3 Steps 1–4, before any run exists, and Step 5 partitions runs wherever one of them changes. Every atom of a run therefore agrees with the run on all three (AST-5).

**[ATM-2]** `linebreak_class` is the UAX #14 class of the atom's first code point. `LineBreakClass` is one byte wide, but its numeric values are not part of the contract: they move when Unicode adds a class, so it is never stored or exchanged as a number. The segmenter always runs over the original text, so LB9 and LB10 see the real sequence.

**[ATM-3]** `AtomFlags` is a bit set (`AtomFlags::EMPTY`, `AtomFlags::ALL`, `AtomFlags::bits`, `AtomFlags::from_bits_truncate`, `AtomFlags::is_empty`, `AtomFlags::contains`, `AtomFlags::intersects`, `AtomFlags::union`, `AtomFlags::insert`, `AtomFlags::remove`, `AtomFlags::set`; bits outside `ALL` are masked off). Its flags are:

| Flag | Meaning |
|---|---|
| `AtomFlags::IS_CJK_IDEOGRAPH` | a Han ideograph or full-width kana |
| `AtomFlags::ALLOW_CJK_JUSTIFY_AFTER` | the boundary after the atom is an inter-character justification opportunity |
| `AtomFlags::NO_LETTER_SPACING` | a cursive script that admits no letter spacing (Arabic, Hanifi Rohingya, Mandaic, Mongolian, N'Ko, Phags Pa, Syriac) |
| `AtomFlags::IS_EMOJI_SEQUENCE` | a UTS #51 emoji sequence (core, ZWJ, keycap, flag, tag, modifier) |
| `AtomFlags::IS_OPEN_PUNCT`, `AtomFlags::IS_CLOSE_PUNCT` | opening or closing punctuation |
| `AtomFlags::HAS_VARIATION_SELECTOR` | the atom contains U+FE0E, U+FE0F or an IVS selector |
| `AtomFlags::IS_APOSTROPHE` | a resolved intra-word apostrophe (§3 Step 2) |
| `AtomFlags::BINDS_TO_PREVIOUS` | a continuation atom of an FE-15 dash, ellipsis or middle-dot run sharing its predecessor's `ShapingRun` (§3 Step 3) |
| `AtomFlags::L1_RESETTABLE` | an atom whose first code point UAX #9 L1 resets with whitespace: class WS, S, BN, an isolate initiator or PDI, or an embedding or override control (FE-30) |
| `AtomFlags::HANGABLE_START` | an opening mark that may hang at a line start: UAX #14 class OP, or General Category Ps or Pi (ST1-3) |
| `AtomFlags::HANGABLE_END` | a stop or comma that may hang at a line end: U+002C, U+002E, U+3001, U+3002, U+FF0C or U+FF0E (ST1-3) |

**[ATM-4]** The flags of a cluster are derived, never stored (AX-2): `AtomFlags::for_cluster` takes `AtomFlags::FIRST_ATOM_ONLY` (`IS_OPEN_PUNCT`, `BINDS_TO_PREVIOUS`, `HANGABLE_START`) from the first atom, `AtomFlags::LAST_ATOM_ONLY` (`ALLOW_CJK_JUSTIFY_AFTER`, `IS_CLOSE_PUNCT`, `HANGABLE_END`) from the last, and ORs every other flag. An empty slice gives no flags.

```rust
pub struct InlineDecorationSpan {
    pub span_id: u64,
    pub byte_range: TextRange,
    pub start_padding: f32,
    pub end_padding: f32,
    pub break_mode: BoxDecorationBreak,
}
pub enum BoxDecorationBreak { Slice, Clone }
pub struct AtomicEditSpan { pub span_id: u64, pub byte_range: TextRange }
pub struct CompositionSpan { pub byte_range: TextRange }
```

**[SPN-1]** An `InlineDecorationSpan` gives a stretch of text padding and a fragmentation mode. Its range is non-empty and its paddings are finite and non-negative (`InlineDecorationSpan::is_valid`). Under `BoxDecorationBreak::Slice` the paddings apply only at the span's logical start and end; under `BoxDecorationBreak::Clone` every line fragment receives both.

**[SPN-2]** An `AtomicEditSpan` is a caret and deletion unit (a mention, a page-reference chip, an inline code span). It has a non-empty range. It MAY wrap across lines: whether it can break is decided only by the `Discretionary` nodes the frontend emits inside it (AX-4). No caret stop lies strictly inside it (`AtomicEditSpan::is_interior`, §6.1), and Backspace and Delete remove it whole (§6.5). Membership of an atom is looked up, never stored on atoms: `ParagraphAst::atomic_span_containing` (binary search), or a cursor in an in-order sweep.

**[SPN-3]** The two streams are orthogonal: a span may be either, both or neither, and they use independent id spaces. Neither stores colours or radii.

**[SPN-4]** When a decoration span has `start_padding > 0`, no `CjkAutospace` glue is inserted at its start, on either side of the boundary; symmetrically at its end when `end_padding > 0` (`InlineDecorationSpan::forbids_cjk_autospace_at`).

**[SPN-5]** `CompositionSpan` carries IME pre-edit styling. It is not stored in the AST, never changes segmentation and never makes an interior caret stop legal; its edges may lie on code point boundaries inside an atom.

### 2.3 Shaped clusters and fonts

```rust
pub struct ShapedCluster {
    pub orientation: GlyphOrientation,
    pub byte_range: TextRange,
    pub atom_range: Range<usize>,
    pub font: FontRef,
    pub bidi_level: BidiLevel,
    pub flags: AtomFlags,
    pub glyph_range: Range<usize>,
    pub natural_width: f32,
    pub safe_to_break_before: bool,
    pub safe_to_concat_before: bool,
}
pub struct RawShapedGlyph {
    pub glyph_id: u32,
    pub x_advance: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub is_synthesized: bool,
}
pub struct FontId(pub u32);
pub struct FontRef { pub font_id: FontId, pub scale: f32 }
pub enum GenericFamily { Serif, SansSerif, Monospace, Cursive, Fantasy, SystemUi, Emoji }
pub enum FontFamily { Named(Arc<str>), Generic(GenericFamily) }
pub type FamilyStack = Arc<[FontFamily]>;
pub struct FontWeight(..);            // 1..=1000, by construction
pub enum FontSlant { Normal, Italic, Oblique }
pub struct FontStyle { pub weight: FontWeight, pub slant: FontSlant }
pub struct FontFeature { pub tag: [u8; 4], pub value: u32 }
pub struct FontVariation { .. }       // a finite axis coordinate, by construction
pub struct FontFeatures { .. }        // canonical by construction (FNT-2)
pub struct FontInstanceRef<'a> { pub font: FontRef, pub font_size: f32, pub variations: &'a [FontVariation] }
```

**[CLU-1]** A cluster covers `K >= 1` whole atoms; `atom_range` is **run-local**: it indexes the run's atom slice (`ParagraphAst::run_atoms`, or an annotation's atoms), which is the `atoms` argument of `TextShaper::shape_run` and `TextShaper::reshape_window`. `K > 1` is a ligature (`ShapedCluster::is_ligature`, `ShapedCluster::atom_count`); a tate-chu-yoko cluster also has `K > 1` and is not a ligature (§2.7).

**[CLU-2]** A cluster's `byte_range`, `flags`, `bidi_level` and `orientation` are validated caches (AX-8) that keep a cluster slice self-contained: `byte_range` runs from its first atom's start to its last atom's end, `flags` is `AtomFlags::for_cluster` of its atoms (ATM-4), and `bidi_level` and `orientation` are those of its atoms, which a cluster never straddles. The validator checks each for exact equality.

**[CLU-3]** `glyph_range` indexes the glyph buffer stored beside the cluster: `ParagraphAst::glyphs`, an annotation's `glyphs`, or the `ShapeOutput::glyphs` of the shaper call that produced it. In cluster order the ranges tile that buffer. A cluster may have no glyph. Glyphs are in logical order, including the order within the cluster.

**[CLU-4]** A cluster is single-font: every glyph belongs to `font`. `FontRef::scale` is the fallback scale (1.0 as requested; `FontRef::unscaled`).

**[CLU-5]** `safe_to_break_before` is true when HarfBuzz's `UNSAFE_TO_BREAK` is absent at the cluster's start; `safe_to_concat_before` when `UNSAFE_TO_CONCAT` is absent. `UNSAFE_TO_BREAK` implies `UNSAFE_TO_CONCAT`, so `safe_to_concat_before` implies `safe_to_break_before` (`ShapedCluster::safety_flags_consistent`). The end of text is always safe.

**[CLU-6]** `natural_width` is the cluster's final inline advance, explicit letter spacing, word spacing and ruby expansion included. It is finite and non-negative. No equality with the sum of `RawShapedGlyph::x_advance` is assumed. There is no stored `chws`-off width.

**[CLU-7]** `RawShapedGlyph::is_synthesized` marks a glyph the shaper inserted (the hyphen of a `TrailingEdit`), since a raw glyph has no source range; once positioned, such a glyph carries `byte_range: None` (AX-1).

**[CLU-8]** A cluster belongs to exactly one text box. Box boundaries are chosen before clusters are assigned; a boundary inside a multi-atom cluster is resolved by re-shaping that window with the boundary as a cut (`LeadingEdit::None`, `TrailingEdit::None`), and the first cluster after the cut has `safe_to_concat_before == false`.

**[FNT-1]** `FontWeight::try_new` accepts `1..=1000` (`FontWeight::NORMAL` 400, `FontWeight::BOLD` 700, `FontWeight::get`). `FontVariation::try_new` accepts only a finite coordinate and maps `-0.0` to `0.0`, so equality and hashing by bit pattern are sound (`FontVariation::tag`, `FontVariation::value`).

**[FNT-2]** `ShapingRun::features` is the single source of OpenType features and variation coordinates. `FontFeatures` is canonical by construction: its fields are private, and `FontFeatures::push` and `FontFeatures::push_variation` keep each list strictly ascending by tag with the **last** setting for a tag replacing an earlier one (as in CSS cascade order). Equal effective settings are therefore equal values however they were pushed, so `Eq` and `Hash` agree with effective equality and the value is a valid cache key as built, with nothing to normalise or check. The `Default` value is the empty set; `FontFeatures::as_slice` and `FontFeatures::variations` read the two lists in tag order.

**[FNT-3]** `FontId` names a physical face, not a size or variation instance; `FontInstanceRef` is the complete query context of a font instance. A `FontId` is meaningful only within the id space of the shaper that issued it. A renderer resolves it to font bytes and a face index only through the `FontSource` that covers that id space, typically the font registry the shaper was built on and shared with the renderer (HST-2), never through its own font lookup; `TextShaper` gains no method for it.

```rust
pub struct FontMetrics {
    pub vertical_ascent: f32, pub vertical_descent: f32, pub vertical_line_gap: f32,
    pub ideographic_baseline_offset: f32, pub central_baseline_offset: f32,
    pub hanging_baseline_offset: f32, pub mathematical_baseline_offset: f32,
    pub ascent: f32, pub descent: f32, pub line_gap: f32,
    pub underline_position: f32, pub underline_thickness: f32,
    pub strikeout_position: f32, pub x_height: f32, pub cap_height: f32,
    pub use_typo_metrics: bool,
}
```

**[FNT-4]** Raw `FontMetrics` are valid (`FontMetrics::is_valid`) when all fifteen numbers are measurements (MSR-1); ascent, descent, line gap, their vertical equivalents, underline thickness, x-height and cap height are non-negative; the horizontal and the vertical ascent-plus-descent are each positive; and every derived height is finite. The four baseline offsets, `underline_position` and `strikeout_position` are positive towards line-over and negative towards line-under, the one vertical sign convention of the engine (OUT-1 `y_offset`, ORI-2 `over_to_block`); OpenType `post` `underlinePosition` and OS/2 `yStrikeoutPosition` already use it, so a typical underline position is negative and a typical strike-out position positive, and both pass through unchanged. `FontMetrics::natural_line_height` is `ascent + descent + line_gap`. `use_typo_metrics` is OS/2 `fsSelection` bit 7.

### 2.4 The item sequence: `WordNode`

```rust
pub enum WordNode {
    Box(BoxNode),
    Glue(GlueNode),
    Discretionary(DiscretionaryNode),
    Tab { byte_range: TextRange },
}
pub enum BoxNode {
    Text { run_index: usize, byte_range: TextRange, atom_range: Range<usize>,
           cluster_range: Range<usize>, natural_width: f32 },
    InlineObject(InlineObjectBox),
    SpanBoundary { byte_range: TextRange, span_id: u64, edge: SpanEdge,
                   width: f32, bidi_level: BidiLevel },
}
pub struct InlineObjectBox {
    pub baseline_align: BaselineAlignment,
    pub byte_range: TextRange,
    pub box_id: u64,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub overflow_policy: BoxOverflowPolicy,
    pub bidi_level: BidiLevel,
}
pub enum SpanEdge { Leading, Trailing }
pub enum BoxOverflowPolicy { Clip, AllowOverflow, ScaleToFit }
pub struct GlueNode {
    pub original_bidi_class: BidiClass,
    pub bidi_level: BidiLevel,
    pub is_collapsible: bool,
    pub byte_range: TextRange,
    pub kind: GlueKind,
    pub natural_width: f32,
    pub max_stretch: f32,
    pub max_shrink: f32,
}
pub enum GlueKind { WordSpace, CjkAutospace, CjkPunctCompression }
pub enum GlueTier { Tier0 = 0, Tier1 = 1, Tier2 = 2 }
pub struct DiscretionaryNode {
    pub byte_range: TextRange,
    pub penalty: i32,
    pub flagged: bool,
    pub pre_break_width: f32,
    pub post_break_width: f32,
    pub no_break_width: f32,
    pub action: BreakAction,
    pub cloned_spans: SmallVec<[u64; 2]>,
}
pub enum BreakAction {
    Direct,
    HardBreak,
    Hyphenate(HyphenOpportunity),
    CjkPunctAdjust,
    MathOperator { repeat: Option<RepeatedOperator> },
}
pub struct RepeatedOperator { pub glyph_id: u32, pub advance: f32 }
pub type ReplacementText = Box<str>;
pub struct HyphenOpportunity {
    pub prefix_replacement: Option<ReplacementText>,
    pub suffix_replacement: Option<ReplacementText>,
    pub prefix_cut_bytes: u32,
    pub suffix_cut_bytes: u32,
}
```

The four variants are the box-and-glue algebra; the frontend compiles every linguistic rule into them (AX-4).

**[WN-1]** A line may end **if and only if** it is at a `Discretionary` (`WordNode::is_break_opportunity`); the single exception is the emergency cluster break of an overflowing empty line (§5 Stage 1.5). Glue is never an implicit break point.

**[WN-2]** A breakable space is `[Box, Glue(WordSpace), Discretionary]`; several consecutive spaces are `[Box, Glue, Glue, Discretionary, Box]`; a space after which the frontend allows no break (FE-23) is `[Box, Glue]`. A no-break space character is not glue (WN-12). When a line breaks there, the spaces belong to the trailing edge of line *k* and line *k + 1* starts at the discretionary's `byte_range.end()`.

**[WN-3]** Closing punctuation (UAX #14 CL, CP, EX, IS, SY), inseparable characters (IN) and the box after opening punctuation (OP) are fused to their neighbour by the **absence** of a `Discretionary`, never by a node of their own. The segmenter's opportunities are never contradicted.

**[WN-4]** Every item stores its position in the source (`WordNode::byte_span`). Zero-width nodes have an empty range.

**Text boxes.**

**[WN-5]** A `BoxNode::Text` stores its run, its bytes, and its paragraph-level `atom_range` and `cluster_range`, never empty and inside the run's `ShapingRun::atom_range` and `ShapingRun::cluster_range`. `byte_range` and `atom_range` are validated caches (AX-8): the atoms run from the first cluster's first atom to the last cluster's last atom, and the bytes from the first of those atoms to the last. `ParagraphAst::text_box` resolves the box in O(1) into a `TextBoxView`. Within the box the clusters partition the atoms in logical order and the atoms are non-empty and contiguous in bytes (`ParagraphAst::is_self_contained`).

**[WN-6]** `natural_width` is a validated cache (AX-8): exactly the sum of the box's clusters' `natural_width`.

**Inline objects.**

**[WN-7]** An `InlineObject` owns the bytes carets and selections address it by, usually one U+FFFC, and never an empty range. Its width, ascent and descent are finite and non-negative; its `bidi_level` is the resolved level of its source. Its reference width is the inline extent of the slot it is placed in, indent and obstacles deducted (SLT-5); on an unobstructed, unindented line that is `wrap_width`. `BoxOverflowPolicy::Clip` clamps its rendering to the reference width; `BoxOverflowPolicy::AllowOverflow` lets an object that sits alone on a line exceed `wrap_width`; `BoxOverflowPolicy::ScaleToFit` scales width, ascent and descent uniformly by `reference / width`.

**Span boundaries.**

**[WN-8]** Every decoration span has exactly one `SpanBoundary` with `SpanEdge::Leading` at `byte_range.start()` and one with `SpanEdge::Trailing` at `byte_range.end()`, even with zero padding. A boundary owns the empty range at its anchor; its `width` is the span's `start_padding` or `end_padding`, and its `bidi_level` the level of the first (leading) or last (trailing) inline entity inside the span, a text atom or an `InlineObject`, or the paragraph's `Direction::base_level` when the span holds neither (AX-8).

**Glue.**

**[WN-9]** `GlueKind::tier` is the only authority for a glue's priority tier: `WordSpace` and `CjkPunctCompression` are `GlueTier::Tier0`, `CjkAutospace` is `GlueTier::Tier1`. No kind is `GlueTier::Tier2`: inter-ideographic stretch is derived from `ALLOW_CJK_JUSTIFY_AFTER` on the clusters, never emitted as glue.

**[WN-10]** Synthetic glue (`GlueKind::is_synthetic`: `CjkAutospace`, `CjkPunctCompression`) owns no bytes. It is always collapsible, its `original_bidi_class` is `BidiClass::Bn`, and its `bidi_level` is that of the inline entity (text atom or `InlineObject`) before it, or the paragraph's base level when none precedes; at a line edge it collapses to zero width under both `HangingWhitespaceMode`s (ST1-12). A `WordSpace` owns a non-empty run of word-space characters and nothing else (`is_word_space`: the Unicode space separators except the ideographic space U+3000, WN-11, and the no-break spaces, WN-12). Its bytes have no `GraphemeAtom`, `ShapedCluster` or glyph: a run's `byte_range` MAY span them (AST-5) while its `atom_range` and `cluster_range` cover only its text boxes (AST-4). The frontend sizes it from the space advance of the primary font of the style at its bytes and that style's `word_spacing` (FE-27, SHP-20); on a line it produces no `PositionedGlyph`, only displacement and caret stops (CRT-2).

**[WN-11]** U+3000 is not glue: the frontend shapes it as ordinary text in a `BoxNode::Text`, followed by a break opportunity (UAX #14 class BA).

**[WN-12]** No-break spaces (`is_no_break_space`: U+00A0, U+2007, U+202F) are not glue: like U+3000, the frontend shapes one as ordinary text in a `BoxNode::Text`, and its UAX #14 class GL leaves no break opportunity on either side. `is_word_space` is false for them, so no `WordSpace` owns one. A `WordSpace` is therefore always collapsible (`is_collapsible == true`): at a line end it collapses outside `visible_byte_end` or hangs outside the body (ST1-12), and inside a line it lies between entities, so a line's visible content always ends on a glyph, a box or a tab (SLT-3, SRC-3).

**[WN-13]** `CjkPunctCompression` is shrink only: `natural_width == 0`, `max_stretch == 0`, and `max_shrink` at most half the `font_size` of the run of the nearest text box before it (FE-31), within the geometry tolerance.

**[WN-14]** `original_bidi_class` is the class of the glue's source before any resolution, read by UAX #9 L1 (§5 Stage 2): `BidiClass::Ws` for a `WordSpace`, whose characters are all class WS, and `BidiClass::Bn` for synthetic glue (WN-10).

**Tabs.**

**[WN-15]** A `Tab` owns its tab character (a non-empty range). It stores no bidi level: UAX #9 L1 resets a segment separator to the paragraph level, `ParagraphAst::base_direction`'s `Direction::base_level` (AX-2). Its width is resolved during layout (§5 Stage 3.1).

**Discretionaries.**

**[WN-16]** `pre_break_width` is added to the end of line *k* when the line breaks here, `post_break_width` to the start of line *k + 1*, and `no_break_width` to the current line when it does not. All three are finite and signed. Actions materialise only at committed breaks (AX-6).

**[WN-17]** A soft opportunity owns the empty range at its break position (`DiscretionaryNode::byte_offset`, `DiscretionaryNode::direct`). A `HardBreak` with a non-empty range owns exactly the three bytes of a U+2028 inside the paragraph (`DiscretionaryNode::hard_break`). A `HardBreak` with an empty range is the paragraph-end sentinel (`DiscretionaryNode::paragraph_end`, `DiscretionaryNode::is_paragraph_end`): it owns `text.len()..text.len()` and is the last item and the last discretionary, exactly once. A hard break has `no_break_width == 0`.

**[WN-18]** `action == BreakAction::HardBreak` exactly when `penalty == DiscretionaryNode::MANDATORY_BREAK_PENALTY` (`DiscretionaryNode::is_mandatory`); a penalty below it has no meaning and is rejected. A forbidden break is an omitted node: `penalty >= DiscretionaryNode::FORBIDDEN_BREAK_PENALTY` is rejected.

**[WN-19]** The penalty scale:

| Penalty | Meaning |
|---|---|
| `DiscretionaryNode::MANDATORY_BREAK_PENALTY` (−10000) | a hard break |
| `DiscretionaryNode::NATURAL_PENALTY` (0) | before a breakable word space, between ideographs, between segmented Thai, Lao, Khmer and Myanmar words, after U+002D |
| `DiscretionaryNode::URL_PENALTY_MIN`..=`DiscretionaryNode::URL_PENALTY_MAX` (25..=50) | inside a URL |
| `DiscretionaryNode::HYPHEN_PENALTY` (135), flagged | standard hyphenation |
| `DiscretionaryNode::SPELLING_CHANGE_HYPHEN_PENALTY` (250), flagged | spelling-change hyphenation |
| `DiscretionaryNode::CLONED_SPAN_PENALTY` (+300) | added to every non-mandatory break inside a `Clone` span |
| `DiscretionaryNode::MATH_RELATION_PENALTY` (500) | after a top-level inline-maths relation |
| `DiscretionaryNode::MATH_BINARY_OPERATOR_PENALTY` (700) | after a top-level inline-maths binary operator |
| `DiscretionaryNode::FORBIDDEN_BREAK_PENALTY` (10000) | never a node |

**[WN-20]** `flagged` marks a break that the Knuth–Plass consecutive-flag demerit counts (§5 Stage 1.6). Every `Hyphenate` break is flagged; a break after an explicit hyphen MAY be.

**[WN-21] Cloned padding.** Inside a span with `BoxDecorationBreak::Clone`, every discretionary, a mandatory one included, lists the span's id in `cloned_spans`, adds the span's `end_padding` to `pre_break_width` and its `start_padding` to `post_break_width`; a non-mandatory one also adds `CLONED_SPAN_PENALTY` to its penalty. The list is complete: every `Clone` span that contains the break strictly inside (`start < offset < end`) appears, once. Cloning is orthogonal to the action. With `clone_pre = Σ end_padding` and `clone_post = Σ start_padding` over `cloned_spans`:

- `Direct` and a hard break in the text: `pre_break_width == clone_pre` and `post_break_width == clone_post`;
- `MathOperator`: `pre_break_width == clone_pre` and `post_break_width == clone_post + repeat.advance` (zero without a repeat); the operator is repeated at the start of line *k + 1*, so its advance is booked there (AX-5);
- the sentinel lists no span and has both widths `0`;
- `Hyphenate` and `CjkPunctAdjust` carry their own shaping deltas on top of the clone sums: `net_pre = pre_break_width - clone_pre` and `net_post = post_break_width - clone_post`.

Equalities hold within the geometry tolerance (§2.6). The clone sums and the repeated advance folded into the widths are a validated cache (AX-8); the shaping deltas have no other home.

**[WN-22] Hyphenation.** A `Hyphenate` break lies between two text boxes of one run that meet at its offset. Its cuts mirror libhyphen's `(rep, pos, cut)`: `prefix_cut_bytes` original bytes are removed immediately before the break and replaced by `prefix_replacement`, `suffix_cut_bytes` immediately after it and replaced by `suffix_replacement`; a cut without replacement is a deletion. Each cut ends on an atom boundary inside the break's **hyphen chain**: the maximal sequence of text boxes of one run joined only by `Hyphenate` breaks that meet them end to end, stopped by glue, a tab, an inline object, a span boundary, a run change or any other discretionary. `HyphenOpportunity` has no penalty of its own.

**[WN-23]** `ParagraphAst::break_window` returns the `ReshapeWindow` to re-shape when a `Hyphenate` break is taken: the box on each side, extended along the chain as far as a cut reaches. `ParagraphAst::rejoin_windows` returns, for the untaken `Hyphenate` breaks of a committed line whose following cluster has `safe_to_concat_before == false`, the windows that restore the unbroken shaping, with overlapping windows merged so each atom is shaped once (§5 Stage 1.6).

```rust
pub struct ReshapeWindow { pub run_index: usize, pub atom_range: Range<usize>, pub byte_range: TextRange }
```

**[WN-24] CJK punctuation seams.** Where two adjacent CJK punctuation clusters are compressed by `chws` and a break between them is allowed, the frontend emits each at its compressed width with a `CjkPunctAdjust` discretionary between them. `net_pre` is the signed delta added to the last cluster of the preceding text box when the break is taken, `net_post` the delta added to the first cluster of the following one. At least one is non-zero; a non-zero one needs that neighbouring box, and the cluster's width plus the delta stays non-negative.

**[WN-25] Inline maths.** An external maths engine decomposes a top-level formula into `InlineObject` items separated by `MathOperator` discretionaries after relations (penalty 500) and binary operators (700). A sub-formula in braces, a fraction or a radical is one object with no break inside.

### 2.5 The paragraph AST

```rust
pub struct ParagraphAst {
    pub writing_mode: WritingMode,
    pub bidi_spans: Vec<ExplicitBidiSpan>,
    pub annotations: Vec<AnnotationSpan>,
    pub base_direction: BidiDirection,
    pub strut_metrics: FontMetrics,
    pub strut_font_size: f32,
    pub runs: Vec<ShapingRun>,
    pub atoms: Vec<GraphemeAtom>,
    pub clusters: Vec<ShapedCluster>,
    pub glyphs: Vec<RawShapedGlyph>,
    pub items: Vec<WordNode>,
    pub decoration_spans: Vec<InlineDecorationSpan>,
    pub atomic_spans: Vec<AtomicEditSpan>,
}
pub struct ShapingRun {
    pub baseline_align: BaselineAlignment,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub orientation: GlyphOrientation,
    pub features: FontFeatures,
    pub byte_range: TextRange,
    pub families: FamilyStack,
    pub font_size: f32,
    pub style: FontStyle,
    pub script: ScriptTag,
    pub locale: LocaleTag,
    pub bidi_level: BidiLevel,
    pub decoration: TextDecoration,
    pub atom_range: Range<usize>,
    pub cluster_range: Range<usize>,
}
pub struct TextDecoration { pub underline: bool, pub strikethrough: bool }
pub struct TextBoxView<'a> {
    pub run_index: usize,
    pub byte_range: TextRange,
    pub atom_start: usize,
    pub atoms: &'a [GraphemeAtom],
    pub clusters: &'a [ShapedCluster],
    pub natural_width: f32,
}
```

**[AST-1]** One `ParagraphAst` is one UAX #9 paragraph. Offsets inside it are paragraph-local. It owns all its data and stores no text: shapers and validators receive the paragraph text per call.

**[AST-2]** The text contains no paragraph separator: no UAX #9 class-B character (LF, CR, NEL U+0085, PS U+2029, U+001C–U+001E) and neither VT U+000B nor FF U+000C (`is_paragraph_separator`). U+2028 ends a line, not the paragraph. `split_paragraphs` (§3) is how a caller gets there.

**[AST-3]** Item ranges are monotone (`prev.end <= next.start`; zero-width nodes may share an offset) and the non-empty ones concatenate to exactly `0..text.len()`. Among zero-width nodes at one offset, a `CjkAutospace` glue precedes every break opportunity.

**[AST-4] Storage.** Shaped text is stored once per paragraph in three flat arrays: `atoms`, the `clusters` that group them and the `glyphs` that render them, each in run order and then logical order. The runs' `atom_range` tiles `atoms` and their `cluster_range` tiles `clusters`, in run order. The clusters of each run partition its atoms (their run-local `atom_range` are non-empty and tile the run's atom slice in order). The clusters' `glyph_range` tiles `glyphs` in cluster order. In item order, the text boxes of one run take consecutive non-empty cluster ranges inside the run's and together cover it exactly, so every text box resolves. `ParagraphAst::run_atoms`, `ParagraphAst::run_clusters`, `ParagraphAst::cluster_glyphs` and `ParagraphAst::text_box` resolve the slices.

**[AST-5] Runs.** A run is homogeneous in family stack, style, size, features, script, locale, bidi level, orientation and decoration; its atoms and clusters repeat its script, bidi level and orientation and must agree with it (ATM-1, CLU-2). Its `byte_range` is the run's extent in the source, which MAY include bytes owned by glue or other non-text items between its text boxes; every text box of the run lies inside it. Body runs ascend and do not overlap. A run's size is finite and positive, its script resolved (not `Zyyy` or `Zinh`), its letter and word spacing finite, its explicit baseline shift finite, and its orientation valid for the writing mode and its level (§2.7). Non-zero letter spacing is forbidden on a run that holds a `NO_LETTER_SPACING` atom. `ShapingRun::direction` is RTL exactly when its level is odd. `atom_range` and `cluster_range` only locate the run's slices.

**[AST-6] Strut.** `strut_metrics` are the raw metrics of the primary font, as the shaper returned them for the requested variation instance and `strut_font_size`; the frontend never adjusts them for a line-height policy. They are valid `FontMetrics` (FNT-4) and `strut_font_size` is finite and positive.

**[AST-7]** `decoration_spans` ascend by start and, for equal starts, descend by end; ids are unique; spans nest properly (disjoint, or one contains the other; equal ranges are allowed, the lower index being the outer); depth is at most `MAX_DECORATION_NESTING_DEPTH` (8); paddings are valid (SPN-1); every range fits the text.

**[AST-8]** Items respect span enclosure: every item inside a decoration span's range lies strictly between its `Leading` and `Trailing` boundaries, every item outside it does not, and no item straddles an edge. A break opportunity or `CjkAutospace` glue at the span's start precedes the `Leading` boundary and one at its end follows the `Trailing` boundary. Boundaries nest LIFO: at a shared start the outer span's `Leading` precedes the inner's, at a shared end the inner's `Trailing` precedes the outer's. Adjacent spans meet as `[A Trailing, Glue(CjkAutospace), Discretionary, B Leading]`.

**[AST-9]** `atomic_spans` ascend by start and are pairwise disjoint; ids are unique; ranges are non-empty and fit the text. Both edges are the start or end of a text atom or of an inline object, never inside glue, a tab or a line terminator, and the span holds at least one atom or object.

**[AST-13] Level floor.** Every level of the body lies at or above the paragraph's `Direction::base_level`, as UAX #9 guarantees: the levels of runs (and so of their atoms and clusters, AST-5), inline objects, glue and span boundaries. Tabs and hard breaks store none (WN-15).

**[AST-10]** The validator neither re-segments the text nor reruns UAX #9: accepting an AST does not certify it was produced by a conforming implementation. It never repairs input.

```rust
pub struct ValidatedInputs<'a> { .. }   // inputs that passed the input gate, with the resolved policy
pub enum GeometryAnswer { Open(LineGeometry), Obstructed { clear_at: f32 } }   // a provider answer that passed G3; an obstructed answer is empty, so only its `clear_at`, proved ahead, is kept
```

**[AST-11] Validation entry point.** `ValidatedInputs::try_from_ast(ast, text, constraints, options)` is the input gate (G1, AX-3): it returns the first violation, or a `ValidatedInputs` holding the inputs and the resolved policy (`ValidatedInputs::policy`). Violations are checked in this order, which is their precedence: the policy (constraints, options, then the strut); a paragraph separator in the text (AST-2); run and atom storage (AST-4); presentation (AST-5); bidi and annotation spans (BID-2, ANN-2); shaping runs (AST-5); decoration and atomic spans (AST-7, AST-9); items outside the text, then item order (AST-3); each item in order; the paragraph end (WN-17); span-boundary order, then span enclosure (AST-8); source coverage (AST-3). `check_layout_inputs` runs the same gate and returns `Ok(())`; `layout_paragraph` runs it first and propagates its error. Behind the gate, `ValidatedInputs::text_box` returns `None` only for an item that is not text, `ValidatedInputs::run_atoms`, `ValidatedInputs::run_clusters` and `ValidatedInputs::cluster_glyphs` index directly, and `ValidatedInputs::line_geometry` is the only geometry query layout makes (G3); it returns a `GeometryAnswer`, so an obstructed answer carries the offset the gate proved ahead. The raw lookups on `ParagraphAst` stay total for data that has not passed the gate. Its time is expected O(total input records and bytes) with O(atoms + boundaries + spans) extra memory for its edge indexes: it neither sorts nor copies the input, and no check rescans all items for each span or atom.

```rust
pub enum LayoutInputError {
    OrientationMismatch,
    InvalidFontMetrics,
    InvalidShaperMetrics,
    InvalidSpacing,
    InvalidBidiSpan { span: usize },
    InvalidAnnotationSpan { annotation: usize },
    InvalidLayoutOptions,
    InvalidLineGeometry,
    ClearanceExhausted,
    RefitExhausted,
    NonMonotonicGeometry,
    InvalidConstraints,
    ParagraphSeparatorInText,
    InvalidStrut,
    RunOutsideText { run: usize },
    InvalidShapingRun { run: usize },
    InvalidRunStorage { run: usize },
    InvalidGlyphStorage { cluster: usize },
    RunIndexOutOfRange { item: usize },
    ItemOutsideText { item: usize },
    UncoveredSource { byte: usize },
    ScriptMismatch { item: usize },
    ItemOutsideRun { item: usize },
    InvalidTextWidth { item: usize },
    ItemNotSelfContained { item: usize },
    InvalidInlineBox { item: usize },
    InvalidTab { item: usize },
    InvalidGlue { item: usize },
    InvalidDiscretionary { item: usize },
    InvalidHyphenOpportunity { item: usize },
    InvalidSpanBoundary { item: usize },
    UnknownSpan { item: usize },
    InvalidDecorationSpan { span: usize },
    MissingParagraphEnd,
    MissingSpanBoundary { span: usize },
    ItemsOutOfOrder { item: usize },
    SpanEnclosure { span: usize, item: usize },
    InvalidAtomicSpan { span: usize },
    InvalidBidiLevel { item: usize },
}
```

**[AST-12]** `LayoutInputError` is `#[non_exhaustive]`. Each variant has one root cause:

| Variant | Raised when | Enforces |
|---|---|---|
| `OrientationMismatch` | a run's, atom's or cluster's orientation disagrees with its run or does not suit the writing mode or its level, or a tate-chu-yoko run is not one cluster in one text box spanning the run | AST-5, ORI-3, ORI-4 |
| `InvalidFontMetrics` | the raw strut metrics are not valid `FontMetrics` although the strut's own ascent, descent and line gap pass (otherwise `InvalidStrut`) | AST-6, FNT-4 |
| `InvalidShaperMetrics` | the metrics a shaper answers at layout time are not valid `FontMetrics`, or resolve under the line height to an invalid line box (G2) | SHP-9, FNT-4, MET-1 |
| `InvalidSpacing` | a run's spacing or baseline shift is not a measurement (MSR-1), or a letter-spaced run holds a `NO_LETTER_SPACING` atom | AST-5 |
| `InvalidBidiSpan { span }` | an explicit bidi span breaks §2.8 | BID-2 |
| `InvalidAnnotationSpan { annotation }` | an annotation breaks §2.9 | ANN-2, ANN-3 |
| `InvalidLayoutOptions` | an invalid clamp policy | OPT-3 |
| `InvalidLineGeometry` | a provider answer outside the wrap envelope or out of order, or an obstructed answer without progress ahead | GEO-3, GEO-4 |
| `ClearanceExhausted` | more than 1024 obstructed provider answers in a row for one line | GEO-4 |
| `RefitExhausted` | one line still outgrew its envelope after 16 refits | GEO-6 |
| `NonMonotonicGeometry` | an expanded block envelope introduced inline space | GEO-4 |
| `InvalidConstraints` | a non-positive or NaN wrap width, a non-finite indent, or (without a provider) an indent outside `[0, wrap_width)` | GEO-2 |
| `ParagraphSeparatorInText` | the text holds a paragraph separator | AST-2 |
| `InvalidStrut` | the strut's own ascent, descent or line gap, the strut font size or a `LineHeight::Fixed` multiplier is not valid, or the resolved strut is invalid | AST-6, MET-1 |
| `RunOutsideText { run }` | a run's range exceeds the text or splits a UTF-8 sequence | AST-5 |
| `InvalidShapingRun { run }` | a run's size is not a positive measurement, its script is unresolved, or a body run starts before the previous one ends | AST-5, MSR-1 |
| `InvalidRunStorage { run }` | the runs do not tile the atoms or clusters, or a run's text boxes do not cover its atoms and clusters | AST-4 |
| `InvalidGlyphStorage { cluster }` | the clusters' glyph ranges do not tile the glyphs | AST-4 |
| `RunIndexOutOfRange { item }` | a text box names a run that does not exist | AST-4 |
| `ItemOutsideText { item }` | an item's bytes exceed the text or split a UTF-8 sequence | AST-3 |
| `UncoveredSource { byte }` | the items leave a byte unowned; `byte` is the first | AST-3 |
| `ScriptMismatch { item }` | an atom of a text box has a script other than its run's | ATM-1, AST-5 |
| `ItemOutsideRun { item }` | a text box lies outside its run's bytes, or one of its atoms or clusters has another bidi level than its run | AST-5, CLU-2 |
| `InvalidTextWidth { item }` | a box's `natural_width` is not the sum of its clusters', or a cluster width, scale or raw glyph measurement is not a measurement | WN-6, CLU-6, MSR-1 |
| `ItemNotSelfContained { item }` | a text box has no atoms, leaves its run's ranges, is out of order among its run's boxes, has atoms that are not contiguous or do not tile its bytes, has clusters that do not partition its atoms, or has a cluster whose bytes, flags or safety flags disagree with its atoms | WN-5, CLU-5, AST-4 |
| `InvalidInlineBox { item }` | an inline object's size is not a non-negative measurement, an invalid baseline alignment, or no bytes | WN-7 |
| `InvalidTab { item }` | a tab owns no bytes | WN-15 |
| `InvalidGlue { item }` | a glue size is not a non-negative measurement, a compression pool is not shrink only or shrinks more than half its run's font size, a glue is not collapsible, its `original_bidi_class` is not `Ws` for a `WordSpace` or `Bn` for synthetic glue, or the glue's bytes do not match its kind | WN-10, WN-12 to WN-14 |
| `InvalidDiscretionary { item }` | a width is not a measurement, the penalty is below the mandatory penalty, forbidden or disagrees with the action, the bytes break WN-17, `cloned_spans` repeats or misses a span, or the widths disagree with WN-21 or WN-24 | WN-16 to WN-18, WN-21, WN-24 |
| `InvalidHyphenOpportunity { item }` | a `Hyphenate` break is not flagged, not between two same-run boxes that meet at it, or has a cut off an atom boundary or outside its chain | WN-20, WN-22 |
| `InvalidSpanBoundary { item }` | a boundary is not empty, not at its anchor, has the wrong width, a level other than that of the first or last inline entity inside its span (the base level when it holds none), or breaks LIFO order | WN-8, AST-8 |
| `UnknownSpan { item }` | an item names a decoration span that does not exist, or lists in `cloned_spans` a span that is not `Clone` or does not contain the break strictly inside | WN-8, WN-21 |
| `InvalidDecorationSpan { span }` | a decoration span breaks AST-7 | AST-7, SPN-1 |
| `MissingParagraphEnd` | the items do not end with the sentinel | WN-17 |
| `MissingSpanBoundary { span }` | a decoration span lacks a boundary | WN-8 |
| `ItemsOutOfOrder { item }` | an item starts before the previous one ends, or a break opportunity precedes a `CjkAutospace` at one offset | AST-3 |
| `SpanEnclosure { span, item }` | an item is on the wrong side of a span's boundaries or straddles an edge | AST-8 |
| `InvalidAtomicSpan { span }` | an atomic span breaks AST-9 | AST-9 |
| `InvalidBidiLevel { item }` | a text box's run, an inline object, a glue or a span boundary has a level below the paragraph's base level | AST-13 |

### 2.6 Positioned output

```rust
pub struct PositionedParagraph {
    pub policy: LayoutPolicySnapshot,
    pub base_direction: BidiDirection,
    pub truncated_range: Option<TextRange>,
    pub total_block_size: f32,
    pub lines: Vec<PositionedLine>,
}
pub struct LayoutPolicySnapshot {
    pub writing_mode: WritingMode,
    pub strut: StrutMetrics,
    pub line_height: LineHeight,
    pub whitespace: HangingWhitespaceMode,
    pub hang_mode: HangMode,
    pub wrap_width: f32,
    pub slot_source: SlotSource,
    pub clamp: Option<LineClampPolicy>,
}
pub enum SlotSource {
    Standard { first_line_indent: f32, hanging_indent: f32 },
    Provider,
}
pub struct LineContext {
    pub writing_mode: WritingMode,
    pub base_direction: BidiDirection,
    pub strut: StrutMetrics,
    pub line_height: LineHeight,
    pub whitespace: HangingWhitespaceMode,
    pub hang_mode: HangMode,
    pub wrap_width: f32,
    pub line_index: usize,
}
pub struct PositionedLine {
    pub slots: SmallVec<[PositionedSlot; 1]>,
    pub is_ellipsized: bool,
    pub annotations: Vec<PositionedAnnotationRun>,
    pub atomic_fragments: Vec<AtomicFragment>,
    pub byte_range: TextRange,
    pub visible_byte_end: ByteIndex,
    pub baseline_y: f32,
    pub ascent: f32,
    pub descent: f32,
    pub line_height: f32,
    pub is_justified: bool,
    pub visual_runs: Vec<VisualGlyphRun>,
    pub visual_boxes: Vec<PositionedInlineBox>,
    pub tabs: Vec<PositionedTab>,
    pub span_fragments: Vec<PositionedSpanFragment>,
    pub caret_stops: Vec<CaretStop>,
    pub decorations: Vec<DecorationLine>,
}
pub struct PositionedSlot { pub constraint: InlineSlotConstraint, pub content_start: f32, pub visual_width: f32 }
pub enum HangSide { Left, Right }
pub enum GlyphRole { Trailing, ZeroAdvance, Hanging, Body }
pub struct VisualGlyphRun {
    pub orientation: GlyphOrientation,
    pub font_size: f32,
    pub variations: SmallVec<[FontVariation; 2]>,
    pub font: FontRef,
    pub bidi_level: BidiLevel,
    pub byte_range: TextRange,
    pub glyphs: Vec<PositionedGlyph>,
}
pub struct PositionedGlyph {
    pub glyph_id: u32,
    pub x: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub advance: f32,
    pub byte_range: Option<TextRange>,
}
pub struct PositionedInlineBox {
    pub baseline_align: BaselineAlignment,
    pub box_id: u64,
    pub byte_range: TextRange,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub overflow_policy: BoxOverflowPolicy,
    pub bidi_level: BidiLevel,
}
pub struct PositionedTab { pub byte_range: TextRange, pub x: f32, pub width: f32 }
pub struct PositionedSpanFragment {
    pub span_id: u64,
    pub span_range: TextRange,
    pub byte_range: TextRange,
    pub x: f32,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub bidi_level: BidiLevel,
}
pub struct AtomicFragment {
    pub atomic_id: u64,
    pub full_byte_range: TextRange,
    pub fragment_byte_range: TextRange,
    pub x: f32,
    pub width: f32,
    pub bidi_level: BidiLevel,
}
pub struct PositionedAnnotationRun {
    pub annotation_id: u64,
    pub base_byte_range: TextRange,
    pub side: AnnotationSide,
    pub font: FontRef,
    pub font_size: f32,
    pub variations: SmallVec<[FontVariation; 2]>,
    pub orientation: GlyphOrientation,
    pub baseline_offset: f32,
    pub max_overhang: f32,
    pub block_extent: f32,
    pub base_over: f32,
    pub base_under: f32,
    pub glyphs: Vec<PositionedGlyph>,
}
pub enum DecorationKind { Underline, Strikethrough }
pub struct DecorationLine {
    pub kind: DecorationKind,
    pub byte_range: TextRange,
    pub x: f32,
    pub width: f32,
    pub y_offset: f32,
    pub thickness: f32,
}
pub enum CaretAffinity { Downstream, Upstream }
pub struct CaretStop { pub x: f32, pub offset: ByteIndex, pub affinity: CaretAffinity, pub level: BidiLevel }
```

**Coordinates.**

**[OUT-1]** Lines are in canonical flow space (§2.7): `x` is line-local and inline, `baseline_y` block-progressing. A glyph's `x` is its **pen position**, not its drawing origin. Its offsets are flow-relative: `x_offset` runs along the inline axis and `y_offset` towards line-over, in every writing mode and orientation (SHP-19). It is drawn at `x + x_offset` and `baseline_y + over_to_block(y_offset)` (ORI-2), which is `baseline_y - y_offset` except in `VerticalLr`. Containment is judged on the pen interval `[min(x, x + advance), max(x, x + advance)]`; an advance MAY be negative. A zero-advance mark is a pen position, not an extent, and is exempt from containment.

**[OUT-2]** Equality and ordering of output geometry use the tolerance `max(LINE_GEOMETRY_ABS_EPSILON, LINE_GEOMETRY_REL_EPSILON · max(|a|, |b|))` with `LINE_GEOMETRY_ABS_EPSILON = 1e-3` and `LINE_GEOMETRY_REL_EPSILON = 1e-5`, everywhere and only in that form. A finite and an infinite value are never approximately equal.

**[OUT-3]** Glyph runs are in screen order, left to right, and the glyphs of a run too: pen positions never decrease, within a run and from the last glyph of one run to the first of the next. Inline boxes and tabs are in logical order.

**Derived line geometry.**

**[OUT-4]** A line's body width is the sum of its slots' content widths (`PositionedLine::visual_width`). A glyph has one role (`GlyphRole`, derived from its position and never stored, AX-2), the first that applies: *trailing* (its bytes start at or after `visible_byte_end`), *zero-advance* (`advance == 0`), *hanging* (no slot's content block holds its pen interval), or *body*. "Holds" and "beyond tolerance" both use one tolerance per line, at the scale of its body width, so a glyph is hanging exactly when it overflows the body beyond that tolerance. The hanging extents are the largest overflow of a hanging glyph's pen interval, beyond tolerance, beyond the left edge of the first occupied slot's content (`PositionedLine::hanging_left`) and beyond the right edge of the last one's (`PositionedLine::hanging_right`); `PositionedLine::hanging_glyphs` yields the hanging glyphs. None of these is stored. A slot is occupied when it has a non-zero content width or holds a visible glyph with source bytes, an inline box or a visible tab; the first and last occupied slots bound the body (`BodyExtent`). Occupancy is decided by the content blocks and the tolerance alone, never by roles: a glyph whose pen interval lies within tolerance of a zero-width slot is held by that slot (so it is body, or zero-advance, never hanging) and makes the slot occupied. So occupancy and roles are not circular: no slot, used or unused, holds a hanging glyph.

**Slots and the content block.**

**[SLT-1]** A line has one `PositionedSlot` per available interval of its geometry (§5), carrying the `InlineSlotConstraint` it was laid out in, so validation never queries a provider. A line has at least one slot. The constraints ascend, are disjoint and lie in the line's inline envelope (`InlineSlotConstraint::lies_in_envelope`: `start >= 0` and, for a finite `wrap_width`, `end <= wrap_width`, within tolerance).

**[SLT-2]** A slot's content occupies the block `[content_start, content_start + visual_width]` (`PositionedSlot::content_end`), finite and non-negative, starting inside its constraint. The constraint is divided exactly into alignment offset `[constraint.start(), content_start)`, the content block, and slack.

**[SLT-3]** Each content block is exactly the span of the content placed in it. Content is body glyphs (by pen interval), inline boxes, visible tabs and decoration fragments (whose padding is advance no glyph covers). Trailing whitespace, zero-advance glyphs and hanging glyphs are not content, except that the part of a hanging glyph inside the block counts for the slot it hangs from. The line-end edge of the block is exact: in an LTR paragraph the content's right edge is `content_end()`, in an RTL paragraph its left edge is `content_start`. Only the line-start edge MAY stand off from the content, by the width of preserved leading glue. `WordSpace` glue has no output entity anywhere on a line (WN-10): inside the body it is the gap between entities, and its bytes are reached through caret stops alone. A slot without content has `visual_width == 0`.

**[SLT-4]** Every body glyph, inline box, visible tab, span fragment and atomic fragment lies wholly inside one slot's content block.

**[SLT-5] Overflow.** A content block ends inside its constraint, and every `Clip` or `ScaleToFit` inline box ends inside its slot (WN-7), except in the single-entity overflow state: the line has exactly one slot, its content starts at the constraint's start and is wider than the constraint, and it is one indivisible entity, either (A) exactly one `AllowOverflow` inline box and no glyph run or tab, or (B) the glyphs of one indivisible cluster (the emergency cluster break), with no box or tab; the entity's width is the block's, or, inside padded decoration spans, the fragments reach both edges of the block while every fragment starts at the entity's start and the entity reaches the visible end. A synthesized-only ellipsis suffix MAY accompany either. The slot need not be as wide as `wrap_width`: an entity too wide for an indented line or for a provider's only slot is placed there all the same, from the slot's start.

**Source ownership.**

**[SRC-1]** A glyph is synthesized exactly when its `byte_range` is `None` (`PositionedGlyph::is_synthesized`, AX-1). A present range is non-empty and lies inside the line's `byte_range`. Glyph ranges are whole clusters. Synthesized glyphs are never copied to the clipboard (§6.6).

**[SRC-2]** The bytes of every visual run, inline box (never empty), tab (never empty), decoration, caret stop, span fragment and atomic fragment lie inside the line's `byte_range`.

**[SRC-3]** `byte_range` includes the line's trailing whitespace; `visible_byte_end` lies inside it and is a boundary of its content: the line's start or end, or the start or end of a non-synthesized glyph, an inline box or a tab. Glyphs and tabs whose bytes start at or after `visible_byte_end` are trailing whitespace.

**[SRC-4]** A visual run's orientation suits the writing mode and its level (§2.7); its font size and fallback scale are finite and positive; it has at least one glyph.

**Line box.**

**[BOX-1]** `baseline_y`, `ascent`, `descent` and `line_height` are finite and `line_height` positive. Under `LineHeight::Fixed` every line's `line_height`, `ascent` and `descent` equal the strut's: a fixed line box never grows, for fallback glyphs or annotations. Under `LineHeight::Normal`, `line_height` is at least the strut's.

**[BOX-2]** An empty line (`byte_range` empty, not ellipsized) has no visual run, box, tab, span fragment, atomic fragment or annotation, zero content width, and the strut's line height.

**[BOX-3]** An ellipsized line is the last line of a clamped paragraph. An ellipsized line with an empty `byte_range` holds only synthesized glyphs (or none, for an empty ellipsis).

**Hanging punctuation and trailing whitespace.**

**[HNG-1]** A hanging glyph (OUT-4) overflows the body beyond tolerance; it MAY straddle the body edge. It hangs from the outer slot on its side: the part of it that is not overflow lies in that slot's content. Unused slots and internal obstacles hold no hanging glyph (see OUT-4 for occupancy). The glyph MAY lie wholly beyond the body edge, with no part of it in the slot's content: the part that is not overflow is then empty, so the condition holds vacuously and the glyph is accepted as hanging, subject to HNG-2. It must be *outer-clear* (`BodyExtent::is_outside_clear`): it passes a body edge only on a side where the line has no further slot, so it never reaches into an internal obstacle or an unused slot. A glyph outside every slot's content that does not hang this way is a `BodyGlyphOutsideBlock`; one that does is a hanging glyph, whatever laid it out, and is then subject to HNG-2 and HNG-4.

**[HNG-2]** A side hangs only where `HangMode::allows(side, base_direction, line_index == 0)` permits (§5 Stage 1.3). Under `HangEndMode::Allow` a mark hangs at the line end only if it would otherwise wrap: the slot it hangs from is too narrow for its body plus the overflow (an unbounded slot never needs to hang). `HangEndMode::Force` hangs regardless.

**[HNG-4] One mark per side.** At most one character (one source byte range) hangs on each physical side of a line, and it is the outermost glyph on that side: no other body glyph's pen interval reaches further towards that side. "Other body glyph" means every glyph that is neither hanging nor trailing whitespace and is not one of the mark's own glyphs, zero-advance glyphs included: a zero-advance glyph past the mark is a `HangingGlyphNotOutermost`. A glyph with an advance that reaches past the mark is itself hanging (OUT-4), and a second character there is a `TooManyHangingGlyphs`. A hanging mark's level is the paragraph's base level, so it lies at the physical edge after L2 (ST1-3).

**[HNG-3]** Trailing whitespace is never hanging (its role is *trailing*, OUT-4) and never counts towards the hanging extents. Trailing `WordSpace` glue has no glyph (WN-10): under `HangingWhitespaceMode::Preserved` it contributes only caret stops beyond the last visible entity, on the far edge of the base direction. The glyph and tab rules below bind any trailing whitespace that does have an entity, such as a tab, U+3000 or a whitespace glyph from a caller's own AST. Under `HangingWhitespaceMode::Collapsible` such glyphs and tabs have zero width. Under `HangingWhitespaceMode::Preserved` they are exempt from SLT-4 but lie on the far edge of the base direction (UAX #9 L1): right of the body in an LTR paragraph, left of it in an RTL one.

**Decoration span fragments.**

**[FRG-1]** A `PositionedSpanFragment` is one post-bidi group of a decoration span on a line: a maximal sequence of the span's items that is contiguous both in visual order and in logical order, so that its bytes are one range (ST2-3). A visual group whose logical bytes have a gap splits there. Its `span_range` is the span's whole range, the same on every fragment of the span; its `byte_range` lies inside `span_range` and inside the line. `PositionedSpanFragment::is_first_fragment` (it starts where the span starts) and `PositionedSpanFragment::is_last_fragment` (it ends where the span ends) follow from the two ranges. Its sizes are finite and non-negative.

**[FRG-2]** Every glyph (hanging, trailing-whitespace and synthesized glyphs excepted) and inline box whose bytes lie inside a fragment of span *S* lies inside at least one fragment of *S*. The check is containment, not tiling.

**Atomic fragments.**

**[FRG-3]** An `AtomicFragment` is formed like a decoration fragment (FRG-1), contiguous visually and logically. It carries its span's `full_byte_range`, the same on every fragment of the span, and a non-empty `fragment_byte_range` inside it and inside the line. The fragments of one span on a line agree on `full_byte_range`, are disjoint, and together cover exactly the part of the span on the line. Geometry is finite with a non-negative width.

**Caret stops.**

**[CRT-1]** `caret_stops` are in visual order (non-decreasing `x`), lie inside the line's bytes and name the line's own index.

**[CRT-3] Split carets.** At a direction boundary an offset has two stops on one line at different positions: the stop at the trailing visual edge of the entity that ends at the offset is `CaretAffinity::Upstream` and carries that entity's level, and the stop at the leading visual edge of the entity that starts there is `CaretAffinity::Downstream` and carries its level. An offset with one position keeps the affinity INT-20 gives it.

**[CRT-2]** When a glyph, inline box or tab has bytes that contain or touch a stop's offset, the stop's level is the level of one of them (a glyph lends its visual run's level, a box its own, a tab the paragraph level); at a boundary either neighbour qualifies. An offset inside the line's `byte_range` that no entity touches lies in bytes that emit no entity, such as multi-space or preserved trailing `WordSpace` glue (WN-10); its stop carries the paragraph level or the level of the nearest entity before or after it in byte order. On a line with no glyph, box or tab that is the paragraph level.

**Decorations.**

**[DEC-1]** A `DecorationLine`'s `y_offset` is an offset from the baseline positive towards line-over (FNT-4): a renderer draws the stroke at `baseline_y + over_to_block(y_offset)`, so "the lowest `underline_position`" of ST5-1 is the minimum. A `DecorationLine` has finite coordinates, a non-negative width and thickness, and bytes inside the line.

**Annotations.**

**[ANO-1]** A `PositionedAnnotationRun` attaches to a non-empty visible base: `base_byte_range` lies inside `byte_range.start()..visible_byte_end` and in exactly one slot. Every non-synthesized body glyph intersecting the base lies wholly inside it and in that slot, and at least one exists; glyph-less source (glue) MAY lie inside the base, so its envelope is the union of the intersecting body glyphs' intervals.

**[ANO-2]** Its glyphs carry `byte_range: None` (AX-1) and finite geometry, and lie within both `max_overhang` beyond the base envelope and the slot. Its font size and scale are positive; its extents finite and non-negative; its orientation valid for the writing mode at level 0.

**[ANO-3]** `base_over` and `base_under` are the producer-measured largest extents of the base's glyphs and boxes towards line-over and line-under: a local fact of the run. The annotation sits no closer to the baseline than its base: `baseline_offset >= base_over` for `Over`, `baseline_offset <= -base_under` for `Under`. Under `LineHeight::Normal` the line's ascent (descent) is at least the larger of the strut's and `base_over + block_extent` (`base_under + block_extent`); under `Fixed` the box stays the strut's and the annotation overflows.

**[ANO-4]** Annotation ids are unique on a line and in the paragraph; two annotations on one side never share base bytes; an empty line has none.

**Validating a line.**

**[LV-1]** `PositionedLine::validate(ctx)` checks a line against a `LineContext` and returns the first violation; `PositionedLine::is_valid` is its boolean form. The context is the paragraph's policy plus the base direction and the line's index (`LayoutPolicySnapshot::line_context`, `LineContext::for_paragraph`). There is no default context.

**Validating a paragraph.**

**[PAR-1]** `LayoutPolicySnapshot::resolve(ast, constraints, options)` resolves the policy once: it validates the constraints and clamp, resolves the strut with `LineHeight::resolve_strut` from the AST's raw metrics, `strut_font_size` and writing mode, and records `SlotSource::Standard` with both indents when no geometry provider is set, `SlotSource::Provider` otherwise. `LayoutPolicySnapshot::is_valid` holds for every snapshot `resolve` can produce: a valid strut, line height and clamp, a positive wrap width, and standard indents in `[0, wrap_width)`.

**[PAR-2]** `PositionedParagraph::validate_geometry()` reads only the paragraph: a valid policy; at least one line; each line valid under `policy.line_context(base_direction, i)`; under `SlotSource::Standard`, each line has exactly its one standard slot (`SlotSource::standard_slot`: `[first_line_indent, wrap_width]` on line 0 and `[hanging_indent, wrap_width]` after); line boxes from `WritingMode::line_bounds` that are finite, forward and non-overlapping; `ascent + descent <= line_height`; and a finite `total_block_size` no smaller than the last line's bottom. Under `SlotSource::Provider` only the slots' shape is checked: a provider's answers cannot be recomputed after layout.

**[PAR-3]** `PositionedParagraph::validate(text)` adds what needs the source text: the lines' ranges tile the rendered text from 0 on character boundaries; an unclamped paragraph renders all of `text` with no ellipsized line; a clamped one has a clamp policy, a non-empty `truncated_range` from the last line's end to `text.len()`, and exactly its last line ellipsized; annotation ids are unique across lines; the fragments of one decoration span carry one `span_range` and hold exactly one first and one last fragment (the last MAY fall in the truncated tail); the fragments of one atomic span carry one `full_byte_range` and cover every rendered byte of it. It runs in O(n log n) in the size of the output (AX-7).

```rust
pub struct ValidatedLayout<'a> { .. }   // a PositionedParagraph that passed the output gate, with its text
```

**[PAR-5] Output gate.** `ValidatedLayout::try_from_paragraph(paragraph, text)` runs `PositionedParagraph::validate` once (G4, AX-3) and returns its first violation or a `ValidatedLayout` that holds the paragraph and the text (`ValidatedLayout::paragraph`, `ValidatedLayout::text`). The queries of §6 take it as their receiver and do not check the output again.

**[PAR-4]** `total_block_size` includes clearance and is the authority for coordinate transforms (§2.7). `base_direction` is the AST's.

```rust
pub enum LineInvariantError {
    InvalidSlots { reason: SlotErrorReason },
    OrientationMismatch,
    UnplacedTateChuYoko { run: usize },
    InvalidGlyphSource { run: usize, glyph: usize },
    InvalidAnnotation { index: usize, reason: AnnotationErrorReason },
    InvalidParagraph { reason: ParagraphErrorReason },
    NonFiniteLineMetrics,
    NonPositiveLineHeight { expected: f32, actual: f32 },
    VisibleEndOutsideRange,
    VisibleEndNotOnBoundary,
    EmptyLineHasVisualRuns,
    EmptyLineHasBoxes,
    EmptyLineHeightMismatch { expected: f32, actual: f32 },
    EmptyLineHasExtent,
    LineHeightMismatch { expected: f32, actual: f32 },
    LineMetricsMismatch { expected_ascent: f32, expected_descent: f32, actual_ascent: f32, actual_descent: f32 },
    LineHeightBelowStrut { minimum: f32, actual: f32 },
    InvalidVisualRun { run: usize },
    GlyphsOutOfVisualOrder { run: usize, glyph: usize },
    InvalidAtomicFragment { index: usize },
    NonFiniteGlyph { run: usize, glyph: usize },
    TrailingWhitespaceNotCollapsed { run: usize, glyph: usize },
    BodyGlyphOutsideBlock { run: usize, glyph: usize },
    InvalidInlineBox { index: usize },
    InlineBoxOutsideBlock { index: usize },
    EmptyLineHasTabs,
    EmptyLineHasSpanFragments,
    EmptyLineHasAtomicFragments,
    InvalidSpanFragment { index: usize },
    SpanFragmentOutsideBlock { index: usize },
    SpanFragmentOutsideLine { index: usize },
    SpanFragmentOutsideSpan { index: usize },
    GlyphOutsideSpanFragment { span_id: u64, run: usize, glyph: usize },
    InlineBoxOutsideSpanFragment { span_id: u64, index: usize },
    InlineBoxWiderThanWrapWidth { index: usize },
    LineWiderThanWrapWidth { visual_width: f32, wrap_width: f32 },
    SlotOverflow { slot: usize, extent: f32, capacity: f32 },
    InvalidTab { index: usize },
    TabOutsideBlock { index: usize },
    TrailingTabNotCollapsed { index: usize },
    TrailingWhitespaceOnWrongSide { run: usize, glyph: usize },
    TrailingTabOnWrongSide { index: usize },
    InvalidCaretStop { index: usize },
    InvalidDecoration { index: usize },
    HangingNotAllowed { side: HangSide },
    HangingWithoutOverflow { side: HangSide },
    VisualRunsOutOfOrder { run: usize },
    TooManyHangingGlyphs { side: HangSide },
    HangingGlyphNotOutermost { run: usize, glyph: usize },
}
pub enum SlotErrorReason { InvalidWrapWidth, NoSlots, InvalidConstraint, InvalidContent, WidthNotContentSpan, ContentOutsideSlots }
pub enum AnnotationErrorReason {
    OnEmptyLine, InvalidMetrics, OrientationMismatch, BaseOutsideLine, NoGlyphs, InvalidGlyph, DuplicateId,
    OverlapsOnSide, GlyphStraddlesBase, NoBaseGlyphs, BaseSpansSlots, OutsideOverhang, InsideBase, DoesNotFitLineBox,
}
pub enum ParagraphErrorReason {
    InvalidPolicy, NoLines, InvalidBlockSize, LineBoxesOverlap, LineBoxExceedsLineHeight, NotStandardSlots,
    SourceNotCovered, InvalidTruncation, DuplicateAnnotation, InconsistentSpanFragments, InconsistentAtomicFragments,
}
```

**[LV-2]** All four enums are `#[non_exhaustive]`; `PositionedParagraph::validate_geometry` and `PositionedParagraph::validate` report `(line_index, error)`, a paragraph-level violation with the line where it was found.

| Variant | Raised when | Enforces |
|---|---|---|
| `InvalidSlots { reason }` | see the slot reasons below | SLT-1 to SLT-3 |
| `OrientationMismatch` | a visual run's orientation does not suit the writing mode or its level | SRC-4 |
| `UnplacedTateChuYoko { run }` | a visual run is a tate-chu-yoko group, whose placement is not defined yet | ORI-7 |
| `InvalidGlyphSource { run, glyph }` | a glyph's range is empty or outside the line, or an ellipsis-only line holds a source glyph | SRC-1, BOX-3 |
| `InvalidAnnotation { index, reason }` | see the annotation reasons below | ANO-1 to ANO-4 |
| `InvalidParagraph { reason }` | see the paragraph reasons below | PAR-2, PAR-3 |
| `NonFiniteLineMetrics` | a line metric is not finite | BOX-1 |
| `NonPositiveLineHeight { expected, actual }` | a line height or the strut height is not positive | BOX-1 |
| `VisibleEndOutsideRange` | `visible_byte_end` lies outside `byte_range` | SRC-3 |
| `VisibleEndNotOnBoundary` | `visible_byte_end` is not a content boundary | SRC-3 |
| `EmptyLineHasVisualRuns`, `EmptyLineHasBoxes`, `EmptyLineHasTabs`, `EmptyLineHasSpanFragments`, `EmptyLineHasAtomicFragments` | an empty line carries that entity | BOX-2 |
| `EmptyLineHeightMismatch { expected, actual }` | an empty line's height is not the strut's | BOX-2 |
| `EmptyLineHasExtent` | an empty line's slots hold a non-zero width | BOX-2 |
| `LineHeightMismatch { expected, actual }` | under `Fixed`, the line height is not the strut's | BOX-1 |
| `LineMetricsMismatch { .. }` | under `Fixed`, ascent or descent is not the strut's | BOX-1 |
| `LineHeightBelowStrut { minimum, actual }` | under `Normal`, the line height is below the strut's | BOX-1 |
| `InvalidVisualRun { run }` | a run has no glyph, bytes outside the line, or a non-positive size or scale | SRC-2, SRC-4 |
| `GlyphsOutOfVisualOrder { run, glyph }` | a pen position lies left of the previous one in its run | OUT-3 |
| `VisualRunsOutOfOrder { run }` | the first pen position of a run lies left of the last one of the previous run | OUT-3 |
| `InvalidAtomicFragment { index }` | an atomic fragment breaks FRG-3 | FRG-3 |
| `NonFiniteGlyph { run, glyph }` | a glyph coordinate is not finite | OUT-1 |
| `TrailingWhitespaceNotCollapsed { run, glyph }`, `TrailingTabNotCollapsed { index }` | trailing whitespace has width under `Collapsible` | HNG-3 |
| `TrailingWhitespaceOnWrongSide { run, glyph }`, `TrailingTabOnWrongSide { index }` | preserved trailing whitespace is not on the far edge of the base direction | HNG-3 |
| `BodyGlyphOutsideBlock { run, glyph }` | a body glyph lies outside every content block, or a hanging glyph does not hang from an outer slot | SLT-4, HNG-1 |
| `HangingNotAllowed { side }` | punctuation hangs where the hang mode forbids it | HNG-2 |
| `HangingWithoutOverflow { side }` | under `Allow`, a mark hangs although it would have fitted | HNG-2 |
| `TooManyHangingGlyphs { side }` | more than one character hangs on one side | HNG-4 |
| `HangingGlyphNotOutermost { run, glyph }` | a body glyph reaches further towards the side a glyph hangs on | HNG-4 |
| `InvalidInlineBox { index }` | a box has a non-finite coordinate, a negative size, an invalid alignment, or bytes empty or outside the line | SRC-2, WN-7 |
| `InlineBoxOutsideBlock { index }` | a box lies outside every content block | SLT-4 |
| `InlineBoxWiderThanWrapWidth { index }` | a `Clip` or `ScaleToFit` box ends beyond `wrap_width` | SLT-5 |
| `LineWiderThanWrapWidth { visual_width, wrap_width }` | the body is wider than `wrap_width` outside the single-entity state | SLT-5 |
| `SlotOverflow { slot, extent, capacity }` | a block ends beyond its constraint while the line fits `wrap_width`, outside the single-entity state | SLT-5 |
| `InvalidTab { index }` | a tab has a non-finite coordinate, a negative width, or bytes empty or outside the line | SRC-2 |
| `TabOutsideBlock { index }` | a visible tab lies outside every content block | SLT-4 |
| `InvalidSpanFragment { index }` | a fragment has a non-finite coordinate or a negative size | FRG-1 |
| `SpanFragmentOutsideBlock { index }` | a fragment lies outside every content block | SLT-4 |
| `SpanFragmentOutsideLine { index }` | a fragment's bytes lie outside the line | SRC-2 |
| `SpanFragmentOutsideSpan { index }` | a fragment's bytes lie outside its `span_range` | FRG-1 |
| `GlyphOutsideSpanFragment { span_id, run, glyph }`, `InlineBoxOutsideSpanFragment { span_id, index }` | a glyph or box of a span lies outside every fragment of it | FRG-2 |
| `InvalidCaretStop { index }` | a stop breaks CRT-1 or CRT-2 | CRT-1, CRT-2 |
| `InvalidDecoration { index }` | a decoration breaks DEC-1 | DEC-1 |

| `SlotErrorReason` | Raised when |
|---|---|
| `InvalidWrapWidth` | the context's wrap width is not positive |
| `NoSlots` | the line has no slot |
| `InvalidConstraint` | a constraint lies outside the envelope, or overlaps or precedes the previous one |
| `InvalidContent` | a block is not finite, is negative or starts outside its constraint |
| `WidthNotContentSpan` | a block is not the span of its content |
| `ContentOutsideSlots` | the line has content but no occupied slot |

| `AnnotationErrorReason` | Raised when |
|---|---|
| `OnEmptyLine` | an empty line has an annotation |
| `InvalidMetrics` | a size, scale, offset or extent is not finite, or not positive / non-negative where required |
| `OrientationMismatch` | the run's orientation does not suit the writing mode |
| `BaseOutsideLine` | the base is empty or outside the visible bytes |
| `NoGlyphs` | the run has no glyph |
| `InvalidGlyph` | a glyph carries a body byte range or a non-finite coordinate |
| `DuplicateId` | another annotation of the line has the same id |
| `OverlapsOnSide` | another annotation on the same side shares base bytes |
| `GlyphStraddlesBase` | a body glyph intersects the base without lying inside it |
| `NoBaseGlyphs` | no body glyph lies in the base |
| `BaseSpansSlots` | the base is not inside one slot |
| `OutsideOverhang` | a glyph lies beyond the overhang allowance or the slot |
| `InsideBase` | the annotation sits closer to the baseline than its base |
| `DoesNotFitLineBox` | under `Normal`, the line box is too small for the base and the annotation |

| `ParagraphErrorReason` | Raised when |
|---|---|
| `InvalidPolicy` | the policy is not one `resolve` could produce |
| `NoLines` | the paragraph has no line |
| `InvalidBlockSize` | `total_block_size` is not finite, negative, or ends above the last line |
| `LineBoxesOverlap` | a line box is not finite, is inverted, or starts above the previous one's bottom |
| `LineBoxExceedsLineHeight` | `ascent + descent > line_height` |
| `NotStandardSlots` | under `Standard`, a line's slots are not its standard slot |
| `SourceNotCovered` | the lines do not tile the rendered text on character boundaries |
| `InvalidTruncation` | the ellipsized line, `truncated_range` and the clamp disagree |
| `DuplicateAnnotation` | two lines carry an annotation with the same id |
| `InconsistentSpanFragments` | a decoration span's fragments disagree on its range or lack exactly one first and one last |
| `InconsistentAtomicFragments` | an atomic span's fragments disagree on its range or miss rendered bytes |

### 2.7 Writing modes and orientation

```rust
pub enum WritingMode { HorizontalTb, VerticalRl, VerticalLr }
pub enum TextOrientation { Mixed, Upright, Sideways }
pub enum GlyphOrientation { Horizontal, VerticalUpright, VerticalSideways, HorizontalInVertical }
pub enum BaselineAlignment { Auto, Alphabetic, Ideographic, Central, Hanging, Mathematical,
                             LineTop, LineBottom, Offset(f32) }
```

**[ORI-1] Flow space.** Layout runs in canonical flow coordinates: inline `x`, block-progressing `y`. `WritingMode::flow_to_physical_point` maps a flow point to paragraph-local physical coordinates: `HorizontalTb` to `(x, y)`, `VerticalLr` to `(y, x)`, `VerticalRl` to `(total_block_size - y, x)`; `WritingMode::physical_to_flow_point` inverts it, and `WritingMode::flow_to_physical_rect` and `WritingMode::physical_to_flow_rect` normalise the transformed corners. Placing the paragraph in a container is a host translation.

**[ORI-2] Line-over.** Offsets towards line-over are not block coordinates: a positive over offset maps to a negative block delta in `HorizontalTb` and `VerticalRl` and a positive one in `VerticalLr` (`WritingMode::over_to_block`). A line's block bounds are `[baseline - ascent, baseline + descent]`, except in `VerticalLr`, `[baseline - descent, baseline + ascent]` (`WritingMode::line_bounds`). `AnnotationSide::Over` is above horizontal text and right of vertical text in both vertical modes.

**[ORI-3] Orientation.** `TextOrientation` is the frontend's policy: `Mixed` resolves each character from its vertical-orientation property, `Upright` forces upright glyphs and an LTR paragraph, `Sideways` shapes horizontally and rotates clockwise. Each run, visual run and annotation run carries a resolved `GlyphOrientation`. `Horizontal` is valid only in `HorizontalTb`, the three others only in a vertical mode, and `VerticalUpright` and `HorizontalInVertical` additionally only at an even level (`GlyphOrientation::is_valid`).

**[ORI-4] Tate-chu-yoko.** A `HorizontalInVertical` run is one indivisible horizontal group: one cluster covering all of the run's atoms, referenced by one text box spanning the run, so no break can fall inside it. Its `natural_width` is the group's advance along the vertical line. It is atomic for editing only when an `AtomicEditSpan` covers it.

**[ORI-5] Vertical metrics.** In a vertical mode the strut and line extents come from the vertical metrics (§5 Stage 4).

**[ORI-7] Unplaced tate-chu-yoko.** OUT-1 cannot yet express where the glyphs of an upright horizontal group go: they share one inline extent. Until Phase 1 defines that placement, the output gate rejects a `HorizontalInVertical` visual run (`UnplacedTateChuYoko`), so every validated output can be drawn exactly (HST-5). Phase 1 lifts this together with the placement rule.

**[ORI-6] Baselines.** Baseline offsets in `FontMetrics` and `BaselineAlignment::Offset` are positive towards line-over; an explicit offset is finite (`BaselineAlignment::is_valid`).

**[BAS-1] Baseline alignment.** The dominant baseline of a line is the alphabetic one (offset 0) in `HorizontalTb` and the central one in the vertical modes; `BaselineAlignment::Auto` resolves to it. The paragraph's `strut_metrics` define the line's baseline grid relative to the alphabetic baseline: 0 for `Alphabetic`, and the strut's `ideographic_baseline_offset`, `central_baseline_offset`, `hanging_baseline_offset` and `mathematical_baseline_offset` (FNT-4). A run aligned to a named baseline *B* moves by `strut.offset(B) − font.offset(B)` towards line-over, its font's *B* onto the strut's; an inline object aligned to *B* places its baseline at `strut.offset(B)`; `Offset(d)` moves either by `d` from the dominant baseline. `LineTop` and `LineBottom` are placed in a second pass: the line box is first computed from the strut and the other items, and under `LineHeight::Normal` a line-relative item taller than `ascent + descent` deepens it by the difference on its free side (below for `LineTop`, above for `LineBottom`); then a `LineTop` item's top is flush with the line's over edge and a `LineBottom` item's bottom with its under edge. Under `LineHeight::Fixed` the box is the strut's and objects still take their shifts. A shift towards line-over adds to a glyph's `y_offset`, and an object's `PositionedInlineBox::y` is its negation, growing downwards.

### 2.8 Explicit bidi spans

```rust
pub struct ExplicitBidiSpan { pub span_id: u64, pub byte_range: TextRange, pub control: BidiControlKind }
pub enum BidiControlKind { Isolate(Option<BidiDirection>), Override(BidiDirection) }
```

**[BID-1]** A caller's out-of-band directional controls travel as `ParagraphAst::bidi_spans`, a stream independent of the decoration and atomic spans. `BidiControlKind::Isolate` is FSI (`None`), LRI or RLI up to PDI; `BidiControlKind::Override` is LRO or RLO up to PDF. `span_id` is a caller handle the engine does not interpret. Directional controls written in the text (LRE, RLE, LRO, RLO, PDF, LRI, RLI, FSI, PDI) are not converted into spans: they stay text atoms, so their bytes keep an owner (AST-3), and the resolver reads them where they stand (FE-30).

**[BID-3] Two streams, one stack.** The resolver sees each span as its control characters inserted at its edges. At one offset, closing span edges come first (inner first), then opening ones (outer first), then the text there: a span opens outside an in-text control at its start and closes outside one at its end. BID-2's `MAX_BIDI_SPAN_DEPTH` bounds only the nesting of `bidi_spans`, checked by the input gate; UAX #9 X1 to X8 bound the combined stack of spans and in-text controls by `BIDI_MAX_DEPTH`. P2 and P3 skip the content of isolate spans as they skip that of in-text isolates.

**[BID-2]** Ids are unique; ranges are non-empty and fit the text; both ends lie on content edges (the start or end of a text atom, glue, tab, inline object or discretionary); spans ascend by start and, for equal starts, descend by end, and nest properly; equal ranges are rejected; nesting depth is at most `MAX_BIDI_SPAN_DEPTH` (63), counted on the source spans independently of resolved levels.

### 2.9 Annotations

```rust
pub struct AnnotationSpan {
    pub annotation_id: u64,
    pub base_byte_range: TextRange,
    pub side: AnnotationSide,
    pub kind: AnnotationKind,
    pub block_extent: f32,
    pub annotation_text: Box<str>,
    pub run: ShapingRun,
    pub atoms: Vec<GraphemeAtom>,
    pub clusters: Vec<ShapedCluster>,
    pub glyphs: Vec<RawShapedGlyph>,
}
pub enum AnnotationSide { Over, Under }
pub enum AnnotationAlignment { Center, SpaceAround, SpaceBetween, Start }
pub enum AnnotationKind { Ruby { alignment: AnnotationAlignment, max_overhang: f32 }, Emphasis }
```

**[ANN-1]** An annotation (ruby or emphasis) is shaped independently, in annotation-local coordinates: its run's `byte_range` is `0..annotation_text.len()`, its run's `atom_range` and `cluster_range` start at 0, its atoms tile `annotation_text`, and its clusters' glyph ranges tile its own `glyphs`. Its run, font, flag, orientation, level and safety rules are the body's (§2.3, §2.5).

**[ANN-2]** Ids are unique. The base is non-empty, fits the text and has both ends on atom and cluster edges of one text-only stretch, so no discretionary, glue, tab or inline object lies inside it. On each side bases ascend by start and do not overlap; bases on opposite sides MAY overlap.

**[ANN-3]** `annotation_text` is non-empty and holds no line or paragraph separator (`is_line_or_paragraph_separator`). `block_extent`, the room the annotation needs beyond its base on its side, gap included, is finite and non-negative. A ruby's `max_overhang` is finite and non-negative. An `Emphasis` mark is exactly one atom and one cluster; a tate-chu-yoko annotation run is one cluster.

**[ANN-4] Ruby compilation** (*future*). A ruby is an indivisible grouped span expanded during shaping and spacing, before break compilation. Its base capacity is `max(base_width, ruby_width - 2 · max_overhang)`. Expansion is distributed over the base clusters in proportion to their raw widths, equally when the raw total is zero. With annotations on both sides of one base, the width requirement is the larger of the two. Placement keeps the ruby inside its slot. Emphasis repeats one shaped mark over each eligible atom of the base, excluding spaces and opening or closing punctuation. Truncation and editing never split an annotated group.

---

## 3. Frontend analysis

```rust
pub struct FrontendOptions {
    pub writing_mode: WritingMode,
    pub text_orientation: TextOrientation,
    pub locale_policy: LocalePolicy,
    pub autospace_profile: AutospaceProfile,
    pub quote_policy: QuotePolicy,
    pub default_locale: LocaleTag,
    pub base_direction: Option<BidiDirection>,
    pub line_break_strictness: LineBreakStrictness,
    pub word_break_option: WordBreakOption,
}
pub enum LocalePolicy { ExplicitOnly, ExperimentalHeuristics }
pub enum LineBreakStrictness { Loose, Normal, Strict, Anywhere }
pub enum WordBreakOption { Normal, BreakAll, KeepAll }
pub enum AutospaceProfile { Jlreq, CssCompat }
pub enum QuotePolicy { Standard, CjkAware }
pub struct ParagraphSlice { pub content: TextRange, pub separator: TextRange }
```

**[FE-1] Paragraph splitting.** `split_paragraphs(text)` returns one `ParagraphSlice` per paragraph in absolute document bytes. It splits on `is_paragraph_separator` characters, a CRLF pair counting as one separator. Empty input gives one empty paragraph; a final separator gives a final empty paragraph (`"abc\n"` gives `(0..3, 3..4)` and `(4..4, 4..4)`); U+2028 stays in the content. Each content slice is laid out on its own, with paragraph-local offsets.

**[FE-2]** The frontend records raw strut metrics and never applies a line-height policy; `FrontendOptions` has no line-height field. Its output is independent of column width.

**[FE-3] Order.** Grapheme atoms (Step 1), glyph orientation in vertical modes (§2.7), bidi resolution including explicit spans (Step 2), script and locale (Steps 3 and 4), run splitting (Step 5), shaping, spacing and ruby expansion, then item compilation (Step 6).

**[FE-4] Complexity.** Steps 1, 2, 4 and 5 are linear. Steps 2 and 3 use bounded stacks (depth 63 by default) and a memoised next-strong look-ahead computed by one reverse sweep, so they stay linear. No bound is stated for dictionary or LSTM segmentation in Step 6.

### Step 1: grapheme atoms and emoji

**[FE-5]** Atoms are UAX #29 extended grapheme clusters, GB9c included, with `InCB` read from `DerivedCoreProperties.txt`. A Devanagari conjunct such as क्ष is one atom.

**[FE-6]** `IS_EMOJI_SEQUENCE` is set after segmentation, with no second state machine, on a core emoji sequence, a ZWJ sequence, a keycap sequence, a flag (two regional indicators), a tag sequence or a modifier sequence. A ZWJ inside Indic or Arabic text is `Script=Inherited` and does not make its atom an emoji. A modifier sequence needs no U+FE0F; otherwise a base with `Emoji_Presentation=No` not followed by U+FE0F, or followed by U+FE0E, is text style and is not assigned `Zsye`. Where UTS #51 and UAX #29 disagree on cluster boundaries, the engine follows UAX #29.

**[FE-7]** `HAS_VARIATION_SELECTOR` is set on an atom that contains U+FE0E, U+FE0F or an IVS selector U+E0100..U+E01EF.

### Step 2: bidi resolution and pairing

**[FE-8] Base direction.** The caller's direction, `FrontendOptions::base_direction`, wins (UAX #9 HL1). Otherwise P2 and P3: the first L, AL or R character, skipping isolates, decides; level 1 for AL or R, 0 otherwise.

**[FE-9] Two pairing tables.** Table B, used by UAX #9 N0, is exactly `BidiBrackets.txt`, with the BD16 rules and its 63-element stack. Its `Bidi_Paired_Bracket` is read as the `Bidi_Mirroring_Glyph` of a character whose `Bidi_Paired_Bracket_Type` is not None (the two agree for every bracket of the data version), and BD16's canonical equivalence (U+2329 and U+232A pair as U+3008 and U+3009) compares the NFD forms of the opener and of the closer's paired bracket through `icu_normalizer` (SCO-8). Table S, used only by script resolution (Step 3), adds guillemets, curly quotes and CJK corner brackets, pairs by code point and takes a locale-aware override table from CLDR when the locale is known. The override is keyed by the primary language subtag of the atom's stated locale (its style's, else `default_locale`; the heuristics of FE-16 run after Step 3 and do not feed it): `de` pairs „ with “ and ‚ with ‘; `pl` „ with ”; `fr` and `ru` « with »; `ja` and `zh` 「 with 」 and 『 with 』; `sv` and `fi` ” with ” and ’ with ’, a pair whose one character closes when its opener is on the stack and opens otherwise. The override's role for a character it names wins over the default table's, so under `de` “ only closes. Table S never influences bidi.

**[FE-10] Atom levels.** An atom takes the level of its first code point; non-spacing marks follow their base. A character that UAX #9 X9 removes (an embedding or override control, PDF, or class BN such as U+00AD, U+200B or U+2060) is retained as UAX #9 §5.2 describes: it takes the resolved level of the character before it, else the paragraph's base level. Runs split at every level change (Step 5), so no cluster straddles two levels.

**[FE-11] Apostrophes.** U+0027, U+2019 and U+02BC are "ambiguous quote or apostrophe". The filter marks one `IS_APOSTROPHE` and keeps it off the Table S stack only if a Word_Break AHLetter lies on both sides (UAX #29 WB6/WB7). A closer matches only an opener of the same pair type on the stack.

**[FE-12]** `ExplicitBidiSpan`s are honoured as UAX #9 isolate and override controls at their boundaries (§2.8, BID-3). An inline object takes part in resolution as one character of class ON, a word space as its own characters (class WS) and a tab as class S; a U+2028 is class WS.

### Step 3: script resolution

**[FE-13] Default algorithm.** Explicit scripts stand. Combining marks inherit their base's script. `Script_Extensions` are used: the run's script set is intersected with each new character's. Paired punctuation takes the script of its enclosing text through a bounded stack (an opener that is Common is pushed; a closer takes its opener's script; an unmatched closer leaves the stack; a full stack drops its oldest entry). A Common character with East Asian Width W, F or H and no extensions takes the Han script list. Any other Common character takes the preceding strong script, or at paragraph start the first following one.

**[FE-14] CJK-aware quotes** (`QuotePolicy::CjkAware`). "CJK" means East Asian Width W, F or H, or a script in {Hani, Hira, Kana, Hang, Bopo}. A quote pair is CJK if its left anchor is, else its right anchor, else if its interior contains CJK and the opener follows no ASCII space; the chosen script is the neighbours' common Han script or `Hani`. An unclosed opener stays on the bounded stack and resolves at overflow or paragraph end from its left neighbour, else the next strong script, else the paragraph default. The policy is deterministic for a given text, not stable under edits.

**[FE-15] Dashes, ellipses, middle dots.** A maximal run of U+2014, U+2026 or U+00B7 resolves as one unit: CJK if exactly one neighbouring non-punctuation anchor is CJK, Han if both are, the left neighbour's if they differ, otherwise the host script. Such a run is also unbreakable (§5 Stage 1): no opportunity lies inside it, so on a line that holds anything before it, it moves to the next line whole. A run that starts a line and is wider than the slot is split by the emergency break of ST1-5 like any other unit, since only one cluster may overflow a slot (SLT-5). A middle dot (U+00B7 or U+2027) that resolves to a CJK script never begins a line after the atom it touches: no opportunity lies before it, under any strictness but `Anywhere`, as WN-3 fuses closing punctuation (UAX #14 leaves U+00B7 ambiguous, and resolved to AL it would allow `号 ÷ ·`).

### Step 4: locale

**[FE-16]** Explicit tags win; untagged text takes `default_locale` under `ExplicitOnly`. Under `ExperimentalHeuristics` only, within a UAX #29 sentence (whose tagged atoms count as evidence but keep their tags), on the scripts resolved in Step 3: kana present makes the untagged Han and kana atoms `ja-JP`; else Hangul present makes the untagged Han and Hangul atoms `ko-KR`; U+0402, U+040B, U+0452 or U+045B (Ђ, Ћ, ђ, ћ) present makes the untagged Cyrillic atoms `sr-Cyrl`, so that an all-capitals Serbian heading resolves like lower-case text; every other untagged atom takes `default_locale`. Kana and Hangul take the locale with Han so that a run does not split between them. The engine never switches locale on the frequency of ideographs.

### Step 5: shaping runs

**[FE-17]** Runs are the intersection of the script, locale, bidi-level and orientation spans with the caller's style spans (family, size, weight, slant). Colour never splits a run: it is applied at paint time. Atoms never straddle runs.

### Step 6: segmentation and item compilation

**[FE-18] Segmenters.** Line, grapheme and word segmentation use stable interfaces with pinned data whose actual versions `conformance_report()` names (SCO-4). Line breaking takes `FrontendOptions::line_break_strictness` (`LineBreakStrictness`: Loose, Normal by default, Strict, Anywhere), `FrontendOptions::word_break_option` (`WordBreakOption`: Normal by default, BreakAll, KeepAll) and a content locale, with one cached segmenter per option set. Khmer, Lao, Myanmar and Thai use the LSTM or dictionary model; Chinese and Japanese line breaking needs no dictionary.

**[FE-19] Compilation.** A breakable space becomes `[Glue(WordSpace), Discretionary(penalty 0)]` and one with no break after it only the glue (WN-2); word spaces become `WordSpace`, glyph-less glue sized by WN-10, while U+3000 and the no-break spaces are text (WN-11, WN-12). A UAX #14 opportunity between boxes becomes a `Discretionary`, a mandatory one a `HardBreak`. Punctuation fusion follows WN-3. Tabs become `Tab`, U+2028 a `HardBreak`, span edges `SpanBoundary` nodes, and the paragraph ends with the sentinel (WN-17).

**[FE-20] Autospace.** `CjkAutospace` is inserted between two atoms that touch in bytes, with no glue, tab, hard break or inline object between them and no padded decoration span edge at their offset (SPN-4), when one is of the CJK class and the other of the Western class, each judged by its first code point before script resolution (FE-13):

- **CJK class**: `Script` in {Hani, Hira, Kana, Hang, Bopo}, East Asian Width W or F, General Category neither punctuation (P\*) nor Cf, and UAX #14 class none of OP, CL, CP, QU and IN;
- **Western class**: East Asian Width neither W nor F, and either a letter (General Category L\*) of `Script` Latn, Grek or Cyrl, or a decimal digit (Nd).

Punctuation is never of either class, so nothing is inserted beside a bracket or a pause mark. The glue never shows at a line edge: it precedes the break opportunity at its offset (AST-3) and collapses there (WN-10). Its sizes come from the profile, with `em` the `font_size` of the CJK-class atom's style (1 ic is taken as 1 em):

| Profile | `natural_width` | `max_shrink` (shrinks to) | `max_stretch` (stretches to) |
|---|---|---|---|
| `AutospaceProfile::Jlreq` (default) | 1/4 em | 1/8 em (1/8 em) | 1/4 em (1/2 em) |
| `AutospaceProfile::CssCompat` | 1/8 em | 0 (rigid) | 0 (rigid) |

**[FE-21] Hyphenation and splits.** Where a word has hyphenation opportunities, the frontend splits its clusters at each into adjacent text boxes of one run and interleaves flagged `Hyphenate` discretionaries (WN-22) with `pre_break_width` the estimated hyphen width. Where a split falls inside a multi-atom cluster it re-shapes that window (CLU-8).

**[FE-22] URLs.** A tailoring hook adds opportunities inside URLs (after `://`, at transitions, never beside `(` or `[`, and in pieces of 16 bytes or more), with penalties in the URL band. A URL starts at a scheme before `://`, or at `www.` that begins a word: after white space or punctuation other than `.`, `-`, `_`, `@`, `/` and `:`, so an e-mail domain or a path segment is not a URL. A URL ends before trailing sentence punctuation and closing quotes (`.`, `,`, `;`, `:`, `!`, `?`, ASCII quotes, and any non-ASCII punctuation such as `”` or `。`), and before a `)` or `]` it did not open.

**[FE-23] No-line-end words.** A locale-keyed table binds listed one-letter words to the next word by emitting no `Discretionary` after them; for Czech it is `AIiVvOoUuSsZzKk`, for English it is empty.

**[FE-24] Word movement** uses a separate UAX #29 word pass (§6.5), not these items.

### The entry point

```rust
pub struct TextStyle {
    pub families: FamilyStack,
    pub font_size: f32,
    pub style: FontStyle,
    pub features: FontFeatures,
    pub decoration: TextDecoration,
    pub baseline_align: BaselineAlignment,
    pub letter_spacing: f32,
    pub word_spacing: f32,
    pub script: Option<ScriptTag>,
    pub locale: Option<LocaleTag>,
}
pub struct StyleSpan { pub byte_range: TextRange, pub style: TextStyle }
pub struct InlineObjectInput {
    pub byte_range: TextRange,
    pub box_id: u64,
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
    pub baseline_align: BaselineAlignment,
    pub overflow_policy: BoxOverflowPolicy,
}
pub struct AnnotationInput {
    pub annotation_id: u64,
    pub base_byte_range: TextRange,
    pub side: AnnotationSide,
    pub kind: AnnotationKind,
    pub text: Box<str>,
    pub style: TextStyle,
}
pub struct ParagraphInput<'a> {
    pub text: &'a str,
    pub default_style: &'a TextStyle,
    pub style_spans: &'a [StyleSpan],
    pub inline_objects: &'a [InlineObjectInput],
    pub decoration_spans: &'a [InlineDecorationSpan],
    pub atomic_spans: &'a [AtomicEditSpan],
    pub bidi_spans: &'a [ExplicitBidiSpan],
    pub annotations: &'a [AnnotationInput],
}
pub fn compile_paragraph(input: &ParagraphInput<'_>, options: &FrontendOptions,
    shaper: &dyn TextShaper) -> Result<ParagraphAst, FrontendError>;
pub enum FrontendError {                // #[non_exhaustive]
    ParagraphSeparatorInText { byte: ByteIndex },
    TextTooLong,
    InvalidStyle { span: Option<usize> },
    InvalidStyleSpan { span: usize },
    InvalidInlineObject { object: usize },
    InvalidDecorationSpan { span: usize },
    InvalidAtomicSpan { span: usize },
    InvalidBidiSpan { span: usize },
    Unsupported { feature: FrontendFeature },
    ShaperContract { run: usize },
}
pub enum FrontendFeature { VerticalWritingMode, Annotations }
```

**[FE-25] Input.** `compile_paragraph(input, options, shaper)` compiles one paragraph into a `ParagraphAst` in the order of FE-3. `ParagraphInput::text` is one paragraph (AST-2), and every range of the input is local to it. A `TextStyle` is complete: nothing cascades from one style to another. `ParagraphInput::style_spans` are non-empty, ascending, disjoint and on character boundaries; a byte no span covers takes `ParagraphInput::default_style`. A style's features are canonical as built (FNT-2). A style's `TextStyle::script` stands over script resolution (FE-13) and its `TextStyle::locale` over the default locale (SCO-5). A paragraph in which no character has a resolved script (every one is `Zyyy` or `Zinh`) and no style states one resolves to `ScriptTag::LATIN`. The default style also gives the strut: `strut_font_size` is its `font_size` and `strut_metrics` what `TextShaper::resolve_primary_font_metrics` returns for it, so that an empty paragraph, whose items are the sentinel alone, has a valid strut (AST-6).

**[FE-26] Guarantee.** For an input the frontend accepts and a shaper that keeps its contract (§4), the result passes the input gate (G1) under any valid constraints and options. The frontend checks its own input (FE-28) and whether a shaper's clusters can be assigned to text boxes at all; whether the measurements a shaper answered are valid is checked once, by G1 (AX-3).

**[FE-27] Objects, spaces and spacing.** Each `InlineObjectInput` becomes one `BoxNode::InlineObject` that owns its bytes (WN-7) at the paragraph level; those bytes are neither segmented into atoms nor shaped. Object ranges are non-empty, ascending, disjoint and on grapheme boundaries, and the sizes are finite and non-negative. A `WordSpace` glue has `natural_width = max(0, a + word_spacing)`, where `a` is `TextShaper::space_advance` (SHP-20) of the primary font of the style at its bytes and `word_spacing` that style's (the clamp absorbs a negative `word_spacing` only: an advance `a` that is negative or not finite is never clamped, the glue keeps it and fails the input gate as `InvalidGlue`, SHP-18, AX-3), with `max_stretch = natural_width / 2` and `max_shrink = natural_width / 3`. A cluster's `natural_width` is `max(0, a + letter_spacing)`, where `a` is the sum of its glyphs' advances (CLU-6): negative spacing never makes a width negative. A run whose atoms are `NO_LETTER_SPACING` takes no letter spacing, whatever its style says (CSS Text 4 §8.2), so a cursive run's `letter_spacing` is 0 (AST-5). An `AnnotationInput` is shaped on its own, in its own style, into an `AnnotationSpan` (§2.9).

**[FE-28] Errors.** `FrontendError` is `#[non_exhaustive]`. Each variant has one root cause:

| Variant | Root cause | Requirement |
|---|---|---|
| `ParagraphSeparatorInText { byte }` | the text holds a paragraph separator, first at `byte` | AST-2 |
| `TextTooLong` | the text is longer than `u32::MAX` bytes, the range of a shaping cluster value | FE-25 |
| `InvalidStyle { span }` | a font size is not finite and positive, or a spacing is not a measurement; `None` names the default style | FE-25, AST-5 |
| `InvalidStyleSpan { span }` | a style span is empty, outside the text, off a character boundary, out of order or overlapping | FE-25 |
| `InvalidInlineObject { object }` | an object's range is empty, outside the text, off a grapheme boundary, out of order or overlapping, or a size is not a non-negative measurement | FE-27, WN-7 |
| `InvalidDecorationSpan { span }` | a decoration span is out of order, overlaps without nesting, nests too deep, repeats an id, is empty or outside the text, has an invalid padding, or has an edge off a grapheme boundary or inside an object | AST-7 |
| `InvalidAtomicSpan { span }` | an atomic span is out of order or overlapping, repeats an id, is empty or outside the text, has an edge that is not the start or end of a text atom or object, or holds neither | AST-9 |
| `InvalidBidiSpan { span }` | an explicit bidi span repeats an id, is empty or outside the text, has an edge off a grapheme boundary or inside an inline object, is out of order, overlaps without nesting, has the same range as its predecessor, or nests deeper than `MAX_BIDI_SPAN_DEPTH` | BID-2 |
| `Unsupported { feature }` | the input asks for something the crate does not compile yet | RM-11 |
| `ShaperContract { run }` | the shaper's clusters do not partition the atoms of a run in order | SHP-4, SHP-5 |

`FrontendFeature` names what `Unsupported` refuses: `FrontendFeature::VerticalWritingMode` and a non-empty stream of `FrontendFeature::Annotations`. A feature leaves the list when it is implemented; an input is never accepted and then compiled without it. Decoration spans, atomic spans, baseline alignment, right-to-left paragraphs, bidi text and bidi spans are implemented and have no variant.

**[FE-29] Phase 1 analysis.** The crate's `compile_paragraph` implements FE-25 to FE-28 for horizontal left-to-right text. Step 1 segments the whole text into UAX #29 graphemes; a lone tab becomes a `Tab`, a lone U+2028 a `HardBreak`, a lone `is_word_space` character a `WordSpace` glue, and every other grapheme an atom of level 0 whose class is that of its first code point and whose flags are `IS_OPEN_PUNCT` (class OP), `IS_CLOSE_PUNCT` (CL, CP), `HAS_VARIATION_SELECTOR` (FE-7), `IS_EMOJI_SEQUENCE` (FE-6; a lone regional indicator is none), `IS_APOSTROPHE` (FE-11) and `IS_CJK_IDEOGRAPH` (a wide Han, Hiragana or Katakana first code point). Step 3 implements FE-13 with the Table S stack of FE-9 and its locale override: the bracket pairs of `BidiBrackets.txt`, read through `Bidi_Paired_Bracket`, and the quotation pairs “ ”, ‘ ’, « », ‹ ›, „ “ and ‚ ‘, where “ and ‘ close a pair only when its German-style opener is on the stack. A `Script_Extensions` set, or the Han list {Hani, Hira, Kana, Hang, Bopo} for a wide character without one, takes the preceding strong script when it holds it, else the first following strong script when it holds that, else Han when it holds Han, else its first member. Emoji atoms take `Zsye` and are transparent to their neighbours' resolution. FE-14 applies under `QuotePolicy::CjkAware` to matched quotation pairs, a white-space neighbour never being CJK; FE-15 applies to runs of one of its three characters, marking each continuation atom that shares its predecessor's `ShapingRun` with `AtomFlags::BINDS_TO_PREVIOUS` so it never breaks inside, under any strictness, and drops the opportunity before a CJK middle dot that touches the atom before it. Step 4 keeps a style's locale; an untagged atom takes the locale FE-16 infers from its sentence under `ExperimentalHeuristics` when FE-16 infers one, else `default_locale`. Runs are the maximal sequences of atoms that agree on style, script and locale; glue, tabs, breaks and objects between them stay inside the run's bytes. The UAX #14 opportunities of the segmenter for the options' strictness and word option (`Anywhere`: every grapheme boundary) become `Direct` discretionaries after the unit that ends there, never inside an atom or object nor after a hard break; FE-23 removes the one after the word space that follows a listed one-letter word of the atom's locale language. FE-22 finds a URL as a scheme and `://`, or `www.` at a word start, up to white space and without trailing sentence punctuation or an unopened closing bracket; it adds opportunities after `://`, before `.`, `,`, `-`, `_`, `?`, `#`, `%` and `~`, after `/` and on both sides of `=` and `&`, never beside `(` or `[`, all at `URL_PENALTY_MIN`, and at every grapheme boundary of a piece of 16 bytes or more between them at `URL_PENALTY_MAX`; every opportunity strictly inside a URL takes a URL-band penalty. A decoration span edge starts a new text box, and its `SpanBoundary` nodes follow the order of AST-8; `Clone` padding is folded into every discretionary the span holds strictly inside (WN-21); both span streams are stored in the AST as given. A text box is a maximal sequence of atoms of one run with no other item between them; a cluster that straddles a box boundary is re-shaped on each side of the cut (CLU-8). Phase 2 replaces the level-0 rule by FE-30 and adds FE-31; hyphenation (FE-21, Phase 3) is not compiled yet.

**[FE-30] Phase 2 bidi analysis.** Step 2 runs the whole of UAX #9 on the paragraph, in `frontend/bidi.rs`, over the classes of `icu_properties` (SCO-8): P2 and P3 unless `FrontendOptions::base_direction` is given (HL1), X1 to X10 over the text's own controls and the `bidi_spans` merged by BID-3, W1 to W7, N0 with BD16 (FE-9), N1, N2, I1 and I2, with the removed characters retained by FE-10. Inline objects, word spaces, tabs and U+2028 take part as FE-12 says. Every atom, inline object and word space takes the resolved level of its first code point; a span boundary takes WN-8's level and synthetic glue WN-10's; `ParagraphAst::base_direction` is the resolved paragraph direction. An atom whose first code point has class WS, S, BN, LRI, RLI, FSI, PDI, LRE, RLE, LRO, RLO or PDF is flagged `L1_RESETTABLE`, so that Stage 2 applies L1 without reading the text (AX-4). Runs split at every level change (FE-17) and the frontend cuts a text box at every edge of an explicit bidi span, re-shaping a cluster that straddles it (CLU-8). The same module holds the line-level rules L1 and L2 that the conformance suites (RM-1) run on the resolver's output and that Stage 2 reuses (SCO-6).

**[FE-31] Phase 2 spacing and flags.** After Step 4 the frontend sets, from each atom's first code point and resolved script:

- `NO_LETTER_SPACING` on atoms whose script is Arab, Rohg, Mand, Mong, Nkoo, Phag or Syrc; kashida is not applied (ST3-5);
- `HANGABLE_START` and `HANGABLE_END` as ATM-3 lists them;
- `ALLOW_CJK_JUSTIFY_AFTER` on a CJK-class atom (FE-20) unless it is the last atom before a hard break or the paragraph end, the next atom is closing or prohibited-start punctuation (`IS_CLOSE_PUNCT`, UAX #14 class CL, CP, EX, IS or SY, or U+3001, U+3002, U+FF0C or U+FF0E), or glue (of any kind, synthetic included), a tab or an inline object follows it, so that a boundary is never counted in two tiers.

It inserts `CjkAutospace` by FE-20 and, where the run's locale does not turn compression off, `CjkPunctCompression` between two touching punctuation atoms of one run: after a closing or stop mark (U+3001, U+3002, U+FF0C, U+FF0E, U+FF1A, U+FF1B, a wide or full-width atom of UAX #14 class CL or CP, or a closing quote ’ / ”) that a wide or full-width punctuation atom or curly quote (‘ ’ “ ”) follows, between two opening brackets or opening quotes (wide/full-width class OP or ‘ / “), or between a middle dot (U+00B7, U+30FB, U+2027) and an adjacent closing/stop mark or opening bracket/quote. Compression is off for Traditional Chinese: a `zh` locale with the script `Hant`, or with no script and the region TW. It is not inserted where the shaper already compressed the pair (`chws`) or where a mark is narrow: both clusters must be wider than 3/4 of the run's `font_size`. The glue has `natural_width` 0, `max_stretch` 0 and `max_shrink` half the run's `font_size` (or a quarter for a middle-dot pair; WN-13); the frontend cuts the text box at every candidate seam before shaping, so a seam whose pair the shaper compressed keeps a plain box boundary. Synthetic glue follows the `Trailing` boundaries and precedes the discretionary and the `Leading` boundaries at its offset (AST-3, AST-8).

---

## 4. Shaping bridge

### 4.1 The trait

```rust
pub trait TextShaper: Send + Sync {
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics;
    fn resolve_primary_font_metrics(&self, families: &[FontFamily], style: FontStyle, script: ScriptTag,
        locale: &LocaleTag, font_size: f32, variations: &[FontVariation]) -> (FontId, FontMetrics);
    fn shape_run(&self, text: &str, run: &ShapingRun, atoms: &[GraphemeAtom], output: &mut ShapeOutput);
    fn reshape_window(&self, request: &ReshapeRequest<'_>, output: &mut ShapeOutput);
    fn ligature_carets(&self, instance: FontInstanceRef<'_>, glyph_id: u32, direction: Direction)
        -> SmallVec<[f32; 4]>;
    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32;
}
pub struct ShapeOutput { pub clusters: Vec<ShapedCluster>, pub glyphs: Vec<RawShapedGlyph> }
impl ShapeOutput { pub fn clear(&mut self); }
pub struct ReshapeRequest<'a> {
    pub text: &'a str, pub run: &'a ShapingRun, pub atoms: &'a [GraphemeAtom], pub window: TextRange,
    pub leading_edit: &'a LeadingEdit, pub trailing_edit: &'a TrailingEdit,
}
impl ReshapeRequest<'_> { pub fn plain(text, run, atoms, window) -> ReshapeRequest<'_>; }
pub enum LeadingEdit { None, ReplaceSuffix { replacement: Box<str>, cut_bytes: u32 } }
pub enum TrailingEdit { None, AppendHyphen { hyphen: char },
                        ReplacePrefix { replacement: Box<str>, cut_bytes: u32, hyphen: char } }
pub struct NoopShaper;
```

**[SHP-1]** `TextShaper` is `Send + Sync`: its methods take `&self`, and a backend with interior caches synchronises them itself.

**[SHP-2] Configuration.** Family stack, style, size, script, locale, bidi level, features and variations come only from `run` (`ShapingRun::features` for features and variations); there are no duplicate configuration arguments. `run.atom_range` and `run.cluster_range` are ignored by the shaper: they only locate the run's slices in the AST.

**[SHP-3] Text.** `text` is the whole paragraph text and `run.byte_range` selects the run, so atom offsets are absolute paragraph indices; the backend MAY use text outside the run as shaping context. `atoms` is the run's atom slice (`ParagraphAst::run_atoms`).

**[SHP-4] Output buffers.** `shape_run` and `reshape_window` write to a caller-owned `ShapeOutput`: they **append** clusters to `ShapeOutput::clusters` and glyphs to `ShapeOutput::glyphs`, never clearing either; `ShapeOutput::clear` empties both vectors, keeping their allocations, for a host that reuses one buffer. A cluster's `atom_range` indexes `atoms`; its `glyph_range` indexes `ShapeOutput::glyphs` in absolute positions of that vector. In cluster order the new glyph ranges tile the glyphs the call appended.

**[SHP-5] `shape_run`** shapes one complete run, in logical order (SHP-11), with the clusters partitioning `atoms`. It shapes the run's atom slices, each contiguous in bytes, and never the bytes of the run's `byte_range` that no atom covers (glue, tabs, inline objects: AST-5), which reach it only as shaping context. For a `HorizontalInVertical` run it returns exactly one cluster covering all atoms, with every glyph in logical order, `natural_width` the group's advance along the vertical line, and both safety flags as for the start of the run. The group takes one face: the first atom's face if it covers every atom, else the first face, among those chosen for its atoms and then the run's candidates, that does, else the first atom's. A `LineOnly` backend chooses faces itself; when it sets the group in more than one face, the cluster takes its first glyph's face and a glyph from another face becomes that face's `.notdef` (glyph 0), keeping its advance.

**[SHP-6] `reshape_window`** re-shapes a window around a break, a hyphenation point or a split. Its inputs are a `ReshapeRequest` (`text`, `run`, `atoms`, `window` and the two edits; `text`, `run` and `atoms` mean what SHP-2 and SHP-3 say for `shape_run`) and a `ShapeOutput`; `ReshapeRequest::plain` is the request with no edit at either edge. `window` is in paragraph coordinates, lies within `run.byte_range`, and starts and ends on atom boundaries. Its output clusters partition exactly the atoms inside the window, with `atom_range` indexing `atoms` as for `shape_run`; ranges always describe the original text. `LeadingEdit` acts on the window's start only and `TrailingEdit` on its end only: a regular break passes one edit and `None` for the other edge, the middle fragment of a word split twice passes both, and both `None` is a plain, text-preserving re-shape. `ReplacePrefix` removes `cut_bytes` immediately before the window end and `ReplaceSuffix` immediately after the window start; neither adds the other's hyphen; the two cuts do not overlap; each cut ends on an atom boundary. A window carrying a hyphen holds at least one atom, and the hyphen's glyphs are appended to its last cluster with `RawShapedGlyph::is_synthesized` (CLU-7). With `cut_bytes == 0` and a non-empty replacement, the replacement's glyphs belong to the window's last (`ReplacePrefix`) or first (`ReplaceSuffix`) cluster. For a `HorizontalInVertical` run the kept text of the window stays one cluster, as SHP-5 says for `shape_run`, and is never split per face; the cluster takes the face of the first kept glyph, and a kept glyph from another face becomes that face's `.notdef` (glyph 0), keeping its advance. The empty cluster of a cut atom takes the face of the nearest kept cluster on its side of the window (the first kept one for a leading cut, the last for a trailing cut), so a replacement attached to it is shaped in the face of the text it replaces; only a window that keeps no atom falls back to the stack's first face. `TrailingEdit::for_opportunity` reads only the prefix side of a `HyphenOpportunity` (giving `ReplacePrefix`, or `AppendHyphen` when the prefix is unchanged); `LeadingEdit::for_opportunity` reads only the suffix side (giving `ReplaceSuffix`, or `None`).

**[SHP-7] `ligature_carets`** returns the GDEF caret positions of a ligature glyph (components minus one), or none. Positions are in layout units, in increasing physical `x` from the glyph's left origin over `(0, width)` whatever the direction (GDEF stores them so), with font size and fallback scale already applied (effective size `font_size · font.scale`), never design units. In a right-to-left cluster the leftmost position is the boundary before the last component (INT-5).

**[SHP-8] Metrics.** `font_metrics` returns the unadjusted metrics of an instance. `resolve_primary_font_metrics` returns the primary face and its complete `FontMetrics` (FNT-4), vertical and baseline metrics included, for the requested variation instance; the frontend stores the result unchanged as the raw strut.

**[SHP-9]** `NoopShaper` is a non-rendering, zero-metric implementation that closes the interface for tests. Its zero metrics are not valid (FNT-4), so layout that asks it for metrics stops at the shaper gate with `InvalidShaperMetrics`. Its space advance is 0.

**[SHP-20] `space_advance`** returns the nominal advance of U+0020 in a font instance along the inline axis of `orientation`: the horizontal advance, or the vertical one for `GlyphOrientation::VerticalUpright`. It is in layout units with `font_size · font.scale` applied, finite and non-negative. No OpenType feature, letter spacing or word spacing enters it, and it is never measured by shaping a space (SHP-5): the frontend adds the style's `word_spacing` itself (FE-27). A face without a space glyph answers `0.25 · font_size · font.scale`. The frontend is its only caller, so the value reaches layout as a glue width, which the input gate checks (WN-10, AX-3); the shaper gate, which checks answers given at layout time, has no part in it.

### 4.2 Implementation requirements

**[SHP-10] Req. 1, whole runs.** `shape_run` receives a whole run, preserving cross-word kerning, `calt` and `chws`; context is lost only at font switches inside the run.

**[SHP-11] Req. 2, RTL.** The buffer direction is RTL when the run's level is odd. The shaper alone owns mirroring: the frontend never substitutes mirrored code points. A backend's own mirroring table belongs to the backend; the HarfRust backend reads it from ICU4X's `Bidi_Mirroring_Glyph` (SCO-8), so it agrees with the resolver's brackets. A backend that returns visual order reverses to logical order before returning, including the glyph order within a cluster, and `x_offset` values survive the reversal.

**[SHP-12] Req. 3, fallback.** The font is chosen per atom with script and locale as the fallback key; all code points of an atom use one font; the run is split into font runs, each shaped, and `FontRef::scale` is carried. If no face covers a whole atom, the best face covering its first code point shapes all of it (unsupported marks use its glyph 0); failing that, a deterministic final missing-glyph face. `Hani` skips Latin-only faces; `Zsye` routes to colour emoji faces.

**[SHP-26] Coverage under normalisation.** A face covers an atom (SHP-12, SHP-25) when its character map covers every code point of the atom, or every code point of the atom's NFC composition, or every code point of its NFD decomposition (`icu_normalizer`): the backend normalises while it shapes, so `e` + U+0301 is covered by a face that maps only U+00E9, and U+00E9 by one that maps only `e` and U+0301.

**[SHP-13] Req. 4, cluster merging.** Glyphs are merged by cluster value and then forced to merge by `atom_range`, so no caret or break falls between a base and its mark, whatever the backend's cluster level.

**[SHP-14] Req. 5, caching.** Whole runs are cached under the text and shaping context, family stack, style, features, variations, script, locale, level, direction and size. No partial or chunked shaping. An edit relays out the whole paragraph.

**[SHP-15] Req. 6, `chws` repair.** No `chws`-off width is stored and no unconditional second pass runs. A font with `chws` is re-shaped locally where a chosen break splits punctuation context; a font without it uses the metrics-stage punctuation adjustment. Re-shaping happens only at actual boundaries.

### 4.3 Backend behaviour

How HarfBuzz, Pango, Blink, Parley and cosmic-text cluster, re-shape and fall back is surveyed in RESEARCH §4.3; it is informative.

### 4.4 Locale

**[SHP-16]** The run's locale reaches the backend as its language (HarfBuzz `hb_buffer_set_language`, DirectWrite `localeName`, CoreText `kCTLanguageAttributeName`), so `locl` selects localised glyphs.

### 4.5 Vertical metrics

**[SHP-17]** `resolve_primary_font_metrics` and `font_metrics` return vertical and baseline metrics for the requested instance; a bridge whose font lacks them supplies them (SHP-8). The middle-end reads them in vertical modes (§5 Stage 4.1).

### 4.6 Profiles

**[SHP-18]** A full-contract backend carries the whole contract. The `LineOnly` profile, realised by `LineOnlyShaper` (SHP-24), is degraded, for a backend that exposes only line shaping: it reconstructs clusters from glyph byte indices, reports `safe_to_break_before = false` at ambiguous seams, does not honour `locl` and has no ligature carets. A full-contract integration calls HarfRust, CoreText or DirectWrite directly. Under either profile a backend neither repairs a number it is given nor one it returns (AX-3). It never sees a size that is not a positive finite measurement: the frontend rejects such a style (`InvalidStyle`) before its first shaper call, and the input gate rejects such a run (`InvalidShapingRun`) before layout calls `font_metrics`, `ligature_carets` or `reshape_window`. What it answers is judged by the gates and never clamped or zeroed by the backend: a non-finite advance or offset fails the input gate (`InvalidTextWidth`), a non-finite or negative `space_advance` fails it as a glue width (`InvalidGlue`, WN-10, SHP-20), and the shaper gate checks the answers given at layout time (MSR-1).

```rust
pub struct LineGlyph { pub font: FontId, pub glyph_id: u32, pub byte_index: usize, pub x_advance: f32,
                       pub x_offset: f32, pub y_offset: f32 }
pub trait LineShaper: Send + Sync {
    fn shape_line(&self, text: &str, families: &[FontFamily], style: FontStyle, font_size: f32,
        features: &FontFeatures) -> Vec<LineGlyph>;
    fn font_metrics(&self, instance: FontInstanceRef<'_>) -> FontMetrics;
    fn resolve_primary_font_metrics(&self, families: &[FontFamily], style: FontStyle, script: ScriptTag,
        locale: &LocaleTag, font_size: f32, variations: &[FontVariation]) -> (FontId, FontMetrics);
    fn space_advance(&self, instance: FontInstanceRef<'_>, orientation: GlyphOrientation) -> f32;
}
pub struct LineOnlyShaper<B> { .. }     // LineOnlyShaper::new(backend), ::backend()
```

**[SHP-25] Face profiles and CJK routing.** `FontRegistry::add_face` records a `FaceProfile` for each face from its tables. The face is CJK (`FaceProfile::cjk`) when its character map holds one of 一 人 中 大 日 国 あ の ア 가 한 ㄅ, or when OS/2 claims both a CJK Unicode range (bits 48 to 59) and a CJK code page (bits 17 to 21); a face that maps only shared punctuation (“ ” ‘ ’ … — ·) is not. A CJK face's `FaceProfile::region` comes from the `GSUB` language systems first (`ZHS` Simplified, `ZHT` Traditional, `ZHH` or `ZHTM` Hong Kong, `JAN` Japanese, `KOR` Korean; several regions make it `CjkRegion::Pan`), else from the words of its typographic and legacy family names when they agree (`SC`, `TC`, `HK`, `JP`, `KR` and their spelled-out forms), else from its OS/2 code pages when they name exactly one region; otherwise it is `None`. `FontRegistry::add_face_with_locale` states the profile instead: a CJK locale gives a CJK face of its region (`und-Hani` a pan-CJK one), another locale a face that is not CJK, and `und` the profile read from the tables. `FontRegistry::profile` returns the recorded profile. A locale names a region from its subtags alone: Chinese (`zh`, `cmn`) by script (`Hant` Traditional, or Hong Kong with region `HK` or `MO`; `Hans` Simplified), else by region (`TW` Traditional; `HK`, `MO` Hong Kong; any other or none Simplified); `yue` Hong Kong unless `Hans`; `ja` Japanese; `ko` Korean. A run whose script is `Hani`, `Hira`, `Kana`, `Hang` or `Bopo` prefers the regions of its locale (Traditional, then Hong Kong, for `zh-TW`; Hong Kong, then Traditional, for `zh-HK`), else those its script implies (kana Japanese, Hangul Korean, Bopomofo Traditional), else none. An atom of such a run whose first code point is ASCII (U+0000 to U+007F), such as the digits FE-13 folds into a Han run, keeps SHP-12's stack order, so it takes the stack's leading Latin face. For every other atom of such a run the HarfRust backend takes, in stack order: the first CJK face that covers the whole atom and has the first preferred region or is pan-CJK, then one with each later preferred region; else the first CJK face that covers it; else SHP-12's fallback over every face, CJK faces first at each step and as the final missing-glyph face. Other runs keep SHP-12's stack order. The run's locale reaches HarfRust in full, which maps it to the OpenType language system (`zh-TW` and `zh-Hant` to `ZHT`, `zh-HK` to `ZHH`, `zh-CN` and `zh` to `ZHS`, `ja` to `JAN`), so a pan-CJK face's `locl` gives the regional glyphs (SHP-16). Nothing guesses a region from the characters: inline language switches are style spans (FE-25).

**[SHP-24] The `LineOnly` adapter.** `LineOnlyShaper` is the `TextShaper` of the `LineOnly` profile over a `LineShaper`, a backend that shapes one horizontal left-to-right string at a time, choosing faces itself, and returns its glyphs left to right, each with its face, its advance and offsets in layout units and `byte_index`, the offset in the string of the first character of its source cluster; all glyphs of one character come from one face. The adapter shapes the run's text without context and maps each glyph to the atom holding its byte index; clusters then merge to whole atoms as in SHP-13, a face change inside an atom keeping the atom's first face. Glyphs out of atom order whose atom ranges overlap merge into one cluster with the earlier face, so every glyph stays on its own atom; a glyph left in a cluster of another face becomes that face's `.notdef` (glyph 0), keeping its advance, since its id means nothing there. A seam is reported safe (`safe_to_break_before` and `safe_to_concat_before`) only at the start of the shaped text and after a white-space character, and unsafe everywhere else. `reshape_window` shapes the leading replacement, the kept text and the trailing replacement with its hyphen as one string; the replacement glyphs are synthesized and go on the window's first or last cluster, the cut atoms keep empty clusters, and the first cluster is not safe to concatenate (CLU-8). The backend's metrics answer `font_metrics`, `resolve_primary_font_metrics` and `space_advance`; `ligature_carets` answers none, and no locale reaches the backend, so `locl` is not honoured (SHP-18). The adapter forwards the backend's advances, offsets and `space_advance` unchanged, as SHP-18 requires: a non-finite glyph number reaches the input gate, which rejects it (`InvalidTextWidth`), and is not replaced by zero here.

**[SHP-19] Offset axes.** The bridge stores glyph offsets flow-relative (OUT-1): `x_offset` along the inline axis, positive towards the inline end, and `y_offset` towards line-over. Horizontal shaping already uses these axes, also for a `VerticalSideways` glyph, which is rotated clockwise afterwards. For `VerticalUpright` the bridge converts the backend's vertical-shaping offsets, given in the font's own up-and-right frame: `x_offset = -dy` (font up is the inline start) and `y_offset = dx` (font right is line-over). Any vertical-origin adjustment is folded into the offsets, so the drawing origin is where the glyph outline's own origin goes.

### 4.7 The HarfRust backend

```rust
pub struct FontRegistry { .. }          // in-memory faces; an id is a registration index
pub struct HarfRustShaper { .. }        // the backend over one shared registry
pub struct FaceProfile { pub cjk: bool, pub region: Option<CjkRegion> }
pub enum CjkRegion { Simplified, Traditional, HongKong, Japanese, Korean, Pan }
```

**[SHP-21] Registry.** A `FontRegistry` holds font faces in memory and is the id space of the shaper built on it. `FontRegistry::add_face` registers a `FontFace` under a family name and a `FontStyle` and returns its `FontId`, the index of the face in registration order, or `None`, registering nothing, when the bytes hold no font at the face index or the font's `units_per_em` is 0, by which no metric could be scaled; the bytes are shared, never copied (HST-1). `FontRegistry::set_generic` names the family a `GenericFamily` resolves to. `FontRegistry::new` is empty. A registry is filled before it is shared and does not change afterwards, so an id is never reused and a face stays resolvable (HST-2). It is a `FontSource`.

**[SHP-22] Shaper.** `HarfRustShaper::new` takes the shared registry and `HarfRustShaper::registry` returns it, for the renderer. The shaper is a `FontSource` over the same ids. It implements `TextShaper` with HarfRust for shaping and `read-fonts` for metrics. A face is parsed once, when it is registered, together with its metrics in font units; a variable instance is built once per set of coordinates and cached with its metrics, so no call parses a font again. `ligature_carets` reads the GDEF ligature caret list of the instance's face, brought forward from RM-7: a coordinate caret (format 1) as given, a variable one (format 3) with its delta at the instance's location, device tables ignored; a glyph with a contour-point caret (format 2), whose outline the backend does not load, or with no entry, answers none, which leaves the equal-slot fallback of INT-5.

**[SHP-23] Backend metrics and matching.** The HarfRust backend scales font units by `font_size · font.scale / unitsPerEm` and fills every `FontMetrics` field. It reads `unitsPerEm` and the ascenders and descenders of `hhea`, OS/2 typo and `vhea` as the font states them, signs included, never the normalised values of its font parser (which replaces an out-of-range `unitsPerEm` with 1000 and makes every ascent positive): a negative ascent or descent reaches the shaper gate (SHP-18, FNT-4), and a non-zero `unitsPerEm` outside 16–16384 scales as stated (SHP-21). The line uses OS/2 typo metrics when `USE_TYPO_METRICS` is set and they are non-zero, else `hhea`, else typo, else the OS/2 Windows extents with no gap, else 0.8 em above and 0.2 em below. `x_height` and `cap_height` come from OS/2, else the top of the ink of `x` and `H`, else 0.5 em and 0.7 em. The underline comes from `post`, else −0.1 em with 0.05 em thickness; the strike-out position from OS/2, else half the x-height. The vertical line comes from `vhea`, else half an em on each side with no gap. The ideographic, hanging and mathematical offsets come from the horizontal `BASE` axis (`DFLT`, else `latn`, else the first script), relative to `romn`, else `-descent`, `0.8 · ascent` and `0.5 · x_height`; the central offset is the midpoint of `ideo` and `idtp`, else the ideographic offset plus half an em. Faces are matched by family in stack order, a generic family through `FontRegistry::set_generic`, and within a family by slant before weight; every other registered face follows in registration order as the last resort of SHP-12. An instance whose id names no face answers all-zero metrics, which the shaper gate rejects.

---

## 5. Middle-end layout

```rust planned
pub fn break_lines_parameterized(inputs: &ValidatedInputs<'_>, beam_width: usize,
    window_depth: usize) -> Vec<usize>;
```

**[ME-1]** `layout_paragraph(ast, text, shaper, constraints, options)` lays out one paragraph and returns a `PositionedParagraph` or the first `LayoutInputError`. It first passes the inputs through the input gate (`ValidatedInputs::try_from_ast`), which resolves the policy once (PAR-1), and asks the shaper for metrics only through the shaper gate (G2). With a geometry provider it also probes the first line (GEO-5).

**[ME-2] Phase 1 layout.** The crate's `layout_paragraph` breaks every strategy greedily at natural widths: a line ends at the last opportunity of the lowest ST1-1 tier that fits before its content overflows, at the first mandatory break, or, when nothing fits, at the emergency break of ST1-5 (before the overflowing piece when something visible precedes it, else after it alone, together with the break that follows it across glue). An emergency seam whose cluster is not safe to break re-shapes the cluster on each side with no edit; when the re-shaped sides no longer fit, the seam moves back one piece at a time, dropping the abandoned re-shape, until the line fits or holds one piece; a seam never falls inside an FE-15 run while something precedes the run on the line: the run moves whole to the next line. A run that starts the line and is wider than the slot is split at the overflowing cluster like any other unit. It fills the first slot of each line's geometry and leaves the others empty; a taller line re-queries its envelope, and an obstructed one moves to `next_feature_block_offset`. Lines are reordered by UAX #9 L1 and L2 on their resolved levels. Under `LineHeight::Normal` a line's ascent and descent are the largest of the strut's and its clusters' and objects', each with half its line gap, and its height their sum; under `Fixed` the box is the strut's. An object follows its overflow policy against its slot's width (WN-7). Justification, hanging punctuation, hyphenation, annotations and clamping are not applied yet: the line is ragged, nothing hangs, annotations are not placed and every line is rendered.

**[ME-3] Phase 2 layout.** ME-2 stands with these additions: Stage 2 applies L1 by ST2-1 and L2 through the frontend's shared rule (FE-30), and assembles fragments by ST2-3; tabs measure from the slot's line-start edge (ST3-1); under `LayoutOptions::justify` the greedy breaker fits with shrink and Stage 3 justifies by ST3-2 to ST3-4, compressing the lines that end the paragraph or at a hard break only under `LayoutOptions::compress_last_lines`; marks hang by ST1-3 under the `HangMode`; synthetic glue collapses at line edges (WN-10). Hyphenation, annotations, clamping and the optimising strategies remain Phase 3 or later.

### Line geometry and slots

```rust
pub struct LayoutConstraints {
    pub first_line_indent: f32,
    pub hanging_indent: f32,
    pub geometry_provider: Option<Arc<dyn LineGeometryProvider>>,
    pub wrap_width: f32,
    pub tab_stops: TabStops,
}
pub struct InlineSlotConstraint { .. }   // [start, end), well formed by construction
pub struct LineGeometry {
    pub slots: SmallVec<[InlineSlotConstraint; 2]>,
    pub next_feature_block_offset: Option<f32>,
}
pub trait LineGeometryProvider: Debug + Send + Sync {
    fn query_line_geometry(&self, line_index: usize, block_start: f32, block_end: f32) -> LineGeometry;
}
```

**[GEO-1]** An `InlineSlotConstraint` is an inline interval: `InlineSlotConstraint::try_new` accepts a finite start, an end strictly beyond it, and either a finite end with a finite width or an end of positive infinity (no wrapping). `InlineSlotConstraint::start`, `InlineSlotConstraint::end` and `InlineSlotConstraint::width` read it.

**[GEO-2]** `LayoutConstraints::is_valid`: `wrap_width` is positive and not NaN (positive infinity is allowed); both indents are finite; without a provider each indent lies in `[0, wrap_width)`. `LayoutConstraints::default` has an infinite wrap width, zero indents and `TabStops::DEFAULT`. Without a provider the geometry of line *i* is one slot, `[first_line_indent, wrap_width]` on line 0 and `[hanging_indent, wrap_width]` after (`LayoutConstraints::resolve_line_geometry`).

**[GEO-3]** A provider answers in flow coordinates for a line index and a block envelope `[block_start, block_end)`. Its slots ascend, are disjoint and lie in the envelope `[0, wrap_width]`; a `next_feature_block_offset` lies beyond `block_start` by a measurement (MSR-1; `LineGeometry::is_valid`). An empty answer means the envelope is obstructed. Every provider answer is validated when queried (`LayoutConstraints::query_checked_geometry`, the provider gate G3), and a violation is `InvalidLineGeometry`; the envelope is computed by layout and is not checked, and without a provider the geometry derives from constraints the input gate accepted and is returned unchecked (AX-3).

**[GEO-4]** For a fixed line index and block start, an expanded envelope only removes space: each returned slot lies inside a slot of the smaller envelope's answer (`LineGeometry::is_subset_of`; `NonMonotonicGeometry` otherwise). An empty answer to an unexpanded query needs a `next_feature_block_offset` at least 1e-4 layout units ahead, which the provider gate checks; more than 1024 obstructed attempts in a row is `ClearanceExhausted` (AX-6).

**[GEO-5]** The first-line probe tries only what can begin at block start: the strut and the input up to the first break opportunity (inline objects, text cluster heights and, under `LineHeight::Normal`, the `block_extent` of annotations whose base starts before that break). A taller item later in the paragraph is not first-line evidence. An expanded probe that comes back empty is a contract check, not a rejection of the position.

**[GEO-6] Fitting** (*future*). The fitter keeps a non-decreasing envelope and never shrinks a committed line height. It returns an explicit failure, or uses a specified fallback, rather than clip source to force convergence; no two-iteration convergence is claimed. Multi-slot fitting establishes legal break boundaries at obstacle crossings; slot storage is inline-ascending while logical consumption follows the base direction; justification fits each occupied slot independently; a bidi-reordered line proves that its indivisible clusters fit their assigned slots.

### Layout options

```rust
pub struct LayoutOptions {
    pub justify: bool,
    pub compress_last_lines: bool,
    pub clamp: Option<LineClampPolicy>,
    pub strategy: LineBreakStrategy,
    pub hang_mode: HangMode,
    pub line_height: LineHeight,
    pub whitespace: HangingWhitespaceMode,
    pub soft_hyphen: SoftHyphenPolicy,
}
pub struct TabStops { .. }              // a finite, positive step, by construction
pub enum LineBreakStrategy { Greedy, Balanced, Publication }
pub enum LineHeight { Normal, Fixed(f32) }
pub struct StrutMetrics { pub ascent: f32, pub descent: f32, pub line_gap: f32 }
pub enum HangingWhitespaceMode { Collapsible, Preserved }
pub enum SoftHyphenPolicy { Keep, Strip }
pub struct HangMode { pub start: HangStartMode, pub end: HangEndMode }
pub enum HangStartMode { None, FirstLine, AllLines }
pub enum HangEndMode { None, Allow, Force }
pub struct LineClampPolicy { pub max_lines: Option<usize>, pub max_block_size: Option<f32>, pub ellipsis: Box<str> }
```

**[OPT-1]** `TabStops::try_new` accepts a finite, positive step (`TabStops::step`); `TabStops::DEFAULT` is 8. Stops lie at every multiple of the step along the line's inline progression, measured from the line-start edge of the slot (ST3-1). `TabStops::next_stop(x)` is the first stop strictly beyond the distance `x`, or `None` when `x` is not finite or no finite forward stop is representable.

**[OPT-2]** `LineHeight::is_valid`: `Normal`, or `Fixed(m)` with `m` finite and positive.

**[OPT-3]** A `LineClampPolicy` is valid (`LineClampPolicy::is_valid`) when at least one limit is set, `max_lines` is positive, `max_block_size` is finite and positive, and the ellipsis holds no line or paragraph separator; an empty ellipsis is silent truncation. `LineClampPolicy::max_lines` and `LineClampPolicy::max_block_size` build one limit with U+2026 as the ellipsis.

**[OPT-4]** `HangMode::NONE` hangs nothing. `HangMode::allows(side, direction, is_first_line)` maps logical edges to physical sides: the start edge is the left side of an LTR paragraph and the right side of an RTL one (`HangSide::is_line_start`); the start edge hangs under `AllLines`, and under `FirstLine` only on line 0; the end edge hangs unless `HangEndMode::None`, whatever the line.

### Stage 1: line breaking and boundary re-shaping

**[ST1-1] 1.1 Overflow ladder.** Break opportunities are used in this order: UAX #14 opportunities with the CL/CP/EX/IS/SY/IN prohibitions intact; tailored opportunities (URLs); and only for a unit wider than the whole line, the emergency break (1.5). A discretionary's tier on the ladder is read from its penalty net of its `CLONED_SPAN_PENALTY`s (WN-21): in the URL band below `URL_PENALTY_MAX`, a URL break at punctuation, tier 1; at `URL_PENALTY_MAX`, a break inside a long URL piece, tier 2; any other, hyphenation and math breaks included, tier 0. A line-fitting strategy without a global cost (`Greedy`) ends a line at a fitting break of the lowest tier the line holds. If the last text box with the boxes fused to it does not fit, the whole unit moves to the next line: there is no emergency break between fused boxes while the unit still fits.

**[ST1-2] 1.2 Kinsoku and compression.** Closing brackets, full stops and commas never begin a line and opening brackets never end one. The width adjustment of consecutive and line-edge punctuation is selected by profile (`zh-Hans`, `zh-Hant`, `ja`).

**[ST1-3] 1.3 Hanging punctuation.** Under `HangEndMode::Allow` a full stop or comma (`HANGABLE_END`) that is the line's last visible cluster and would otherwise move to the next line hangs outside the text block; under `Force` it hangs even if the line could hold it. An opening mark (`HANGABLE_START`) that is the line's first visible cluster hangs at the line start under `HangStartMode::FirstLine` (line 0 only; a line after a hard break is not a first line) or `AllLines`. At most one character hangs at each edge. A mark hangs only when its level is the paragraph's base level, which puts it at the physical edge of the line after L2 in O(1); a `SpanBoundary` of non-zero width, or cloned padding, between the mark and that edge prevents hanging. 0.85 of the mark's advance protrudes beyond the content block's edge and 0.15 lies inside it: the hanging extent is the protruding part (OUT-4, HNG-1), and the line's body is measured with the inside part only. A full-width CJK mark keeps its ink in one half of its em box (a stop's or comma's left half, an opening bracket's right half), so protruding by more than half the advance puts the ink itself outside the text block, as JLREQ's hanging punctuation does, rather than only the blank half, which would look like half-width trimming. The breaker credits the protrusion when it fits a line (Stage 1.6.2): under `Allow` only when the line would not fit without it. Under `HangEndMode::None` and `HangStartMode::None` (the defaults) nothing protrudes.

**[ST1-4] 1.4 Boundary re-shaping.** A taken `Hyphenate` break re-shapes `ParagraphAst::break_window`: line *k* the part before the break with `TrailingEdit::for_opportunity` and the locale's hyphen (U+002D by default), line *k + 1* the part after it with `LeadingEdit::for_opportunity`. Following the HarfBuzz recipe, re-shaping walks back from the break to the first cluster start that is not unsafe to concatenate, shapes to the end of the line, checks that the result is clear at its start and splices, else steps back again; the next line applies the mirror image. At a chosen break between two `chws`-compressed punctuation clusters, the line-end mark is trimmed to half width when it does not fit and the line-start opening mark is restored to full width.

**[ST1-5] 1.5 Emergency cluster break.** A single unit wider than `wrap_width` on an empty line splits only where a UAX #29 grapheme boundary and a cluster boundary coincide; an unsafe seam re-shapes both sides. A bridge-produced cluster is never split; one indivisible cluster wider than the column occupies its own overflowing line (SLT-5 B).

**1.6 One parameterised breaker.**

**[ST1-6] 1.6.1 One state machine.** One function, `break_lines_parameterized(ast, constraints, options, beam_width, window_depth)`, runs every strategy; `LineBreakStrategy` will map onto its parameters, `Greedy` to (1, 1), `Balanced` (the default) to (2, 2) and `Publication` to (8, 6); until then every strategy breaks greedily (ME-2). There are no separate greedy and optimal code paths. A line is one step `(a, b]` between consecutive chosen breakpoints, and every mandatory breakpoint is chosen.

**[ST1-7] 1.6.2 Prefix sums.** Before scanning, the breaker builds prefix sums over the items of natural width and of per-tier stretch and shrink. For a candidate from discretionary *a* (exclusive) to *b* (inclusive), `b_vis` walks back from *b* over the collapsible glue immediately before it; under `Collapsible` width and capacities integrate over `(a, b_vis]`, and under `Preserved` the glue in `(b_vis, b)` keeps its width as trailing whitespace outside justification. The width is `W(a, b) = post_break_width(a) + (Prefix[b_vis] - Prefix[a]) + pre_break_width(b)`: interior discretionaries contribute `no_break_width`. Each cluster flagged `ALLOW_CJK_JUSTIFY_AFTER`, except the last visible one, adds the inter-character stretch `0.25 · font_size` of its run to Tier 2; `CjkPunctCompression` adds to Tier 0 shrink and `CjkAutospace` to Tier 1. Synthetic glue in `(b_vis, b)` collapses under `Preserved` too (WN-10). The `font_size` argument of `line_fit` is `ParagraphAst::strut_font_size`. The `Greedy` breaker fits a line at natural width when `LayoutOptions::justify` is false, which keeps every Phase 1 break; when it is true, a line fits while `W(a, b) - Z <= slot width`, so that it can be compressed (ST3-3). A line that ends the paragraph or at a hard break fits with shrink only under `LayoutOptions::compress_last_lines`; by default it fits at natural width, and when only shrink would fit it the line ends at the last earlier break that fits. Capacities are taken over the visible suffix after the candidate's last tab (Stage 3.1).

```rust
pub struct TierCapacities { pub stretch: [f32; 3], pub shrink: [f32; 2] }
pub struct LineFit { pub ratio: f32, pub badness: f32, pub overfull: bool, pub excess_slack_bits: u64 }
```

**[ST1-8] 1.6.3 Tiered badness.** `line_fit(slack, capacities, font_size)` with `slack = wrap_width - W(a, b)`, `Y_01 = TierCapacities::primary_stretch`, `Y_total = TierCapacities::total_stretch` and `Z = TierCapacities::total_shrink` has five exhaustive cases:

- **A**, `|slack| <= FIT_EPSILON` (1e-3), or `slack == +∞` (an unbounded line, GEO-2, whose glue never stretches; AX-6): ratio 0, badness 0;
- **B**, `slack < 0`: overfull (`overfull` set, ratio `OVERFULL_RATIO` −2, badness `OVERFULL_BADNESS` 1 000 000) if `Z <= CAPACITY_EPSILON` (1e-5) or `-slack > Z`, else ratio `slack / Z` and badness `100 |r|³`;
- **C**, `slack <= Y_01`: ratio `slack / Y_01`, badness `100 r³`;
- **D**, `slack <= Y_total`: with `r2 = (slack - Y_01) / Y_2`, ratio `1 + r2` and badness `100 + 300 r2³` when `Y_01 > 0`, else `400 r2³`;
- **E**, otherwise: with `r_excess = (slack - Y_total) / max(font_size, 1)` and a base of 400 if `Y_2 > 0`, 100 if only `Y_01 > 0`, 0 otherwise, ratio `2 + r_excess` and badness `min(MAX_BADNESS, base + 400 r_excess³)` with `MAX_BADNESS = UNDERFULL_BADNESS_CAP = 100 000`. At saturation `LineFit::comparison_key` orders by `(badness, excess_slack_bits)`, so the smaller excess wins.

Precondition: `slack` is finite or `+∞`, `font_size` finite and non-negative, every capacity finite and non-negative. The input gate guarantees it for every sum the breaker forms (MSR-1); `line_fit` does not check it. Badness is continuous at every seam; beyond `Y_01` it climbs through Tier 2 towards 400, so the optimiser prefers breaks that need only word spaces and autospace.

**[ST1-9] 1.6.4 `Balanced`.** At each line boundary the machine keeps at most two candidates near the margin, the furthest feasible one and the preceding natural or low-penalty one, in fixed stack state; while it crosses the next line it compares the two-line demerits of both paths and commits the winner for the earlier line at once.

**[ST1-10] 1.6.5 Dominator commit and lazy re-shaping.** When every live path traces back to the same discretionary within the window, every line up to it is committed. `reshape_window` is never called while candidates are evaluated, except for the emergency seam of ST1-5 (AX-6); when a line is committed, a `Hyphenate` action or a boundary without `safe_to_concat_before` re-shapes once. **Rejoin rule:** every untaken `Hyphenate` break inside the committed line whose following cluster has `safe_to_concat_before == false` is re-shaped as one window with `LeadingEdit::None` and `TrailingEdit::None` (`ParagraphAst::rejoin_windows`). If the exact width then overflows by more than `Z`, the breaker falls back to the beam's runner-up for that line. Phases 1 and 2 reflow the whole paragraph.

**[ST1-11] 1.6.6 Demerits.** Demerits follow Knuth and Plass from the tiered badness and the penalty *n* of node *b*: `(1 + b + n)²` for `n >= 0`, `(1 + b)² - n²` for a negative finite penalty, `(1 + b)²` for a forced break, plus α when this break and the previous one are both flagged, γ across incompatible fitness classes, and a runt penalty for a last line of one word or fewer than four characters. A break is feasible only below the tolerance; an emergency pass always finds a solution. The number of active nodes is capped.

**[ST1-12] 1.7 Trailing whitespace.** Synthetic glue at a line edge collapses to zero width under both modes (WN-10). Under `Collapsible` (CSS `normal`), only trailing glue with `is_collapsible` collapses to zero width; it stays in the line's `byte_range` and `visible_byte_end` marks the last visible cluster; no-break spaces are text and stay in the body (WN-12). Under `Preserved` (CSS `pre-wrap`) trailing spaces hang outside the body but keep selectable, caret-reachable positions: caret stops on the far edge of the base direction, with no glyph (HNG-3, CRT-2).

### Stage 2: per-line bidi reordering

**[ST2-1]** After line breaks are fixed, each line is reordered independently. **L1:** segment separators (tabs) and paragraph separators, any whitespace or isolate formatting characters before them, and any such sequence at the end of the line reset to the paragraph level, using the original classes. The middle-end reads them from the items, never from the text (AX-4): a glue whose `GlueNode::original_bidi_class` is WS or BN (every glue, WN-14), a cluster flagged `L1_RESETTABLE` (FE-30; with UAX #9 §5.2, the removed controls and BN reset with whitespace) and a tab. A `SpanBoundary` behaves as BN: it never interrupts such a sequence, and it is never reset on its own: after L1 a `Leading` boundary takes the level of the next item of its span on the line and a `Trailing` one that of the previous item, keeping its own when the line holds none. U+2028 is class WS.

**[ST2-2] L2:** from the highest level on the line to the lowest odd level, including levels not present, every contiguous sequence at that level or higher is reversed. A cluster's glyphs are reversed when its final reversal count is odd. N0 and BD16 are not re-run per line.

**[ST2-3] Fragments.** `SpanBoundary` items take part in L2 at their post-L1 level. After L2, the items of one span that are contiguous both visually and logically merge into one `PositionedSpanFragment` from their leftmost to their rightmost edge, its bytes the cover of theirs; a span split by L2 yields one fragment per such group, and a visual group whose logical bytes have a gap splits there (FRG-1). A group of zero-width boundaries alone, with no content or padding, is dropped, so a span keeps exactly one first and one last fragment (PAR-3). Atomic fragments are grouped the same way (FRG-3). Cloned padding at a line edge extends the fragment at that physical edge: the line end is the right edge in an LTR paragraph and the left edge in an RTL one. A fragment's `bidi_level` is the level of its first item in visual order.

### Stage 3: justification and tabs

**[ST3-1] 3.1 Tabs.** A tab advances to `TabStops::next_stop` of its distance from the slot's line-start edge along the inline progression of the base direction (OPT-1): from `InlineSlotConstraint::start` rightwards in an LTR paragraph, and from `InlineSlotConstraint::end` leftwards in an RTL one, or from the flow origin 0 when that end is infinite. The distance is the advance of everything logically before the tab on its line (the line-start padding included); L1 puts the tab at the paragraph level, so that content lies on its line-start side whatever the levels inside it. A stop is reachable only inside the tab's slot: when `next_stop` is `None` or its distance exceeds the slot's `InlineSlotConstraint::width`, the tab has no reachable stop. The line then breaks before the tab at an earlier opportunity that fits, if it has content before the tab and such an opportunity exists; otherwise the tab is consumed with zero advance and the line ends right after it, at the opportunity that follows it or, with none, at the tab's end, a second exception to WN-1 besides the emergency break (AX-6). On a line with a tab, glue left of the last tab is frozen at natural width in scoring and in justification alike.

**[ST3-2] 3.2 Priority ladder.** Justification applies only when `LayoutOptions::justify` is set, and only to lines that end neither the paragraph nor at a hard break, with `slack` the slot's width less the line's visible width `W` (hanging halves outside, ST1-3). Only the glue and gaps after the line's last tab take part (ST3-1). Expansion spends Level 0 (`GlueTier::Tier0`: word spaces, up to their `max_stretch`), then Level 1 (`Tier1`: CJK autospace, up to their `max_stretch`), then Level 2 (`Tier2`: the `ALLOW_CJK_JUSTIFY_AFTER` gaps of ST1-7, `0.25 · font_size` each), then Level 3, each level filled before the next and shared within a level in proportion to the capacities. Level 3 is the residual inter-cluster expansion: `0.125 · font_size` of the run at every boundary between two visible clusters of the line with nothing but glue or span boundaries between them (an inline object interrupts it), except after a cluster that already has a Tier 2 gap, after a `NO_LETTER_SPACING` cluster and after a cluster whose run's script is Thai, Laoo, Khmr or Mymr (ST3-5). Expansion at a gap after a cluster lies on that cluster's logical end: in an LTR cluster it is added to the advance of its last glyph in visual order; in an RTL cluster it is space before its first glyph in visual order. The same ladder drives the badness (ST1-8).

**[ST3-3] 3.3 Compression.** Lines are broken optimistically and then compressed (ST1-7): a line with `slack < 0` and `-slack <= Z` shrinks Tier 0 (word spaces and `CjkPunctCompression`, shared in proportion to their `max_shrink`, `TierCapacities::shrink[0]`), then Tier 1 (the autospace gap down to its minimum), until it is flush. A line that ends the paragraph or at a hard break is never stretched; it is compressed this way only under `LayoutOptions::compress_last_lines` (off by default, ST1-7).

**[ST3-4] 3.4 Tolerance.** The tolerance is `slack <= Y_total`, a `LineFit::ratio` of at most 2: Tiers 0 to 2 cover the slack. A line beyond it is rejected by `Balanced` and `Publication` when an alternative exists. Otherwise, and under `Greedy`, Level 3 covers what it can: a line with `slack <= Y_total + Y_3` is justified flush, and an over-loose one spends every level to its cap and leaves the rest ragged at the line end. `PositionedLine::is_justified` is true exactly for a flush justified line, a compressed last line (ST3-3) included.

**[ST3-5] 3.5 Scripts.** `NO_LETTER_SPACING` atoms receive no inter-letter expansion, and no kashida is inserted in Phase 2, so Arabic lines justify at word spaces only. Thai, Lao, Khmer and Burmese expand at word separators only.

### Stage 4: strut, blank lines and vertical extents

**[MET-1] 4.1 Strut.** `LineHeight::resolve_strut(font, font_size, mode)` resolves the active strut from the raw metrics: `Normal` keeps the font's ascent, descent and line gap; `Fixed(m)` gives a total height `m · font_size`, splits the leading `total - (ascent + descent)`, negative included, equally above and below, and sets the line gap to 0. A vertical mode reads `vertical_ascent`, `vertical_descent` and `vertical_line_gap`. The result is valid (`StrutMetrics::is_valid`) when its fields are finite and `StrutMetrics::line_height` (`ascent + descent + line_gap`) is finite and positive; a negative ascent or descent alone is valid. The raw metrics are never mutated.

**[MET-2]** Under `Normal` an inline box's layout bounds enclose all its glyphs, fallback glyphs included; under `Fixed` they derive from the primary font alone and fallback glyphs MAY overflow the line box. The font's line gap enters only under `Normal`.

**[MET-3] 4.2 Blank lines.** *N* hard breaks give *N* + 1 lines, and a trailing U+2028 gives one trailing blank line: `""` is one line `0..0`; `"abc"` one line `0..3`; `"abc\u{2028}"` the lines `0..6` (visible end 3) and `6..6`.

**[MET-4] 4.3 Marks.** Shaper `y_offset` is preserved exactly, with no clamp; rendering MAY clip.

**[MET-5] 4.4 Annotation room.** A side needs `base_over + block_extent` (`Over`) or `base_under + block_extent` (`Under`) from the base. Under `Normal` the line's ascent (descent) grows to the larger of the strut's and that requirement and the annotation sits at `baseline_offset >= base_over` (`<= -base_under`); under `Fixed` the box stays the strut's and the annotation overflows (ANO-3).

### Stage 5: decorations

**[ST5-1]** Underline and strike-through merge across clusters adjacent in visual order and the glue between them, synthetic glue included; a decoration and a span fragment stop at the content block's edge and never cover the protruding part of a hanging mark (SLT-4, ST1-3). An underline uses the lowest `underline_position` of the intersecting fonts, the minimum under FNT-4's sign, with that font's thickness. Phase 1 also merges across the word spaces between two decorated clusters, never into trailing whitespace; a strike-through takes the `strikeout_position` of the cluster that opens it and the `underline_thickness` of that font, since `FontMetrics` has no strike-out thickness.

### Line clamping

**[CLP-1]** *Future.* `LayoutOptions::clamp` truncates a paragraph at `max_lines` lines (empty lines from hard breaks counted, clearance jumps not) or `max_block_size`, keeping at least one line. Truncation respects cluster, tate-chu-yoko, atomic-edit and ruby boundaries. The suffix is one synthesized sequence (every glyph without a byte range). The paragraph reports the unrendered tail as `truncated_range` and ends with exactly one ellipsized line (PAR-3). In `VerticalLr` the block limit uses the mode's line bounds.

---

## 6. Carets, hit testing and selection

```rust
pub struct CursorPosition { pub byte_offset: ByteIndex, pub affinity: CaretAffinity }
pub struct HitResult { pub position: CursorPosition, pub inside: bool }
pub struct CaretVisual { pub primary_rect: Rect, pub secondary_rect: Option<Rect> }
pub enum Motion { Left, Right, Previous, Next, WordLeft, WordRight, WordPrevious, WordNext,
                  LineStart, LineEnd, Up, Down }
pub enum FlowMotion { InlinePrev, InlineNext, BlockPrev, BlockNext }
pub enum ParagraphEdge { Start, End }
pub enum CursorMove { Moved(CursorPosition),
                      Exited { edge: ParagraphEdge, inline_position: Option<f32> } }
pub struct Selection { pub anchor: CursorPosition, pub focus: CursorPosition }
pub struct SelectionRect { pub rect: Rect, pub line: usize, pub level: BidiLevel, pub is_line_end_extension: bool }
pub enum BoxHeightStyle { Tight, Max, Strut }
```

```rust
impl ValidatedLayout<'_> {
    fn hit_test(&self, point: Point) -> HitResult;
    fn hit_test_on_line(&self, line: usize, inline_position: f32) -> HitResult;
    fn caret_geometry(&self, position: CursorPosition) -> CaretVisual;
    fn move_cursor(&self, position: CursorPosition, motion: Motion,
                   preferred_inline: Option<f32>) -> CursorMove;
    fn selection_geometry(&self, shaper: &dyn TextShaper, range: TextRange, height: BoxHeightStyle) -> Vec<SelectionRect>;
    fn range_rects(&self, range: TextRange) -> Vec<Rect>;
    fn rect_for_offset(&self, offset: ByteIndex) -> Rect;
}
impl ValidatedInputs<'_> {
    fn delete_backward_range(&self, offset: ByteIndex) -> TextRange;
    fn delete_forward_range(&self, offset: ByteIndex) -> TextRange;
    fn copied_text(&self, range: TextRange) -> String;
}
pub fn utf8_to_utf16(text: &str, offset: ByteIndex) -> Option<usize>;
pub fn utf16_to_utf8(text: &str, offset: usize) -> Option<ByteIndex>;
```

The receivers are the gate types (AX-3), which carry the text the data was validated against, so no query takes it again (AX-2). Visual queries read the caret-stop table of the validated output. Word motions segment the receiver's text. `BoxHeightStyle::Tight` asks the shaper, through the shaper gate, for the metrics of each run's font instance (`VisualGlyphRun::font`, `font_size`, `variations`). Deletion ranges read atoms, flags and atomic spans from the validated AST and code points from its text. `hit_test_on_line` places the caret on one line, by index, at a position along the inline axis in flow space; it is how a caller re-enters a paragraph on its first or last line (INT-15).

The data types and three pure helpers (`AtomicFragment::hit_test`, `Motion::flow_direction`, `resolve_ligature_carets`) exist, and so does every query of the block (INT-20, INT-21).

### 6.1 Caret representation

**[INT-1]** A caret position is `(byte_offset, affinity)`. `CaretAffinity::Upstream` leans towards the start of the text: at a soft wrap, upstream is the end of the first line and downstream the start of the second; affinity also resolves a bidi boundary.

**[INT-2]** A caret's `byte_offset` is an atom boundary. A deletion range need not be (§6.5).

**[INT-3]** Each line has one `CaretStop` per visual boundary, in visual order, with two stops at a direction boundary; hit testing, caret geometry, movement and selection share this table (CRT-1, CRT-2).

**[INT-4]** No caret stop lies strictly inside an `AtomicEditSpan` (`AtomicEditSpan::is_interior`), even where the span wraps or holds several words; only its start and end produce stops.

### 6.2 Ligature carets

**[INT-5]** A cluster of one atom, or flagged `IS_EMOJI_SEQUENCE`, is one caret slot. A cluster of *K* > 1 atoms has *K* sub-slots: `resolve_ligature_carets(width, atom_count, carets)` accepts exactly *K* − 1 finite, strictly increasing positions inside `(0, width)` from `TextShaper::ligature_carets`, and otherwise divides the width equally; it is the shaper gate for carets (G2) and the only place they are checked. Its `width` is a validated cluster width, a precondition it does not check (AX-3); a zero-width cluster has no sub-slots. In RTL the sub-slots run right to left in logical order: the positions, increasing in physical `x` from the cluster's left edge (SHP-7), are placed at `left + position` and paired from left to right with the cluster's atoms in reverse logical order. A tate-chu-yoko cluster is not subdivided.

**[INT-6]** Collapsed trailing whitespace has no slot; a click beyond the content is resolved by INT-8. Preserved trailing spaces keep caret stops on the far edge of the base direction, beyond the last visible entity, and no glyph (HNG-3).

### 6.3 Hit testing

**[INT-7]** `hit_test(point)` clamps `point.y` to the first or last line (`inside == false` when clamped), finds the line by binary search, and within it takes the nearest caret stop: in a slot, the left half maps to the leading side and the right half to the trailing side by level parity.

**[INT-8]** A point beyond the physical edge of a line's visual content converges on the extreme caret stop in that physical direction (`PositionedLine::extreme_caret_stop`; equal-`x` ties resolve by visual order). It coincides with `(visible_byte_end, Upstream)` only when the content at that edge has the base direction; an unconditional jump to `visible_byte_end` is forbidden. Trailing whitespace lies on the far edge of the base direction (UAX #9 L1).

**[INT-9]** A click on the second half of the last visible grapheme of a wrapped line yields `(visible_byte_end, Upstream)`; one before the first grapheme of the next line yields `(offset, Downstream)`.

**[INT-10] Atomic spans.** Hit testing reads `PositionedLine::atomic_fragments` directly (`AtomicFragment::hit_test`): it splits at the clicked fragment's midpoint; at an even level physical left maps to the span start (downstream) and right to its end (upstream), at an odd level the halves swap, and the midpoint belongs to the right half.

### 6.4 Caret geometry

**[INT-11]** Affinity chooses the line and the side at a soft wrap or a bidi boundary: `primary_rect` is at the stop of the position's offset and affinity (CRT-3), and `secondary_rect` is `Some` at the stop of the same offset with the other affinity on the same line, if and only if its `x` differs from the primary one beyond the geometry tolerance.

### 6.5 Movement and editing

**[INT-12] Deletion is logical.** Backspace deletes the last code point of the atom before the offset, except that an atom flagged `IS_EMOJI_SEQUENCE` or `HAS_VARIATION_SELECTOR`, a CRLF pair and a tag sequence go whole; Delete has its own forward rule. When the atom before (Backspace) or after (Delete) the offset lies in an `AtomicEditSpan` (`ParagraphAst::atomic_span_containing`), the whole span is deleted.

**[INT-13] Arrows are visual.** Inline-axis arrows (`FlowMotion::InlinePrev` and `FlowMotion::InlineNext` from `Motion::flow_direction`: Left and Right in horizontal modes, Up and Down in vertical ones) move across the line's caret stops (one keystroke crosses a whole atomic span); past a line end they skip collapsed whitespace to the neighbouring line. A move towards the line start of the base direction from the first line's last stop on that side, or towards the line end from the last line's last stop on that side, returns `CursorMove::Exited` at `ParagraphEdge::Start` or `ParagraphEdge::End` with no inline position; the engine decides this from the visual stops, so a caller never infers it from offsets in bidi text. Logical and word motions exit the same way at offset 0 and at the text end; `LineStart` and `LineEnd` never exit. Shift with an arrow extends the **focus** visually; a selection is stored logically as anchor and focus; Backspace and Delete stay logical. `Motion` separates physical Left and Right from logical Previous and Next. A visual step goes to the next stop in that direction whose `x` differs from the current one beyond the geometry tolerance, so that a keystroke never stalls on coincident stops; of several stops at the target `x` it takes the first in visual order when moving right and the last when moving left. Word motions stay logical (INT-20).

**[INT-14] Words** move by UAX #29 word boundaries in a separate pass, not by `BoxNode::Text`.

**[INT-15] Block-axis movement** (`FlowMotion::BlockPrev` and `FlowMotion::BlockNext`: Up and Down in horizontal modes, Right and Left in `VerticalRl`, Left and Right in `VerticalLr`) keeps a preferred position along the inline axis in flow space, held by the caller (`preferred_inline`, taken from the caret when `None`), and samples the target line through it (`hit_test_on_line`); the caller resets it after any inline-axis move or edit. `BlockPrev` on the first line and `BlockNext` on the last return `CursorMove::Exited` at `ParagraphEdge::Start` or `ParagraphEdge::End` with that inline position; the caller re-enters the neighbouring paragraph with `hit_test_on_line` on its last or first line, or, with no neighbour, places the caret at the text start or end.

### 6.6 Selection geometry

**[INT-16]** Selection is stored logically and drawn as visual pieces: the merge runs over visual order and joins only visually adjacent selected slots (and the glue margins between two selected slots), never bridging unselected ones; an empty range gives no rectangle. A selected line end MAY add an extension rectangle (`is_line_end_extension`). `BoxHeightStyle::Tight` is the union of the selected instances' ascent and descent; `BoxHeightStyle::Max` adds line gap and the tallest inline box; `BoxHeightStyle::Strut` uses the resolved strut. No ink bounds are needed.

**[INT-17] Clipboard.** Copying reads the source text, so a synthesized glyph (SRC-1) is never copied. An actual U+00AD in the source follows `SoftHyphenPolicy` (`SoftHyphenPolicy::Keep` by default). Caret positions inside a rebuilt hyphenation prefix map to the original bytes.

### 6.7 Platform interfaces

**[INT-18]** `rect_for_offset` is the primary caret rectangle of the offset, downstream. `range_rects(range)` places an IME candidate window, and `utf8_to_utf16` and `utf16_to_utf8` convert offsets, rejecting one inside a code point or past the text. *Future:* `offset_for_point` (`hit_test`), a `CompositionSpan` that never changes segmentation (SPN-5), and per-line accessibility exports of character lengths, positions and widths and word starts.

### 6.8 Writing modes, slots and truncation

**[INT-19]** Physical arrows resolve once, at the interaction boundary, through `Motion::flow_direction(mode)` to `FlowMotion`; logical and word motions resolve to `None` there. Queries transform at their physical boundary (§2.7) and then work in flow space. A hit on an obstacle between slots snaps to the nearer caret stop, ties towards positive inline. Selections split by slot. Ruby hits map to the base text; an annotation is never a caret target. Ellipsis hits, and caret requests into the hidden tail, map upstream to `truncated_range.start()`. Tate-chu-yoko text is atomic for editing only under an `AtomicEditSpan`.

**[INT-20] Phase 1 queries.** A line's caret stops are the visual edges of its pieces (clusters, objects, tabs, word spaces and span edges), with the INT-5 sub-slots inside a ligature, one stop per distinct offset and position, none strictly inside an atomic span. At a direction boundary the two stops of one offset take CRT-3's affinities. A soft-wrapped line's last logical stop is upstream; under `Collapsible` it has no stop past its visible end, whose offsets belong to the next line's start. A line that ends at a hard break or the paragraph end keeps stops on its trailing word spaces, on the far edge of the base direction: with their widths under `Preserved`, all at the edge under `Collapsible`, so the end of the text stays reachable; no stop lies after a U+2028, whose end starts the next line. `hit_test` places the point on the first line whose box ends below it, the last one past them; on a line, a point inside an atomic fragment snaps by INT-10, one beyond an edge by INT-8, and any other point takes the nearer of the two stops around it, the right one at the midpoint. `caret_geometry` finds the stop with the position's offset and affinity, else its offset, else the nearest offset before it on its line, and returns zero-width rectangles over the line box. `Previous` and `Next` move to the neighbouring caret offset of the paragraph, downstream; the word motions to the neighbouring caret offset that is also a UAX #29 word boundary, `WordLeft` and `WordRight` resolved through the base direction.

**[INT-21] Phase 1 selection, deletion and copying.** `selection_geometry` walks each line's caret stops in visual order; the stretch between two neighbouring stops is selected when the bytes between their offsets lie inside the range, and selected stretches merge until an unselected one; a stretch whose stops share an offset neither joins nor breaks a merge. No line-end extension is drawn. A `Tight` piece spans the ascents and descents, without half-leading, of the font instances of the selected glyph runs it crosses, asked through the shaper gate, and of the inline boxes it holds, else the strut; `Max` spans the line box and `Strut` the resolved strut. `range_rects` returns the same pieces over the line box. `delete_backward_range` first moves an offset past the text or off a character boundary back to one; at 0 it is empty; when the byte before the offset lies in an atomic span it is the span, in an inline object the object, at the end of an atom flagged `IS_EMOJI_SEQUENCE` or `HAS_VARIATION_SELECTOR` the atom, and otherwise the last code point before the offset. `delete_forward_range` is empty at the end of the text and otherwise the atomic span or object holding the byte after the offset, else the atom starting there, else the next code point. `copied_text` returns the source bytes of the range, clamped to character boundaries, dropping U+00AD under `SoftHyphenPolicy::Strip` (INT-17).

---

## 7. Roadmap and acceptance

**[RM-1]** Every phase passes the Unicode conformance data for its components against the pinned data version (`GraphemeBreakTest.txt`, `WordBreakTest.txt`, `LineBreakTest.txt`, `BidiTest.txt`, `BidiCharacterTest.txt`).

**[RM-12] Recorded conformance differences.** Where a pinned segmenter's stable rule tables predate its data version, the RM-1 suite records the cases that differ, by line, in `mutil-engine/tests/assets/ucd/known-differences.txt`, and asserts exactly that set and its count, so that a fix and a regression both fail it; a suite with no recorded section passes in full. The set is regenerated from the harness's output when a dependency changes, never edited by hand. With ICU4X 2.4 the grapheme suite records 10 cases (the Unicode 18.0 form of GB9c), the line suite 328 (the stable line segmenter's Unicode 15.1 rules), and the word, `BidiTest.txt` and `BidiCharacterTest.txt` suites none; the `unstable` constructors, which FE-18's stable interfaces exclude, are not used to close them.

### Phase 0: foundation (done)

**[RM-2]** The data model of §2 with its parsed value types, the coordinate transforms of §2.7, the input validator (§2.5) and the output validators (§2.6), checked geometry queries, the tiered badness function, the `TextShaper` contract with `NoopShaper`, and the contract tests (`tests/contracts/`, one test binary; span-scaling benchmarks are `#[ignore]`d and run explicitly). No segmenter, real shaper, line breaker or editing query exists yet.

**[RM-9] Phase 0 freeze.** From the end of Phase 0 the existing requirement identifiers and their meaning, the public types and signatures in checked `rust` blocks, and the contract tests that assert them are frozen. Later phases MAY, without reopening the freeze: add public types, functions or variants together with their checked spec blocks (`scripts/check_spec.py`); add requirements under new identifiers (DOC-2); turn *future* statements into descriptions of the implemented behaviour (ME-1, RM-2, SCO-4, §6); and replace the tests that assert only Phase 0 skeleton behaviour, such as an empty `lines` vector or `NoLines`. Any other change to a frozen item needs an execution defect that shows a real contradiction in this specification. The boundary, relative to the repository root:

- frozen: `mutil-engine/src/types/*`; the data definitions in `mutil-engine/src/frontend/{ast,options,span,annotation}.rs`; `mutil-engine/src/shaping/{traits,profile,noop}.rs`; `mutil-engine/src/middleend/{output,options,badness,tolerance,input_validation,output_validation,error_messages,validated,shaper_gate}.rs`; `SPECIFICATION.md`; `mutil-engine/tests/contracts/*`;
- frozen under the additive rule: `mutil-engine/src/lib.rs`, `mutil-engine/src/{types,frontend,shaping,middleend}/mod.rs` and `mutil-engine/Cargo.toml`, together with the existing public items of `mutil-engine/src/middleend/{layout,interaction}.rs` (among them `is_word_space`, `is_paragraph_separator`, `split_paragraphs`, `layout_paragraph` and the §6 data types and helpers). Their signatures and meaning are frozen; new modules, exports, dependencies and implementations MAY be added;
- open: the new frontend modules (`segment`, `bidi`, `script`, `compile`) that build a `ParagraphAst`, `mutil-engine/src/shaping/{harfrust,gpui}.rs`, and the bodies of `layout_paragraph` (the `Greedy` solver and vertical stacking) and of the §6 queries.

**[RM-10] Recorded exceptions to RM-9.** With the owner's explicit approval, the host contract (§8) changed frozen items: FNT-3 and the `FontId` documentation scope a `FontId` to its issuing shaper's id space and resolve it through a `FontSource` that need not be the shaper (the shaper-bound wording rested on a wrong premise); OUT-1 places the over offset through `over_to_block`, removing its contradiction with ORI-2 in `VerticalLr`, and states the offset axes (SHP-19); and the output gate rejects tate-chu-yoko runs until Phase 1 defines their placement (ORI-7). At the start of Phase 1 the owner approved these further exceptions, each an execution defect of the Phase 0 text or a decision the first implementation forced:

- **Toolchain.** The minimum supported Rust version rises from 1.85 to 1.88, the floor of the ICU4X release whose data is Unicode 18.0.0 (SCO-3, SCO-8).
- **Tier direction (SCO-6).** The diagram had `shaping` depend on `frontend`, while FE-3 has the frontend shape. It now reads `types <- shaping <- frontend`; `ShapingRun`, `TextDecoration`, `HyphenOpportunity` and `ReplacementText` moved to `mutil-engine/src/types/` (`run.rs`, `hyphen.rs`), unchanged, and the frontend re-exports them.
- **Space advance (SHP-20).** `TextShaper` gained `space_advance`, so that a word space is sized without shaping one (WN-10, SHP-5); `NoopShaper` and the contract tests' shaper implement it.
- **No-break spaces (WN-2, WN-10, WN-12, FE-19, ST1-12).** As glyph-less glue that stays in the body, a no-break space at a line end left the content block and `visible_byte_end` on no entity, which SLT-3 and SRC-3 forbid. It is now text, like U+3000; `is_word_space` excludes it, and the input gate accepts only a collapsible `WordSpace`.
- **Indented single-entity overflow (SLT-5).** The overflow state required a slot as wide as `wrap_width`, so an entity too wide for an indented first line had no valid output. The state now needs one slot, content from its start, and content wider than it.
- **Decoration sign (FNT-4, DEC-1, ST5-1).** No text fixed the sign of `underline_position`, `strikeout_position` or `DecorationLine::y_offset`; they now follow the engine's one convention, positive towards line-over.
- **Refused features (FE-28).** `FrontendFeature` gained `BidiText` and `BaselineAlignment`, so that Phase 1 refuses, rather than compiles wrongly, text with non-zero levels and baselines it does not place.
- **GDEF ligature carets (SHP-22, RM-7).** Reading GDEF caret lists moved from Phase 3 to Phase 1, where the HarfRust backend reads them; INT-5's equal-slot fallback remains for glyphs without carets and for the `LineOnly` profile.
- **Phase 1 is unidirectional (RM-3, RM-11).** UAX #9 resolution (frontend Step 2) moves to Phase 2 with Stage 2 (RM-5).
- **Oversized FE-15 runs (FE-15, ME-2).** An unbreakable dash, ellipsis or middle-dot run wider than an empty line's slot had no valid output: kept whole, several clusters overflowed the slot, which SLT-5 forbids. Such a run now moves whole only when something precedes it on the line, and is otherwise split by ST1-5.
- **Ladder tiers (ST1-1, ME-2).** ME-2's greedy breaker took the furthest fitting opportunity of any kind, which skipped ST1-1's order and broke inside long URL pieces while a slash or a space fitted. ST1-1 now assigns each discretionary a tier by its penalty, and the greedy breaker ends a line at the lowest tier that fits.
- **CJK face routing (SHP-12, SHP-25, TYP-7).** SHP-12 keyed fallback by script and locale, but the HarfRust backend chose by character map in stack order, so a Latin face first in the stack served a Han run's “ ” … — · with its narrow glyphs, and a stack of regional CJK faces ignored the run's locale. SHP-25 now profiles faces and routes CJK runs. `FontRegistry` gained `add_face_with_locale` and `profile`, with the new types `FaceProfile` and `CjkRegion`, and `LocaleTag` gained the constructor `try_from_str_lenient` in the frozen `types/tag.rs`; existing signatures are unchanged. The vendored Noto Sans JP subset was regenerated with U+00B7 and the Han of the regional tests.
- **Locale heuristics and Table S overrides (FE-9, FE-16).** The CLDR override of Table S is fixed as a table keyed by the primary language of the stated locale. FE-16 is scoped to a UAX #29 sentence, and kana and Hangul take the inferred locale with Han, so that a run does not split between them; the Serbian evidence covers the capitals Ђ and Ћ as well as ђ and ћ.
- **Locale tag length (TYP-4, TYP-7).** RFC 5646 sets no maximum length, so `LocaleTag::try_from_str` and `LocaleTag::try_from_str_lenient` allocated in proportion to any host string before rejecting it. Both now reject a tag longer than 128 bytes before allocating.

At the start of Phase 2 the owner approved these further exceptions, each a decision the bidi, autospace and justification design forced:

- **Normalisation data (SCO-8, FE-9, SHP-11, SHP-26).** `icu_normalizer` of the same ICU4X release joins the data sources and `Cargo.toml`, and the HarfRust backend is built with its `icu` feature, so that BD16's canonical equivalence, the shaper's mirroring and normalisation, and face coverage under NFC and NFD all read one data source; the `Cargo.lock` contract test covers `icu_normalizer_data`.
- **In-text controls (BID-1, BID-3, FE-10, FE-28).** BID-1 had every source control travel out of band, which left their bytes without an owner (AST-3) and could not express LRE and RLE. In-text controls and BN characters stay atoms with UAX #9 §5.2 levels; `bidi_spans` is the caller's own stream, merged by BID-3, and `FrontendError` gains `InvalidBidiSpan`.
- **New atom flags (ATM-3, ATM-4).** `AtomFlags` gains `L1_RESETTABLE`, `HANGABLE_START` and `HANGABLE_END`, so that L1 and hanging read flags, not text (AX-4); `AtomFlags::ALL`, `FIRST_ATOM_ONLY` and `LAST_ATOM_ONLY` grow with them in the frozen `types/atom.rs`.
- **Levels of boundaries and synthetic glue (WN-8, WN-10, WN-14, AST-12, AST-13).** A span boundary took the level of the first or last *atom* of its span, which split a span that starts with an object into two first fragments. It now takes that of the first or last inline entity, else the base level. Synthetic glue is collapsible, of class BN and at its predecessor's level; the input gate checks glue classes, the compression bound of WN-13 and the level floor (`LayoutInputError::InvalidBidiLevel`).
- **Fragments (FRG-1, FRG-3, ST2-3).** A post-L2 visual group can hold logically non-contiguous bytes (levels 0, 1, 2, 2 with a span over the first three), which one range cannot describe. Fragments are now contiguous both visually and logically.
- **Output order and hanging (OUT-3, HNG-4, LV-2).** The output gate checks screen order across visual runs (`VisualRunsOutOfOrder`) and at most one outermost hanging mark per side (`TooManyHangingGlyphs`, `HangingGlyphNotOutermost`).
- **Tabs (OPT-1, ST3-1).** Stops were measured from the flow-space origin, which in an RTL line made a tab's width depend on its line's alignment. They are now measured from the slot's line-start edge in the base direction, for LTR too.
- **Ligature carets in RTL (SHP-7, INT-5).** GDEF positions are physical, increasing from the glyph's left edge; RTL sub-slots pair them with the atoms in reverse order instead of mirroring them.
- **Split carets and visual steps (CRT-3, INT-11, INT-13).** The primary caret follows the affinity, the stops of a direction boundary carry CRT-3's affinities, and a visual step skips coincident stops.
- **Spacing and justification (FE-20, FE-27, ST1-3, ST1-7, ST3-2 to ST3-5, ST5-1).** Autospace classes and sizes are fixed (CssCompat is rigid, with 1 ic taken as 1 em), a cursive run takes no letter spacing, the Tier 2 capacity is normative, compression shares Tier 0 in proportion to capacity instead of the profile order ST3-3 listed, the tolerance and Level 3 are defined, `Greedy` fits with shrink under `justify`, and a hanging mark protrudes by 0.85 of its advance, so that a full-width CJK mark's ink lies outside the text block. A line that ends the paragraph or at a hard break is never stretched and, unless `LayoutOptions::compress_last_lines` is set, never compressed either: by default the breaker ends it earlier instead.

### Phase 1: foundational core

**[RM-3]** Scope: frontend Steps 1 to 6; the `TextShaper` with a HarfRust backend and the `LineOnly` profile; cluster merging by `atom_range`; the `Greedy` breaker with the overflow ladder, kinsoku and emergency splitting; atomic-span caret suppression, hit-test snapping and whole-span deletion; the strut; unidirectional caret, hit-test and selection queries.

**[RM-4]** Acceptance: a Devanagari conjunct is one atom under Unicode 15.1 or later and shapes in one run and font without a dotted circle (pre-15.1 data splits it); Thai, Lao, Khmer and Burmese break at model boundaries and the test records which model; a run of *N* `!` wider than the column wraps at cluster boundaries only after it reaches an empty line, and no line begins with CL, CP, EX, IS or SY except from the overflow fallback; CJK quotes, ellipses and dashes take the intended script and do not split across lines; emoji sequences move the caret atomically, Backspace removes them whole and removes a combining mark alone; long URLs wrap without overflow; `conformance_report()` names the actual data versions; a mention chip has no interior caret stop, a click inside it snaps to an end, and Backspace and Delete remove all of it, wrapped or not.

**[RM-11] Unidirectional Phase 1.** Phase 1 laid out left-to-right text only; Phase 2 lifted this (FE-30). The paragraph level is 0, and every atom, cluster, run and item takes it; a right-to-left base direction, vertical writing modes and the span streams not yet compiled are refused by `FrontendError::Unsupported` (FE-28) until their part of RM-3 or of a later phase is implemented. Base-direction detection (FE-8), the pairing tables (FE-9), atom levels (FE-10) and explicit bidi spans (FE-12) belong to Phase 2.

**[RM-13] Phase 1 status.** RM-3's scope is implemented as FE-29, SHP-22 to SHP-25, ME-2, BAS-1, INT-20 and INT-21 describe, and RM-4's clauses are the tests of `mutil-engine/tests/pipeline/acceptance.rs`, which name the two clauses tested elsewhere. The Unicode suites of RM-1 run with the differences RM-12 records. The vendored fonts hold no colour emoji face, so the emoji clauses test atoms, carets and deletion, not glyph choice.

### Phase 2: bidi, autospace and justification

**[RM-5]** Scope: RTL shaping with single-owner mirroring and logical order; Stage 2; split carets; visual arrows and Shift-arrow extension; `CjkAutospace` profiles; span fragments after L2; `ALLOW_CJK_JUSTIFY_AFTER`; Stage 3; hanging punctuation; inline objects with their overflow policies; continuous underlines.

**[RM-6]** Acceptance: Arabic and Hebrew order correctly across wraps, including a level-2 run in an RTL paragraph, whitespace before a mid-line tab and trailing whitespace; brackets mirror under BD16/N0, guillemets in an Arabic paragraph resolve as neutrals and a 64-deep opener nest behaves as BD16 specifies; mixed CJK and Latin autospaces by profile and justifies flush through the ladder; over-loose lines follow the tolerance rule; a `Clone` chip over two lines has full padding on both fragments, `Slice` only at its true ends, and a span split by L2 yields two fragments; cursive scripts get no letter spacing; clicks beyond either end of an RTL line, a mixed line, and above and below the text return the specified stops with the right `inside` flag.

**[RM-14] Phase 2 status.** RM-5's scope is implemented as FE-30, FE-31, SHP-26, ME-3, ST1-3, ST2-1 to ST2-3, ST3-1 to ST3-5, CRT-3, INT-5, INT-11 and INT-13 describe, under the exceptions RM-10 records for Phase 2. RM-6's clauses are the tests of `mutil-engine/tests/pipeline/acceptance_phase2.rs`, over the vendored Hebrew and Arabic subsets, and the `BidiTest.txt` and `BidiCharacterTest.txt` suites of RM-1 run in full.

### Phase 3: optimisation, hyphenation and performance

**[RM-7]** Scope: hyphenation behind an adapter (Liang patterns; libhyphen-style dictionaries for spelling changes); prefix re-shaping; synthesized hyphens; `Balanced` and `Publication` with thresholds, passes and the active-node cap; lazy re-shaping and dominator commits; whole-run caches; `locl` through the full-contract profile.

**[RM-8]** Acceptance: narrow English and German columns hyphenate with balanced line densities, honouring minimum characters on each side, and nothing is hyphenated in a word containing U+00AD; copied text holds no synthesized hyphen and U+00AD follows `SoftHyphenPolicy`; `zh-Hans`, `zh-Hant`, `ja` and `ko` variants resolve through `locl` under the full-contract profile; spelling-change hyphenation produces the replacement forms in a Dutch, a Catalan and a Hungarian test; candidate evaluation never calls `reshape_window`, which runs once per committed hyphenated boundary, and a micro-overflow falls back to the runner-up path; full single-paragraph relayout is benchmarked, and no frame-time guarantee is made before an invalidation model exists.

---

## 8. Host contract

This section binds any renderer that draws validated output. It states only what decides whether the drawn text is correct; drawing order, caching and other integration practice are non-normative and live in `docs/host-integration.md`.

```rust
pub struct FontBytes(..);               // shared font file bytes, never copied
pub struct FontFace { .. }              // bytes and face index, private fields
pub trait FontSource: Send + Sync { fn resolve_font(&self, font: FontId) -> Option<FontFace>; }
pub struct PlacedRun<'a> { .. }         // one body or annotation glyph run with its line
pub enum PlacedRunSource<'a> { Body(&'a VisualGlyphRun), Annotation(&'a PositionedAnnotationRun) }
pub struct PlacedGlyph { pub glyph_id: u32, pub origin: Point }
```

**[HST-1] Fonts.** A renderer resolves each run's `FontId` (`PlacedRun::font`) through the `FontSource` that covers the issuing shaper's id space (FNT-3), with `FontSource::resolve_font`, and draws the face `FontFace::face_index` of `FontFace::bytes`. Variation coordinates come only from `PlacedRun::variations`. `FontBytes::from_static`, `FontBytes::new` and `FontBytes::from_shared` wrap static data, an owned buffer and a shared buffer without copying their bytes (`FontBytes::as_bytes`).

**[HST-2] Font lifetime.** A `FontSource` never reuses an id, and keeps a face resolvable while any output that names its id may still be drawn. Never unloading a face within a session is the simplest way to satisfy this; a host that tracks its cached output MAY unload a face no output names.

**[HST-3] No re-shaping.** A renderer draws the glyph ids of the output, run by run, at the origins `ValidatedLayout::glyph_runs` gives. It yields every glyph of the paragraph: each line's visual runs, then its annotation runs (`PlacedRun::source`, `PlacedRunSource::Body`, `PlacedRunSource::Annotation`), with `PlacedRun::line`, `PlacedRun::font`, `PlacedRun::font_size`, `PlacedRun::orientation` and `PlacedRun::glyphs`. It never re-shapes, re-positions, substitutes or merges glyphs.

**[HST-4] Placement and orientation.** A `PlacedGlyph`'s `origin` is the paragraph-local physical point where the glyph outline's own origin is drawn: OUT-1's drawing origin mapped by the writing mode (ORI-1). An annotation glyph's baseline is the line's moved towards line-over by its run's `baseline_offset` (ANO-3, ORI-2), and its offsets are applied from there. The output gate checks that every origin is finite (AX-3), so `PlacedRun::glyphs` never yields a non-finite one. `Horizontal` and `VerticalUpright` glyphs are drawn unrotated there; a `VerticalSideways` glyph is rotated 90° clockwise about it, with `y` growing downwards. Vertical-origin and baseline adjustments are already in the origin (SHP-19), so a renderer never reads a font's vertical metrics or vertical-origin table to correct a position.

**[HST-5] Locked geometry.** A renderer keeps every glyph origin and advance exactly, on both axes, under any scale, and never changes a variation coordinate, `opsz` included. How it rasterises an outline (bitmap atlas, distance field, path rendering) is its own choice.

**[HST-6] Hinting.** A renderer that draws horizontal text at one fixed scale MAY hint along the block axis only. A renderer that applies a continuous scale, or draws vertical text, uses no grid fitting on either axis, since snapping to device pixels would break HST-5.

**[HST-7] Colour glyphs.** The text colour applies to monochrome glyphs and to COLR layers that reference the foreground colour (palette index 0xFFFF). It never recolours COLR/CPAL palette colours, sbix or CBDT bitmaps.

**[HST-8] Query geometry.** The rectangles of the §6 queries (INT-18) are in the same paragraph-local physical space as `PlacedGlyph::origin`; a range that wraps gives several. A host maps them to the screen by the paragraph's position in its container and then its own view transform.
