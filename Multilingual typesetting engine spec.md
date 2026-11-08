# Technical Specification for a Multilingual Universal Typesetting and Interactive Layout Engine (revised draft, research date 2026-11-07)

**Status of this document.** This is a revision of the user's draft specification, rebuilt against the reference documents and implementations catalogued in the research notes. The draft's section structure (1 to 7), three-tier architecture, data-structure names and phased roadmap are retained wherever they survive scrutiny. Every requirement carries one of three labels:

- **[Sourced]**: backed by a reference listed in the References section. Reference tags such as `[U3]` point to that list.
- **[Design decision]**: a choice made by this specification. It may be informed by sources but is not stated by them.
- **[Unverified]**: asserted by the draft or by background knowledge, but not confirmed by the research. It is kept because it is plausible and useful, and it must be validated before it is relied upon.

Version-specific facts carry their dates. Where a source page is itself marked draft, proposed or preliminary, that is said so. Non-official sources are flagged in the References.

**Summary of what changed.** The draft's architecture stands. Its main errors fall into six groups. First, break safety: `is_safe_to_break_after` inverts the HarfBuzz flag's polarity and omits `UNSAFE_TO_CONCAT`, which is the flag that matters for hyphenation `[B1]`. Second, Unicode conformance: the bracket table cannot be extended with quotation marks without breaking UAX #9 conformance, L1 and L2 are mis-stated, and grapheme segmentation needs the Indic conjunct rule GB9c `[U2][U3]`. Third, line breaking: "unified punctuation binding" is already UAX #14 behaviour, the "emergency break between punctuation marks" contradicts UAX #14 and CJK practice, the Knuth–Plass demerits are incomplete, and the 5,000-byte switch conflicts with the 500,000-byte target `[U7][L1][L2]`. Fourth, CJK typography: the autospace numbers, the `1.5x` justification guard and the absence of a compression pool are not supported by JLReq, CLREQ or CSS Text 4 `[W2][W4][W5]`. Fifth, interaction: caret affinity, RTL line ends, Shift+arrow behaviour, Backspace granularity and selection rectangles all need rewriting `[C1][C2][C3]`. Sixth, platform claims: the GPUI adapter cannot carry the contract as written, and `hypher` cannot supply spelling-change hyphenation `[B6][H1]`.

---

## Changes from the draft

| # | Draft claim (location) | Verdict | Correction in this revision | Source |
|---|---|---|---|---|
| 1 | `is_safe_to_break_after: bool` derived from `UNSAFE_TO_BREAK` (2.3) | CORRECTED | HarfBuzz's flag marks the start of the cluster a glyph belongs to, so the field becomes `safe_to_break_before`. Add `safe_to_concat_before`, which needs `HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT` (HarfBuzz 4.0.0, 2022-03-01). | B1 |
| 2 | Req. 1 (shape whole run) and Req. 5 (split runs above 256 bytes at whitespace) | CORRECTED (contradictory) | Cache chunks, but splice only where both sides are clear of `UNSAFE_TO_CONCAT`; otherwise re-shape across the seam. 256 bytes is retained as a design decision. | B1 |
| 3 | Bridge Req. 2: shaper normalises to logical order | CONFIRMED, tightened | HarfBuzz mirrors during shaping and returns visual order for RTL buffers. The bridge must reverse to logical order, and the shaper alone owns mirroring. | B1, B3, U3 |
| 4 | Bridge Req. 3: per-atom fallback | CORRECTED | Select the font per atom, require one font per atom, split into font runs, shape each. Carry a `scale` as DirectWrite does. | B4, S4, B5 |
| 5 | Step 1: UAX #29 clusters; UTS #51 "state machine" | CORRECTED | Use extended grapheme clusters including GB9c and `InCB`. Classify clusters afterwards instead of running a second state machine. | U2, U5 |
| 6 | Step 1: emoji get script `Zsye` | AMBIGUOUS | Emoji are `Script=Common` in the UCD. `Zsye` stays as an engine-internal pseudo-script. Keycap and tag sequences were missing; ZWJ qualifies only between pictographics. | U5, U8 |
| 7 | Step 1: merge FE0E/IVS into atoms | CONFIRMED functionally | GB9 already attaches them. FE0E is VS15, not an IVS selector. `HAS_VARIATION_SELECTOR` becomes a post-hoc flag. | U2, U6, U8 |
| 8 | Step 2: first-strong skips isolates | CONFIRMED | Retained as P2/P3 with HL1. | U3 |
| 9 | Step 2: extend the paired-bracket table with quotes | CORRECTED | Two tables: `BidiBrackets` for UAX #9 N0 (BD16, 63-element stack, canonical equivalents) and a superset only for script pairing. | U3, U8, S2, S3, S1 |
| 10 | Step 2: apostrophe filter on U+2019 between Latin/Cyrillic/Greek letters; example `rock 'n' roll` | CORRECTED | The example does not satisfy the rule. Cover U+0027, U+2019, U+02BC; use the UAX #29 WB6/WB7 letter-on-both-sides test; leading and trailing apostrophes are out of scope. | U2, U8 |
| 11 | Step 3: closed-pair anchors, unclosed-quote phases, "jitter-free" | AMBIGUOUS / UNSUPPORTED | Default to the Script_Extensions set-intersection algorithm with a bounded stack. The draft's CJK-aware heuristic survives as an optional policy with defined terms. No stability guarantee is claimed. | U4, S1, S2, S4 |
| 12 | Step 3: U+2014/U+2026/U+00B7 as a unit; "Western" | AMBIGUOUS | Map to the host script; define precedence; treat the run as unbreakable. | U8, W5 |
| 13 | Section 3: "six passes, strict O(N)" | UNSUPPORTED | Linear only with bounded stacks and memoised look-ahead. Segmentation models and line breaking are stated separately. | U4, L1 |
| 14 | Step 4: six "Serbian-exclusive" letters | CORRECTED | Only U+0452 and U+045B are absent from Macedonian's CLDR main exemplars. | U9 |
| 15 | Step 4: kana or Hangul anywhere sets the paragraph locale | AMBIGUOUS | Explicit tags first; detection per run or sentence. Design decision. | none found |
| 16 | Step 6: `icu_segmenter::LineSegmenter` over the atom sequence | CORRECTED | The API takes text and returns offsets. Offset 0 is discarded; mandatory breaks are derived by the caller. | S6 |
| 17 | Step 6: dictionary models for Thai, Lao, Khmer, Burmese | CORRECTED | `new_auto` uses LSTM when available; dictionary is `new_dictionary`. `LineSegmenter` does not segment Chinese or Japanese. | S6 |
| 18 | Step 6: `PunctuationRun`, `mandatory_bind_count = 1`, class list EX/CL/IS/SY/IN | CORRECTED | UAX #14 already forbids breaks before CL, CP, EX, SY, IS (LB15d), IN. The whole run binds to the preceding cluster. CP was missing. | U7 |
| 19 | Stage 1: emergency breaks between P_0 and P_1 | CORRECTED | Not a UAX #14 opportunity; allowed only as an overflow fallback for a unit wider than the line. Dash and ellipsis pairs never split. | U7, W1, W5 |
| 20 | Stage 1: `Demerits = (1 + 100·\|r\|^3 + p)^2` | CORRECTED | Full Knuth–Plass form with negative and forced penalties, flagged-break and fitness-class terms, infeasibility at `r < -1`, thresholds and an emergency pass. | L1, L2 |
| 21 | Stage 1: "A, I, a" orphan penalty | UNSUPPORTED | Replaced by a locale-keyed no-line-end table. The Czech set is `AIiVvOoUuSsZzKk`. Empty for English. | L8 |
| 22 | Stage 1: switch to greedy above 5,000 bytes; Phase 3 target of 500,000 bytes at 120 FPS | CORRECTED (inconsistent) | Incremental-relayout design (greedy resynchronisation, windowed optimisation, viewport-first). Labelled design inference; no cited engine reaches the target. | L1, L4, L6 |
| 23 | Stage 1: restore `uncompressed_width` at a break | PARTIALLY CONFIRMED | Line-end marks are trimmed and line-start opening marks restored. Hanging punctuation is added as a mode. | W2, W4, W5 |
| 24 | Stage 2: L1 resets trailing hanging whitespace | CORRECTED | L1 also resets segment and paragraph separators and whitespace or isolate formatting characters before them, using original classes. | U3 |
| 25 | Stage 2: L2 from the maximum odd level | CORRECTED | From the highest level of any parity down to the lowest odd level. | U3 |
| 26 | Stage 3: `CjkAutospace` 0.125 em natural, 0.125 em stretch, 0.0625 em shrink | CORRECTED / UNSUPPORTED | JLReq: 1/4 em natural, minimum 1/8 em, maximum 1/2 em (1/3 em in some styles). CSS Text 4: 0.125 ic. Two profiles, sizes in ic. | W2, W4, W5 |
| 27 | Stage 3: `slack > capacity * 1.5` aborts justification | UNSUPPORTED | Replaced by the JLReq priority ladder, a punctuation-compression pool, and an adjustment-ratio tolerance. | W2, W4 |
| 28 | 2.2: `IS_CURSIVE_CONNECTED` includes Devanagari | CORRECTED | CSS Text 4 Appendix D lists Arabic, Hanifi Rohingya, Mandaic, Mongolian, N'Ko, Phags Pa, Syriac. Devanagari is not listed. Flag split in two. | W2 |
| 29 | Phase 3: integrate `hypher` with `replacement_prefix` | CORRECTED | `hypher` returns slices of the input and cannot express replacements. Use libhyphen-style data for non-standard hyphenation. | H1, H3 |
| 30 | German `ck -> k-k` | AMBIGUOUS | UAX #14 and libhyphen label it pre-reform. Modern examples: Catalan, Dutch, Hungarian, Swedish. | U7, H3 |
| 31 | Phase 1: GPUI adapter; Phase 3: locale through GPUI for `locl` | AMBIGUOUS / UNSUPPORTED | GPUI exposes no cluster spans, safety flags, bidi or locale. The adapter is a degraded profile, or the engine bypasses `PlatformTextSystem`. | B6 |
| 32 | 6.4: primary caret "aligned with current affinity", secondary at alternate boundary | CORRECTED | Primary is the base-direction insertion point; affinity selects the line or side; the secondary caret exists only if its x differs. | C1 |
| 33 | 6.3(c): click beyond the right edge returns `visible_byte_end` | CORRECTED | Wrong for RTL and mixed lines. Use the visual end edge and the extreme caret stop. Add y clamping and an `inside` flag. | C1 |
| 34 | 6.5: Shift+arrow is logical | CORRECTED | Shift+arrow extends the focus visually; selection is stored as anchor and focus. Backspace and Delete stay logical. | C2 |
| 35 | 6.5: Backspace steps by `GraphemeAtom` | CORRECTED | Code-point granularity by default, whole-unit removal for emoji sequences, flags, keycaps, tags and CRLF. | C3 |
| 36 | 6.2: equal ligature sub-slots for Latin-like bases only | CORRECTED | Use GDEF `LigCaretList` first, equal split second; the script restriction is dropped. | B7, C1 |
| 37 | 6.5: word movement along `WordNode::Word` | UNSUPPORTED | `WordNode` is a line-breaking unit. Use UAX #29 word boundaries. | U2, U7 |
| 38 | 6.6: merge contiguous slots | CORRECTED | Merge visually adjacent slots only; define `SelectionRect`, newline extension and height styles. | C1 |
| 39 | 2.6: `is_synthesized` for "soft hyphens" | CORRECTED | The generated glyph is a hyphen. U+00AD is a source character with its own copy policy. | W1, C4 |
| 40 | Section 1: "deterministic without language tags" | AMBIGUOUS | Explicit tags are honoured first; derived locales are a documented fallback. | none found |
| 41 | Stage 4: diacritic clamp ±1.5 line height; Stage 5: lowest `underline_position` | UNSUPPORTED | Kept as design decisions. No precedent was found. | none found |
| 42 | Backends: rustybuzz implied | CORRECTED | rustybuzz is archived. HarfRust 0.15.0 (2026-10-22) is the maintained successor. | B2 |

### Changes from Phase 0 implementation

| # | Item | Change | Where |
|---|---|---|---|
| 43 | `reshape_window` | Gains `atoms` and `features` parameters. | 4.1 |
| 44 | `BoundaryEdit` | Fully listed; `ReplaceSuffix` added to mirror `suffix_replacement`. | 4.1 |
| 45 | `PositionedLine`, `PositionedGlyph` | `hanging_left`, `hanging_right`, `is_hanging` added; `visual_width` excludes hanging. | 2.6 |
| 46 | `RawShapedGlyph` | Defined; carries `is_synthesized` so a generated hyphen survives into the output. | 2.3 |
| 47 | `FontFeatures` | Also carries variation-axis settings (cache key, 4.2 req. 5). | 4.1 |
| 48 | Safety-flag implication | Stated over the `safe_*` booleans; an early implementation tested the converse. | 2.3 |
| 49 | Option and strategy enums | `HangMode`, `LineHeight`, `QuotePolicy`, `AutospaceProfile`, `HangingWhitespaceMode`, `LineBreakStrategy`, `SoftHyphenPolicy`, `ShaperProfile` defined as plain enums; the spec named them in prose only. | 2.4, 3, 4.6, 5, 6.6 |
| 50 | `HyphenOpportunity` | `cut_bytes` split into `prefix_cut_bytes` and `suffix_cut_bytes`; `byte_offset` is the original break position. | 2.4 |
| 51 | `PositionedLine` | `visual_boxes: Vec<PositionedInlineBox>` places inline objects. | 2.6 |
| 52 | `HangMode` | `AllowStart` added for leading-edge hanging (superseded by row 58). | 5 |
| 53 | Cluster flags | Per-flag aggregation instead of a plain union. | 2.2, 2.3 |
| 54 | Line validation | `LineContext`; `Preserved` trailing whitespace exempt; mixed absolute and relative tolerance; line height checked against the resolved strut. | 2.6, 5 |
| 55 | `TextShaper` | `Send + Sync`; `BoundaryEdit::prefix_for` and `suffix_for` read one side each. | 4.1 |
| 56 | Chunk splicing | Reworded to the conservative two-sided rule. | 4.2 |
| 57 | `PositionedLine` | `tabs: Vec<PositionedTab>` places tabs; `PositionedTab { byte_range, x, width }` added. | 2.6 |
| 58 | `HangMode` | (refined by row 66) Flat enum replaced by `struct HangMode { start: bool, end: HangEndMode }` with `HangEndMode { None, Allow, Force }`; start and end combine. | 5 |
| 59 | Line validation | Three passes (finiteness, envelope, containment); tabs and boxes under the same containment rule; `LineContext` gains `hang_mode` and `base_direction`; hanging gated by `HangMode`; preserved trailing whitespace must lie on the L1 far edge. | 2.6, 5 |
| 60 | `BoundaryEdit` | (superseded by row 65) Edge of each variant stated: `None` symmetric, `AppendHyphen` and `ReplacePrefix` trailing, `ReplaceSuffix` leading. | 4.1 |
| 61 | `ligature_carets` | Gains `font_size: f32`; output is relative to the glyph origin in layout units. | 4.1 |
| 62 | `shape_run` | `text` is the whole paragraph text; cross-font limit of contextual features stated. | 4.1 |
| 63 | Option carriers | `FrontendOptions` and `LayoutOptions` structs. No layout entry point is defined yet (Open questions 29 to 31). | 3, 5 |
| 64 | `ParagraphAst::strut_metrics` | Now the **raw** `FontMetrics` returned by the shaper, plus `strut_font_size`; the frontend never applies `LineHeight`. The middle-end resolves the active strut. | 2.5, 5 |
| 65 | `BoundaryEdit` | Split into orthogonal `LeadingEdit { None, ReplaceSuffix }` and `TrailingEdit { None, AppendHyphen, ReplacePrefix }`; `reshape_window` takes both. | 4.1 |
| 66 | `HangMode` | `start` is `HangStartMode { None, FirstLine, AllLines }`; `LineContext::line_index` supplies the first-line fact. | 5 |
| 67 | `LayoutConstraints`, `TabStops` | Container geometry (`wrap_width`, `tab_stops`) separated from `LayoutOptions` (policy only). | 5 |
| 68 | `layout_paragraph` | Entry point `(ast, text, shaper, constraints, options) -> PositionedParagraph`; Phase 0 skeleton that checks inputs and returns an empty paragraph. `middleend` now depends on `shaping`. | 5 |
| 69 | Out-of-bounds hit | Converges to the physical extreme caret stop of the line (`PositionedLine::extreme_caret_stop`). | 6.3 |
| 70 | `ligature_carets` | Output stated as layout coordinates after size and scale. | 4.1 |

### Changes in Revision 2.1 (algebraic AST consolidation, rich inline primitives, parameterised beam line breaking)

