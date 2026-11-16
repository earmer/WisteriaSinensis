# AGENTS.md

Operational guide for coding agents working in `WisteriaSinensis`.

## Repository Layout

- `SPECIFICATION.md` — Normative specification (axioms `AX-1`–`AX-7` and rule IDs).
- `mutil-engine/` — Core Rust crate implementing the specification.
  - `mutil-engine/tests/contracts/` — Contract tests.
  - `mutil-engine/tests/pipeline/` — End-to-end pipeline tests.
- `scripts/check_spec.py` — Verifies that `SPECIFICATION.md` and `mutil-engine` public types/members/rule IDs match.
- `docs/adr/` — Architecture Decision Records.
- `.github/agents/` — Specialized review/verification agent definitions.

## Required Verification Commands

Run these before concluding any code change:

```bash
# From mutil-engine/
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

# From repository root
python3 scripts/check_spec.py
```

## Co-Development Workflow Rules

1. **No self-authored tests during implementation (`docs/adr/0001-independent-test-subagent.md`).**
   - When adding features or creating new source files, do **not** write new tests yourself.
   - Delegate new test authoring to a separate sub-agent given *only* the relevant `SPECIFICATION.md` rules, public API signatures/docstrings, and test harness conventions—not the implementation body or diff.
   - Running existing tests (`cargo test --all-targets`) during implementation is allowed.

2. **Run the Axiom Checker on your diff (`.github/agents/axiom-checker.md`).**
   - Before finishing a feature or PR, run a sub-agent with `.github/agents/axiom-checker.md` against your `git diff` (especially newly added files) to verify `AX-1`–`AX-7` compliance.

## Code Taste & Local Clarity Rules

1. **Name domain data; keep signatures, functions, and files bounded.**
   - Do **not** add `#[allow(clippy::too_many_arguments)]`—group related arguments into a named parameter/state struct, and split multi-phase functions into focused phase helpers that preserve the spec's order of operations.
   - **Size guidelines (non-test code):** aim for functions ≤ **80 lines** (hard ceiling **120 lines**, excluding doc comments) and files ≤ **600 lines** (hard ceiling **800 lines**, excluding `#[cfg(test)]`). Split larger modules by responsibility or invariant family (e.g., storage, spans, annotations).
   - Replace multi-field tuples and `bool` flags (e.g., `(usize, bool, f32, f32)`) with small named structs and two-variant enums, and use descriptive variable names in non-trivial layout/geometry math.

2. **Trust gates uniformly; keep errors specific.**
   - Code behind a trust gate (`G1`–`G4`) must rely on the gate's postconditions uniformly—do not mix `.expect(...)`, `.ok_or(...)`, and redundant `.filter(...)` checks for the same invariant, and share a single helper/constant when multiple loops enforce the same bound.
   - Do not overload an error type or variant across unrelated failure causes (e.g., input validation vs. shaper/provider contract breach vs. clearance/refit exhaustion); give distinct root causes distinct variants.
