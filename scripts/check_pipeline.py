#!/usr/bin/env python3
"""Ratchet for the type-state pipeline refactor (ADR-0003, ADR-0004).

Counts the structural smells that the refactor removes and compares each count
with its ceiling in `scripts/pipeline_ceilings.json`. A count above its ceiling
fails the check. A count below its ceiling is reported, so the PR that achieved
it can lower the ceiling (`--update` writes the current counts). Ceilings only
ever go down; a metric is done when its ceiling reaches its target.

Test code is excluded: `tests/` directories, `tests.rs` files and everything
after a file's first `#[cfg(test)]`.
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


def source_lines(directory: Path):
    """Yields `(path, line)` for the non-test lines of every `.rs` file."""
    for path in sorted(directory.rglob("*.rs")):
        if path.name == "tests.rs" or "tests" in path.relative_to(directory).parts:
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.lstrip().startswith("#[cfg(test)]"):
                break
            yield path, line


def count(directory: Path, pattern: str) -> int:
    regex = re.compile(pattern)
    return sum(1 for _, line in source_lines(directory) if regex.search(line))


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
        "panics_behind_gates": count(under("middleend"), r"panic!\(|\.expect\("),
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
        print(f"{name:24} {value:4}  target {target:<3} {status}")

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
