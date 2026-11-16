# ADR-0001: Delegate New Test Authoring to a Minimal-Context Sub-Agent

- **Status:** Accepted
- **Date:** 2026-11-16

## Context

When the same agent that implements a feature or creates a new file also writes the tests for it in the same conversation, it shares the exact mental model, assumptions, and blind spots of the implementation. This produces **self-proving tests** that mirror the implementation's logic (and any bugs in it) rather than independently verifying the contract in `SPECIFICATION.md`.

## Decision

1. **No self-authored tests during implementation.**
   When adding a feature, fixing a bug, or creating a new source file, the implementing agent must **not** write new tests for that change. Running existing tests (`cargo test --all-targets`) to verify compilation and regression safety remains allowed.

2. **Delegate test authoring to a minimal-context sub-agent.**
   New tests must be written by a separate sub-agent launched with **strictly bounded context**:
   - **Provide:**
     - The relevant rule IDs and sections from `SPECIFICATION.md` (e.g., `FE-*`, `SHP-*`, `OUT-*`, `PAR-*`).
     - The public API surface (public structs, enums, traits, function signatures, and docstrings).
     - The target test location and existing harness conventions (`mutil-engine/tests/contracts/` or `mutil-engine/tests/pipeline/`).
   - **Withhold:**
     - The implementation diff, private helper bodies, internal control flow, and the implementer's reasoning.

3. **Prefer integration tests over inline unit tests.**
   Place new tests in `mutil-engine/tests/` so the test sub-agent exercises the public contract as a black box without needing to inspect private module internals.

4. **Independent failure resolution.**
   If a test written by the sub-agent fails against the implementation, verify which side disagrees with `SPECIFICATION.md` before changing either the test or the code.

## Consequences

- **Positive:** Tests validate `SPECIFICATION.md` contracts independently, catching mismatched assumptions, missing edge cases, and accidental API leaks.
- **Negative:** Requires keeping public signatures and `SPECIFICATION.md` crisp before delegating tests, and adds one sub-agent round-trip per feature.