| # | Item | Change | Where |
|---|---|---|---|
| 71 | `PositionedGlyph.x` | Strictly the **pen advance position**; the drawing origin is `x + x_offset` (`x_offset` added). Containment tests `[x, x + advance]`. Reconciles the Phase 0 audit with the earlier text. | 2.6 |
| 72 | `WordNode` | Six ad-hoc variants replaced by `Box`, `Glue`, `Discretionary`, `Tab` after Omega / LuaTeX. `HardBreak`, `PunctuationRun` (and `PunctBind`) and nested `hyphen_breaks` are absorbed into `Discretionary`. Breaks happen only at a `Discretionary`. | 2.4 |
| 73 | `Discretionary` | Knuth–Plass scalar penalty generalised to `(pre_break_width, post_break_width, no_break_width)` plus a lazy `BreakAction`; unifies hyphenation, `chws` seams, cloned span padding, maths operator breaks and mandatory breaks. | 2.4 |
| 74 | `GlueTier` | `Tier0` punctuation pool and word space, `Tier1` CJK autospace, `Tier2` inter-ideographic gaps; the JLReq / CLREQ ladder enters the piecewise badness. | 2.4, 5 Stage 1.6.3 |
| 75 | Rich inline spans | `InlineDecorationSpan` (`BoxDecorationBreak::{Slice, Clone}`) and `AtomicEditSpan`, orthogonal; `AtomFlags::IN_ATOMIC_EDIT_SPAN`; `CjkAutospace` suppressed beside padded span edges. | 2.2, 2.5, 3 |
| 76 | Inline boxes and maths | `BoxOverflowPolicy::{Clip, AllowOverflow, ScaleToFit}`; breakable formulas are flat rigid boxes between `Discretionary` nodes (500 after relations, 700 after binary operators). | 2.4 |
| 77 | `PositionedSpanFragment` | `span_fragments` on `PositionedLine` reports post-bidi span boxes with `is_first_fragment` and `is_last_fragment`; `validate` gains fragment finiteness, containment and span-membership checks. | 2.6, 5 Stage 2 |
| 78 | Line breaker | One parameterised beam-lookahead state machine; `LineBreakStrategy` is `Greedy` (1, 1), `Balanced` (2, 2, default), `Publication` (8, 6); dominator pruning, lazy reshape, incremental resynchronisation. | 5 Stage 1.6 |
| 79 | Atomic spans in interaction | No caret stops strictly inside an `AtomicEditSpan`; hit tests snap to its boundaries; Backspace and Delete remove it whole. | 6.1, 6.3, 6.5 |
| 80 | `icu_segmenter` 2.4.0 | `LineSegmenter` must use a `neo` (or `*_17_*`) constructor; grapheme and word segmenters keep their defaults. The constructor names in the Revision 2.1 text (`try_new_auto_neo`, `new_for_non_complex_scripts_neo`) do not exist in 2.4.0; the real ones were checked against the crate source. | 1.1, 3 Step 6.1 |

### Rulings on Revision 2.1 (answers to Open questions 33 to 38, 41 and 42)

| # | Item | Change | Where |
|---|---|---|---|
| 81 | Order of `Discretionary` and `Glue` | A breakable space is `[Box, Glue.., Discretionary]`: the break comes **after** the glue run, so the spaces stay on the trailing edge of line *k*. `b_vis` is found by walking back over `WordSpace` and `CjkAutospace` glue. | 2.4, 5 Stage 1.6.2 |
| 82 | Hard-break bytes | `DiscretionaryNode.byte_offset` becomes `byte_range`: empty for a soft opportunity, the exact newline sequence for a `HardBreak` (empty only at the paragraph end). The next line starts at `byte_range.end`; no byte is unowned. | 2.4 |
| 83 | Tier 2 and the compression pool | No explicit Tier 2 glue: the middle-end adds `cjk_inter_char_max_stretch` to `Stretch[Tier2]` for every cluster carrying `ALLOW_CJK_JUSTIFY_AFTER` except the last visible one. New `GlueKind::CjkPunctCompression` (`Tier0`, shrink only) for the punctuation pool. | 2.4, 5 Stage 1.6.2 |
| 84 | Badness function | Five exhaustive cases (exact fit, compression, primary expansion, Tier 2 spill, underfull) with explicit `Y_t`, `Z_t`; implemented as `line_fit`. | 5 Stage 1.6.3 |
| 85 | Cloned padding and `CjkPunctAdjust` | `BreakAction::CloneSpanPadding` removed; `DiscretionaryNode.cloned_spans` carries the span ids, orthogonal to the action. `CjkPunctAdjust` carries two geometric advance overrides instead of cluster indices. | 2.4 |
| 86 | Span streams | Proper nesting for decoration spans, strictly disjoint atom-aligned atomic spans, `bidi_level` on `SpanBoundary`, no visual styling in the AST. | 2.2, 2.4, 2.5 |
| 87 | Hit test across a wrapped atomic span | The midpoint is that of the clicked fragment, with the fragment's bidi level deciding which half is the start; `PositionedSpanFragment` gains `bidi_level`. | 2.6, 6.3 |
| 88 | `wrap_width` in validation | `LineContext.wrap_width`; a line wider than it is valid only as a single overflowing entity. `LineContext::from_options` takes the width. | 2.6 |

### Second rulings (answers to Open questions 43 to 48 and the implementation choices)

| # | Item | Change | Where |
|---|---|---|---|
| 89 | Badness case E | One rule for every `Y_total`: `r_excess = (Δ − Y_total) / max(font_size, 1)`, `r = 2 + r_excess`, `b = min(100 000, 400 + 400·r_excess³)`. C0 with case D, monotone in capacity, below the overfull value. | 5 Stage 1.6.3 |
| 90 | `CjkPunctAdjust` | Unit variant. The trim and restore deltas are `pre_break_width` and `post_break_width`, applied to the edge clusters of the neighbouring text boxes. | 2.4 |
| 91 | `PositionedInlineBox.overflow_policy` | Validation knows the policy: `Clip` and `ScaleToFit` boxes must end within `wrap_width`; the two single-entity overflow exemptions are tightened. | 2.6 |
| 92 | Equal-range decoration spans | Allowed with distinct ids; the lower index is the outer span; `SpanBoundary` nodes follow strict LIFO order, enforced. | 2.5 |
| 93 | Paragraph-end sentinel | `HardBreak { is_paragraph_end: true }` is the empty range `text.len()..text.len()`, the last discretionary, at most once; `false` is a real, non-empty terminator. | 2.4 |
| 94 | Atomic span edges | Both edges coincide with the start or end of a text atom or inline object, and the span wraps at least one; never inside glue or a tab. | 2.5 |
| 95 | Confirmations | `MAX_DECORATION_NESTING_DEPTH = 8`; `cloned_spans` requires the break strictly inside the span; `LineContext::default().wrap_width = ∞`; the hit-test tie rule (midpoint = physical right half). | 2.4, 2.5, 2.6, 6.3 |

### Final rulings (Phase 0 closure)

| # | Item | Change | Where |
|---|---|---|---|
| 96 | Atomic spans wrap content | Locked: an `AtomicEditSpan` must contain at least one text atom or inline object, besides having both edges on atom or object edges. A span of whitespace alone is invalid. | 2.5 |
| 97 | Case E at zero stretch | Kept as implemented: with `Y_total == 0` and `Δ > ε` the line is genuinely underfull and starts at base badness 400; the jump from 0 is intended and the order among rigid underfull lines stays strict. | 5 Stage 1.6.3 |
| 98 | Line emission | Every mandatory breakpoint, real or sentinel, terminates exactly one `PositionedLine`; N newlines give N + 1 lines. `items` must end with the sentinel (an empty paragraph included). | 5 Stage 1.6, Stage 4.2 |
| 100 | Monotone items and span enclosure | `items` progress monotonically through the text (`prev.end <= next.start`). Items inside a span lie strictly between its `Leading` and `Trailing` nodes, items outside do not; a break opportunity at `s..s` precedes `Leading`, one at `e..e` follows `Trailing`, so breaking beside a chip never strands a boundary on the adjacent line. | 2.5 |
| 101 | Seam spacing outside the chip | A `Glue(CjkAutospace)` at `s..s` sits before the span's `Leading`, one at `e..e` after its `Trailing`, like the break opportunities. At a seam shared by adjacent spans the canonical order is `[A Trailing, Glue(CjkAutospace), Discretionary, B Leading]`: LIFO places the boundaries, and among the zero-width nodes at one offset a `CjkAutospace` glue precedes every break opportunity (`ItemsOutOfOrder` otherwise), as a breakable space's glue precedes its discretionary. | 2.5 |
| 102 | Review-driven tightening | Hyphenation breaks must be `flagged`; `cloned_spans` must list every `Clone` span the break is strictly inside; `CjkAutospace` and `CjkPunctCompression` glue own no bytes; a `SpanBoundary` width equals its edge's padding and its level is at most `BIDI_MAX_DEPTH + 1`; a lone CR or LF of a CRLF pair is not a terminator; a text box lies inside its run and caches the total of its clusters; atoms are ordered; a span fragment's bytes lie inside its line. | 2.4, 2.5, 2.6 |
| 99 | Mandatory boundary pair | Every decoration span has exactly one `Leading` boundary at `byte_range.start` and one `Trailing` at `byte_range.end` in `items`, even with zero padding, in strict LIFO order. | 2.4, 2.5, 5 Stage 2 |
| 103 | Break widths, hyphen chain, atomic flag | `pre_break_width` / `post_break_width` include the cloned padding (`clone_pre = Σ end_padding`, `clone_post = Σ start_padding` over `cloned_spans`): `Direct`, non-sentinel `HardBreak` and `MathOperator { None }` must equal the sums, the sentinel is `0.0`; `CjkPunctAdjust` works on the net deltas (at least one non-zero, neighbour and non-negativity per non-zero one); `Hyphenate`'s net deltas are the shaping deltas. Hyphen cuts are validated against the contiguous same-run hyphen-connected text chain, not the adjacent box. `IN_ATOMIC_EDIT_SPAN` mirrors `atomic_spans` exactly (`AtomicFlagMismatch`). CodeRabbit's autofix stays off. | 2.4, 2.2, 2.5 |
| 104 | Padded single-entity overflow | Exemptions A and B of Pass 3(h) accept an entity wrapped in padded decoration spans: every fragment covers exactly the entity's bytes, the entity lies inside the body and the fragments reach both of its edges; without fragments the entity starts at 0 and fills `visual_width`. No new field on `PositionedSpanFragment`. Text fixes: Step 6.6 order, stale zero-width sentence removed, rule 3 cut scope, Stage 4.2 blank-line range, 2.6 prose. | 2.6, 3, 5 Stage 4.2 |
| 105 | Second CodeRabbit pass | A `CjkAutospace` glue may not follow a zero-width break opportunity at the same offset (the seam order is now enforced, not implied). A cluster total must be finite; the `CjkPunctAdjust` liveness test uses the net deltas only, whatever the clone padding; the hyphen-chain walk stops at the cut's target, so validation stays linear. `line_fit` accepts `primary_stretch()` / `total_stretch()` at their f32 rounding as seam values. Rule 5 states that mandatory breaks inside a `Clone` span also carry the padding and ids. | 2.4, 2.5, 5 Stage 1.6.3 |
| 106 | Hyphenation run, trailing fragment bytes | A `Hyphenate` discretionary needs the same `run_index` on both neighbouring text boxes (`InvalidHyphenOpportunity` otherwise). In the padded overflow exemptions A and B a fragment may run on over the line's trailing whitespace: it starts at the entity's start, ends at or after its end, and `entity_range.end >= visible_byte_end`. | 2.4, 2.6 |

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

Implementations must use UCD data files of the same Unicode version (for example `BidiBrackets-18.0.0.txt`, dated 2026-02-03 `[U8]`). Libraries that predate Unicode 15.1 data silently split Indic conjuncts `[U2]`. **[Design decision]** Each component's data version is pinned and reported by a `conformance_report()` call, and the test suites in section 7 are run against it. A relevant ecosystem fact: the default `icu_segmenter` 2.4.0 (2026-10-22) line segmenter is compatible with UAX #14 version 15.1.0, the `*_17_*` constructors with 17.0.0 and the `*_neo_*` constructors with 18.0.0 `[S6]`. The default therefore lags rev 57 by three Unicode versions. **Pinning (Revision 2.1, row 80).** Frontend Step 1 (grapheme segmentation) and 6.5 (word segmentation) use the default `GraphemeClusterSegmenter` and `WordSegmenter` constructors, which already carry UAX #29 18.0.0 (GB9c, `InCB`). Frontend Step 6 (line segmentation) must call a `neo` constructor (or the `*_17_*` fallback) explicitly and `conformance_report()` records the active UAX #14 data version; the exact constructor names are in section 3, Step 6.1.

**1.2 Determinism and language tags.** The central objective is to resolve historical Unicode ambiguities deterministically. **[Design decision]** Explicit language tags (BCP 47) supplied by the caller always win. Where none is supplied, the engine derives locales by the conservative rules in section 3, Step 4, which are heuristics and not standards. The engine is deterministic for a given input, option set and data version; it does not claim to recover the author's intent.

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
pub struct TextRange { pub start: ByteIndex, pub end: ByteIndex } // half-open, code point aligned
pub struct ScriptTag([u8; 4]);          // ISO 15924; Zsye is an engine pseudo-script (see 3, Step 1)
pub struct LocaleTag(/* BCP 47 */);
pub type BidiLevel = u8;                // even = LTR, odd = RTL (UAX #9)
```

**[Design decision]** All internal offsets are UTF-8 byte offsets and synthetic elements never displace them. Platform text-input and accessibility layers commonly use other units (Chromium's editing state machines work in UTF-16 code units `[C3]`), so the engine provides `utf8_to_utf16(offset)` and `utf16_to_utf8(offset)` and rejects or rounds offsets that fall inside a code point (section 6.7).

`ScriptTag`: after frontend resolution no atom may remain `Zyyy` (Common) or `Zinh` (Inherited) `[U4]`. `Zsye` is not a Unicode `Script` property value: emoji code points are `Script=Common` in Scripts.txt (18.0). `Zsye` is an ISO 15924 code that UTS #51 §4.3 describes as "prefer emoji style", and UTS #51 warns that it is unsuitable for general language tags `[U5][U8]`. Its use here is an **[Design decision]**: an engine-internal font-fallback key.

### 2.2 Logical atom: `GraphemeAtom`

An atom is one **extended** grapheme cluster (EGC) per UAX #29, including GB9c `[U2]`.

```rust
pub struct GraphemeAtom {
    pub byte_range: TextRange,
    pub script: ScriptTag,
    pub bidi_level: BidiLevel,        // derived from code point levels, see 3 Step 2
    pub linebreak_class: LineBreakClass, // class of the base code point [Design decision]
    pub flags: AtomFlags,
}
```

`linebreak_class` is a per-code-point property in UAX #14. The draft did not define the atom-level rule; **[Design decision]**: the atom takes the class of its base (first) code point, while the segmenter itself always runs over the original text so that LB9 and LB10 see the real sequence `[U7]`.

Flags (changes from the draft in bold):

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
| **`IN_ATOMIC_EDIT_SPAN`** | The atom lies inside an `AtomicEditSpan` (mention, chip, code span; Revision 2.1, row 75). Caret stops strictly inside the span are suppressed and Backspace / Delete remove the whole span (section 6). `check_layout_inputs` keeps the flag in step with `atomic_spans`: every atom inside one carries it, no atom outside carries it (`AtomicFlagMismatch`, row 103). |

**Removed:** `IS_LATIN_LIKE_LIGATURE_BASE`. The restriction of caret subdivision to Latin, Cyrillic and Greek had no source (section 6.2). **Split:** `IS_CURSIVE_CONNECTED` becomes `NO_LETTER_SPACING` and `KASHIDA_CAPABLE`. The draft's inclusion of Devanagari in the cursive set is not supported by the CSS list. Whether Indic scripts should also forbid letter-spacing because of the headline (shirorekha) is a **[Design decision]** pending the Indic layout requirements (Open question 13).

**Cluster flag aggregation [Design decision].** A `ShapedCluster` (2.3) takes its `flags` from the atoms in its `atom_range` per flag, not as a plain union: `IS_OPEN_PUNCT` from the **first** atom only; `ALLOW_CJK_JUSTIFY_AFTER` and `IS_CLOSE_PUNCT` from the **last** atom only (the boundary after a ligature is the one after its last component; an opportunity inside the ligature does not exist); every other flag, `IN_ATOMIC_EDIT_SPAN` included, is the bitwise OR of all atoms. An empty atom list gives no flags. The reference implementation is `AtomFlags::for_cluster`.

**Rich inline span inputs (Revision 2.1, row 75) [Design decision].** Notion-grade inline elements (long breakable user mentions, page-reference chips, inline `code`, inline maths) must not be locked into rigid unbreakable boxes. The frontend input contract therefore carries two span streams beside the style spans. They are **orthogonal**: the first governs geometry, the second editing. A mention is usually both, an inline code span often only the first.

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
```

Both streams are carried by `ParagraphAst` (2.5). **Frontend rule for CJK autospace near spans (3, Step 6.6).** When an `InlineDecorationSpan` has `start_padding > 0.0`, no `CjkAutospace` glue may be inserted at `byte_range.start`, neither immediately outside nor immediately inside the boundary; the symmetric rule holds at `byte_range.end` when `end_padding > 0.0`. This prevents double spacing between a padded chip border and an adjacent Han ideograph. (Reference: `InlineDecorationSpan::forbids_cjk_autospace_at`.)

### 2.3 Shaped unit: `ShapedCluster`

```rust
pub struct FontRef { pub font_id: FontId, pub scale: f32 } // scale: DirectWrite MapCharacters returns one

pub struct ShapedCluster {
    pub byte_range: TextRange,
    pub atom_range: Range<usize>,       // K >= 1 atoms; a cluster never splits an atom
    pub font: FontRef,                  // was font_id: FontId
    pub bidi_level: BidiLevel,
    pub flags: AtomFlags,
    pub glyphs: SmallVec<[RawShapedGlyph; 2]>, // logical order, see 4.2
    pub natural_width: f32,
    pub uncompressed_width: f32,        // width with chws off; costs a second shaping pass (4.3)
    pub safe_to_break_before: bool,     // was is_safe_to_break_after; true = no UNSAFE_TO_BREAK
    pub safe_to_concat_before: bool,    // new; true = no UNSAFE_TO_CONCAT
}
```

