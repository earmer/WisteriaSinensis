# Plan: verify once (ADR-0004)

The REVIEWTEMP findings are verified at `28eb37d`, and the gate audit in VERIFYDESIGN is checked against the code. Each stage keeps the AGENTS.md verification commands green. Tests come from an independent test agent (ADR-0001).

## Decisions taken
- **Stopless lines.** A line lying wholly inside an atomic span MAY have no caret stops. G4 does not require stops on any line, since no consumer needs them: the queries skip a stopless line or snap to the span's edges, and stay total when no line has stops (ADR-0004 rule 1, second half).
- **Ascenders, descenders and underline thickness are read by their size.** The backend reads them through read-fonts' `Metrics`, which also restores the MVAR deltas. `unitsPerEm` stays the stated value.
- **Numbers.** `f32` hardening now. Fixed-point is deferred.
- **Plain data.** `PositionedParagraph` keeps its `pub` fields, so G4 has to be complete for hand-built values.

## Stages
1. **G3**
   - Move `query_checked_geometry` onto `ValidatedInputs`.
   - Seal `GeometryAnswer`: `Open(OpenGeometry)` is non-empty and offers `first_slot()`, and `Obstructed(Clearance)`.
   - Reject an envelope that `f32` cannot represent (`block_start + height <= block_start`), and an obstruction whose `clear_at` cannot fit a strut.
   - In G4, require each line box to keep its stated height.
2. **G4 and the interaction queries**
   - Exact caret order.
   - Exact order of line bottoms.
   - Caret offsets on character boundaries.
   - Policy consistency: `is_valid` matches what `resolve` can produce.
   - `edge_caret_stop` returns an `Option`, and the queries skip stopless lines.
   - A NaN hit test no longer reports `inside`.
   - `move_logical` stops at the rendered end.
   - The demo no longer draws a negative `strokeRect`.
3. **Fonts and G2**
   - Metrics come from read-fonts' `Metrics`, with MVAR deltas and sizes.
   - The G2 reshape window keeps the original font identity.
   - Update SHP-23.
4. **G1 internals**
   - `check_boundary_order` returns the boundary positions, with no `Default` placeholders.
   - Atomic-span checks run after the item order is proved.
5. **Cross-stream ownership**
   - Charge cloned padding per span, by each span's own break mode and padding.
   - The emergency seam repair looks past span edges.
   - Ligature chains are repaired back to a safe-to-concat boundary.
   - A taken break at a seam that is not safe to break re-shapes, as the emergency seam does.
   - Add the TCY ligature-caret guard.
   - Under `Fixed`, `tallest_first_line` is the strut.
6. **Tests and ratchet**
   - A test agent writes the contract tests for stages 1–5, plus the gate-completeness property test.
   - `check_pipeline.py` ratchets the panics behind gates, now at 0. The last two re-looked up an item, already matched as text, by its index (`text_view`, `TextBoxes::view`). `BoxNode::Text(TextBox)` now hands the matched box itself to `ValidatedInputs::text_view`, as `InlineObject` does (pipeline rule 6).
   - `tests/assets/WisteriaVarTest.ttf` is a synthetic variable font for the MVAR, avar, feature-variation and variable-caret tests. It has three axes, the custom sans-to-serif `SERF` among them.
