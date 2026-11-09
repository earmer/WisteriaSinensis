# Spec history

Historical record of spec revisions and rulings. Superseded by the spec; kept because code comments and commit history cite these row and ruling numbers.

---

## Changes from the draft

| # | Draft claim (location) | Verdict | Correction in this revision | Source |
|---|---|---|---|---|
| 1 | `is_safe_to_break_after: bool` derived from `UNSAFE_TO_BREAK` (2.3) | CORRECTED | HarfBuzz's flag marks the start of the cluster a glyph belongs to, so the field becomes `safe_to_break_before`. Add `safe_to_concat_before`, which needs `HB_BUFFER_FLAG_PRODUCE_UNSAFE_TO_CONCAT` (HarfBuzz 4.0.0, 2022-03-01). | B1 |
| 2 | Req. 1 (shape whole run) and Req. 5 (split runs above 256 bytes at whitespace) | CORRECTED (contradictory) | Superseded: cache whole ShapingRuns; no 256-byte chunking or incremental splice contract. | B1 |
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
| 22 | Stage 1: switch to greedy above 5,000 bytes; Phase 3 target of 500,000 bytes at 120 FPS | CORRECTED (inconsistent) | Superseded: Phase 1/2 reflow whole paragraphs; incremental performance targets await frontend invalidation. | L1, L4, L6 |
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
| 41 | Stage 4: diacritic clamp ±1.5 line height; Stage 5: lowest `underline_position` | UNSUPPORTED | Diacritic clamp removed; retain real shaper offsets. Underline placement remains a design decision. | none found |
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
| 68 | `layout_paragraph` | Entry point `(ast, text, shaper, constraints, options) -> Result<PositionedParagraph, LayoutInputError>`; invalid input is propagated, valid input may return the Phase 0 shell. `middleend` now depends on `shaping`. | 5 |
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
| 78 | Line breaker | One parameterised beam-lookahead state machine; `LineBreakStrategy` is `Greedy` (1, 1), `Balanced` (2, 2, default), `Publication` (8, 6); dominator pruning, lazy reshape, full-paragraph relayout. | 5 Stage 1.6 |
| 79 | Atomic spans in interaction | No caret stops strictly inside an `AtomicEditSpan`; hit tests snap to its boundaries; Backspace and Delete remove it whole. | 6.1, 6.3, 6.5 |
| 80 | Segmenter integration | Stable constructors only; actual installed data versions reported per component. | 1.1, 3 Step 6.1 |

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
| 89 | Badness case E | Base is 400 with Tier2, 100 with primary capacity only, 0 with no capacity; capped ties compare excess slack. | 5 Stage 1.6.3 |
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
| 97 | Zero-capacity badness | Base 0; excess cubic approaches 0 as slack approaches 0, subject to exact-fit tolerance. | 5 Stage 1.6.3 |
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

## Resolved and superseded open questions

Moved verbatim from the spec's "Open questions and unverified items"; the original question numbers are kept.

