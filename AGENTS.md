# AGENTS.md

Operational guide for coding agents working in `WisteriaSinensis`.

## Repository Layout

- `SPECIFICATION.md` — Normative specification (axioms `AX-1`–`AX-7` and rule IDs).
- `mutil-engine/` — Core Rust crate implementing the specification.
  - `mutil-engine/tests/contracts/` — Contract tests.
  - `mutil-engine/tests/pipeline/` — End-to-end pipeline tests.
- `scripts/check_spec.py` — Verifies that `SPECIFICATION.md` and `mutil-engine` public types/members/rule IDs match.
- `docs/adr/` — Architecture Decision Records (ADR-0001 test authoring, ADR-0002 public API).
- `.github/agents/` — Specialized review/verification agent definitions.

## Required Verification Commands

Run these before concluding any code change:

```bash
# From mutil-engine/
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
cargo +1.88 check --all-targets

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

3. **The public API is not frozen (`docs/adr/0002-public-api-is-not-frozen.md`).**
   - Public types, members, functions, variants, rule IDs, the demo's C ABI and its JSON may be renamed, removed or reshaped at any time. No deprecation, compatibility shim or semver bump is needed.
   - Do not keep an awkward interface only to avoid changing it. Choose the better design, and do not leave aliases or wrappers for old callers.
   - In the same change, update every caller, the tests, `SPECIFICATION.md` (so `scripts/check_spec.py` passes) and `demo/web/`.

## Code Taste & Local Clarity Rules

1. **Name domain data; keep signatures, functions, and files bounded.**
   - Do **not** add `#[allow(clippy::too_many_arguments)]`—group related arguments into a named parameter/state struct, and split multi-phase functions into focused phase helpers that preserve the spec's order of operations.
   - **Size guidelines (non-test code):** aim for functions ≤ **80 lines** (hard ceiling **120 lines**, excluding doc comments) and files ≤ **600 lines** (hard ceiling **800 lines**, excluding `#[cfg(test)]`). Split larger modules by responsibility or invariant family (e.g., storage, spans, annotations).
   - Replace multi-field tuples and `bool` flags (e.g., `(usize, bool, f32, f32)`) with small named structs and two-variant enums, and use descriptive variable names in non-trivial layout/geometry math.

2. **Trust gates uniformly; keep errors specific.**
   - Code behind a trust gate (`G1`–`G4`) must rely on the gate's postconditions uniformly—do not mix `.expect(...)`, `.ok_or(...)`, and redundant `.filter(...)` checks for the same invariant, and share a single helper/constant when multiple loops enforce the same bound.
   - Do not overload an error type or variant across unrelated failure causes (e.g., input validation vs. shaper/provider contract breach vs. clearance/refit exhaustion); give distinct root causes distinct variants.

3. **Do not hand-roll what the language, `std`, or this crate already provides.**
   - Before writing a loop, check for an existing helper: search the crate (`grep`) for the same computation, and prefer `std`/iterator methods—`find`, `position`, `any`/`all`, `min_by`/`max_by` (first/last of ties), `fill`, `map_or`, `zip`, `chunk_by`, `windows`, `retain`, `concat`—over manual index/flag loops.
   - Reuse domain helpers instead of restating them inline: `TextRange::{contains, contains_range, cover}`, `is_extent`/`is_measurement` (pass them directly: `.into_iter().all(is_extent)`), the shaper-gate `checked_*`/`resolved_line_box` helpers, `single_region`, and the existing `*_position`/`adjacent_*` helpers in `interaction.rs`.
   - When two functions share setup, a call sequence, or a conversion (e.g. buffer configuration, glyph scaling, rectangle corner normalisation), extract one private helper; do not copy-paste and tweak.
   - Use string `match` tables (on `to_ascii_lowercase()`/`to_ascii_uppercase()` when case-insensitive) instead of long `eq_ignore_ascii_case` / `==` chains.

4. **Remove noise; let control flow carry the meaning.**
   - Bind fields in the first `match`/`let … else`; never re-match the same value afterwards with an `unreachable!` arm, and never `if x.is_some() { x.unwrap()/expect() }`—use `if let`, `let … else`, or `Option::filter` (e.g. `last_mut().filter(..)`).
   - Avoid single-use temporaries and flags (`let finished = …; if finished { break }`, `let ok = …; if ok { Ok(()) } else { Err(..) }` where a direct `break`/`map_or` is clearer), redundant conjuncts already implied by an earlier early-return, and annotations the compiler infers.
   - Do not keep dead code: a branch whose result equals the fallback, a field nobody reads, or a `Result`/`Option` return that can never fail. Delete it rather than leaving it "for later".
   - Build similar struct values with update syntax (`..Self::direct(at)`, `..run.clone()`) instead of repeating every field.
   - Simplification must preserve behaviour and readability: if a rewrite is not shorter **and** at least as clear (e.g. it needs extra intermediate collections to satisfy the borrow checker), keep the original.
