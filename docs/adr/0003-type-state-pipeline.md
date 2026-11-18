# ADR-0003: A Type-State Pipeline, Evolved in Place

- **Status:** Accepted
- **Date:** 2026-11-18
- **Plan:** `docs/refactor-plan.md`

## Context

The engine's four core subsystems are the frontend compiler, input gate G1, line breaking and building, and output gate G4. In each of them, phase order and the preconditions between phases exist only as statement order and comments:

- **Compiler:** creates empty vectors that later phases fill, builds a placeholder AST, and has query-named methods that mutate state.
- **Layout:** stores item indexes and re-matches them behind `unreachable!`, and its line breaker and builder each decide the same questions on their own (bugs B1 and B2).
- **G1:** error precedence is recorded only in prose.
- **G4:** numbers its passes in doc comments, and re-derives what counts as content in seven places.

The typesetting order is fixed by the specification: bidi, script, break opportunities, shaping, G1, breaking, G4. Nothing ever reorders, skips or plugs in phases.

## Decision

1. **Static phase functions, no framework.**
   - Each phase is a function, or a `&self` method on read-only context.
   - It takes the outputs of earlier phases by reference and returns its own value.
   - There is no pass manager, no `trait Pass`, no shared mutable IR and no `pipeline/` module.
   - Rust's variable scope is the pipeline manager: a phase moved before its input exists does not compile.
2. **Evolve the existing code in place, one subsystem per PR.**
   - No parallel new pipeline is built for code to migrate onto.
   - Change a signature and let `rustc` list every call site.
3. **Gates return proofs.**
   - `ValidatedInputs` and `ValidatedLayout` stay the crate-boundary proofs.
   - Inside a subsystem, a phase that establishes an invariant returns it as a value, for example `TextBoxes`, `DecorationIds` or `BodyExtent`.
   - Later phases take that value as an argument instead of re-checking or `expect`ing.
4. **Orchestrators are wiring.**
   - `compile_paragraph`, `validate_layout_inputs`, `PositionedLine::validate` and `Layout::run` list named phase calls only.
   - Their statement order is the error precedence.
5. **Borrow AST nodes; index layout's own products.**
   - Immutable AST nodes are borrowed as `&'a` for the AST's lifetime, as in `Piece<'a>`, `Token::Break(&'a DiscretionaryNode)` and `LineEnd::At { brk: &'a … }`.
   - Data that `Layout` produces and owns, such as the `reshaped` cluster map, is referred to by index or by a short local borrow, never by a field that borrows `Layout` itself.
   - `'a` never leaks into the types of `interaction`.
6. **Mutable passes over owned data are named and ordered by data.**
   - Only `resolve_*` and `mark_*` passes take `&mut`, and only on data the orchestrator owns, for example `&mut [GraphemeAtom]`, whose `script`, `bidi_level` and flags are AST fields (AX-8).
   - A mark pass that reads another pass's output takes that output as a parameter, as in `mark_cjk_justification(&mut atoms, &candidates)`.
   - Mark passes that write unrelated flags may rely on statement order, and say so in a comment.
7. **A ratchet enforces progress.** `scripts/check_pipeline.py` counts the smells this ADR removes. CI fails when a count rises above its ceiling in `scripts/pipeline_ceilings.json`, and every PR that lowers a count also lowers its ceiling.

## Consequences

- **Positive:**
  - The compiler, not reviewers or tests, rejects a phase moved out of order or a forgotten check.
  - Each orchestrator documents its pipeline.
  - Agents work through local, compiler-guided changes with no dual architecture to confuse them.
- **Negative:**
  - The first steps add lines, because they name values that were implicit before. The net reduction comes from the deletions that follow.
  - Public signatures change. ADR-0002 allows this, and each PR updates `SPECIFICATION.md` in the same change.