20. **`HyphenOpportunity.cut_bytes`. Resolved.** Split into `prefix_cut_bytes` and `suffix_cut_bytes`; `byte_offset` is the original break position; each count ends on an atom boundary (2.4). `TrailingEdit::for_opportunity` and `LeadingEdit::for_opportunity` read one side each (4.1).
21. **`reshape_window` edge semantics. Resolved** (with 31 open). `None` is symmetric, `AppendHyphen` and `ReplacePrefix` act on the trailing edge, `ReplaceSuffix` on the leading edge (4.1).
22. **Non-glyph items in the output. Resolved.** Inline boxes by `PositionedInlineBox`, tabs by `PositionedTab` (2.6). Glue still yields no entity, so line validation is containment, not tiling.
24. **Resolved strut. Resolved, reversed.** `ParagraphAst::strut_metrics` is the raw shaper metrics with `strut_font_size`; the middle-end resolves the strut with `LineHeight::resolve_strut` (5, Stage 4.1).
25. **`HangMode`. Resolved.** `start` is `HangStartMode::{None, FirstLine, AllLines}`, `end` is `HangEndMode`; the first-line rule uses `LineContext::line_index` (5, rule 3). "First line" is line index 0, so a line after a hard break is not first (**[Design decision]**).
26. **Cluster flag union. Resolved.** Per-flag aggregation (2.2): open punctuation from the first atom; `ALLOW_CJK_JUSTIFY_AFTER` and close punctuation from the last; the rest ORed.
29. **Option aggregates. Resolved.** `FrontendOptions` (analysis policy), `LayoutOptions` (layout policy) and `LayoutConstraints { wrap_width, tab_stops }` (container geometry); entry point `layout_paragraph(ast, text, shaper, constraints, options)`. `TabStops` is a uniform `step` (**[Design decision]**; the spec never defined the stops). A base-direction override is not yet an input. `layout_paragraph` returns `Result<PositionedParagraph, LayoutInputError>` and propagates `check_layout_inputs` failures.
30. **Who resolves the strut. Resolved**: the middle-end (24).
32. **`HangMode::start` scope. Resolved** (25).
33. **Order of `Discretionary` and `Glue` (Revision 2.1). Resolved (ruling 1, row 81).** Emission order is `[Box, Glue.., Discretionary]`; the width formula walks back from the break over the glue run to `b_vis`. Under `Preserved`, glue in `(b_vis, b)` hangs on the far edge of the base direction.
34. **Source bytes of a hard break (Revision 2.1). Resolved (ruling 2, row 82).** `DiscretionaryNode.byte_range` replaces `byte_offset`: empty for soft opportunities, the terminator for a hard break (empty only at the paragraph end).
35. **Tier 2 glue and the punctuation pool (Revision 2.1). Resolved (ruling 3, row 83).** Tier 2 capacity is derived from `ALLOW_CJK_JUSTIFY_AFTER`; `GlueKind::CjkPunctCompression` is the Tier 0 pool.
36. **Badness beyond the tiers (Revision 2.1). Resolved (ruling 4, row 84).** Five-case function in Stage 1.6.3. Superseded by the second rulings (rows 89 to 95).
37. **Payload of `CloneSpanPadding` and `CjkPunctAdjust` (Revision 2.1). Resolved (ruling 5, row 85).** `DiscretionaryNode.cloned_spans` replaces the action; `CjkPunctAdjust` carries two advance overrides. Superseded by the second rulings (rows 89 to 95).
38. **Span streams (Revision 2.1). Resolved (ruling 6, row 86).** Proper nesting, disjoint atom-aligned atomic spans, `SpanBoundary.bidi_level`, no styling in the AST. Superseded by the second rulings (rows 89 to 95).
39. **Hyphenation split (Revision 2.1). Decided.** A word with hyphenation opportunities is split into adjacent text boxes (rule 3 of 2.4); "keep the clusters intact and anchor the offset" is not representable in a linear sequence. A split inside a multi-atom cluster is resolved by re-shaping the window (2.3 note 3); if that break is not taken, the rejoin rule of 5, Stage 1.6.5 re-shapes both sides together, so the unbroken rendering keeps its ligature and kerning.
40. **Segmenter constructors. Superseded.** Stable interfaces and actual-version reporting replace the historical neo constructor requirement.
41. **`AllowOverflow` validation (Revision 2.1). Resolved (ruling 8, row 88).** `LineContext.wrap_width` and check (h) of 2.6. Superseded by the second rulings (rows 89 to 95).
42. **Hit test across a wrapped atomic span (Revision 2.1). Resolved (ruling 7, row 87).** Midpoint of the clicked fragment, oriented by its `bidi_level`.
43. **Equal-range decoration spans (Revision 2.1 rulings). Resolved (row 92).** Allowed with distinct ids; the lower index is the outer span; `SpanBoundary` order is LIFO and enforced. `MAX_DECORATION_NESTING_DEPTH = 8` is locked.
44. **Atomic-span alignment inside glue (Revision 2.1 rulings). Resolved (row 94).** Edges must coincide with a text-atom or inline-object edge, and the span must wrap one; glue and tab interiors are rejected.
45. **Hard break at the paragraph end (Revision 2.1 rulings). Resolved (row 93).** A real break owns its non-empty terminator; the sentinel is the empty range at the end of the text, last, once.
46. **Case E growth (ruling 4). Resolved (row 89).** Unified case E with the excess normalised by the font size and capped at `100 000`.
47. **Redundant adjust data (ruling 5). Resolved (row 90).** `CjkPunctAdjust` is a unit variant; the widths carry the deltas.
48. **Overflow policy not visible in the output (ruling 8). Resolved (row 91).** `PositionedInlineBox.overflow_policy`.
