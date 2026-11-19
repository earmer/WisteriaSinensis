# ADR-0004: What "Verify Once, Trust Inside" Covers

- **Status:** Accepted
- **Date:** 2026-11-19
- **Plan:** `docs/verify-once-plan.md`

## Context

ADR-0003 made the gates return proofs, and AX-3 tells code behind a gate to trust them. A review of `28eb37d` found that the two halves did not meet:

- **Coverage gaps.** Consumers assumed facts that no gate checked:
  - a line has caret stops;
  - caret stops and line boxes are exactly sorted (`partition_point`);
  - a `clear_at` leaves room for a line in `f32`;
  - a resolved strut has a non-negative box.
  A gate checked each field on its own. It did not check what downstream arithmetic or indexing needs.
- **Forgeable proofs.** `GeometryAnswer` was a `pub enum` and `query_checked_geometry` was `pub` on unvalidated constraints. Without a provider it returned `Open` with no slots, which breaks `Open`'s own invariant.
- **A misplaced boundary.** The HarfRust backend read raw tables to avoid read-fonts' normalisation. That treated a font-format convention as a contract breach and lost the MVAR deltas.
- **Unowned invariants.** Atomic spans × line breaking, span edges × seam repair, and nested cloned padding each span two streams, and no type or gate owned them.

About half of the defects were ordinary unimplemented logic (seam repair, per-span padding). Better proofs would not have prevented those, and this ADR does not claim they would.

## Decision

1. **A gate proves every precondition of its proof's consumers.**
   - That includes cardinality ("non-empty"), exact order where a consumer binary-searches, cross-field consistency, and closure under the arithmetic the consumer does next (`clear_at + strut > clear_at`).
   - A fact a gate cannot guarantee is not an invariant. The consumer handles it as a normal case: an `Option`, or a skipped line, never a panic.
2. **Only a gate builds a proof.**
   - Proof types have private fields, and their constructors are private to the gate's module.
   - A `pub` proof enum carries private-field payloads, as with `GeometryAnswer::Open(OpenGeometry)`.
   - A gate is reachable only through the proof that precedes it. G3 is a method of `ValidatedInputs`, not of `LayoutConstraints`.
   - Plain-data inputs and outputs (`ParagraphAst`, `PositionedParagraph`) keep their public fields. G1 and G4 must therefore accept exactly what their consumers can handle, whoever built the value.
3. **Format conventions are read, not judged.**
   - A backend reads third-party data through the parser's documented model, for example read-fonts' variation-aware `Metrics`.
   - The specification states which conventions it adopts, for example that an ascender, descender or thickness is read by its size.
   - The gates judge what a `TextShaper` answers. They do not judge the font file's encoding choices.
4. **Query arguments are not gated.**
   - A `Point`, an offset or a `CursorPosition` arrives anew with every query.
   - Queries are total over them: NaN, out-of-range values and off-boundary offsets do not panic.
5. **The assumption lives on the proof.**
   - A lookup that can fail if the gate is incomplete is a method of the proof type (`ValidatedLayout`, `OpenGeometry`), not an index scattered through consumers.
   - A property test (`tests/contracts/gate_completeness.rs`) feeds arbitrary values through each gate. For whatever the gate accepts, it asserts that the consumers neither panic nor return non-finite or negative geometry.
   - `scripts/check_pipeline.py` ratchets the panics that remain behind gates.

## Consequences

- **Positive:**
  - The proof-audit table ("assumed by consumers, not checked") becomes something a test keeps true, rather than prose.
  - G3 can no longer be bypassed.
- **Negative:**
  - G4 grows a few checks: exact order, box height, caret stops on character boundaries, policy consistency.
  - The interaction queries gain an `Option` path for lines with no caret stops.
- **Not done:**
  - Fixed-point or `f64` layout units. This is deferred to an ADR before the Phase 3 prefix-sum breaker. `f32` stays, guarded by closure checks at G3 and G4.
