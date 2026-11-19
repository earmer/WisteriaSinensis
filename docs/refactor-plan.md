# Refactor plan: type-state pipeline, evolved in place

- **Status:** accepted (ADR-0003); in progress on `refactor/type-state-pipeline`
- **Date:** 2026-11-18
- **Base:** `main` at `ce40651`
- **Evidence:** the two-pass scan (redundancy, maintainability), plus a combined proof copy (§3)
- **Ratchet:** `scripts/check_pipeline.py` enforces the §7 metrics

## 1. Decision

Rewrite the engine's four core subsystems as flat, statically ordered phase functions:

- the frontend compiler;
- input gate G1;
- the line breaker and builder;
- output gate G4.

Each phase takes the previous phases' outputs as arguments and returns its own value. Gates return proof values that later code takes as typed arguments. Each orchestrator is plain wiring: named phase calls in order, and that order is the error precedence.

**We will not build:**
- a pass manager or `trait Pass`;
- a shared mutable IR;
- a `pipeline/` module;
- a parallel new pipeline to migrate onto.

The typesetting order is fixed by the spec (UAX #9 → script → break opportunities → shaping → G1 → breaking → G4), so a dynamic framework would be accidental complexity. It would also bring back the problem we are fixing: phases reading fields that an earlier phase was supposed to fill.

**We evolve the existing code in place,** one subsystem per PR. Changing `fn shape_runs(&mut self, ast: &mut ParagraphAst)` into `fn shape_runs(&self, …) -> Shaped` makes `rustc` list every call site to fix. A phase that is moved before its input exists no longer compiles.

## 2. What is wrong today (measured on `main`)

The flaw is the same in all four subsystems. **Phase order and the preconditions between phases exist only as statement order and comments.**

| Subsystem | Evidence |
|---|---|
| Compiler (`frontend/compile/mod.rs:212-295`) | `Compiler::new` creates 4 empty `Vec`s (`atoms`, `atom_styles`, `atom_locales`, `units`, plus `seams`) that later phases fill. `compile()` is 11 `&mut self` calls in a row: `empty_ast()` makes a placeholder AST, `shape_runs` and `assemble_items` fill it, and `confirm_compression` `retain`s `self.seams` in place, so the field means "candidates" before it and "confirmed" after it. `breaks_after` is query-named but is the only place that sets `BINDS_TO_PREVIOUS`. 11 `&mut self` methods in `compile/`. |
| Line breaking (`middleend/layout/`) | `PieceKind::{Object, Glue} { item: usize }` and `LineEnd::At(usize)` store an index. The code then re-matches it to get the type back: **all 8 `unreachable!` sites in layout** (`line_box.rs:80`, `breaking.rs:145`, `line_builder.rs:88, 515`, `mod.rs:414, 506`, `justify.rs:35`, plus `interaction/motion.rs:75`) come from this. `fit`, `emergency` and the builder each decide hang, content and trailing glue on their own, and that caused bugs B1 and B2. `fit` re-shapes through `emergency` as a side effect, and `place_next_line` clones and rolls back the whole `reshaped` map. `LineBuildState` stores derivable fields (`body_len = order.len()`, `soft_wrap`) and is mutated by `align_line_content` and then read as final. |
| G1 (`input_validation/mod.rs:306-360`) | Named phases are mixed with 4 inline loops, and error precedence is recorded only in a prose list. "Text box is resolved" is handled in 5 idioms. `InvalidFontMetrics` is used for two unrelated causes (B5). |
| G4 (`output_validation/`) | `validate` is 7 bare calls with "Pass N" numbering in doc comments. `occupied_slots` is computed twice, and `tolerance(visual_width, 0)` four times. Seven sites each decide "is this glyph content?" in their own way. `is_hanging` is stored, though the geometry already determines it. |

The **gate boundaries are already correct**: `ValidatedInputs` (G1) and `ValidatedLayout` (G4) are proof types, and layout consumes `&ValidatedInputs`. The work is **inside** each subsystem. We do not need new gate types at the crate boundary.

Two further symptoms:
- 39 `super::super::` imports.
- `demo` ABI ownership is a convention between Rust and JS, which caused B4, the `demo_free` trap I introduced.

## 3. Proof that the plan works on this codebase

I applied every verified pass-2 patch to one fresh copy of `main`, in plan order: demo ABI, breaking, compiler, G1, G4, types. All six patches applied with no conflicts.

| Check | Result on the combined copy |
|---|---|
| `cargo test --all-targets` | **612 pass**, 0 fail (169 + 240 + 5 + 198; +1 repro test) |
| `cargo clippy --all-targets -D warnings` | clean |
| `RUSTDOCFLAGS=-D warnings cargo doc` | clean |
| `cargo +1.88 check --all-targets` | clean |
| demo clippy, tests, `build.sh` | clean, 19 tests pass |
| `demo/tests/wasm_roundtrip.mjs` (new, Node) | **pass**. The same test traps on today's `main` build. |
| `scripts/check_spec.py` | 5 disagreements, all **expected**: the `FontFeatures`/`TextRange` spec edits not yet written (FNT-2, FE-25, TYP-2) |
| `cargo fmt --check` | 4 files need `cargo fmt` (formatting only) |

Two things went wrong. Both were trivial, and both are now covered in the plan:
- `demo/src/fonts.rs` called the removed `FontFeatures::new`; the fix is a one-word change to `default()`.
- `demo/build.sh` ignored `CARGO_TARGET_DIR`.

The combined size is +740 / −501 source lines over 29 files, and +105 / −50 test lines. These first steps are **not** smaller in lines. They buy explicit order and proof values. The line reductions come from the deletions in §5, stage 7.

The bugs are proven, each with a reproduction:
- **B1:** `"  (aaaa bb"` at width 38 under `HangStartMode::AllLines` returns `Ok` from `layout_paragraph`, then `ValidatedLayout::new` rejects it.
- **B2:** the emergency path breaks after 6 characters where 7 fit.
- **B3:** an unbreakable word of 2,000 or more characters fails G1 (`InvalidTextWidth`); 1,000 characters lay out.
- **B4:** `demo_free` traps at 98 bytes or more.

So the approach is safe to apply in steps. Each subsystem changes independently, the existing 611 tests are the safety net, and the type system orders the work.

## 4. Target shape per subsystem

### 4.1 Frontend compiler

This is Jimena's sketch, corrected against the real code:

```rust
pub fn compile_paragraph(input: &ParagraphInput, options: &FrontendOptions, shaper: &dyn TextShaper)
    -> Result<ParagraphAst, FrontendError>
{
    check_input(input, options)?;                                   // FE-28, read-only
    let styles = Styles::new(input);                                // normalized once (FontFeatures canonical by construction)
    let levels = bidi::resolve_text(input, options.base_direction); // Step 2 (today hidden in Compiler::new)
    let graphemes = segment::grapheme_boundaries(input.text);
    let mut units = Units::build(input, &styles, &levels, &graphemes)?; // atoms carry level + L1_RESETTABLE; no resolve_levels overwrite
    check_span_edges(input, &units)?;
    script::resolve(&mut units.atoms, input.text);                  // Step 3: writes atom.script (an AX-8 field)
    let locales = locale::resolve(input, &units, options);          // Step 4: returned, not a field
    spacing::mark_spacing_flags(&mut units.atoms);                  // FE-31 flags from first code point + script
    let candidates = spacing::candidate_seams(&units.atoms, &locales); // FE-20/31, pure
    spacing::mark_cjk_justification(&mut units.atoms, &candidates); // reads the candidates: ordered by data
    breaks::mark_bound_atoms(&mut units.atoms);                     // the BINDS_TO_PREVIOUS writer, named as a pass
    let breaks_after = breaks::breaks_after(&units, &graphemes, &locales); // pure query
    let cuts = breaks::box_cuts(&units, &breaks_after, &candidates);
    let runs = runs::build(&units, &styles, &locales);
    let shaped = shape::shape_runs(input, &units, &runs, &cuts, shaper)?; // -> Shaped { runs, clusters, glyphs }
    let seams = spacing::confirmed_seams(&candidates, &shaped);     // a different variable from `candidates`
    let items = items::assemble(&units, &cuts, &breaks_after, &seams, &shaped);
    Ok(ParagraphAst { atoms: units.atoms, runs: shaped.runs, clusters: shaped.clusters,
                      glyphs: shaped.glyphs, items, ..ParagraphAst::from_input(input, &levels, shaper) })
}
```

Where the sketch differed from the real code:
- **Atoms are written in place on purpose.** `script`, `bidi_level` and the flags are AST fields that AX-8 allows. So `script::resolve`, `mark_spacing_flags`, `mark_cjk_justification` and `mark_bound_atoms` take `&mut [GraphemeAtom]`, which `units` owns locally. Only these named `resolve_*`/`mark_*` passes may take `&mut`. Rust cannot order two `&mut` passes over the same atoms, so the rule is: a mark pass that reads another's output takes that output as a parameter (`mark_cjk_justification(&mut atoms, &candidates)`). Passes that write unrelated flags rely on statement order, with a comment saying so.
- **`resolve_spacing` is split three ways.** Today it writes four flags, computes the seams, then writes `ALLOW_CJK_JUSTIFY_AFTER`, which reads those seams. That becomes `mark_spacing_flags`, then the pure query `candidate_seams`, then `mark_cjk_justification`.
- **Levels are computed earlier than the sketch shows.** They are already computed before units today (in `Compiler::new`). The plan moves that call into the orchestrator, where it can be seen.
- **The strut and the passed-through spans** (`decoration_spans`, `atomic_spans`, `base_direction`) are built in one place, `ParagraphAst::from_input`, instead of as `empty_ast` placeholders.

### 4.2 G1 input gate

`ValidatedInputs::new` stays the proof. Inside it:

```rust
pub(super) fn validate_layout_inputs(...) -> Result<LayoutPolicySnapshot, LayoutInputError> {
    let policy = LayoutPolicySnapshot::resolve(ast, constraints, options)?;
    check_single_paragraph(text)?;
    let boxes: TextBoxes = storage::check_storage(ast)?;   // proof: every text box resolved
    storage::check_presentation(ast, &boxes)?;
    storage::check_runs(ast, text)?;
    spans::check_bidi_and_annotations(ast, text)?;
    let ids: DecorationIds = spans::check_spans(ast, text)?;
    items::check_items_fit(ast, text)?;
    items::check_item_order(ast)?;
    items::check_each_item(ast, text, &boxes, &ids)?;
    items::check_paragraph_end(ast)?;
    Ok(policy)
}
```

Statement order is the error precedence, written once.
- **Already VERIFIED:** the named phases (m2).
- **New in the plan:** the `TextBoxes` and `DecorationIds` proofs; they replace `by_id[...]` and the five "resolved?" idioms. Split `InvalidFontMetrics` (B5).

### 4.3 Line breaking and building

The pieces become typed borrows of the AST (the AST outlives `Layout`, so there is no borrow conflict):

```rust
enum PieceKind<'a> { Cluster { run: usize, cluster: usize }, Object(&'a InlineObjectBox),
                     Glue(&'a GlueNode), Tab, SpanEdge(&'a SpanBoundary) }
enum Token<'a> { Piece(Piece<'a>), Break(&'a DiscretionaryNode) }      // no cloned Break struct
enum LineEnd<'a> { At { token: usize, brk: &'a DiscretionaryNode }, Before(usize) }
```

That **deletes all 8 `unreachable!` re-matches**.

**Lifetimes (ADR-0003 rule 5):** only immutable AST nodes are borrowed as `&'a`. Clusters stay `Cluster { run, cluster }` indexes, because the emergency path's re-shaped clusters live in `Layout.reshaped: HashMap<usize, Reshaped>`, which `Layout` owns. A `Piece<'a>` that borrowed from it would make `Layout` borrow itself. `'a` never reaches `interaction` types. `BoxNode::InlineObject` becomes a named struct (`InlineObjectBox`) so a reference to it can be taken. That is a public-API change, which ADR-0002 allows.

**One source of truth for measuring a line:**
- `Piece::is_edge_transparent()` is the single "may sit between a mark and the edge" predicate. It fixes B1 (VERIFIED, m3).
- `LineWalk` / `walk_piece` / `within()` is shared by `fit` and `emergency`. It fixes B2 (VERIFIED, m3).
- `place` then takes its numbers from the same walk, or at least `debug_assert!`s that they agree.

**`fit` becomes pure:** `fn fit(&self, start: LineStart, slot) -> LineFit { end, reshapes }`. The caller commits the reshapes, so the clone-and-rollback of `reshaped` disappears.

**`LineBuildState` is split by phase:** `PlacedLine` (pieces, order, end) goes into `align` and comes out as `AlignedLine` (final coordinates). Derived fields (`body_len`, `soft_wrap`) become methods. The `run` loop becomes `cursor = cursor.advance(end)`, with the paragraph-end invariant made explicit.

### 4.4 G4 output gate

```rust
pub fn validate(&self, ctx: &LineContext) -> Result<(), LineInvariantError> {
    self.check_finite()?;
    self.check_envelope(ctx)?;
    self.check_sources(ctx)?;
    let glyphs = self.sourced_glyphs();
    self.check_caret_stops(ctx, &glyphs)?;
    self.check_annotation_records(ctx)?;
    let Some(body) = self.check_slots(ctx)? else { return self.check_empty_line() };  // -> Option<BodyExtent>
    self.check_glyph_placement(ctx, &body)?;
    self.check_object_placement(ctx, &body)?;
    self.check_hanging(ctx, &body)?;
    self.check_annotation_placement(ctx, &body, &glyphs)?;
    self.check_span_fragments(&body)?;
    self.check_slot_overflow(ctx)?;
    self.check_slot_spans(&body)
}
```

- **Remove `PositionedGlyph::is_hanging` first** (prototype: 196/196 pipeline tests pass). That removes two error variants and four representable invalid states.
- **`GlyphRole { Trailing, ZeroAdvance, Hanging, Body }`** via `PositionedLine::role(&g)` is the single content classifier (VERIFIED, m1).
- **`BodyExtent { first, last, left, right, eps }`** is built once by `check_slots` and replaces:
  - `occupied_slots()`, computed twice;
  - the `(left, right)` and `(usize, usize)` tuples;
  - four `tolerance(visual_width, 0)` calls;
  - the containment closures, which become methods on it.
- **Line geometry** (`visual_width`, `hanging_*`) moves out of `output_validation/mod.rs` into one geometry module that both the validator and interaction import.

### 4.5 Demo ABI

`buffers.rs` keeps a table of buffers by address.
- `demo_free(address)` takes no length.
- Inputs are read through the table with a bounds check.
- The demo has 0 `unsafe` blocks.
- `wasm.js` uses one scoped `#call` with a single `finally`.
- `wasm_roundtrip.mjs` runs in CI. Native `cargo test` cannot catch allocator faults.

All of this is VERIFIED (m9).

## 5. Execution: PRs in order

Each PR is self-contained. It passes every AGENTS.md check plus the axiom checker, and its regression tests are written by an independent test sub-agent (ADR-0001). The PRs in a row of the same tier touch disjoint files, so worktree sub-agents can run them in parallel.

| Stage | PR | Contents | Starts from | Depends on |
|---|---|---|---|---|
| 0 | **Rules first** | ADR-0003, the AGENTS.md pipeline rules and checks, this plan, and the `check_pipeline.py` ratchet. | — | — |
| 1 | **Bugs with known causes** | B1/B2: `is_edge_transparent` and `LineWalk`. B4: the buffer table and `wasm_roundtrip.mjs`. B7: the URL bare-scheme rule, decided against FE-22. | m3, m9 patches | 0 |
| 1b | **B3 (cause unknown)** | Find the cause of the long-word `InvalidTextWidth` rejection, then fix it with one width-sum helper shared by the frontend and G1, or with a tolerance that scales with the count. Kept off the critical path. | probe | 0 |
| 2a | **Compiler pipeline** | §4.1: `Shaped` and a single AST literal; `mark_bound_atoms`; `Units::build` with levels; locales as a value; `resolve_spacing` split three ways; `ParagraphAst::from_input`; the `checks.rs` unit-edge unification. | m5 + g5 patches | 0 |
| 2b | **G1 pipeline, plus B5** | §4.2: named phases and the `TextBoxes`/`DecorationIds` proofs. **Owns B5:** split `InvalidFontMetrics`, with spec and error table. Plus the pass-1 g2 helpers. | m2 + g2 | 0 |
| 3 | **Layout pipeline** | §4.3: typed pieces; `InlineObjectBox` struct **with its spec edits**; `Token::Break(&DiscretionaryNode)`; `LineEnd` carries the break; pure `fit` returning `LineFit`; `PlacedLine`/`AlignedLine`; `cursor.advance`. | g3 patches on top of stage 1 | 1 |
| 4 | **G4 pipeline** | §4.4: remove `is_hanging` **with the HNG-1/HNG-3/OUT-4 rewrites and error-table rows**; `GlyphRole`; `BodyExtent`; named passes; `Reason::X.at(i)`; `approx_lt`; the geometry module move. | planB prototype + m1 + g1 | 3 |
| 5 | **Canonical value types** | `FontFeatures` sorted by construction; `TextRange` with private fields; `IndexRange`; the constructor naming convention; **its own** spec edits (FNT-2, FE-25, TYP-2). | m8 patch | 2a, 2b, 3, 4 |
| 6a | **Shaping internals** | Only inside `shaping/`: sanitise only at the gates (delete the dead metric guards); shared `finish_window`; remove `face_segments`; flatten `shape_segment`. | m7 + g7 | 0 |
| 6b | **Shaper trait signature** | `ReshapeRequest`/`ShapeOutput` (removes `#[allow(too_many_arguments)]`); guard `font_size` at input; the SHP-18/24 sentence. Changes `TextShaper`, so it touches `compile/shape.rs`, `layout/breaking.rs`, `shaper_gate.rs`, `types/` and about 8 test files. | — | 2a, 3, 6a |
| 7 | **Deletions** | The dead API; the remaining pass-1 Plan A items; `super::super` → `crate::` (tolerance included, target strictly 0). Most of the net line reduction comes from here. | g1–g9 | 2a–6b |
| 8 | **Demo app, owns B6** | `state.view`; a DOM-free `selection.js`; **B6** selection invalidation. | m9 | 1 |

**Ownership is unconditional:** every bug and item belongs to exactly one stage.

**File-set check (done):** the verified patches do not overlap. m7/g7 touch only `shaping/`, m5/g5 only `frontend/compile/`, and g3 only `middleend/layout/` and `tolerance.rs`. Stage 6 is still split, because 6b's trait change reaches the compiler and layout.

**Parallel tiers:**
- **Tier 1:** 1, 1b, 2a, 2b and 6a.
- **Tier 2:** 3 and 8, after 1.
- **Tier 3:** 4 after 3, and 6b after 2a, 3 and 6a.
- **Tier 4:** 5, then 7.

**Each PR's own proof:**
- Every existing test still passes.
- The new regression tests (for bug PRs) fail on `main` and pass on the branch.
- `check_spec.py` reports agreement after that PR's spec edits.
- The axiom checker passes AX-1..AX-7.
- The PR body reports the exit-criteria metrics in §7.

## 6. Conventions for AGENTS.md (stage 0, now in AGENTS.md "Pipeline Rules")

1. **No half-built contexts.** No struct is created with empty `Vec`s or placeholder values for later phases to fill. Phase outputs are values passed to the next phase. Output structs are built in one literal.
2. **Phases transform values.** A phase takes earlier outputs by reference and returns its own value. Only passes named `resolve_*`/`mark_*` may take `&mut`, and only on data the orchestrator owns. Query-named methods take `&self`.
3. **Orchestrators are wiring.** `compile_paragraph`, `validate_layout_inputs`, `PositionedLine::validate` and `Layout::run` contain no loops, `match`es or domain logic: named phase calls only. Their order is the error precedence.
4. **Proofs instead of re-checks.** A gate's result is passed on as a value (`ValidatedInputs`, `TextBoxes`, `BodyExtent`). Downstream code takes that value and never re-derives or `expect`s the invariant.
5. **One predicate per concept, as a method on its type.** Any deliberate deviation is commented where it happens.
6. **Typed references, not indexes to re-match.** If code would need `let X = items[i] else { unreachable!() }`, store `&X` instead.
7. **Sanitise numbers only at gates (AX-3).**
8. **Errors:** one variant per root cause, built with `Reason::X.at(index)`.
9. **Constructors:** `new` is total or panics, `try_new` returns `Option`, `try_from_*` returns `Result`.
10. **No `super::super::` imports.**
11. **ABI buffers are owned by address,** and every ABI change runs `wasm_roundtrip.mjs`.

## 7. Exit criteria (measurable, checked at the end)

| Metric | `main` today | Target |
|---|---|---|
| `unreachable!` in `middleend/layout` + `interaction` | 8 | 0 |
| `&mut self` methods in `frontend/compile` | 11 | 0; phases are free functions or `&self`, and `mark_*`/`resolve_*` take `&mut [GraphemeAtom]` |
| Empty or placeholder initialisers in `Compiler::new` / `empty_ast` | 5 + 1 | 0 (`Compiler` may disappear) |
| Inline loops in `validate_layout_inputs` | 4 | 0 |
| Places deciding "is this glyph content?" in G4 | 7 | 1 (`role`), plus commented exceptions |
| `occupied_slots()` calls per line validation | 2 | 0 (`BodyExtent` once) |
| `super::super::` imports | 39 | 0, strictly (`crate::middleend::tolerance`) |
| `unsafe` in `demo/src` | 21 mentions / 3 blocks | 0 blocks |
| Known bugs B1–B7 | 7 open | 0, each with a regression test |
| Non-test source lines | ≈ 18,400 | ≈ 17,000 or fewer after stage 7 |

## 8. Rejected alternatives

- **A dynamic pass manager or `trait Pass` over a shared IR (LLVM/GCC/FFmpeg style).** Our order is static and fixed by the spec. A shared IR would bring back unchecked field dependencies.
- **A new `pipeline/` module with gradual migration.** Two styles would coexist for weeks. Agents with limited context would mix them and write glue code. Changing in place lets `rustc` guide each step.
- **Swapping `unicode-bidi` in for our own bidi code, or `icu_locale_core` in for our locale-tag parser.** SCO-8 and FE-30 forbid the first. The second would narrow which tags we accept (SCAN_TEMP §4).
- **`IndexRange` for the cluster, atom and glyph ranges (stage 5, skipped on purpose).** It would turn six public `Range<usize>` fields into a new type, and every test that builds an inverted range to exercise a gate check with it, for little gain: the gates already check these ranges, and `TextRange` (the byte ranges) is non-invertible. Not forgotten.
- **One content block per line in the output.** GEO-6 and SLT-3/4 need content in several slots.
- **A big-bang rewrite.** The scan shows the code is not rotten: 6–10% is redundant, and most of the size is validation the spec requires. The value is in explicit ordering, not in starting over.

## 9. Risks and how each is contained

| Risk | Containment |
|---|---|
| Error precedence changes when multiple faults are present (G1, G4) | The orchestrator order is written from today's order. The test sub-agent adds multi-fault precedence tests before the G1 and G4 PRs, because none exist now (m2). |
| Floating-point boundary verdicts shift when tolerance scales are unified | Keep each check family's scale and name it. Do not unify scales (m1). |
| `is_hanging` removal makes some diagnostics less specific | Accepted and documented in the spec rewrite (HNG-1, HNG-3, OUT-4). |
| Public API churn for hosts | ADR-0002 allows it. Each PR updates `SPECIFICATION.md`, and `check_spec.py` enforces the match. |
| Parallel agent PRs conflict | Run in tiers on disjoint directories (§5). The proof copy shows that the verified patches compose without conflicts. |

---

Proof artefacts were kept in a session scratchpad outside the repo:
- `compose/`: the combined copy;
- `p2-g*.patch`, `m3.patch`, `m5.patch`: the patches;
- `m3_repro_test.rs`: the B1 reproduction;
- `planB/`: the `is_hanging` prototype.

They are temporary; stage 1 turns them into committed code and tests.
