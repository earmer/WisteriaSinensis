# FINISHED — Phase 0 hardening (TDD)

Status: full test suite green; legacy fixtures migrated; release span-scaling benchmark passes.

## Green

- `tests/phase_zero_regressions.rs` — 66 pass, 1 ignored (5k-span benchmark)
- `tests/api_contracts.rs` — 18 pass
- `badness` unit suite 12 pass, `output` hanging/line-height fixed

`cargo test --test phase_zero_regressions --test api_contracts` : ok.

## Changed

**Paragraph boundaries**
- Implemented `fronten::split_paragraphs(text)->Vec<ParagraphSlice>{content,separator}` in `frontend/mod.rs`: CRLF single separator, empty input → one empty slice, trailing sep → final empty slice, preserves UTF-8 absolute offsets, `U+2028` does not split. Separators: `\n \r \r\n U+0085 U+001C–U+001E U+000B U+000C U+2029` (the last two are application policy, not UAX #9 class B).
- `check_layout_inputs` rejects those separators in `text`; `HardBreak{is_paragraph_end:false}` only `U+2028` (3 bytes), sentinel only `len..len`.

**Entry point**
- `layout_paragraph(...) -> Result<PositionedParagraph, LayoutInputError>` propagates `check_layout_inputs` errors.
- `conformance_report()` returns `None` until components land (no `neo` claim); spec OQ40 updated to stable constructors.

**Shaping identity**
- `ShapingRun` owns `features:FontFeatures`; `TextShaper::shape_run/reshape_window` dropped `families/font_size/features` params (read from `run`).
- Added `FontInstanceRef<'a>{font,font_size,variations}`, `TextShaper::font_metrics(instance)`, `ligature_carets(instance,glyph,dir)`. `VisualGlyphRun` gains `font_size/variations`. `FontId` = face only.
- Spec: single-font cluster invariant (fallback on base code point, glyph 0 for uncovered marks).

**Penalty**
- Removed `HyphenOpportunity.penalty`; `DiscretionaryNode.penalty` sole source. Updated constructors in src + tests.

**Whitespace**
- `GlueNode` gains `original_bidi_class:BidiClass, bidi_level, is_collapsible`. NBSP/U+2007/U+202F must be `is_collapsible=false`, count toward width/`visible_byte_end`, only collapsible trailing may sit in `visible..byte_range.end`. L1 uses `original_bidi_class`.

**Input/metric/span hardening (`check_layout_inputs`)**
- Unconditional exact tiling: non-empty Text/InlineObject/WordSpace-Glue/Tab/HardBreak(`U+2028`) tile `0..len` with no gaps/overlaps; zero-length nodes only on boundaries, not strictly inside content; sentinel-only on non-empty text rejected.
- Cross-structure: `run.bidi_level == atom == cluster`, `safety_flags_consistent()`, `font.scale` finite & >0, `uncompressed_width` finite.
- Positivity: `run.font_size>0`, `strut_font_size>0`, `LineHeight::Fixed(m)` `m>0`, `ascent>=0,descent>=0,sum>0`, resolved `line_height>0`; `PositionedLine` rejects `line_height<=0`.
- Span ordering: decoration ordered start asc/end desc (equal keep order), atomic disjoint and start-ordered; bounded stack sweeps and indexed membership replace repeated span/item scans.
- `TabStops::next_stop(x)->Option<f32>`: `None` if invalid/non-finite/`<=x` (e.g. `1e10` step 8 exhausts).

**Output geometry**
- Hanging: `lo/hi` normalized, overhangs `(0-lo).max, (hi-vw).max`, straddle allowed, reject when both overflows are within geometry tolerance (including zero-advance fake hanging); extents = max overflow. `y_offset` clamp removed.
- `PositionedLine.atomic_fragments: Vec<AtomicFragment{atomic_id,full_byte_range,fragment_byte_range,x_start,x_end,bidi_level}>` decoupled from decoration ids; `hit_test` midpoint rule with parity.
- `resolve_ligature_carets(width,K,carets)`: validates count `K-1`, finite, in `(0,width)`, strictly increasing, else `K` equal parts. `BoxHeightStyle::Tight` = union of instance ascent/descent via `font_metrics` (spec/doc).

**Badness**
- Fixed C0: Case E base `400/100/0` depending on Tier2/primary presence; Tier2-only D curve `400*r2³` (0→400). `LineFit.excess_slack_bits` stores overflowed slack; `comparison_key()=(badness_bits,excess_bits)` breaks saturated ties. `MAX_BADNESS=100000`.
- Tab freezing noted in spec Stage 1/3 (freeze to left of final `Tab`); no breaker yet to apply.

**Spec prose (`Multilingual typesetting engine spec.md`)**
- Removed 256-byte chunk splice, incremental resync, unconditional double-pass `uncompressed_width`; whole-run caching baseline.
- `LocalePolicy::ExplicitOnly` default, heuristics opt-in `ExperimentalHeuristics`.
- Fixed L2 wording highest-of-any-parity→lowest odd, UTS51 modifier/keycap presentation, emergency break only at `ShapedCluster` boundaries (indivisible overflow allowed), IME `CompositionSpan` as styling span not resegmentation, stable segmenter constructors.

**Tests**
- Deleted fragile/size+tautological default asserts; added `phase_zero_regressions` (tiling, levels, safety, scales, separators, metrics, tabs, hanging, NBSP, C0, 5k spans) + `api_contracts` (split, atomic hit-test, carets, tabs, locale, conformance, badness).

## Verification after review fixes

- `cargo test --no-fail-fast --quiet`: 224 passed, 1 ignored; no warnings.
- Release 1250/5000-span benchmark: 0.495 ms / 1.938 ms (3.92× time for 4× input), passes.
- `cargo fmt` and `git diff --check` pass.
- Added probes for glue-only incomplete coverage and sub-tolerance/zero-advance fake hanging.
- Legacy hard-break fixtures use U+2028; partial fixtures now supply full coverage. Mixed-level atoms use separate shaping runs.
- Removed stale spec assertions retaining chunk caches, incremental relayout, diacritic clamps, composition resegmentation and premature type freezing.

## Review follow-up (CodeRabbit / Sourcery)

Fixed in this pass:

- Text-box atoms must tile `byte_range` contiguously (`is_self_contained`), so an
  internal byte cannot go unshaped.
- `check_layout_inputs` rejects impossible bidi levels and non-finite font
  variation coordinates, and validates raw cluster glyph measurements.
- `PositionedLine::validate` now checks atomic fragments (geometry, ranges,
  bidi level, empty lines) and visual-run font instances.
- A non-positive line or strut height reports `NonPositiveLineHeight` instead of
  a bogus mismatch.
- `resolve_ligature_carets` rejects non-finite/non-positive widths and scales
  the index before multiplying, so fallback carets cannot overflow.
- Excess slack for saturated badness ties is stored at `f64` precision.
- U+001C–U+001E split paragraphs and are rejected as paragraph separators;
  `ParagraphSeparatorInText` distinguishes them from bad constraints.
- Hard-break docs match the single-paragraph contract.
- Regression tests added for NBSP glue, atom tiling, separator set, callers,
  atomic fragments, visual runs, and zero-width placement.

Second review round:

- Zero-advance glyphs are exempt from horizontal containment and hanging
  extents; a declared extent must still be backed by a non-zero-advance
  hanging glyph, so zero-width marks cannot fake hanging.
- `check_item_order` runs before `SpanIndex::new`, so indexing is built on
  ordered, in-text items; three unit tests now expect the earlier
  `ItemsOutOfOrder` variant.
- `AtomicFragment` moved into the output module (hit-testing stays in the
  interaction layer); `output` no longer depends on `interaction`.
- `BoxHeightStyle::Tight` and `LocalePolicy::ExperimentalHeuristics` are
  documented as Phase 1/2 forward contracts.
- The tautological composition-range test was removed; a real test arrives
  with the pre-edit styling layer.

Timing is empirical scaling evidence, not a deterministic operation-count proof. Real shapers and the layout algorithm remain outside Phase 0.

## Quick start

```sh
cd mutil-engine
cargo test --test phase_zero_regressions --test api_contracts
cargo test --test strategy_enums
```