**Polarity.** HarfBuzz defines `HB_GLYPH_FLAG_UNSAFE_TO_BREAK` as: if text is broken at the beginning of the cluster the glyph belongs to, both sides need re-shaping. The flag is set on every glyph of the cluster. When it is absent it is safe to break at the beginning of that cluster `[B1]`. Hence "break after cluster *i*" is "before cluster *i+1*", and the end of text is always safe because HarfBuzz has no end-of-text flag position `[B1]`. Blink names its accessor `IsSafeToBreakBefore` for the same reason `[B4]`.

**`UNSAFE_TO_CONCAT`.** Defined as: if text on one side of the start of the cluster changes, the other side's shaping might change. Its absence does not by itself prove that concatenation is safe: only two pieces both clear of the flag can be joined. `UNSAFE_TO_BREAK` always implies `UNSAFE_TO_CONCAT`. The flag is produced only if the buffer flag `HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT` is set, which was added in HarfBuzz 4.0.0 (2022-03-01) `[B1]`. `UNSAFE_TO_BREAK` itself dates from HarfBuzz 1.5.0 (2017-08-23) `[B1]`.

**Phase 0 contract notes [Design decision].** (1) In terms of the two `safe_*` booleans, "`UNSAFE_TO_BREAK` implies `UNSAFE_TO_CONCAT`" reads `safe_to_concat_before ⇒ safe_to_break_before`. The combination `safe_to_break_before = true, safe_to_concat_before = false` is legal (only concatenation is unsafe); the converse is a contradiction. (2) `atom_range` is relative to the atom slice stored beside the cluster: the `atoms` argument of `shape_run`, or `atoms` of the owning `WordNode` after the frontend has re-based it. (3) A cluster belongs to exactly one `WordNode`: split points are chosen before clusters are assigned, and a split inside a multi-atom cluster is resolved by re-shaping that window. (4) Within a node the clusters partition its atoms in logical order, and each `byte_range` runs from its first atom's start to its last atom's end. (5) `flags` is the per-flag aggregate of 2.2, which `WordNode::is_self_contained` checks together with (2) to (4).

### 2.4 Box-and-glue sequence: `WordNode`

```rust
pub enum WordNode {                       // four variants (Revision 2.1, row 72)
    Box(BoxNode),
    Glue(GlueNode),
    Discretionary(DiscretionaryNode),
    Tab { byte_range: TextRange },
}

pub enum BoxNode {
    Text { run_index: usize, byte_range: TextRange, atoms: Vec<GraphemeAtom>,
           clusters: Vec<ShapedCluster>, natural_width: f32 },
    InlineObject { byte_range: TextRange, box_id: u64, width: f32, ascent: f32, descent: f32,
                   overflow_policy: BoxOverflowPolicy },
    SpanBoundary { byte_range: TextRange,   // empty range anchored at the span's start or end
                   span_id: u64, edge: SpanEdge /* Leading | Trailing */,
                   width: f32,              // start_padding or end_padding
                   bidi_level: BidiLevel }, // Leading: level of the first atom in the span; Trailing: of the last
}

pub enum BoxOverflowPolicy {
    Clip,          // clamp visual rendering to wrap_width
    AllowOverflow, // permit visual_width > wrap_width when isolated on an empty line
    ScaleToFit,    // uniformly scale width, ascent and descent by wrap_width / width
}

pub struct GlueNode {
    pub byte_range: TextRange, pub kind: GlueKind, pub tier: GlueTier,
    pub natural_width: f32, pub max_stretch: f32, pub max_shrink: f32,
}
pub enum GlueTier {
    Tier0 = 0, // punctuation compression pool and Western WordSpace
    Tier1 = 1, // CJK-Latin autospace (CjkAutospace)
    Tier2 = 2, // inter-ideographic expansion (ALLOW_CJK_JUSTIFY_AFTER); a tier of the prefix tables only,
               // never carried by a GlueNode (ruling 3)
}
pub enum GlueKind { WordSpace, CjkAutospace, CjkPunctCompression } // the last: Tier0, shrink only

pub struct DiscretionaryNode {
    pub byte_range: TextRange, // empty at the break for a soft opportunity; the newline bytes for a HardBreak
    pub penalty: i32,          // -10000 = mandatory HardBreak; 0 = natural; > 0 = penalised
    pub flagged: bool,         // true for hyphenation breaks; triggers the Knuth-Plass alpha demerits
    pub pre_break_width: f32,  // width added to the end of line k if broken here
    pub post_break_width: f32, // width added to the start of line k+1 if broken here
    pub no_break_width: f32,   // width contributed to the current line if NOT broken here
    pub action: BreakAction,
    pub cloned_spans: SmallVec<[u64; 2]>, // Clone-mode spans this break falls strictly inside (ruling 5)
}
pub enum BreakAction {
    Direct,                                  // zero-cost break between words, Han ideographs, Thai words
    HardBreak { is_paragraph_end: bool },    // mandatory line termination (penalty = -10000)
    Hyphenate(HyphenOpportunity),            // TextShaper::reshape_window at materialisation
    CjkPunctAdjust,                          // unit: the deltas are pre_break_width / post_break_width (ruling, row 90)
    MathOperator { repeat_glyph_id: Option<u32> }, // break after a top-level relation or binary operator
}

pub struct HyphenOpportunity {            // mirrors libhyphen's (rep, pos, cut) [H3]
    pub byte_offset: ByteIndex,
    pub prefix_replacement: Option<CompactString>, // e.g. Dutch "omaatje" -> "oma-tje"
    pub suffix_replacement: Option<CompactString>,
    pub prefix_cut_bytes: u32,             // original bytes removed immediately before byte_offset
    pub suffix_cut_bytes: u32,             // original bytes removed immediately after byte_offset
    pub penalty: i32,
}
```

