#!/usr/bin/env python3
"""Ratchet for the type-state pipeline refactor (ADR-0003, ADR-0004).

Counts the structural smells that the refactor removes and compares each count
with its ceiling in `scripts/pipeline_ceilings.json`. A count above its ceiling
fails the check. A count below its ceiling is reported, so the PR that achieved
it can lower the ceiling (`--update` writes the current counts). Ceilings only
ever go down; a metric is done when its ceiling reaches its target.

Test code is excluded: `tests/` directories, `tests.rs` files and everything
after a file's first `#[cfg(test)]`. Comments and string contents are blanked
first, so prose and messages never count.

The proof kernel (`middleend/proofs.rs`, `middleend/validated.rs`) is where
the facts a gate proved are read (ADR-0004 rule 5). Its panics and indexing
are not counted as smells, but each must name its proof in a `// PROOF(`
comment on its line or in the comment block right above it.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE = ROOT / "mutil-engine" / "src"
DEMO = ROOT / "demo" / "src"
CEILINGS = ROOT / "scripts" / "pipeline_ceilings.json"


KERNEL = (ENGINE / "middleend" / "proofs.rs", ENGINE / "middleend" / "validated.rs")

# A panic that relies on a gate behind it.
PANIC = r"panic!\(|\.expect\("
# The same in business code, where an unwrap or assert is one too.
BUSINESS_PANIC = PANIC + r"|\.unwrap\(|\bassert!\("
# Indexing or slicing: `[` right after a name, a call or another index.
# Attributes (`#[`), macros (`vec![`) and slice types (`&[T]`) do not match.
INDEXING = r"[\w)\]]\["


# The opening of a raw (byte) string: `r"`, `r#"`, `br##"`, ...
RAW_STRING = re.compile(r'b?r(#*)"')


def is_ident(ch: str) -> bool:
    return ch.isalnum() or ch == "_"


def strip_code(code: str) -> str:
    """Blanks comments and the contents of string and char literals, keeping
    every newline so line numbers survive (after `check_spec.strip_comments`)."""
    out, i, n = [], 0, len(code)
    while i < n:
        if code.startswith("//", i):
            j = code.find("\n", i)
            i = n if j < 0 else j
        elif code.startswith("/*", i):
            j = code.find("*/", i + 2)
            j = n if j < 0 else j + 2
            out.append("\n" * code.count("\n", i, j))
            i = j
        elif (raw := RAW_STRING.match(code, i)) and (i == 0 or not is_ident(code[i - 1])):
            # A raw string ends only at `"` and its own hashes: no escapes,
            # and quotes or `//` inside it are text.
            close = '"' + raw.group(1)
            j = code.find(close, raw.end())
            j = n if j < 0 else j + len(close)
            out.append('"' + "\n" * code.count("\n", i, j) + '"')
            i = j
        elif code[i] == '"':
            j = i + 1
            while j < n and code[j] != '"':
                j += 2 if code[j] == "\\" else 1
            out.append('"' + "\n" * code.count("\n", i, j) + '"')
            i = j + 1
        elif code[i] == "'" and (m := re.match(r"'(\\.|[^\\'])'", code[i : i + 4])):
            out.append("' '")
            i += len(m.group(0))
        else:
            out.append(code[i])
            i += 1
    return "".join(out)


def file_lines(path: Path):
    """Yields `(number, code)` for the lines of `path` before its first
    `#[cfg(test)]`, with comments and strings blanked."""
    code = strip_code(path.read_text(encoding="utf-8"))
    for number, line in enumerate(code.split("\n")):
        if line.lstrip().startswith("#[cfg(test)]"):
            break
        yield number, line


def source_lines(directory: Path):
    """Yields the non-test code lines of every `.rs` file under `directory`
    outside the proof kernel."""
    for path in sorted(directory.rglob("*.rs")):
        if path.name == "tests.rs" or "tests" in path.relative_to(directory).parts:
            continue
        if path not in KERNEL:
            yield from (line for _, line in file_lines(path))


def count(directory: Path, pattern: str) -> int:
    regex = re.compile(pattern)
    return sum(1 for line in source_lines(directory) if regex.search(line))


def unproved_kernel_sites() -> list[str]:
    """The panics and indexing of the proof kernel without a `// PROOF(`
    comment on their line or in the comment block right above it."""
    regex = re.compile(f"{PANIC}|{INDEXING}")
    sites = []
    for path in KERNEL:
        raw = path.read_text(encoding="utf-8").splitlines()
        for number, code in file_lines(path):
            if not regex.search(code):
                continue
            above = number - 1
            block = [raw[number]]
            while above >= 0 and raw[above].lstrip().startswith("//"):
                block.append(raw[above])
                above -= 1
            if not any("// PROOF(" in line for line in block):
                sites.append(f"{path.relative_to(ROOT)}:{number + 1}: {raw[number].strip()}")
    return sites


def count_everywhere(pattern: str) -> int:
    """Counts matching lines in every `.rs` file of the engine and the demo,
    tests and benches included, build output excluded."""
    regex = re.compile(pattern)
    return sum(
        1
        for crate in (ROOT / "mutil-engine", ROOT / "demo")
        for path in sorted(crate.rglob("*.rs"))
        if "target" not in path.relative_to(crate).parts
        for line in path.read_text(encoding="utf-8").splitlines()
        if regex.search(line)
    )


def under(*parts: str) -> Path:
    return ENGINE.joinpath(*parts)


def metrics() -> dict[str, int]:
    """Every metric of docs/refactor-plan.md §7, with how it is counted."""
    return {
        # Index-then-re-match sites (§4.3): pieces should borrow typed nodes.
        "unreachable_in_layout": count(under("middleend", "layout"), r"unreachable!")
        + count(under("middleend", "interaction"), r"unreachable!"),
        # Compiler phases that mutate a half-built context (§4.1).
        "mut_self_in_compile": count(under("frontend", "compile"), r"fn \w+[^(]*\(&mut self"),
        # Placeholder-then-fill construction (§4.1).
        "empty_ast_placeholder": count(under("frontend"), r"fn empty_ast\b"),
        # Imports that reach two modules up (AGENTS.md pipeline rule 10).
        "super_super_imports": count(ENGINE, r"super::super::"),
        # Argument lists that should be named request structs.
        "too_many_arguments": count(ENGINE, r"too_many_arguments"),
        # Repeated content-slot derivation in G4 (§4.4).
        "occupied_slots_calls": count(under("middleend"), r"\.occupied_slots\(\)"),
        # Stored-and-validated hang flag (§4.4).
        "is_hanging_field_uses": count(ENGINE, r"\bis_hanging\b"),
        # Unsafe blocks, functions and impls in the demo (§4.5). The
        # `#[unsafe(no_mangle)]` export attribute is not counted: Rust 2024
        # requires it on every ABI export, so it is not a smell.
        "unsafe_in_demo": count(DEMO, r"\bunsafe\s*(\{|fn\b|impl\b)"),
        # Panics that rely on a gate behind them (ADR-0004 rule 5): each is
        # an assumption a gate must prove; a query handles the rest totally.
        # Business code also counts unwraps and asserts. The proof kernel is
        # counted by `unproved_kernel_sites` instead.
        "panics_behind_gates": count(under("middleend"), PANIC)
        - count(under("middleend", "layout"), PANIC)
        - count(under("middleend", "interaction"), PANIC)
        + count(under("middleend", "layout"), BUSINESS_PANIC)
        + count(under("middleend", "interaction"), BUSINESS_PANIC),
        # Kernel panics and indexing that do not name their proof.
        "unproved_kernel_sites": len(unproved_kernel_sites()),
        # Interaction queries read lines and stops through the proof views
        # (`ValidatedLine`, `SortedCaretStops`), never by index.
        "bare_indexing_in_interaction": count(under("middleend", "interaction"), INDEXING),
        # Layout reads AST storage, tokens and scratch columns through the proof
        # kernel (`ValidatedInputs`, `SentinelTokens`, `VisualOrder`), never by
        # bare slice indexing.
        "bare_indexing_in_layout": count(under("middleend", "layout"), INDEXING),
        # Ignored tests: a test either runs in `cargo test` or is a bench in
        # `benches/`, so no test is silently skipped.
        "ignored_tests": count_everywhere(r"#\[ignore\b"),
    }


TARGETS = {
    "unreachable_in_layout": 0,
    "mut_self_in_compile": 0,
    "empty_ast_placeholder": 0,
    "super_super_imports": 0,
    "too_many_arguments": 0,
    "occupied_slots_calls": 0,
    "is_hanging_field_uses": 0,
    "unsafe_in_demo": 0,
    "panics_behind_gates": 0,
    "unproved_kernel_sites": 0,
    "bare_indexing_in_interaction": 0,
    "bare_indexing_in_layout": 0,
    "ignored_tests": 0,
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--update", action="store_true", help="lower the ceilings to the current counts")
    args = parser.parse_args()

    current = metrics()
    ceilings = json.loads(CEILINGS.read_text()) if CEILINGS.exists() else dict(current)
    failed = False
    for name, value in current.items():
        ceiling = ceilings.get(name, value)
        target = TARGETS[name]
        if value > ceiling:
            status, failed = f"FAIL: above ceiling {ceiling}", True
        elif value < ceiling:
            status = f"improved from {ceiling}; lower it with --update"
        else:
            status = "done" if value == target else f"ceiling {ceiling}"
        print(f"{name:28} {value:4}  target {target:<3} {status}")
    for site in unproved_kernel_sites():
        print(f"  no PROOF comment: {site}")

    if args.update:
        lowered = {name: min(ceilings.get(name, value), value) for name, value in current.items()}
        CEILINGS.write_text(json.dumps(lowered, indent=2) + "\n")
        print(f"wrote {CEILINGS.relative_to(ROOT)}")
    if failed:
        print("A pipeline metric went up. See docs/adr/0003-type-state-pipeline.md.", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