**Cut semantics [Design decision].** `byte_offset` is the break position in the **original** text. A replacement that straddles the break (libhyphen's `cut` can, for example `ck` to `k-k` `[H3]`) is split between `prefix_cut_bytes` (bytes removed from the end of the prefix) and `suffix_cut_bytes` (bytes removed from the start of the suffix); each count must end on an atom boundary, so a cut never splits an atom, and each stays inside the **contiguous same-run hyphen chain** of the word: the maximal run of `BoxNode::Text` items sharing one `run_index`, joined only by `Hyphenate` discretionaries that meet the boxes end to end, and stopped by glue, a tab, an inline object, a `SpanBoundary`, a run change or any other discretionary (`reshape_window` takes one `ShapingRun`, and a box cannot straddle a span boundary, row 100). A two-byte spelling-change cut may therefore cross an unchosen hyphenation point one byte away within the same word (row 104). `prefix_replacement` replaces the prefix-side cut and `suffix_replacement` the suffix-side cut. A cut with no replacement text is a pure deletion. Section 4.1 maps each side to its own edit: `TrailingEdit` for the prefix side and `LeadingEdit` for the suffix side. `PositionedLine` validation does not apply to these; `check_layout_inputs` checks that `byte_offset` lies strictly inside the word and that it and both cuts end on atom boundaries of that chain. For a hyphenation, `pre_break_width - clone_pre` and `post_break_width - clone_post` (rule 5 below) are the net prefix and suffix shaping deltas.

**Algebraic model [Design decision, Sourced: L1, L2, W4, W5].** The six ad-hoc variants of the Phase 0 draft (`Word`, `PunctuationRun`, `Glue`, `Tab`, `HardBreak`, `InlineBox`) are replaced by the four-variant algebra of Omega / LuaTeX. `HardBreak`, `PunctuationRun` (and with it `PunctBind`) and the nested `hyphen_breaks` vector are absorbed into `Discretionary`. The frontend is a **compiler from linguistic rules to algebraic primitives**; the middle-end is a language-agnostic solver.

**Fundamental invariant.** The line breaker may end a line **if and only if** it is at a `WordNode::Discretionary` (the single exception is the emergency cluster break on an overflowing empty line, 5 Stage 1.5). `Glue` is never an implicit break point. If a space is breakable the frontend emits the `Discretionary` **after** the `Glue` run (ruling 1, row 81): `[Box, Glue(WordSpace), Discretionary]`, and for several consecutive spaces one `Discretionary` at the end, `[Box, Glue, Glue, Discretionary, Box]`. When the line breaks there, the spaces belong to the trailing edge of line *k* (`PositionedLine::byte_range` includes them, `visible_byte_end` stops at the last visible box, 2.6) and line *k+1* starts at the discretionary's `byte_range.end`. A non-breaking space is `[Box, Glue]`.

**Three-state discretionary.** A `DiscretionaryNode` generalises the Knuth–Plass scalar penalty to `(pre_break_width, post_break_width, no_break_width)` plus a lazy `BreakAction` payload. If the line breaks here, line *k* gains `pre_break_width` and line *k+1* gains `post_break_width`; otherwise the current line gains `no_break_width`. The three widths are signed deltas (a `chws` trim is negative) but must be finite. Hyphenation, `chws` punctuation restoration and trimming, cloned span padding, mathematical operator breaks and mandatory breaks all reduce to this one formula. The payload is applied only when a break is committed (5 Stage 1.6.5); candidate evaluation reads the three widths only. **Byte ownership (ruling 2, row 82).** `byte_range` is an empty range `offset..offset` for every soft opportunity (after a space, between ideographs, a hyphenation point, a maths operator). For a real `HardBreak { is_paragraph_end: false }` it is the exact, non-empty terminator (`\n` 1 byte, `\r\n` 2, U+2028 3). The synthetic **end-of-paragraph sentinel** `HardBreak { is_paragraph_end: true }` has the empty range `text.len()..text.len()`, is the final `Discretionary` of `items` and appears at most once (row 93). Text ending in `\n` therefore ends with the real break followed by the sentinel, which is the trailing blank line; `check_layout_inputs` rejects a non-empty sentinel and an empty real break. **`items` always ends with the sentinel** (row 98): the last item of every `ParagraphAst`, an empty paragraph included, is `Discretionary(HardBreak { is_paragraph_end: true })`; otherwise `check_layout_inputs` returns `MissingParagraphEnd`. When a line ends at *b*, `line.byte_range.end == b.byte_range.end`, `line.visible_byte_end` is the end of the last visible box, and line *k+1* starts at `b.byte_range.end`. No zero-width `Box` is needed for a newline and no byte is left unowned. `check_layout_inputs` requires the terminator text of a non-empty hard-break range to be one of `\n`, `\r`, `\r\n`, VT, FF, U+0085, U+2028, U+2029 (UAX #14 BK, CR, LF, NL).

**Linguistic-to-algebraic compilation rules (Frontend Step 6).**

1. *Elimination of `PunctuationRun` and kinsoku (UAX #14 LB13, LB15d, LB22).* Closing punctuation (CL, CP, EX, IS, SY) and inseparable characters (IN) are emitted as ordinary `BoxNode::Text`. Because the frontend emits **no** `Discretionary` between the preceding box and the punctuation box, the two are algebraically fused; the same holds after opening punctuation (OP), which binds to the following box. The segmenter's output is never contradicted `[U7]`.
2. *Elimination of `no_line_end_words` special cases.* For Czech one-letter prepositions (`AIiVvOoUuSsZzKk`) and U+00A0 the frontend emits `[Box("v"), Glue(WordSpace)]` with no intervening `Discretionary`. A normal breakable space is `[Box("word"), Glue(WordSpace), Discretionary(penalty = 0, widths = 0, Direct)]`.
3. *Elimination of nested `hyphen_breaks`.* Where hyphenation opportunities exist inside a word, the frontend **splits** the word's clusters at each opportunity boundary into adjacent `BoxNode::Text` items and interleaves `DiscretionaryNode`s with `flagged = true`, `penalty = 135` (or the profile value), `pre_break_width = estimated hyphen width`, `post_break_width = 0.0`, `no_break_width = 0.0` and `action = Hyphenate(..)`. The `HyphenOpportunity` repeats the node's break offset (`byte_range` is the empty range there); `check_layout_inputs` requires a non-empty text box on each side meeting at that offset, the two boxes to share one `run_index` (a word is split within a single `ShapingRun`, and the hyphen glyph and `TextShaper::reshape_window` are scoped to one), and each cut to end on an atom boundary within the contiguous same-run hyphen-connected `BoxNode::Text` sequence (rows 104 and 106). (The alternative of keeping the word whole and anchoring the offset is not representable in a linear sequence; Open question 39.)
4. *CJK `chws` punctuation seams.* Where two adjacent CJK punctuation clusters are compressed by `chws` (combined advance `1.5 em` instead of `2.0 em`) and a break between them is permitted (for example a closing quote followed by an opening quote), the frontend emits each cluster at its compressed width and places a `Discretionary` between them with `no_break_width = 0.0`, `pre_break_width` the delta that trims or restores the line-end mark, and `post_break_width` the delta that restores the line-start opening mark to `uncompressed_width`; `action = CjkPunctAdjust`, a **unit variant** (row 90; the earlier cluster indices and the advance overrides both duplicated what the node's widths already say). `pre_break_width` is the signed delta added to the `natural_width` of the last cluster of the immediately preceding `BoxNode::Text` when the break is taken (the end of line *k*); `post_break_width` the delta added to the first cluster of the immediately following one (the start of line *k+1*). Cloned padding (rule 5) rides in the same widths, so `check_layout_inputs` works on the net deltas `net_pre = pre_break_width - clone_pre` and `net_post = post_break_width - clone_post`: at least one must be non-zero (within eps), and each non-zero one requires the neighbouring text box to exist and the cluster's `natural_width` plus the net delta to be non-negative (row 103).
5. *Breakable rich spans.* Inside a span with `BoxDecorationBreak::Clone`, **every** internal `Discretionary`, a mandatory one included, has `pre_break_width` incremented by `end_padding`, `post_break_width` incremented by `start_padding`, and the span's id pushed onto `cloned_spans` (ruling 5, row 85); only a **non-mandatory** one also has its penalty increased by `+300` (so the breaker prefers keeping the chip intact unless that makes the line too bad; a mandatory break keeps `-10000`, it is never penalised); the list is **complete**, every `Clone` span the break is strictly inside appears in it (row 102). Cloning is **orthogonal to the action**: a hyphenation break inside a chip is `Hyphenate(..)` *and* lists the chip, and keeps its own action. When the line breaks there, each listed span receives its cloned `end_padding` on line *k* (already in `pre_break_width`) and its cloned `start_padding` on line *k+1* (in `post_break_width`). `check_layout_inputs` requires every listed id to name a `Clone`-mode decoration span that contains the break **strictly inside** (`start < offset < end`: a break on the edge has nothing to clone) and ids not to repeat. **Widths (row 103).** With `clone_pre = Σ end_padding` and `clone_post = Σ start_padding` over `cloned_spans`, a `Direct`, `HardBreak { is_paragraph_end: false }` or `MathOperator { repeat_glyph_id: None }` break must have `pre_break_width == clone_pre` and `post_break_width == clone_post` (within eps); the sentinel lists no span, so both widths are `0.0`. `CjkPunctAdjust` and `Hyphenate` carry their own deltas on top of the clone sums (rule 4 above; 2.4 cut semantics).
6. *Flattened inline maths.* An external maths engine decomposes a top-level list (`$a + b = c$`) into `BoxNode::InlineObject` items separated by `Discretionary` items with `penalty = 500` after relations and `penalty = 700` after binary operators (TeX `\relpenalty`, `\binoppenalty`). Sub-formulas in braces, fractions or radicals are one `InlineObject` with no internal `Discretionary`.

**Canonical penalty scale.**

| Penalty | Meaning |
|---|---|
| `-10000` (`-INF_PENALTY`) | Mandatory `HardBreak` (`\n`, `\r\n`, U+2028, end of paragraph). |
| `0` | Natural break: before a breakable `WordSpace`, between CJK ideographs, between Thai, Lao, Khmer and Myanmar segmented words, after an explicit hyphen U+002D. |
| `25..50` | URL interior slash and punctuation breaks `[L3]`. |
| `135`, `flagged` | Standard Liang-pattern hyphenation (`hypher`). |
| `250`, `flagged` | Spelling-change hyphenation (libhyphen: Dutch, Catalan, Hungarian). |
| `300` | Added to every break inside a cloned `InlineDecorationSpan` (long mention, inline `code`). |
| `500` | Break after a top-level inline-maths relation. |
| `700` | Break after a top-level inline-maths binary operator. |
| `>= 10000` | Forbidden break: represented by **omitting** the node; a node carrying it is rejected by `check_layout_inputs`. |

`check_layout_inputs` also requires `action == HardBreak` exactly when `penalty <= -10000`, and finite widths. The 25..50 URL band and the 135 / 250 hyphen values restate Typst's and libhyphen's figures `[L3][H3]` and remain starting points to tune.

`GlueKind` is `WordSpace`, `CjkAutospace` or `CjkPunctCompression`. **`GlueTier` (Revision 2.1, row 74)** puts the JLReq / CLREQ priority ladder into the node: `Tier0` for the punctuation compression pool and Western word spaces, `Tier1` for CJK–Latin autospace, `Tier2` for the zero-width inter-ideographic expansion (`ALLOW_CJK_JUSTIFY_AFTER`). `GlueKind::default_tier` maps `WordSpace` and `CjkPunctCompression` to `Tier0` and `CjkAutospace` to `Tier1`.

**Tier 2 is derived, not emitted (ruling 3, row 83).** The frontend emits **no** zero-width `Tier2` glue between ideographs, which would inflate the node count of a CJK paragraph by a third. `ALLOW_CJK_JUSTIFY_AFTER` stays on `ShapedCluster.flags`; when the middle-end builds the prefix tables over `BoxNode::Text` clusters, each cluster carrying the flag adds `cjk_inter_char_max_stretch` (for example `0.25 · font_size`) to `Stretch[Tier2]` at its position, except the last visible cluster at `b_vis`. `check_layout_inputs` rejects a `GlueNode` with `tier == Tier2`. **`CjkPunctCompression`** carries the compressible margin of CJK full-width punctuation that `chws` has not already squeezed, or that JLReq / CLREQ mid-line compression frees: `Tier0`, `natural_width = 0`, `max_stretch = 0`, `max_shrink` up to `0.5 · ic`; `check_layout_inputs` enforces that shape. `CjkAutospace` values depend on the active profile:

| Profile | `natural_width` | `max_shrink` to | `max_stretch` to | Source |
|---|---|---|---|---|
| `Jlreq` (default for ja) | 1/4 em | 1/8 em | 1/2 em (1/3 em in some styles) | JLReq 3.8.3 and 3.8.4 `[W4]` |
| `CssCompat` | 0.125 ic (1/8 of the CJK advance) | not specified | not specified | CSS Text 4 `text-autospace` `[W2]` |

CLREQ 6.3.3 gives "up to 1/4 em" between Han and Western characters, with no space at the line start or end and none beside Chinese pause marks or CJK brackets `[W5]`. **[Design decision]** Sizes are expressed in `ic` (the CJK advance, usually 1 em). Whether the glue also applies to Kana, Cyrillic and Greek neighbours is a design choice; CSS applies `ideograph-alpha` and `ideograph-numeric` to ideographs `[W2]`. The draft's three numbers (0.125, 0.125, 0.0625 em) have no source and are withdrawn.

`Tab`: width is `next_tab_stop(x) - x`, evaluated during layout. A `Discretionary` with `action = HardBreak` (`\n`, `\r\n`, U+2028) terminates the line and disables justification. `BoxNode::InlineObject` is the former `InlineBox` plus `overflow_policy`: **`BoxOverflowPolicy`** (row 76) says what happens to a rigid object wider than the line. `Clip` clamps the rendering to `wrap_width`; `AllowOverflow` lets an object that sits alone on a line exceed `wrap_width`, in which case the line's `visual_width` equals the object's width, so containment `[0, visual_width]` still holds; `ScaleToFit` scales width, ascent and descent uniformly by `wrap_width / width`. `BoxNode::SpanBoundary` carries the padding advance of one end of an `InlineDecorationSpan`; with `Clone` the middle-end re-materialises it at each break listed in `cloned_spans`. Its `bidi_level` is that of the first atom inside the span for a `Leading` edge and of the last for a `Trailing` edge (ruling 6); `check_layout_inputs` compares it with the atoms.

### 2.5 Paragraph AST

```rust
pub struct ParagraphAst {
    pub base_direction: BidiDirection,
    pub strut_metrics: FontMetrics,   // raw, as returned by the shaper; never policy-adjusted
    pub strut_font_size: f32,         // size the metrics were requested at
    pub runs: Vec<ShapingRun>,   // same family stack, style, script, locale, bidi level, decoration
    pub items: Vec<WordNode>,
    pub decoration_spans: Vec<InlineDecorationSpan>, // Revision 2.1: padding and fragmentation (2.2)
    pub atomic_spans: Vec<AtomicEditSpan>,           // Revision 2.1: caret and deletion units (2.2)
}
// Span invariants (ruling 6, row 86; enforced by check_layout_inputs):
//  atomic_spans   : non-empty, strictly disjoint (a.end <= b.start || b.end <= a.start), span_id unique.
//                   Both edges coincide with the start or end of a GraphemeAtom of a BoxNode::Text or of a
//                   BoxNode::InlineObject, and the span wraps at least one of them (a span of whitespace
//                   alone is invalid, row 96): an edge inside a GlueNode, a Tab or a line terminator, or on
//                   an unowned offset, is rejected (row 94). IN_ATOMIC_EDIT_SPAN is set on exactly the atoms
//                   inside an atomic span (AtomicFlagMismatch, row 103).
//  decoration_spans: span_id unique; proper nesting only (disjoint, or one contains the other); partial
//                   overlap is rejected; depth <= MAX_DECORATION_NESTING_DEPTH = 8 (locked). Equal ranges are
//                   allowed: the lower index is the OUTER span (row 92).
//  SpanBoundary pair (row 99): every decoration span has exactly one SpanBoundary(Leading) at
//                   byte_range.start and one SpanBoundary(Trailing) at byte_range.end in items, even when the
//                   padding is zero; they mark is_first_fragment / is_last_fragment after bidi reordering
//                   (5 Stage 2) without cross-referencing atom offsets. Missing nodes: MissingSpanBoundary.
//  Seam spacing (row 101): a Glue(CjkAutospace) at s..s has index < idx_lead, one at e..e index > idx_trail.
//                   Adjacent spans a.end == b.start: [A Trailing, Glue(CjkAutospace), Discretionary, B Leading]; at any
//                   offset a CjkAutospace glue precedes every zero-width break opportunity (ItemsOutOfOrder).
//  Item order and enclosure (row 100; ItemsOutOfOrder, SpanEnclosure):
//                   items are monotone by byte range (prev.end <= next.start; zero-width nodes may share an
//                   offset). For a span [s, e) with boundary indices idx_lead < idx_trail: every item inside
//                   [s, e] sits strictly between them, every item outside does not; a Discretionary at s..s has
//                   index < idx_lead, one at e..e index > idx_trail. Frontends therefore split boxes at
//                   span edges; a box that straddles an edge fails the monotone check.
//  SpanBoundary order: strict LIFO. At byte_range.start the outer span's Leading precedes the inner's; at
//                   byte_range.end the inner's Trailing precedes the outer's. check_layout_inputs rejects a
//                   Trailing that closes a span other than the innermost open one, a repeated Leading, and
//                   an equal-range pair in the wrong order.
//  the two streams are independent id spaces; neither stores colours or radii, only padding and break mode.

pub struct FontMetrics {         // not defined in the draft; field set is a design decision
    pub ascent: f32, pub descent: f32, pub line_gap: f32,
    pub underline_position: f32, pub underline_thickness: f32,
    pub strikeout_position: f32, pub x_height: f32, pub cap_height: f32,
    pub use_typo_metrics: bool,  // OS/2 fsSelection bit 7 [B7]
}
```

OpenType OS/2 `fsSelection` bit 7 (`USE_TYPO_METRICS`) strongly recommends that applications use `sTypoAscender - sTypoDescender + sTypoLineGap` as default line spacing when it is set; vendors are encouraged to set it in version 4 or later tables `[B7]`. GPUI's metrics expose ascent, descent, cap height, x-height and bounding box, but no underline, strikeout or line gap, so an adapter needs another source for them `[B6]`.

### 2.6 Middle-end output

```rust
pub struct PositionedLine {
    pub byte_range: TextRange,        // includes trailing whitespace
    pub visible_byte_end: ByteIndex,
    pub baseline_y: f32, pub ascent: f32, pub descent: f32, pub line_height: f32,
    pub visual_width: f32,            // body only; hanging glyphs excluded
    pub hanging_left: f32,            // new: width hanging outside the left edge (glyph x < 0)
    pub hanging_right: f32,           // new: width hanging outside the right edge
    pub is_justified: bool,
    pub visual_runs: Vec<VisualGlyphRun>,   // screen left to right
    pub visual_boxes: Vec<PositionedInlineBox>, // new: inline objects, see below
    pub tabs: Vec<PositionedTab>,           // new: tabs, see below
    pub span_fragments: Vec<PositionedSpanFragment>, // Revision 2.1: visual boxes of InlineDecorationSpan
    pub caret_stops: Vec<CaretStop>,        // new: visual-order stops, see 6.1
    pub decorations: Vec<DecorationLine>,
}
pub struct PositionedGlyph {
    pub glyph_id: u32,
    pub x: f32,          // STRICT PEN ADVANCE POSITION, not the drawing origin (Revision 2.1, row 71)
    pub x_offset: f32,   // shaper horizontal ink offset; the drawing origin is x + x_offset
    pub y_offset: f32, pub advance: f32,
    pub byte_range: Option<TextRange>,  // None, or an empty range, for synthesized glyphs
    pub is_synthesized: bool,           // generated hyphen; never copied
    pub is_hanging: bool,               // new: hanging punctuation, wholly outside [0, visual_width]
}
pub struct PositionedInlineBox {        // new
    pub box_id: u64, pub byte_range: TextRange,
    pub x: f32,                         // left edge, line-local, within [0, visual_width]
    pub y: f32,                         // box baseline relative to the line baseline, y down
    pub width: f32, pub ascent: f32, pub descent: f32,   // non-negative
    pub overflow_policy: BoxOverflowPolicy,              // from BoxNode::InlineObject (row 91)
}
pub struct PositionedTab {              // new
    pub byte_range: TextRange,
    pub x: f32,                         // left edge, line-local
    pub width: f32,                     // non-negative; next_tab_stop(x) - x
}
pub struct PositionedSpanFragment {     // Revision 2.1, row 77: post-bidi visual background box
    pub span_id: u64,
    pub byte_range: TextRange,          // the part of the span on this fragment
    pub x: f32,                         // left visual edge, line-local
    pub width: f32,                     // non-negative; includes the padding that applies
    pub ascent: f32, pub descent: f32,  // non-negative
    pub is_first_fragment: bool,        // contains the logical start of span_id
    pub is_last_fragment: bool,         // contains the logical end of span_id
    pub bidi_level: BidiLevel,          // level of the text in the fragment; its parity orients hit tests
}
```

**Geometry conventions [Design decision].** `x` is line-local and is the strict **pen advance position** along the line, not the drawing origin; the rasteriser draws the glyph at `(line_origin_x + glyph.x + glyph.x_offset, baseline_y - glyph.y_offset)` (Revision 2.1, row 71, which corrects the Phase 0 reading of `x` as pen position plus `x_offset`). A zero-advance combining mark with a negative `x_offset` at the start of a line therefore has `glyph.x == 0.0` and never trips the containment check; `x` may still be negative for left-hanging punctuation. Containment is judged on the pen interval `[x, x + advance]`. `visual_width` is the body width: the advance envelope of the main content, excluding hanging punctuation and `Preserved` trailing whitespace; the body is `[0, visual_width]`. `ParagraphAst::strut_metrics` is the **raw** primary-font metrics. The middle-end builds the validation `LineContext { strut, line_height, whitespace, hang_mode, base_direction, line_index, wrap_width }` with `LineContext::from_options(raw, strut_font_size, base_direction, options, wrap_width)`, which calls `LineHeight::resolve_strut(raw, strut_font_size)` to obtain the active strut with half-leading folded into ascent and descent (5, Stage 4.1); `strut_font_size` is the size recorded in the AST, since `LayoutOptions` carries no font size. `for_line(i)` gives each line its `line_index`. `PositionedLine::validate` checks a line against that context in three passes.

*Pass 1, finiteness.* Every number of every entity is finite: line metrics and extents, each glyph's `x`, `x_offset`, `advance`, `y_offset`, each inline box's coordinates and sizes, each tab's `x` and `width`, each span fragment's `x`, `width`, `ascent` and `descent`; extents, box sizes, tab widths and fragment sizes are also non-negative. (A glyph `advance` may be negative, since kerning can produce one.) *Pass 2, envelope.* (a) `visible_byte_end` lies within `byte_range`; (b) an empty line (`byte_range` empty) has no visual runs, boxes, tabs or span fragments, zero `visual_width` and hanging extents, and a `line_height` equal to the strut's; (c) under `LineHeight::Fixed` every line's `line_height` equals the strut's, under `LineHeight::Normal` it is at least the strut's (fallback glyphs may enlarge it). *Pass 3, containment and hanging.* (d) every non-hanging, non-trailing glyph, inline box and tab starts at `>= 0` and ends at `<= visual_width` (a glyph by its pen interval; a zero-advance glyph by its pen position, unless it is flagged `is_hanging` and so rides on a hanging base); every `is_hanging` glyph lies wholly outside `[0, visual_width]`; (e) `hanging_left` and `hanging_right` equal the extent of the hanging glyphs on that edge, so are positive exactly when such glyphs exist, and an edge that `HangMode::allows(side, base_direction, line_index == 0)` forbids has none; (f) trailing whitespace (glyphs and tabs whose `byte_range` starts at or after `visible_byte_end`) has zero width under `Collapsible`; under `Preserved` it is exempt from (d) but, by UAX #9 L1 (6.3, Stage 3), lies on the far edge of the base direction, right of the body in an LTR paragraph and left of it in an RTL one, and never counts towards the hanging extents. Comparisons use `max(1e-3, 1e-5 · max(|a|, |b|))`, because `f32` sums at large coordinates drift by more than a fixed epsilon. (g) *(Revision 2.1, row 77)* every `PositionedSpanFragment` satisfies `0 - eps <= x` and `x + width <= visual_width + eps`; every glyph (hanging glyphs, trailing whitespace and synthesized glyphs excepted) or inline box whose `byte_range` lies within the `byte_range` of a fragment of span *S* has its interval contained in **at least one** fragment of *S*. A line that consists of a single `InlineObject` with `AllowOverflow` wider than `wrap_width` sets `visual_width` to the object's width, so (d) and (g) hold unchanged. (h) *(rulings 8 and 91, rows 88 and 91)* every `Clip` or `ScaleToFit` inline box satisfies `x + width <= ctx.wrap_width + eps`; and `visual_width <= ctx.wrap_width + eps`, with two single-entity exemptions. **A:** the line has no visual runs, no tabs and exactly one inline box, whose policy is `AllowOverflow`. **B:** the line has no inline boxes and no tabs, and its glyphs belong to one indivisible cluster (the emergency cluster break on an empty line; they share one non-empty `byte_range`). Let *entity_range* be the box's `byte_range` (A) or the cluster's, and *entity_start_x*, *entity_end_x* the box's `x` and `x + width` (A) or the start of the first and end of the last glyph (B), with *entity_width* = `width` (A) or the sum of the glyph advances, which must equal `entity_end_x - entity_start_x` (B). If `span_fragments` is empty: `entity_start_x == 0` and `visual_width == entity_width` (within eps). If it is non-empty (the entity sits inside padded decoration spans; rows 104 and 106): every fragment starts at `entity_range.start` and ends at or after `entity_range.end`, `entity_range.end >= visible_byte_end` (so any bytes a fragment owns past the entity lie in the trailing whitespace, since (g) keeps fragments inside the line), `0 - eps <= entity_start_x` and `entity_end_x <= visual_width + eps`, and the fragments reach both edges of the body, `min(frag.x) == 0` and `max(frag.x + frag.width) == visual_width` (within eps). A 26-wide cluster with 3 units of padding on each side thus passes at `wrap_width = 30`, with no new field on `PositionedSpanFragment`, while an unpadded entity cannot inflate `visual_width`. `LineContext::default().wrap_width` is infinity (no limit); a fragment's `bidi_level` must not exceed `BIDI_MAX_DEPTH + 1`. The check is containment, not tiling: glue yields no output entity, so entities need not cover the body without gaps.

The draft required a mandatory `byte_range` on every glyph while 6.6 said synthesized glyphs have none. **[Design decision]** The field is `Option<TextRange>`. Because `CjkAutospace` has `start == end`, it cannot be a distinct item inside `line.byte_range`; this is harmless but must be handled in the line builder `[C1]`.

---

## 3. Frontend semantic analysis pipeline

The frontend runs six passes. **Complexity (revised).** The draft's claim of "strict O(N)" is withdrawn as written. Passes 1, 2, 4 and 5 are plausibly linear. Pass 3 as drafted could be quadratic: the interior scan for each pair costs O(pair length), so nested pairs of depth D cost O(N·D), and a rightward search for each unclosed quote costs O(N) on text with no strong character. Real itemisers stay linear because their stacks are bounded: Blink keeps 32 brackets and drops the oldest, Pango 128 and empties the stack when full, and UAX #9 BD16 uses 63 and stops `[S1][S2][U3]`. This specification therefore requires: a bounded stack in passes 2 and 3 **[Design decision: depth 63 by default]**, memoised next-strong look-ahead computed by one reverse sweep **[Design decision]**, and no stated bound for pass 6, because dictionary and LSTM models are documented only qualitatively ("faster than the LSTM model but requires more data"; LSTM "more expensive during segmentation") `[S6]`.

### Step 1: Extended grapheme clusters and emoji classification

1. Segment the text by UAX #29 EGC rules `[U2]`. The rules relevant to the draft are GB9 (`× (Extend|ZWJ)`), GB9a, GB9b, GB11, GB12/GB13 and GB9c. Rev 49 (Unicode 18.0.0, 2026-09-01) states GB9c as `\p{InCB=Linker} \p{InCB=Extend}* × \p{InCB=Consonant}`. Rev 47 (Unicode 17.0, 2025-08-17) had a longer form; the change is logged in rev 49 `[U2]`. GB9c was introduced with Unicode 15.1.0 (rev 43, dated 2023-08-16) `[U2]`. The implementation must read `InCB` from `DerivedCoreProperties.txt`, not only `Grapheme_Cluster_Break`. Devanagari U+0915..U+0939 and U+0958..U+095F are `InCB=Consonant`; U+094D is `InCB=Linker` and GCB Extend `[U8]`. A Devanagari conjunct such as क्ष is then one atom.
2. Classify each cluster after segmentation (no second UTS #51 state machine, so caret movement cannot disagree with segmentation): `IS_EMOJI_SEQUENCE` is set when the cluster is a core emoji sequence, a ZWJ sequence (`emoji_zwj_element ( ZWJ emoji_zwj_element )+`), a keycap sequence (base, optional U+FE0F, U+20E3), a flag sequence (two Regional Indicators), a tag sequence (tag base, tag characters U+E0020..U+E007E, tag end U+E007F), or a modifier sequence `[U5]`. The draft omitted keycap and tag sequences. A ZWJ inside Indic or Arabic text is `Script=Inherited` and must not become `Zsye`; only a ZWJ between pictographics qualifies `[U8]`. A lone Regional Indicator is not a valid flag but is still its own cluster `[U5]`.
3. A cluster whose base has `Emoji_Presentation=No` and which is not followed by U+FE0F is text style (ED-7) and is not assigned `Zsye`. A cluster followed by U+FE0E is text style and is likewise excluded `[U5]`. UTS #51 states that selectors "should not be added to existing text without an explicit directive from the user" `[U5]`.
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

Explicit locale tags win **[Design decision]**. Parley passes the script and the language into font fallback, so locale affects font choice as well as `locl` `[S4]`. Without a tag, the conservative rules are:

1. **Kana present**: Han atoms default to `ja-JP`. **[Design decision]** The scope is a run or sentence, not the whole paragraph. A Chinese sentence quoting kana, or a Japanese sentence of kanji only, defeats the paragraph-wide rule (research inference). U+30FB is not kana by Script property.
2. **Hangul present**: Han atoms default to `ko-KR`. Same scope rule; risk is low because Hanja in Korean text is correct.
3. **Serbian Cyrillic**: of the six letters U+0452, U+0458, U+0459, U+045A, U+045B, U+045F, only **U+0452 (ђ) and U+045B (ћ)** are absent from Macedonian's CLDR main exemplar set. U+0458, U+0459, U+045A and U+045F are all in Macedonian (ј љ њ џ). U+0458 also occurs in `az_Cyrl` and U+045F in Abkhaz. The Bosnian Cyrillic (`bs_Cyrl`) main set contains all six `[U9]`. The discriminators are therefore U+0452 and U+045B; the result is `sr-Cyrl` with the caveat that Bosnian Cyrillic matches too. For Macedonian, ѓ (U+0453), ќ (U+045C) and ѕ (U+0455) are the safe discriminators **[Unverified]** (not checked in CLDR). Whether Montenegrin Cyrillic adds letters is **[Unverified]**; CLDR has no `cnr` file at the paths tried.
4. Otherwise the host default locale. The engine does not toggle locales based on the frequency of Chinese ideographs. This is a **[Design decision]** with no cited source. Detecting `zh-Hant` from traditional-only characters would need Unihan variant data, which was not examined.

No primary source was found for how Chrome, Android, Gecko, WebKit, Windows or macOS derive a Han locale for unmarked text; no browser behaviour is claimed here.

### Step 5: Rich text splitting into shaping runs

Intersect contiguous (`ScriptTag`, `LocaleTag`, `BidiLevel`) spans with caller style spans (family, size, weight, slant). Style variation within a cursive script (for example colour changes across an Arabic word) must not split the `ShapingRun`; colour is applied at paint time over a single shaping context **[Design decision]**. Atoms never straddle runs (this also prevents dotted circles in Indic text, which come from splitting a syllable across runs or fonts `[U2]` research inference).

### Step 6: Segmentation and box-and-glue assembly

1. **Line breaking input.** Run `icu_segmenter::LineSegmenter` over the paragraph **text**, not over atoms: `segment_str` (UTF-8) or `segment_utf8` return byte offsets `[S6]`. Map the offsets to atom boundaries; discard offset 0, which is always returned and is not a real opportunity; determine mandatory breaks (LD7) by checking the Line_Break class of the preceding code point (`BK`, `CR`, `LF`, `NL`) against LB4/LB5, because the iterator "does not distinguish" mandatory breaks from opportunities `[S6]`. **Constructor pinning (Revision 2.1, row 80).** The default constructors of `LineSegmenter` implement UAX #14 15.1.0, so the frontend must call a `neo` constructor explicitly: `LineSegmenter::new_neo_for_non_complex_scripts` (fallible: `try_new_neo_for_non_complex_scripts_unstable`), then `load_lstm()` or `load_dictionary()` on the borrowed segmenter for Khmer, Lao, Myanmar and Thai. The `*_17_*` constructors (`new_17_for_non_complex_scripts`, `try_new_17_for_non_complex_script_unstable`, 17.0.0) are the fallback. Both families need the `unstable` Cargo feature, and the segmenter reports the data version through the `compiled_data` doc tag only; `conformance_report()` records the pinned UAX #14 version. `GraphemeClusterSegmenter::new()` and `WordSegmenter` already carry UAX #29 18.0.0 (including the GB9c `InCB` rule) and keep their default constructors, used by Step 1 and by 6.5. Constructor names were checked against the `icu_segmenter` 2.4.0 source `[S6]`.
2. **Options.** `LineBreakOptions` offers `strictness` (Loose, Normal, Strict (default), Anywhere), `word_option` (Normal, BreakAll, KeepAll) and `content_locale` `[S6]`. There are no per-call overrides, so the engine caches one segmenter per option set. Strict resolves CJ to NS; treating CJ as ID gives CSS "normal" breaking `[U7]`.
3. **Complex scripts.** `LineSegmenter` handles Khmer, Lao, Myanmar and Thai only. `new_auto` uses the LSTM model when available; `new_dictionary` uses word lists; `new_for_non_complex_scripts` disables both `[S6]`. `LineSegmenter` does not segment Chinese or Japanese; CJ dictionary segmentation exists only in `WordSegmenter` `[S6]`. Default UAX #14 gives a break opportunity between ideographs (class ID), so no dictionary is needed for line breaking. Phrase-based breaking (CSS `word-break: auto-phrase`, shipped in Chrome 119 for Japanese using a BudouX port, page dated 2023-12-04) requires a separate model and is out of scope `[S6][W2]`.
4. **Hyphens.** An explicit hyphen (U+002D) is a UAX #14 opportunity. UAX #29 word boundaries deliberately keep hyphens out of word definitions `[U2]`; word-level navigation therefore uses a separate UAX #29 pass (section 6.5), not these nodes.
5. **Punctuation fusion.** Any CL, CP, EX, IS or SY character following a non-space, and any IN, becomes an ordinary `BoxNode::Text` with **no** `Discretionary` between it and the preceding box (2.4, rule 1); there is no `PunctuationRun`. IS carries numeric context (LB15c, LB25), so fusion applies only where the UAX #14 result already forbids the break; the segmenter's output is never contradicted `[U7]`.
6. **Glue and break nodes.** A breakable space becomes `[Glue(WordSpace, Tier0), Discretionary(penalty 0)]`; a non-breaking one only the glue (2.4, rule 2). A UAX #14 opportunity between boxes becomes a `Discretionary` and a mandatory break one with `HardBreak`. Spaces become `WordSpace`. Insert `CjkAutospace` where an ideograph meets a Latin, Greek, Cyrillic or digit atom directly, with no space and no punctuation between, excluding line start and end and the neighbourhood of CJK brackets and pause marks `[W5]`. `CjkAutospace` is further suppressed on both sides of a decoration-span edge that has non-zero padding (2.2). Tabs become `Tab`; newlines a `Discretionary` with `HardBreak`. Span edges become `BoxNode::SpanBoundary`.
7. **URLs [Design decision].** UAX #14 yields few opportunities inside URLs. Typst overrides it with a link-specific rule: it detects `scheme://` or `www.`, breaks after `://`, at other-to-other and alphabetic/digit transitions, never beside `(` or `[`, and anywhere in pieces of 16 bytes or more `[L3]`. The engine offers a similar tailoring hook; the numbers are Typst's, and any adopted values must be validated.
8. **No-line-end words.** A locale-keyed table `no_line_end_words` binds listed one-letter words to the next word by emitting no `Discretionary` after them (2.4, rule 2). For Czech the set of one-letter prepositions and conjunctions is `AIiVvOoUuSsZzKk`, following the Czech Language Institute guideline as summarised by the third-party Typst package `vlna` 0.4.1 (2026-10-30) `[L8]`. It is empty for English. The draft's "A, I, a" had no support. Whether Polish follows the same convention is **[Unverified]**. The table is exported as U+00A0 semantics (GL, LB12) for CSS interoperability `[U7]`.

---

## 4. Abstract shaping bridge

### 4.1 Trait

```rust
pub trait TextShaper: Send + Sync {
    fn resolve_primary_font_metrics(
        &self, families: &[FontFamily], style: FontStyle,
        script: ScriptTag, locale: &LocaleTag, font_size: f32,
    ) -> (FontId, FontMetrics);

    /// Shapes one complete run. Output is in logical order (see 4.2).
    /// `text` is the whole paragraph text; `run.byte_range` selects the run, so
    /// cluster byte offsets are absolute paragraph indices.
    fn shape_run(
        &self, text: &str, run: &ShapingRun, atoms: &[GraphemeAtom],
        families: &[FontFamily], font_size: f32,
        features: &FontFeatures,                 // new: cache-relevant
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// New. Re-shape a window around a break, hyphenation point or word split (5.1).
    /// `atoms` and `features` are the values given to `shape_run` for this run.
    fn reshape_window(
        &self, text: &str, run: &ShapingRun, atoms: &[GraphemeAtom],
        window: TextRange, features: &FontFeatures,
        leading_edit: LeadingEdit,   // None, ReplaceSuffix: acts on the window start only
        trailing_edit: TrailingEdit, // None, AppendHyphen, ReplacePrefix: acts on the window end only
        output_clusters: &mut Vec<ShapedCluster>,
    );

    /// New. GDEF ligature caret positions (n-1 for n components); empty if absent.
    /// Relative to the glyph origin, in layout units (the unit of `font_size`).
    fn ligature_carets(&self, font: FontRef, glyph_id: u32, direction: Direction, font_size: f32) -> SmallVec<[f32; 4]>;
}
```

`reshape_window` and `ligature_carets` are **[Design decisions]** that make the break-safety and caret requirements implementable. The HarfBuzz primitive behind the latter is `hb_ot_layout_get_ligature_carets`, which returns GDEF positions that may need adjustment for kerning `[B1][B7]`. **`ligature_carets` units [Design decision].** The return value is in layout coordinates, never font design units: the backend has already applied the font size and the fallback scale (effective size `font_size · font.scale`). Positions are x offsets relative to the glyph origin, increasing, in the unit of `font_size`. The engine is unit-agnostic and does not know the device pixel ratio, so "physical pixels" means "the units of `font_size`". Doing the conversion in the backend removes design-unit arithmetic from callers. **`shape_run` input [Design decision].** `text` is the whole paragraph text and `run.byte_range` selects the run, so the backend returns absolute paragraph offsets and no layer translates coordinates; the text outside the run may serve as shaping context (HarfBuzz pre- and post-context). **Cross-font limit.** OpenType contextual lookups, kerning and `chws` act only inside one physical font file. When two adjacent clusters are routed to different fonts, passing the full text does not let the backend kern or squeeze across the seam; full-width punctuation squeezing across fonts is a middle-end fallback, computed in the metrics stage (5).

**`reshape_window` contract (Phase 0) [Design decision].** `atoms` and `features` were added to the draft signature: without the atoms the output cannot carry a local `atom_range`, and re-shaping with other features or variations would not splice. `window` is in paragraph text coordinates, lies within `run.byte_range`, and starts and ends on atom boundaries. Output `atom_range`s index `atoms` directly, in the same space as `shape_run` output, and the output clusters partition exactly the atoms inside the window. Ranges always describe the original text (6.6). The edit is two orthogonal one-sided enums: `LeadingEdit` is `None` or `ReplaceSuffix { replacement, cut_bytes }` and acts on the window's start only; `TrailingEdit` is `None`, `AppendHyphen { hyphen }` or `ReplacePrefix { replacement, cut_bytes, hyphen }` and acts on the window's end only. A regular line break passes one edit and `None` for the other edge; the middle fragment of a word split twice ("pro-gram-ming" on a narrow column) passes both; both `None` is a plain, symmetric, text-preserving re-shape. `ReplacePrefix` removes `cut_bytes` immediately before the break point, `ReplaceSuffix` immediately after it, and neither adds the other's hyphen. The two cuts must not overlap (their sum does not exceed the window length). A window carrying a hyphen contains at least one atom, since the hyphen attaches to its last cluster. With `cut_bytes == 0` and a non-empty replacement no cluster covers removed bytes, so the replacement glyphs belong to the window's last cluster (`ReplacePrefix`) or first cluster (`ReplaceSuffix`). A generated hyphen is appended as glyphs of the last cluster, flagged `RawShapedGlyph::is_synthesized`. The two replacing edits mirror `HyphenOpportunity` (2.4): `TrailingEdit::for_opportunity(opportunity, hyphen)` reads only `prefix_replacement` and `prefix_cut_bytes` (giving `ReplacePrefix`, or `AppendHyphen` when the prefix side is unchanged), `LeadingEdit::for_opportunity(opportunity)` reads only the suffix side (giving `ReplaceSuffix`, or `None`). Each `cut_bytes` ends on an atom boundary. `TextShaper` is `Send + Sync`: the methods take `&self`, one backend can serve several layout threads, and a backend with interior caches synchronises them itself.

### 4.2 Implementation requirements

1. **Complete run shaping, with font-run boundaries acknowledged.** `shape_run` receives the whole `ShapingRun` to preserve cross-word kerning, `calt` and `chws`. Context is lost at every font switch inside a run and at every chunk seam in the cache (requirement 5). The draft did not acknowledge either tension.
2. **RTL handling.** Set the buffer direction to RTL when the run's level is odd. HarfBuzz mirrors during shaping: `hb_ot_rotate_chars` substitutes `unicode->mirroring(cp)` if the font has the glyph, otherwise it sets the `rtlm` mask so the feature supplies mirrored glyphs `[B1]`. UAX #9 L4 mirrors a character if and only if its resolved direction is R and `Bidi_Mirrored=Yes`; U+FD3E and U+FD3F are not mirrored `[U3]`. HarfBuzz also reverses the buffer at the end of positioning for backward directions, so an RTL result arrives in **visual** order `[B1]`; Pango compensates with explicit reversals `[B3]`. The bridge therefore reverses to **logical** order before returning, including the glyph order within a cluster. **[Design decision]** The shaper alone owns mirroring; the frontend must never substitute mirrored code points, because mirroring twice flips the glyph back. Verify with a test of an Arabic base plus mark that `x_offset` values survive the double reversal. For DirectWrite and CoreText, mirroring behaviour was not verified from documentation **[Unverified]**.
3. **Per-atom fallback.** No surveyed engine falls back inside a single shaping call. Blink detects `.notdef` per cluster and retries the next font `[B4]`; cosmic-text shapes the whole run, collects clusters with glyph 0, shapes again with the next font and keeps only matching starts `[S5]`; Parley selects a font per character cluster using whole-cluster coverage (`calculate_coverage`) `[S4]`; DirectWrite's `MapCharacters` returns a mapped length, a font (or NULL) and a scale factor `[B5]`. Requirement: select the font per atom using script and locale as the fallback key (as Parley's `FallbackKey(script, language)` does `[S4]`), require that all code points of an atom use one font (this avoids split base/mark and split ZWJ sequences), split the run into font runs, shape each, and carry `FontRef.scale`. `Hani` skips Latin-only fonts; `Zsye` routes to colour emoji fonts (design decision on the pseudo-script, see 2.1).
4. **Many-to-many cluster merging.** Merge glyphs by cluster value, then force merging by `atom_range`. HarfBuzz's default `MONOTONE_GRAPHEMES` level already merges marks into base clusters, but the header recommends `MONOTONE_CHARACTERS` for new clients, where marks keep their own cluster value; merging only by cluster value would then allow carets or breaks between base and mark `[B1]`. A cluster value is the start index of a group; its end comes from the next distinct value in logical order.
5. **Caching.** Cache key: (text slice, font stack hash, **features, variation settings**, style, script, locale, bidi level, **direction**, size). The draft's key omitted features and variations (GPUI's `Font` includes `features`) `[B6]`. **Chunk joins [Design decision]**: runs over 256 bytes are split at whitespace into chunks; each cached chunk records whether its first cluster and its last cluster are clear of `UNSAFE_TO_CONCAT`. Because the flag describes the *start* of a cluster (2.3), no flag speaks for the end of a chunk; the rule follows the header's statement that only two pieces both clear of the flag can be joined, and is deliberately conservative: chunks A and B are spliced only if A's last cluster and B's first cluster are both clear, otherwise a window around the seam is re-shaped with `reshape_window` using the HarfBuzz header recipe `[B1]`. Whether the check on A's last cluster is needed, or B's first cluster alone suffices, was not verified **[Unverified]**; the conservative form can only cause extra re-shaping, never a wrong splice. Alternatively cache whole runs and chunk only pathological ones (the 500,000-byte case). Whitespace is not a guaranteed context boundary, since contextual lookups and `chws` can see across spaces `[B1]`. cosmic-text offers an optional `shape-run-cache` and a small shape-plan cache `[S5]`; GPUI caches whole lines `[B6]`. The claim that browsers cache per word and accept the loss of cross-word context was not verified in this research **[Unverified]**.
6. **`chws` and `uncompressed_width`.** `chws` is an ordinary OpenType feature whose effect is in the advances, so `uncompressed_width` needs a second shaping pass with the feature off (research inference). CSS `text-spacing-trim` is disabled when the font has neither `halt` nor `chws` `[W7]`; the engine therefore needs a table-based fallback mode (character-class trimming) for fonts lacking both **[Design decision]**. Whether Chromium restores width or re-shapes at a break was not verified. The Adobe CCJK blog (2019-04-22) records the feature names `chws` and `vchw` (earlier proposed as `cspc`/`vcsp`) `[W6]`.

### 4.3 Backend notes

HarfBuzz has one font per buffer and no fallback of its own (well known, not separately cited in the notes). Cluster levels (`MONOTONE_GRAPHEMES` 0, `MONOTONE_CHARACTERS` 1, `CHARACTERS` 2, `GRAPHEMES` 3) are defined in `hb-buffer.h`; the manual states that HarfBuzz is concerned with clusters, not graphemes `[B1]`. Pango sets `MONOTONE_CHARACTERS` and a language per buffer but does not request `UNSAFE_TO_CONCAT`; it re-shapes the run at a break `[B3]`. Blink uses `UNSAFE_TO_BREAK` only `[B4]`. cosmic-text and Parley treat re-shaping coarsely `[S5][S4]`. The engine can do better by following the header recipe (5.1).

### 4.4 Locale and `locl`

HarfBuzz takes the language per buffer (`hb_buffer_set_language`, used by Blink and Pango `[B3][B4]`); DirectWrite takes `localeName` in `GetGlyphs`, with the example that the same character may map to different glyphs for ja-jp versus zh-chs `[B5]`; CoreText uses `kCTLanguageAttributeName` to select localised glyphs and locale-specific line-breaking rules `[B5]`. The details of BCP 47 to OpenType language-system tag mapping were not checked **[Unverified]**.

### 4.5 Vertical metrics contract

See 5, Stage 4. `resolve_primary_font_metrics` must return a complete `FontMetrics` (2.5).

### 4.6 GPUI adapter and its limits

GPUI (crate 0.2.2, 2026-10-30; source read from `main`, signatures may differ) exposes `PlatformTextSystem::layout_line(text, font_size, runs: &[FontRun]) -> LineLayout`, where `ShapedGlyph` carries `id`, `position`, `index` (a byte index) and `is_emoji`. It has no cluster spans, no break-safety flags, no bidi, no locale or language field in `Font`, `TextRun`, `FontRun` or `PlatformTextSystem`, and its comments describe RTL as a workaround for "the lack of complete UAX #9 support". Its `LineWrapper` takes break candidates from `LineSegmenter::new_for_non_complex_scripts`, so Thai, Lao, Khmer and Burmese get no dictionary segmentation. `shape_line` asserts there is no `\n` `[B6]`. GPUI also has a missing-glyph sink for clusters that exhausted fallback and tests that its emergency wrapping never splits graphemes, but there is no re-shape after the break `[B6]`.

Consequences, **[Design decision]**: (a) the adapter is a **degraded profile** (`ShaperProfile::LineOnly`): it reconstructs clusters from glyph byte indices, reports `safe_to_break_before = false` conservatively at ambiguous seams, cannot honour `locl`, and cannot provide ligature carets; (b) a full-contract profile bypasses `PlatformTextSystem` and calls HarfRust, CoreText or DirectWrite directly, or patches GPUI. The draft's Phase 3 plan to "pass locale tags through the GPUI bridge" is unsupported by the current API.

---

## 5. Middle-end geometric layout and alignment pipeline

Given `ParagraphAst` and `wrap_width`, the pipeline has five stages.

### Stage 1: Line breaking and boundary re-shaping

**1. Overflow ladder.** Break opportunities are used in this order: (i) UAX #14 opportunities as returned by the segmenter, with the CL/CP/EX/IS/SY/IN prohibitions intact; (ii) tailored opportunities (URLs); (iii) only for a unit wider than the whole line, an emergency break. The CSS equivalents are `overflow-wrap: break-word | anywhere` and `line-break: anywhere`, which break "at otherwise disallowed points" to prevent overflow `[W1]`. If the last text box plus the boxes fused to it (closing punctuation has no `Discretionary` before it, 2.4 rule 1) does not fit on the line, the entire unit moves to the next line. **There are no emergency break points between fused boxes while the unit still fits.** Every break other than the emergency cluster break (5 below) is at a `Discretionary` (2.4). The draft's acceptance test "consecutive exclamation marks do not empty previous lines" is restated in section 7.

**2. Kinsoku and compression [Sourced].** JLReq 2.5.3: closing brackets, full stops and commas must not begin a line; opening brackets must not end a line `[W4]`. The draft covered only the first rule; the line-end prohibition for opening punctuation (UAX #14 class OP) binds an opener to the following cluster. Consecutive punctuation is normally half-width at line start and end; CLREQ 6.3.2.3 says an opening bracket at line start may have half a character trimmed before it and a full-width mark at line end shall be trimmed to half-width `[W5]`. Taiwan publications often do not adjust punctuation width, while most Mainland China and Hong Kong publications do `[W5]`. **[Design decision]** The width-adjustment behaviour is profile-selected (`zh-Hans`, `zh-Hant`, `ja`).

**3. Hanging punctuation [Sourced, new].** JLReq 2.5.1 defines hanging for full stops and commas that would otherwise have to move to a line start, placed just outside the text block `[W4]`. CSS Text 4 defines `hanging-punctuation: none | [first || [force-end | allow-end] || last]` with at most one hanging character at each edge `[W2]`. The engine offers `HangMode { start: HangStartMode, end: HangEndMode }` with `HangStartMode::{None, FirstLine, AllLines}` and `HangEndMode::{None, Allow, Force}`. `FirstLine` lets an opening mark hang only on line index 0 of the paragraph (CSS `first`; a line after a hard break is not a first line), `AllLines` on every line. The edges are logical (start and end of the line in the base direction); `PositionedLine::hanging_left` and `hanging_right` are physical. `HangMode::allows(side, base_direction, is_first_line)` is the mapping: the start edge is the left one for an LTR paragraph and the right one for an RTL paragraph, the end edge the opposite; the end edge ignores the line index. `Allow` hangs a stop or comma only if it would otherwise move to the next line; `Force` hangs it even if the line could have fitted it.

**4. Boundary re-shaping.**
- *Hyphenation.* A hyphenation break extracts the prefix, applies `prefix_replacement` and `prefix_cut_bytes` (`TrailingEdit::for_opportunity`), appends U+002D (or the locale's hyphenation character; CSS has `hyphenate-character` `[W2]`), and calls `reshape_window`. Following the HarfBuzz header recipe: at the end of a line walk back from the break to the first cluster start that is not unsafe-to-concat, shape from there to the end of the line, check that the result is also clear at its start, splice; otherwise step back and repeat. At the start of the next line apply the mirrored forward algorithm, with `suffix_replacement` and `suffix_cut_bytes` (`LeadingEdit::for_opportunity`) `[B1]`. The generated hyphen has `is_synthesized = true`. Pango's approach, re-shaping the whole run, is the unoptimised fallback `[B3]`.
- *Compression recovery.* Where a line break separates two punctuation clusters that were compressed by `chws`, the line-end cluster is **trimmed** (half-width) when it does not fit and the line-start opening mark is **restored** to full width, as CSS `text-spacing-trim: normal` does (full-width opening punctuation at line start; closing punctuation at line end trimmed only if it does not otherwise fit, before justification) `[W2]`. The draft's rule restored only the line-end cluster.

**5. Emergency cluster break.** If a single unit on an empty line exceeds `wrap_width`, split at positions that are **both** UAX #29 grapheme boundaries and `ShapedCluster` boundaries, because shaper clusters may be finer than graphemes at some cluster levels. If `safe_to_break_before` is false at the split, re-shape the fragments independently. The CSS "typographic character unit" is indivisible for the operation (for example Myanmar syllable clusters) `[W1]`. UAX #14 keeps ZWJ emoji sequences together, which the segmenter's example confirms `[S6]`.

**6. One parameterised beam-lookahead line breaker (Revision 2.1, row 78) [Design decision, Sourced: L1, L2, L4, W4].** The pair of strategies is the standard dichotomy (Typst `Simple`/`Optimized`, Android `SIMPLE`/`HIGH_QUALITY`, Chromium greedy plus `ScoreLineBreaker`) `[L3][L6][L4]`. This engine treats the two as the ends of a continuum.

*6.1 Single state-machine law.* The middle-end has no separate code paths for greedy and optimal breaking. One function, `break_lines_parameterized(ast, constraints, options, beam_width: usize, window_depth: usize)`, executes every strategy. `LineBreakStrategy` maps onto its parameters:

| Strategy | `beam_width` *K* | `window_depth` *W* | Allocation | Intended use |
|---|---|---|---|---|
| `Greedy` | 1 | 1 | stack only, O(1) | monospace code blocks, terminal logs, Phase 1 UAX #14 oracle tests |
| `Balanced` (**default**) | 2 | 2 | stack only, O(1) | interactive rich-text editing of long paragraphs |
| `Publication` | 8 | 6 | small bounded buffer | static reading view, print and PDF export |

(`LineBreakStrategy::beam_width` and `window_depth` implement the table. The numbers are **[Design decision]**; the 500 KB at 120 FPS target remains the restated criterion of section 7.)

*Line-emission rule (row 98).* A `PositionedLine` corresponds to one step `(a, b]` between consecutive **chosen** `Discretionary` breakpoints; every mandatory breakpoint (`penalty == -INF_PENALTY`: both kinds of `HardBreak`) is always chosen. The sentinel therefore ends the last line, and after a final newline it ends the one trailing blank line (Stage 4.2 gives the traces).

*6.2 Prefix sums and three-state width evaluation.* Before scanning, the breaker builds a prefix-sum array over the `WordNode` sequence (or evaluates incrementally in one pass), recording cumulative box and glue natural widths and cumulative per-tier stretch and shrink: `PrefixWidth[i]`, `Stretch[tier][i]`, `Shrink[tier][i]`. For a candidate line from `Discretionary` index *a* (exclusive) to *b* (inclusive):

1. `b_vis` is found by walking back from *b* over every immediately preceding `WordSpace` and `CjkAutospace` glue node (ruling 1: the order is `[Box, Glue.., Discretionary]`). Under `Collapsible`, natural width and stretch / shrink are integrated over `(a, b_vis]` only; under `Preserved`, the glue in `(b_vis, b)` keeps its natural width as trailing hanging whitespace on the far edge of the base direction (UAX #9 L1) and takes no part in justification. The `Stretch[Tier2]` table gains `cjk_inter_char_max_stretch` per cluster flagged `ALLOW_CJK_JUSTIFY_AFTER`, the last visible cluster excepted; `CjkPunctCompression` glue contributes to `Shrink[Tier0]`.
2. `W(a, b) = post_break_width(a) + (PrefixWidth[b_vis] − PrefixWidth[a]) + pre_break_width(b)`. The `no_break_width` of the interior discretionaries is already in `PrefixWidth`; the boundary nodes contribute their `post_break_width` and `pre_break_width` instead of their `no_break_width`.

*6.3 Multi-tier piecewise badness.* This puts the JLReq / CLREQ priority ladder (Stage 3.2) into the cost function instead of leaving it to a post-hoc justification patch. For a candidate line `(a, b)` with visible content ending at `b_vis`, the net capacities over `(a, b_vis]` are `Y_t = Stretch[t][b_vis] − Stretch[t][a]` for `t ∈ {0, 1, 2}` and `Z_t = Shrink[t][b_vis] − Shrink[t][a]` for `t ∈ {0, 1}` (`Z_2 = 0`: ideographs never shrink below natural width). Put `Z = Z_0 + Z_1`, `Y_01 = Y_0 + Y_1`, `Y_total = Y_01 + Y_2`, `Δ = wrap_width − W(a, b)` and `ε = 1e-3`. Five exhaustive cases (ruling 4, row 84; implemented as `middleend::line_fit`):

- **A. Exact fit (`|Δ| ≤ ε`).** `r = 0`, `b = 0`.
- **B. Compression (`Δ < −ε`).** If `Z ≤ 1e-5` or `−Δ > Z`, the line is overfull: `r = −2`, `b = 1 000 000` (infeasible in the standard passes, allowed only in the emergency pass). Otherwise `r = Δ / Z ∈ [−1, 0)` and `b = 100·|r|³`.
- **C. Primary expansion (`ε < Δ ≤ Y_01`).** `r = Δ / Y_01 ∈ (0, 1]` and `b = 100·r³`. Tier 2 gaps stay at zero width.
- **D. Tier 2 spill (`Y_01 < Δ ≤ Y_total`, so `Y_2 > 0`).** `Δ_2 = Δ − Y_01`, `r_2 = Δ_2 / Y_2 ∈ (0, 1]`, `r = 1 + r_2`, `b = 100 + 300·r_2³`: 100 at the seam, 400 at `Y_total`.
- **E. Underfull beyond all stretch (`Δ > Y_total`).** One rule for every `Y_total`, zero included (row 89): `r_excess = (Δ − Y_total) / Y_virt` with `Y_virt = max(font_size, 1)`, `r = 2 + r_excess` and `b = min(100 000, 400 + 400·r_excess³)`. As `Δ → Y_total` from above, `r → 2` and `b → 400`, the upper end of case D (C0). For a fixed `Δ`, a larger `Y_total` shrinks the excess, so badness is monotone non-increasing in the stretch capacity, with no threshold at `1e-5`. The cap is below the overfull value, so an underfull line always beats an overfull one and `f32` cannot overflow. With `Y_total == 0` and `Δ > ε` the line cannot be justified flush and starts at base badness 400, against 0 for an exact fit; that jump is intended (row 97), and `b` stays strictly increasing in `Δ`, so rigid underfull candidates remain strictly ordered by slack.

Non-finite inputs give the overfull fit, never a NaN.

The step (`+100` base, `3×` the cubic slope) makes the optimiser prefer breaks that absorb slack with Western spaces and CJK autospace alone, and use inter-ideographic spacing only when unavoidable. Demerits `D(a, b)` follow from `b` and the penalty of node *b* with the Knuth–Plass equations of 6.6, including α (`double_hyphen_demerits` when both ends are `flagged`), γ (`adj_demerits` across incompatible fitness classes) and `runt_penalty` when *b* ends the paragraph and the visible line holds one word or fewer than 4 characters.

*6.4 `Balanced` execution (*K* = 2, *W* = 2).* At each line boundary the state machine keeps at most two candidate breakpoints near the right margin: `B_greedy`, the furthest feasible one (`W ≤ wrap_width + Z`, or the last low-penalty one before overflow), and `B_yield`, the immediately preceding natural (`penalty == 0`) or low-penalty one. Both live in a fixed `[State; 2]` on the stack. While the scanner crosses the next line (*W* = 2) it compares the two-line cumulative demerits `D(line_i) + D(line_{i+1})` of the paths through `B_greedy` and `B_yield`; the winner for `line_i` is committed and materialised at once.

*6.5 Dominator pruning, lazy reshape, resynchronisation (`Balanced` and `Publication`).*

1. *Dominator commit.* When all *k ≤ K* live paths trace their ancestors at depth *d ≤ W* to the same `Discretionary` *B\**, every line up to *B\** is committed immediately.
2. *Lazy reshape.* `TextShaper::reshape_window` is **never** called while candidate edges are evaluated. Only when a line `(a*, b*)` is committed does the middle-end inspect `a*.action` and `b*.action`; if either is `Hyphenate`, or a boundary has `!safe_to_concat_before`, `reshape_window` runs once for that boundary. If the exact width then overflows by more than `Z` (a rare micro-overflow), the breaker falls back to the runner-up state of the beam for that one line.
3. *Incremental resynchronisation.* Each committed `PositionedLine` caches `(start_discretionary_index, start_post_break_width, fitness_class)`. After an edit in a 500 KB paragraph, relayout starts *W* lines before the dirty range and stops, in O(1), as soon as a freshly committed line matches a cached one outside the dirty range on `(start_discretionary_index, start_post_break_width)`; later cached lines are shifted in byte offset and *y*. This is a **design inference** (see the long-paragraph discussion below).

*6.6 Knuth–Plass demerits (the `Publication` profile in full; the others truncate the same recurrence).* Boxes are text boxes and inline objects; glue is the tiered glue of 2.4; penalties are the `Discretionary` nodes. From Knuth and Plass (1981):

- adjustment ratio `r`; badness `b = ∞` if `r < -1` (or undefined), else `100·|r|³`;
- demerits `D = (1 + b + n)² + a` for penalty `n ≥ 0`; `D = (1 + b)² − n² + a` for `−∞ < n < 0`; `D = (1 + b)² + a` for a forced break (`n = −∞`);
- `a = α` when this break and the previous one are both flagged (hyphen); plus `γ` when adjacent lines fall in incompatible fitness classes;
- feasibility threshold `ρ` (a break is feasible only if `r ≤ ρ`); looseness `q` optional `[L1]`.

The draft's expression `(1 + 100·|r|³ + p)²` equals the paper's case `n ≥ 0` but is not the full definition. The TeX program (tex.web 3.141592653, January 2021) differs: `d = line_penalty + b`; if `|d| ≥ 10000` then `d = 100000000` else `d = d²`; `if pi > 0 then d += pi²` (outside the square) `else if pi > eject_penalty then d −= pi²`; `+ double_hyphen_demerits` (or `final_hyphen_demerits`) for consecutive flagged breaks; `+ adj_demerits` if fitness classes differ by more than 1; `inf_bad = 10000` `[L2]`. Fitness classes in TeX: very loose (`b > 99`), loose (`b > 12`), decent, tight. TeX makes up to three passes: pretolerance without hyphenation; then tolerance with hyphenation; then `emergency_stretch` `[L2]`. **[Design decision]** The engine implements the paper's form with the threshold and emergency pass, so a feasible solution always exists; badness is the tiered piecewise function of 6.3 rather than the single cubic. Default numeric parameters are **not** specified here, because plain.tex/LaTeX defaults were not retrieved **[Unverified]**. As a precedent, Typst scores `(1 + badness + penalty)²` with `badness = 100·|ratio|³`, ratio clamped to [-2, 10], an overfull line scoring 1,000,000, a runt cost of 100 and a hyphen cost of 135 with a 15% per-character edge adjustment; its source comment records that the paper's 50 over-hyphenates in Typst's model `[L3]`. These are starting points to tune, not recommendations.

*Penalties to include:* hyphen penalty and consecutive-hyphen term; last-line short-word penalty (Chromium: `kOrphansPenalty = 10000` for a one-word last line; hyphenated second-to-last lines penalised, `kNumLastHyphenatedLines = 2` `[L4]`); locale `no_line_end_words` as bonds (3, Step 6.8). The draft's single-letter Latin penalty is removed.

*Cost model and complexity.* Knuth–Plass running time is bounded by (maximum words per line) × (number of potential breakpoints); active nodes with `r < -1` are removed `[L1]`. Typst's optimised version still loops over active predecessors, pruned by an upper bound from an approximate pass and by overfull/underfull rules `[L3]`. The linear-time `textwrap` optimal-fit works because its cost `gap²` admits SMAWK; TeX's penalty, hyphen and fitness terms do not obviously satisfy the same property (research inference; not verified) `[L7]`. **[Design decision]** The engine caps the number of active nodes (Open question 6 asks for the value).

*Long paragraphs: replacement for the 5,000-byte switch.* (Superseded in structure by 6.4 and 6.5: the default `Balanced` profile is O(1) per line and resynchronises, so no byte threshold remains; the following survey motivates it.) No byte threshold is sourced; the real driver is active-set size, not paragraph length. Production engines restrict global optimisation: Chromium's `ScoreLineBreaker` uses `kMaxLinesForBalance = 6` and `kMaxLinesForOptimal = 4` `[L4]`, Chrome's `text-wrap: pretty` (Chrome 117, page dated 2023-10-23) handles orphans only `[L5]`, and Android recommends `SIMPLE` as "ideal for editor" `[L6]`. WebKit's `pretty` evaluates the whole passage and mentions possible future optimisation for very long blocks `[L5]`. Parley is greedy only `[S4]`. No cited engine documents incremental relayout of a 500 KB paragraph. The following is therefore labelled **design inference, not a cited fact** (the `Balanced` and `Publication` profiles of 6.4 and 6.5 are its concrete form):

1. *Greedy with resynchronisation.* Store per line (start offset, end offset, shaping-state hash). After an edit, re-run greedy from the line before the dirty range; stop as soon as a newly computed line start coincides with an old start at the same style and width state, and shift later offsets. This is valid for greedy because each line depends only on its start; it is not valid for global Knuth–Plass, whose choices depend on later text.
2. *Windowed Knuth–Plass.* Optimise only a bounded window of lines between forced synchronisation breakpoints (for example the 4 to 6 lines Chromium uses, or a larger bounded count) and leave the rest greedy. This is the `window_depth` parameter. Resynchronisation is valid for the windowed forms because a path's cost depends on at most *W* following lines; it is not valid for unbounded global Knuth–Plass.
3. *Viewport-first.* On resize every line changes. Lay out the visible lines first, estimate the remainder from prefix sums of widths (Typst's `Estimates` uses prefix sums `[L3]`), and refine lazily.
4. *Budget.* 120 FPS is 8.33 ms per frame; a full-paragraph pass over about 80,000 words (500 KB at roughly 6 bytes per word, more breakpoints for CJK) cannot meet it. Phase 3's criterion is therefore restated (section 7).

**7. Hanging whitespace.** Two modes **[Design decision]**: `Collapsible` (CSS `normal`-like): trailing `WordSpace` and `CjkAutospace` collapse to zero visual width, are recorded in `line.byte_range`, and `visible_byte_end` marks the last visible cluster; `Preserved` (CSS `pre-wrap`-like): trailing spaces hang outside the line width but keep selectable, caret-reachable slots. CSS Text 3 says preserved trailing spaces at a soft wrap "must (unconditionally) hang" so the user can see them when "selecting or editing", while `break-spaces` does not hang `[W1]`. An editor will normally use `Preserved` on the line containing the caret. Trailing glue is the glue run immediately before the chosen `Discretionary` (ruling 1). **Output [Design decision].** Trailing whitespace glyphs (those whose `byte_range` starts at or after `visible_byte_end`) have zero advance under `Collapsible`. Under `Preserved` they may lie outside `[0, visual_width]`; they are exempt from the body-bounds rule, are not `is_hanging` and never count towards `hanging_left` or `hanging_right`, which describe hanging punctuation only.

### Stage 2: Per-line bidi reordering

After line breaks are fixed, reorder each line independently (UAX #9 L1 to L2):

1. Collect per-cluster levels (clusters never straddle levels; 3, Step 2.3).
2. **L1 (corrected).** On each line, reset to the paragraph embedding level: (a) segment separators (U+0009, class S) and paragraph separators (class B); (b) any sequence of whitespace characters and/or isolate formatting characters (FSI, LRI, RLI, PDI) **preceding** a segment or paragraph separator; (c) any such sequence at the end of the line. Use the **original** bidi classes, not those modified by earlier phases. U+2028 LINE SEPARATOR is class WS, not B `[U3]`. Consequence for `WordNode::Tab`: its level is always the paragraph level, and whitespace before a tab mid-line also takes the paragraph level.
3. **L2 (corrected).** "From the highest level found in the text to the lowest odd level on each line, including intermediate levels not actually present in the text, reverse any contiguous sequence of characters that are at that level or higher." The draft started at the maximum odd level. If the highest level is even (for example level 2 inside an RTL paragraph) the first reversal is skipped. A level-2 cluster inside a level-1 span is not reversed internally but moves as a unit when the level-1 span is reversed `[U3]`. Glyph arrays within a cluster are reversed whenever the cluster's final reversal count is odd.
4. N0 and BD16 are not re-run per line; they were resolved per isolating run sequence in the frontend.
5. **Span fragment assembly (Revision 2.1, row 77) [Design decision, Sourced: U3, C1].** `BoxNode::SpanBoundary` items take part in L2 with the `BidiLevel` of their adjacent inner atom (a leading boundary takes the first atom's level, a trailing one the last atom's). After L2 has put runs, inline boxes and boundaries in screen order, the middle-end sweeps left to right: contiguous visual items with the same `span_id` merge into one `PositionedSpanFragment` spanning from their leftmost to their rightmost edge; if L2 splits one span into two non-contiguous groups on the line, two fragments are emitted (matching the visual discontinuity of `SelectionRect`, 6.6). `is_first_fragment` marks the fragment that holds the logical start of the span, `is_last_fragment` the one that holds the logical end.

### Stage 3: Justification and tab stops

For lines other than the last or those ending in `HardBreak`, compute `slack = wrap_width − line_natural_width`.

1. **Tab stops [Design decision].** If the line contains a `Tab`, glue left of the final tab is frozen at natural width and slack is distributed only to the right of it.
2. **Priority ladder (replaces the draft's primary/secondary pools and the `1.5x` guard).** CSS Text 4 says a justification algorithm "may divide justification opportunities into different priority levels"; opportunities at one level expand or compress equally, and `text-justify` offers `auto | none | inter-word | inter-character | ruby` plus `no-compress` `[W2]`. JLReq 3.8.4 gives the expansion order for Japanese: (1) Western word spaces up to 1/2 em each; (2) the 1/4 em Japanese–Latin gap up to 1/2 em (1/3 em in some styles); (3) other non-prohibited places equally up to 1/4 em; (4) if still short, remaining non-inseparable gaps equally `[W4]`. The draft's `0.25 × font_size` constant matches step (3) but applies only after steps (1) and (2) are exhausted. **[Design decision]** The ladder is `Level0 = GlueTier::Tier0` (`WordSpace` and the punctuation pool), `Level1 = Tier1` (`CjkAutospace`), `Level2 = Tier2` (`ALLOW_CJK_JUSTIFY_AFTER` gaps), `Level3 = residual equal expansion`, with per-level capacities taken from the active profile. The same ladder drives the badness of Stage 1.6.3, so the line breaker already prefers breaks that need only the lower tiers.
3. **Compression pool (new).** JLReq 3.8.3 orders compression in six steps: Western word spaces down to 1/4 em; then the half-em after closing brackets, commas and full stops at line end is removed; then the 1/4 em around middle dots at line end; then the middle-dot 1/4 em mid-line; then the half-em before opening brackets and after closing brackets and commas mid-line; last, the 1/4 em Japanese–Latin gap to a minimum of 1/8 em. A JIS X 4051 variant reorders the middle-dot and bracket steps `[W4]`. That summary came through a summarising fetch tool, so the wording should be checked against the page. Without a compression pool, kinsoku lines are systematically over-loose; the engine breaks optimistically, then compresses. CLREQ 6.3.2.2: two adjacent marks that each take 2 em shrink to 1.5 em, and some styles to 1 em `[W5]`.
4. **Tolerance.** The `slack > capacity × 1.5` guard is removed. Use the adjustment ratio `r = slack / total_stretch` (capacity from the ladder) with a tolerance: lines with `r` above the tolerance are rejected by the `Publication` and `Balanced` profiles when an alternative exists and, under `Greedy` (or when none does), justified as far as the ladder allows and then left ragged **[Design decision]**. JLReq's fall-through (step 4) expands the remaining gaps equally instead of aborting.
5. **Cursive scripts.** `NO_LETTER_SPACING` atoms receive no inter-letter expansion `[W2]`. **Gap in the draft:** with no flags, Arabic lines can only expand at word spaces and will be loose; real Arabic typography elongates (kashida) but the research did not cover alreq, JSTF or `curs` **[Unverified]** (Open question 12). The draft's "Devanagari headlines never break under justification" remains a design requirement **[Design decision]**, not a CSS rule.
6. **Thai, Lao, Khmer, Burmese.** Keeping these in the word-space pool only is permitted by CSS `auto`, which requires only word separators and boundaries between script units as opportunities `[W2]`. Whether Thai tradition prefers inter-cluster expansion is **[Unverified]**.

### Stage 4: Strut metrics, blank lines, vertical clipping

1. **Strut.** The frontend records the raw primary-font metrics (`ParagraphAst::strut_metrics`, `strut_font_size`) and never reads the line-height policy; the middle-end resolves the active strut from `LayoutOptions::line_height` with `LineHeight::resolve_strut` (`Normal`: the font's own ascent, descent and line gap; `Fixed(m)`: height `m · font_size`, half-leading folded in, gap 0). CSS Inline Layout 3: when `line-height` is `normal`, an inline box's layout bounds enclose all its glyphs, even from different fonts. When it is not `normal`, the bounds derive solely from the metrics of the first available font, ignoring fallback glyphs, and leading `L = line-height − (A + D)` splits as half-leading above and below. A box with only fallback glyphs behaves as if it contained a strut with the first font's metrics. The font's line gap enters only when `line-height` is `normal` `[B7]` (CSS Inline 3 §5.3). The draft's fixed-strut model matches CSS for non-`normal` line-height; for the default case the engine offers `LineHeight::Normal` (union of glyph bounds) and `LineHeight::Fixed(multiplier)`. Fixed-strut layouts can let fallback glyphs overflow the line box vertically; this is standard CSS behaviour, not a bug. **Resolved strut [Design decision].** `ParagraphAst::strut_metrics` stores the **raw** primary-font metrics. The active strut is the output of `LineHeight::resolve_strut` applied by the middle-end: `LineHeight::Normal` keeps the font's ascent, descent and line gap; `LineHeight::Fixed(m)` gives total height `m · font_size`, splits the leading `total − (ascent + descent)` (possibly negative) equally above and below, and sets the line gap to 0 (`LineHeight::resolve_strut`). An empty line and every line under `Fixed` has `strut.ascent + strut.descent + strut.line_gap` as its height; under `Normal` a line may be taller but not shorter.
2. **Blank lines.** Hard breaks or consecutive newlines create empty `PositionedLine`s with an empty visible range (`visible_byte_end == byte_range.start`; `byte_range` is empty at the paragraph-end sentinel and covers the newline terminator for an interior blank line), the resolved strut as height and no visual runs, so caret placement and hit-testing work. This follows from the **line-emission rule** (5 Stage 1.6, row 98): every `PositionedLine` is one step `(a, b]` between consecutive chosen breakpoints, the first line starting at the paragraph start and line *k* at `b_{k-1}.byte_range.end`; every mandatory breakpoint, the explicit `HardBreak` and the final sentinel alike, must be chosen as a line's terminating breakpoint. *N* newlines therefore yield exactly *N* + 1 lines and a trailing newline yields one trailing blank line, never two. Traces: `""` gives one line `0..0`; `"abc"` one line `0..3` with `visible_byte_end = 3`; `"abc\n"` the line `0..4` (`visible_byte_end = 3`) and the blank line `4..4`; `"a\n\nb"` the lines `0..2` (visible 1), `2..3` (empty, visible 2) and `3..4`.
3. **Diacritic clamp [Design decision, no precedent found].** `y_offset` is clamped to `[-1.5·line_height, +1.5·line_height]` to contain pathological mark stacking.

### Stage 5: Decorations

Underline and strike-through merge across adjacent clusters and synthetic glue. **[Design decision, unverified against CSS Text Decoration]** Underlines use the lowest `underline_position` among intersecting fonts so lines do not cut CJK stroke bases. The CSS Text Decoration properties (`text-underline-position`, `text-decoration-skip-ink`, `text-underline-offset`) were not examined.

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

**Atomic edit spans (Revision 2.1, row 79) [Design decision].** When `PositionedLine::caret_stops` is built, any atom boundary strictly **inside** an `AtomicEditSpan` (`span.byte_range.start < offset < span.byte_range.end`) produces no `CaretStop`, even if the span wraps across lines or holds several words or ligatures. Only `span.byte_range.start` and `span.byte_range.end` generate stops. Consequently no caret can sit at the soft wrap inside a wrapped chip. (`AtomicEditSpan::is_interior` is the predicate.)

### 6.2 Visual slots and ligature carets

1. A cluster with one atom, or a cluster flagged `IS_EMOJI_SEQUENCE`, forms one slot. A Devanagari conjunct is one atom under GB9c and so one slot automatically; U+0E33 SARA AM is one code point `[U2]`.
2. A cluster with K > 1 atoms (a ligature) divides into K sub-slots. **Source of the divisions:** first the font's GDEF `LigCaretList` through `TextShaper::ligature_carets` (`caretCount` is components minus one, positions increasing; the example "ffi" has 2 carets `[B7]`); otherwise equal division, as Pango does when several characters share a cluster `[C1]`. The positions are unshaped and may need kerning adjustment `[B1]`. The draft's restriction to Latin-like bases is removed: no source supports it. In RTL the sub-slots are laid out right to left in logical order. The consequence of the draft's exclusion for Arabic lam-alef (two atoms, one ligature cluster) would be an unreachable caret between lam and alef; whether engines subdivide Arabic ligatures was not verified **[Unverified]**.
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
5. **Atomic spans (Revision 2.1, row 79).** If the point lands inside a `PositionedSpanFragment` whose `span_id` names an `AtomicEditSpan`, the midpoint rule runs over **that fragment** (`fragment.x + 0.5·fragment.width`), never over the cumulative width of the wrapped span, or the leading edge of a narrow second-line fragment would be unreachable (ruling 7, row 87). The fragment's `bidi_level` orients it: at an even level the left half returns `CursorPosition { byte_offset: span.start, affinity: Downstream }` and the right half `CursorPosition { byte_offset: span.end, affinity: Upstream }`; at an odd level the halves swap. A point with `x < mid` (`mid = fragment.x + 0.5·fragment.width`) is in the physical left half, `x >= mid` in the right half, so the midpoint itself is the right half (confirmed, row 95). Implemented as `PositionedSpanFragment::snaps_to_logical_start`.

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

1. **Backspace and Delete are logical, and their granularity is not the atom's.** Chromium's backspace state machine deletes one code point by default, with no special case for combining marks or Indic vowel signs; it removes CRLF as a unit, keycap sequences whole, emoji modifier, variation selector and ZWJ sequences together, one flag (a regional-indicator pair) at a time, and tag sequences with their base `[C3]` (summarised by a fetch tool; verify line-level detail). **[Design decision]** The engine follows this: delete the last code point of the atom, except for atoms flagged `IS_EMOJI_SEQUENCE`, `HAS_VARIATION_SELECTOR`, CRLF and tag sequences, which are removed whole; the policy is a flag for platform variance. Forward Delete needs its own forward rule. The draft covered only Backspace and treated it as atom stepping. **Atomic spans (Revision 2.1, row 79):** `delete_backward_range(offset)` (Backspace) and `delete_forward_range(offset)` (Delete) return the entire `AtomicEditSpan::byte_range` when the atom immediately before (Backspace) or after (Delete) `offset` carries `AtomFlags::IN_ATOMIC_EDIT_SPAN`; an atomic span joins the whole-unit removal list above.
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

Selection is stored logically and rendered as visual pieces; Android's `getSelectionPath` returns "a potentially discontinuous set of rectangles" for a logical range and an empty path for an empty range `[C1]`. The merge step runs over **visual order** and joins only visually adjacent selected slots (and the glue margins between two selected slots); it never bridges a gap of unselected slots. A margin at a selection edge is split at the slot midpoint or caret stop. A selected newline optionally adds an extension rectangle (`is_line_end_extension`); whether engines do so was not verified **[Unverified]**. `Strut` matches the engine's `strut_metrics`.

**Clipboard.** Copying reads the raw UTF-8 text, so the generated hyphen never appears; CSS Text 3 states that hyphenation "is a rendering effect only" and must not change content, selection or search results `[W1]`. An actual U+00AD in the source is **not** synthesised and would be copied; WebKit once converted it to a space in extracted text and stopped (Bug 27523, noted 2010-11-10), and Firefox kept 0xAD (2009) `[C4]`. **[Design decision]** The engine states a policy flag `copy_soft_hyphen: Keep | Strip`, default `Keep`. A synthesised hyphen slot has no caret stop; caret and hit-test slots inside a rebuilt prefix (spelling replacements) still map to the original bytes.

### 6.7 Platform text-input and accessibility interfaces (new)

The draft exposed only four queries. **[Design decision]** Add: `range_rects(range) -> Vec<Rect>` (IME candidate placement; `NSTextInputClient.firstRect(forCharacterRange:actualRange:)` returns the first logical boundary rectangle `[C5]`), `rect_for_offset`, `offset_for_point` (= `hit_test`), a composition range excluded from grapheme-merge rules, UTF-16 conversion (2.1), and AccessKit-style per-line exports of `character_lengths`, `character_positions`, `character_widths` and `word_starts`, with `TextPosition { node, character_index }` `[C5]`. Whether AccessKit lengths are UTF-8 bytes was not retrieved **[Unverified]**; Windows TSF, Android `InputConnection` and iOS `UITextInput` were not examined.

---

## 7. Phased implementation roadmap and acceptance criteria

All phases share the data structures above. Every phase must also pass the Unicode conformance data for its components, run against the pinned data version (1.1): `GraphemeBreakTest.txt`, `WordBreakTest.txt`, `LineBreakTest.txt`, `BidiTest.txt` and `BidiCharacterTest.txt` **[Design decision; the test files themselves were not fetched]**.

### Phase 0 to Phase 1 transition checklist (Revision 2.1)

The structural updates below are applied to the Phase 0 workspace **before** the Phase 1 segmenter and shaper are written, so the types stay frozen through Phase 3. Status in `mutil-engine`: all five are implemented and covered by tests.

1. `x_offset: f32` added to `PositionedGlyph`; `PositionedLine::validate` checks the pen interval `[x, x + advance]`.
2. `WordNode` refactored into `Box`, `Glue`, `Discretionary`, `Tab` with `GlueTier`, `BoxOverflowPolicy` and `BreakAction`; `PunctBind` removed.
3. `InlineDecorationSpan`, `AtomicEditSpan` and `PositionedSpanFragment` added; the fragment containment check is part of `PositionedLine::validate`; `check_layout_inputs` validates both span streams and every new node.
4. `AtomFlags::IN_ATOMIC_EDIT_SPAN` added, aggregated by OR in `AtomFlags::for_cluster`.
5. `LineBreakStrategy` is `Greedy | Balanced | Publication`, `Balanced` the `Default`, with `beam_width()` and `window_depth()`.

### Phase 1: Foundational core and correctness closure

**Scope.** Data structures (section 2); Frontend Steps 1 to 6 (EGC with GB9c/InCB, two pairing tables, apostrophe filter, script resolution with a bounded stack and Script_Extensions, optional CJK quote policy, conservative locale derivation, `icu_segmenter` integration with `neo` constructor pinning and offset mapping, punctuation fusion, URL tailoring hook, span boundaries and padding-aware autospace suppression); `TextShaper` trait with the HarfRust backend and the degraded GPUI profile (4.6); many-to-many cluster aggregation using `atom_range`; the `Greedy` profile of the parameterised breaker (6.1) with the overflow ladder, kinsoku and emergency cluster splitting; atomic-span caret suppression, hit-test snapping and whole-span deletion; strut metrics; unidirectional caret, hit-test and selection queries including `HitResult`.

**Acceptance criteria (revised).**
- Devanagari conjuncts (for example क्ष) are one atom under Unicode 15.1 or later data and are shaped within one run and one font, so no dotted circle appears. A negative control with pre-15.1 data splits them.
- Thai, Lao, Khmer and Burmese break at model boundaries (LSTM via `new_auto`, or dictionary via `new_dictionary`); the test records which was used.
- `high-quality` breaks after the hyphen only.
- A run of N `!` wider than the column wraps at cluster boundaries only after it reaches an empty line, and no non-empty line begins with a CL, CP, EX, IS or SY character unless produced by the overflow fallback. (This replaces the ill-defined "do not empty previous lines".)
- CJK quotation marks, ellipses and dashes use the intended script and are not split across lines; centring or trailing-edge placement follows the locale profile (Taiwan and Hong Kong centre punctuation; Mainland China places most marks toward the trailing edge `[W5]`), not a script tag.
- Emoji sequences (ZWJ, keycap, flag, tag, modifier) move the caret atomically; Backspace removes them whole while a combining mark is removed alone.
- Long URLs wrap without overflowing; an unbreakable unit wider than the line splits only at grapheme and cluster boundaries.
- `conformance_report()` names the UAX #14 data version actually loaded by the `neo` (or `*_17_*`) constructor, and a test fails if the default 15.1.0 constructor is used.
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

**Scope.** Hyphenation with a `HyphenOpportunity` source behind an adapter: `hypher` 0.1.8 (2026-09-17) for standard Liang-pattern hyphenation (embedded automata; `hyphenate_bounded` for minimum chars each side; no allocations for words up to roughly 41 to 45 bytes, as the README and docs.rs disagree; 48 languages per README versus "more than 30" per docs.rs `[H1]`), and libhyphen-style or `hyphenation` crate extended dictionaries for non-standard hyphenation (Catalan, Dutch, Hungarian, Swedish, pre-reform German). The `hyphenation` crate 0.8.4 (2021-08-19) has had no release since `[H2]`. The libhyphen API returns `rep`, `pos` and `cut` arrays (for example `rep[5]="ff=f", pos[5]=1, cut[5]=2`) and its dictionary syntax is `pattern/change,start,cut` `[H3]`. **German ck → k-k** is the pre-1996 rule and Dutch, Catalan, Hungarian and Swedish give valid modern cases (`omaatje` → `oma-tje`, `paral·lel` → `paral-lel`, `asszonnyal` → `asz-szony-nyal`, `tillata` → `till-lata`) `[H3]`. UAX #14 §5.4 notes that with a soft hyphen the word's spelling is the unhyphenated one (for example tug<SHY>gummi) and gives pre-reform "Drucker" → "Druk- / ker" as an example `[U7]`. Also: prefix re-shaping through `reshape_window`; synthesised hyphens; the `Balanced` and `Publication` profiles (section 5, Stage 1.6) with thresholds, passes and the active-node cap; lazy `reshape_window` at commit time; dominator pruning and incremental resynchronisation (Stage 1.6.5); Latin ligature carets via GDEF; segmented LRU shaping caches with `UNSAFE_TO_CONCAT` splices; locale propagation for `locl` through the full-contract shaper profile (not the GPUI adapter).

**Acceptance criteria (revised).**
- Narrow-column English text and German text hyphenate accurately with balanced line densities, honouring minimum characters before and after the hyphen (CSS `hyphenate-limit-chars` and `hyphenate-limit-lines` are the reference vocabulary `[W2]`), and nothing is hyphenated in a word that contains U+00AD.
- Clipboard copy contains no synthesised hyphen; U+00AD follows the `copy_soft_hyphen` policy.
- CJK localised variants (`zh-Hans`, `zh-Hant`, `ja`, `ko`) resolve correctly via `locl` **under the full-contract shaper profile**.
- Spelling-change hyphenation produces the replacement forms in a Dutch, a Catalan and a Hungarian test.
- Candidate evaluation never calls `reshape_window`; it is called once per committed hyphenated boundary, and a micro-overflow falls back to the runner-up path.
- **Performance [Design decision, to be measured]**: for a 500,000-byte single paragraph, an edit near the viewport re-lays out in an amount of work bounded by the dirty region plus resynchronisation, and a resize lays out the visible lines within an 8.33 ms frame budget (120 FPS) with the remainder refined lazily. The draft's claim of "steady interactive 120 FPS" on a full-paragraph optimal layout is withdrawn: no cited engine does this, and no benchmark for 500 KB single-paragraph layout was found in the research.

---

## Open questions and unverified items

Drawn from the Gaps sections of the research notes.

1. **UAX #14 revision for the version pins.** The ICU4X default constructors implement UAX #14 15.1.0; the `neo` constructors implement 18.0.0 and sit behind the `unstable` Cargo feature `[S6]`; only a `*_for_non_complex_scripts` form exists, and the complex-script models are loaded afterwards with `load_lstm()` or `load_dictionary()` (checked in the 2.4.0 source). The impact on the acceptance tests is untested. The UAX #29 revision first containing a GB9c-like rule (15.1 versus earlier 15.0 data work) was not verified beyond the rev 43 log.
2. **Variation selectors.** Standardized Variation Sequences (`StandardizedVariants.txt`) and Mongolian free variation selectors were not fetched. `emoji-sequences.txt` was not read, so RGI counts per category are unknown.
3. **UAX #9 details not read.** The X1 to X10 depth limit text (maximum depth 125 is background knowledge), the `BidiCharacterTest.txt` data, and the Unicode version that introduced BD14 to BD16. The conformance version of the `unicode-bidi` crate (0.3.18, 2024-12-16) was not checked.
4. **Mirroring in DirectWrite and CoreText**, and CoreText `kCTRunStatusRightToLeft` semantics, were not verified. Whether `hb_buffer_reverse_clusters` is the right way to restore logical order was not researched.
5. **Itemiser behaviour.** ICU's `PAREN_STACK_DEPTH` value was not found; Skia, WebKit, Gecko and swash itemisation were not read. No engine source describes apostrophe-versus-quote disambiguation in script itemisers.
6. **Active-node cap** for Knuth–Plass, default penalties, tolerance values and fitness-class constants: plain.tex/LaTeX defaults and the Liang 1983 dissertation were not retrieved, so no numeric defaults are given. Time complexity of ICU4X dictionary and LSTM segmentation has no primary statement.
7. **Incremental relayout.** No cited source documents windowing or resynchronisation; the design in 5 Stage 1.6 (6.4, 6.5 and the long-paragraph discussion) is inference. Chromium's `ShouldOptimize` criteria, Firefox's `text-wrap: pretty` status, Minikin's `OptimalLineBreaker`, SILE, Pango, Flutter, InDesign, TextKit 2 and Zed were not researched. The date of WebKit's `pretty` post (2025-04-18) comes from a non-official secondary site.
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
20. **`HyphenOpportunity.cut_bytes`. Resolved.** Split into `prefix_cut_bytes` and `suffix_cut_bytes`; `byte_offset` is the original break position; each count ends on an atom boundary (2.4). `TrailingEdit::for_opportunity` and `LeadingEdit::for_opportunity` read one side each (4.1).
21. **`reshape_window` edge semantics. Resolved** (with 31 open). `None` is symmetric, `AppendHyphen` and `ReplacePrefix` act on the trailing edge, `ReplaceSuffix` on the leading edge (4.1).
22. **Non-glyph items in the output. Resolved.** Inline boxes by `PositionedInlineBox`, tabs by `PositionedTab` (2.6). Glue still yields no entity, so line validation is containment, not tiling.
23. **`Preserved` trailing spaces versus hanging. Resolved for the output** (5, Stage 1.7): they are exempt trailing whitespace, not hanging glyphs. 6.2.3 and 6.3.3 are reconciled through the L1 geometry (6.3 rule 3): the extreme visual stop of the visible content; it equals `(visible_byte_end, Upstream)` only when that content has the base direction. **[Design decision, to be confirmed]**: the reviewer's wording ("return the visible end byte" for any click past the rightmost visible glyph of an LTR line) differs for lines ending in an opposite-direction run.
24. **Resolved strut. Resolved, reversed.** `ParagraphAst::strut_metrics` is the raw shaper metrics with `strut_font_size`; the middle-end resolves the strut with `LineHeight::resolve_strut` (5, Stage 4.1).
25. **`HangMode`. Resolved.** `start` is `HangStartMode::{None, FirstLine, AllLines}`, `end` is `HangEndMode`; the first-line rule uses `LineContext::line_index` (5, rule 3). "First line" is line index 0, so a line after a hard break is not first (**[Design decision]**).
26. **Cluster flag union. Resolved.** Per-flag aggregation (2.2): open punctuation from the first atom; `ALLOW_CJK_JUSTIFY_AFTER` and close punctuation from the last; the rest ORed.
27. **Chunk-join wording (4.2 req. 5). Reworded** to a conservative two-sided rule. Still **[Unverified]**: whether the end-of-chunk check is needed at all.
28. **`ligature_carets` units, `shape_run` inputs. Resolved** (4.1): layout units relative to the glyph origin, `font_size · font.scale`; `text` is the whole paragraph. Unverified: whether backends return positions in logical or visual component order for RTL (6.2 says increasing).
29. **Option aggregates. Resolved.** `FrontendOptions` (analysis policy), `LayoutOptions` (layout policy) and `LayoutConstraints { wrap_width, tab_stops }` (container geometry); entry point `layout_paragraph(ast, text, shaper, constraints, options)`. `TabStops` is a uniform `step` (**[Design decision]**; the spec never defined the stops). A base-direction override is not yet an input. Because `layout_paragraph` returns no `Result`, invalid input cannot be reported by it; `check_layout_inputs` returns the reason.
30. **Who resolves the strut. Resolved**: the middle-end (24).
31. **Two-sided windows. Resolved** by `LeadingEdit` and `TrailingEdit` (4.1). Still unverified against a real backend: how replacement glyphs are attributed to clusters when the two cuts leave a window with no cluster boundary between them.
32. **`HangMode::start` scope. Resolved** (25).
33. **Order of `Discretionary` and `Glue` (Revision 2.1). Resolved (ruling 1, row 81).** Emission order is `[Box, Glue.., Discretionary]`; the width formula walks back from the break over the glue run to `b_vis`. Under `Preserved`, glue in `(b_vis, b)` hangs on the far edge of the base direction.
34. **Source bytes of a hard break (Revision 2.1). Resolved (ruling 2, row 82).** `DiscretionaryNode.byte_range` replaces `byte_offset`: empty for soft opportunities, the terminator for a hard break (empty only at the paragraph end).
35. **Tier 2 glue and the punctuation pool (Revision 2.1). Resolved (ruling 3, row 83).** Tier 2 capacity is derived from `ALLOW_CJK_JUSTIFY_AFTER`; `GlueKind::CjkPunctCompression` is the Tier 0 pool.
36. **Badness beyond the tiers (Revision 2.1). Resolved (ruling 4, row 84).** Five-case function in Stage 1.6.3. Superseded by the second rulings (rows 89 to 95).
37. **Payload of `CloneSpanPadding` and `CjkPunctAdjust` (Revision 2.1). Resolved (ruling 5, row 85).** `DiscretionaryNode.cloned_spans` replaces the action; `CjkPunctAdjust` carries two advance overrides. Superseded by the second rulings (rows 89 to 95).
38. **Span streams (Revision 2.1). Resolved (ruling 6, row 86).** Proper nesting, disjoint atom-aligned atomic spans, `SpanBoundary.bidi_level`, no styling in the AST. Superseded by the second rulings (rows 89 to 95).
39. **Hyphenation split (Revision 2.1). Decided.** A word with hyphenation opportunities is split into adjacent text boxes (rule 3 of 2.4); "keep the clusters intact and anchor the offset" is not representable in a linear sequence. A split inside a multi-atom cluster is resolved by re-shaping the window (2.3 note 3).
40. **`icu_segmenter` constructor names. Resolved** (row 80): `new_neo_for_non_complex_scripts` / `try_new_neo_for_non_complex_scripts_unstable`, fallback `new_17_for_non_complex_scripts` / `try_new_17_for_non_complex_script_unstable` (the fallible one is spelled `script`, singular, in 2.4.0). No `*_auto_neo` constructor exists. The UAX #14 revision cited in the rustdoc link is `tr14-55` although the text says 15.1.0; the data tag is read from `icu_segmenter_data::unicode_tag!()` (18.0.0 in 2.4.0).
41. **`AllowOverflow` validation (Revision 2.1). Resolved (ruling 8, row 88).** `LineContext.wrap_width` and check (h) of 2.6. Superseded by the second rulings (rows 89 to 95).
42. **Hit test across a wrapped atomic span (Revision 2.1). Resolved (ruling 7, row 87).** Midpoint of the clicked fragment, oriented by its `bidi_level`.

43. **Equal-range decoration spans (Revision 2.1 rulings). Resolved (row 92).** Allowed with distinct ids; the lower index is the outer span; `SpanBoundary` order is LIFO and enforced. `MAX_DECORATION_NESTING_DEPTH = 8` is locked.
44. **Atomic-span alignment inside glue (Revision 2.1 rulings). Resolved (row 94).** Edges must coincide with a text-atom or inline-object edge, and the span must wrap one; glue and tab interiors are rejected.
45. **Hard break at the paragraph end (Revision 2.1 rulings). Resolved (row 93).** A real break owns its non-empty terminator; the sentinel is the empty range at the end of the text, last, once.
46. **Case E growth (ruling 4). Resolved (row 89).** Unified case E with the excess normalised by the font size and capped at `100 000`.
47. **Redundant adjust data (ruling 5). Resolved (row 90).** `CjkPunctAdjust` is a unit variant; the widths carry the deltas.
48. **Overflow policy not visible in the output (ruling 8). Resolved (row 91).** `PositionedInlineBox.overflow_policy`.

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
