#!/usr/bin/env python3
"""Checks that SPECIFICATION.md and the `mutil-engine` crate name the same things.

The specification is normative and the crate implements it, so every name the
specification gives a type, a field, a variant or a member must exist in the
crate under exactly that name, and every public type of the crate must be
specified. The checker reads both with the standard library only:

1. Every `pub struct` / `pub enum` written in a ```rust block of the
   specification must exist in the crate with exactly the written public
   fields (named or tuple) or variants. A struct written with `{ .. }` is
   opaque: the crate type must have no public field. A block whose info string
   is `rust planned` describes an interface that is specified but not yet
   implemented, and is not checked against the crate.
2. Every public struct and enum of the crate must be written in a checked
   block (completeness).
3. Every backticked path `Type::member` in the specification whose `Type` is a
   crate type must name a field, variant, associated constant or method of
   that type, unless `Type` is a planned type.
4. Every requirement id (`**[ABC-12]**`) is unique, and every id the text
   cites is defined.

Exit status 0 when everything agrees; otherwise each disagreement is printed
and the status is 1.

usage: scripts/check_spec.py [SPECIFICATION.md] [crate-src-dir]
"""

from __future__ import annotations

import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = ROOT / "SPECIFICATION.md"
SRC = ROOT / "mutil-engine" / "src"

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
REQ_ID = r"[A-Z][A-Z0-9]*-\d+"


@dataclass
class TypeDef:
    """A struct or enum with the names that make up its surface."""

    kind: str  # "struct" or "enum"
    name: str
    fields: list[str] = field(default_factory=list)  # public fields, or variants
    tuple_arity: int | None = None  # public tuple fields, for a tuple struct
    opaque: bool = False  # a struct whose fields are all private
    members: set[str] = field(default_factory=set)  # methods and constants


def strip_comments(code: str) -> str:
    """Removes `//` line comments and `/* */` block comments, keeping strings."""
    out, i, n = [], 0, len(code)
    while i < n:
        if code.startswith("//", i):
            j = code.find("\n", i)
            i = n if j < 0 else j
        elif code.startswith("/*", i):
            j = code.find("*/", i + 2)
            i = n if j < 0 else j + 2
        elif code[i] == '"':
            j = i + 1
            while j < n and code[j] != '"':
                j += 2 if code[j] == "\\" else 1
            out.append(code[i : j + 1])
            i = j + 1
        elif code[i] == "'" and re.match(r"'(\\.|[^\\'])'", code[i : i + 4]):
            m = re.match(r"'(\\.|[^\\'])'", code[i : i + 4])
            out.append(m.group(0))
            i += len(m.group(0))
        else:
            out.append(code[i])
            i += 1
    return "".join(out)


def matching(code: str, open_at: int, pair: str = "{}") -> int:
    """Index just past the bracket that closes the one at `open_at`."""
    depth = 0
    for i in range(open_at, len(code)):
        if code[i] == pair[0]:
            depth += 1
        elif code[i] == pair[1]:
            depth -= 1
            if depth == 0:
                return i + 1
    return len(code)


def split_top(body: str) -> list[str]:
    """Splits `body` at commas that are not nested in brackets or generics."""
    parts, depth, cur = [], 0, []
    for i, ch in enumerate(body):
        if ch in "([{<":
            depth += 1
        elif ch in ")]}>" and not (ch == ">" and i > 0 and body[i - 1] == "-"):
            # The `>` of a `->` arrow closes nothing.
            depth -= 1
        if ch == "," and depth == 0:
            parts.append("".join(cur))
            cur = []
        else:
            cur.append(ch)
    parts.append("".join(cur))
    return [p.strip() for p in parts if p.strip()]


def strip_attrs(item: str) -> str:
    return re.sub(r"#\s*\[[^\]]*\]\s*", "", item).strip()


def parse_types(code: str, public_only: bool) -> dict[str, TypeDef]:
    """Struct and enum definitions of `code` (comments already stripped)."""
    types: dict[str, TypeDef] = {}
    vis = r"pub\s+" if public_only else r"(?:pub(?:\([^)]*\))?\s+)?"
    for m in re.finditer(rf"\b{vis}(struct|enum)\s+({IDENT})\b", code):
        kind, name = m.group(1), m.group(2)
        rest = m.end()
        # Skip generics and where clauses up to the body.
        j = rest
        while j < len(code) and code[j] not in "{(;":
            j += 1
        if j >= len(code):
            continue
        t = TypeDef(kind, name)
        if code[j] == ";":
            t.fields = []
        elif code[j] == "(":
            end = matching(code, j, "()")
            items = split_top(code[j + 1 : end - 1])
            public = [i for i in items if re.match(r"pub\s", strip_attrs(i))]
            t.tuple_arity = len(public)
            t.opaque = bool(items) and not public
        else:
            end = matching(code, j)
            body = code[j + 1 : end - 1]
            if kind == "struct":
                items = split_top(body)
                names = []
                saw_private = False
                for item in items:
                    item = strip_attrs(item)
                    if item == "..":
                        t.opaque = True
                        continue
                    fm = re.match(rf"(pub(?:\([^)]*\))?\s+)?({IDENT})\s*:", item)
                    if not fm:
                        continue
                    if fm.group(1) and fm.group(1).strip() == "pub":
                        names.append(fm.group(2))
                    else:
                        saw_private = True
                t.fields = names
                if saw_private and not names:
                    t.opaque = True
            else:
                variants = []
                for item in split_top(body):
                    item = strip_attrs(item)
                    vm = re.match(rf"({IDENT})", item)
                    if vm:
                        variants.append(vm.group(1))
                t.fields = variants
        types.setdefault(name, t)
    return types


def parse_members(code: str, types: dict[str, TypeDef]) -> None:
    """Adds the methods and associated constants of every `impl` block."""
    for m in re.finditer(rf"\bimpl(?:<[^{{]*?>)?\s+(?:({IDENT})(?:<[^{{]*?>)?\s+for\s+)?({IDENT})\b[^{{;]*\{{", code):
        trait, name = m.group(1), m.group(2)
        body = code[m.end() - 1 : matching(code, m.end() - 1)]
        if name not in types:
            continue
        # An inherent impl contributes its public items only; a trait impl
        # contributes every item, since trait items are as public as the trait.
        vis = "" if trait else r"\bpub\s+(?:const\s+)?(?:unsafe\s+)?"
        for fm in re.finditer(rf"{vis}fn\s+({IDENT})", body):
            types[name].members.add(fm.group(1))
        for cm in re.finditer(rf"{vis or r'\b'}const\s+({IDENT})\s*:", body):
            types[name].members.add(cm.group(1))
        for tm in re.finditer(rf"\btype\s+({IDENT})\s*=", body):
            types[name].members.add(tm.group(1))
        if trait:
            types[name].members.add(trait)


def remove_test_modules(code: str) -> str:
    """Drops every `#[cfg(test)] mod name { .. }`: tests are not public surface."""
    while True:
        m = re.search(rf"#\[cfg\(test\)\]\s*mod\s+{IDENT}\s*\{{", code)
        if not m:
            return code
        code = code[: m.start()] + code[matching(code, m.end() - 1) :]


def crate_types(src: Path) -> dict[str, TypeDef]:
    code = "\n".join(strip_comments(p.read_text()) for p in sorted(src.rglob("*.rs")))
    code = remove_test_modules(code)
    types = parse_types(code, public_only=True)
    parse_members(code, types)
    # Type aliases are names too.
    for am in re.finditer(rf"\bpub\s+type\s+({IDENT})\b", code):
        types.setdefault(am.group(1), TypeDef("type", am.group(1), opaque=True))
    return types


def spec_blocks(spec: str) -> tuple[list[str], list[str]]:
    checked, planned = [], []
    for m in re.finditer(r"^```rust([^\n]*)\n(.*?)^```", spec, re.S | re.M):
        (planned if "planned" in m.group(1) else checked).append(m.group(2))
    return checked, planned


def main() -> int:
    if len(sys.argv) > 1 and sys.argv[1] == "--dump":
        for name, t in sorted(crate_types(SRC).items()):
            shape = "opaque" if t.opaque else (f"tuple({t.tuple_arity})" if t.tuple_arity is not None else ", ".join(t.fields))
            print(f"{t.kind} {name}: {shape}  || {', '.join(sorted(t.members))}")
        return 0
    spec_path = Path(sys.argv[1]) if len(sys.argv) > 1 else SPEC
    src = Path(sys.argv[2]) if len(sys.argv) > 2 else SRC
    spec = spec_path.read_text()
    crate = crate_types(src)
    checked_blocks, planned_blocks = spec_blocks(spec)
    written: dict[str, TypeDef] = {}
    for block in checked_blocks:
        written.update(parse_types(strip_comments(block), public_only=True))
    planned: dict[str, TypeDef] = {}
    for block in planned_blocks:
        planned.update(parse_types(strip_comments(block), public_only=True))
    for block in planned_blocks:
        for fm in re.finditer(rf"\bfn\s+({IDENT})", block):
            planned.setdefault(fm.group(1), TypeDef("fn", fm.group(1)))
    errors: list[str] = []

    # 1. Written types match the crate exactly.
    for name, w in sorted(written.items()):
        c = crate.get(name)
        if c is None:
            errors.append(f"spec type `{name}` does not exist in the crate (mark its block `rust planned` if it is future work)")
            continue
        if w.kind == "struct" and c.kind == "struct":
            if w.opaque:
                if c.fields or c.tuple_arity:
                    errors.append(f"`{name}` is written opaque but the crate exposes fields {c.fields or c.tuple_arity}")
            elif w.tuple_arity is not None or c.tuple_arity is not None:
                if w.tuple_arity != c.tuple_arity:
                    errors.append(f"`{name}`: spec has {w.tuple_arity} public tuple fields, crate {c.tuple_arity}")
            elif set(w.fields) != set(c.fields) or c.opaque:
                missing = sorted(set(c.fields) - set(w.fields))
                extra = sorted(set(w.fields) - set(c.fields))
                errors.append(f"`{name}` fields differ: missing in spec {missing}, not in crate {extra}"
                              + ("; the crate type is opaque" if c.opaque else ""))
        elif w.kind == "enum" and c.kind == "enum":
            if set(w.fields) != set(c.fields):
                missing = sorted(set(c.fields) - set(w.fields))
                extra = sorted(set(w.fields) - set(c.fields))
                errors.append(f"`{name}` variants differ: missing in spec {missing}, not in crate {extra}")
        elif c.kind != "type":
            errors.append(f"`{name}` is a {w.kind} in the spec but a {c.kind} in the crate")

    # 2. Completeness: every public crate type is written.
    for name, c in sorted(crate.items()):
        if c.kind in ("struct", "enum") and name not in written:
            errors.append(f"crate type `{name}` is not written in any ```rust block of the spec")

    # 3. Backticked member paths resolve.
    for m in re.finditer(r"`([^`\n]+)`", spec):
        for pm in re.finditer(rf"\b({IDENT})::({IDENT})\b", m.group(1)):
            owner, member = pm.group(1), pm.group(2)
            if owner in planned or owner not in crate:
                continue
            c = crate[owner]
            if member in c.fields or member in c.members:
                continue
            errors.append(f"`{owner}::{member}` names nothing on `{owner}`")

    # 4. Requirement ids are unique.
    seen: dict[str, int] = {}
    for rm in re.finditer(rf"\*\*\[({REQ_ID})\]", spec):
        seen[rm.group(1)] = seen.get(rm.group(1), 0) + 1
    errors += [f"requirement id {i} appears {n} times" for i, n in sorted(seen.items()) if n > 1]

    # 5. Every cited requirement id is defined. A bare id counts only with a
    # known prefix, so that names such as UTF-8 are not mistaken for
    # citations; a bracketed one counts whatever its prefix, so that a typo
    # such as [AAX-1] is caught.
    prefixes = {i.rsplit("-", 1)[0] for i in seen}
    for cm in re.finditer(rf"(\[)?\b({REQ_ID})\b(\])?", spec):
        rid, bracketed = cm.group(2), bool(cm.group(1) and cm.group(3))
        known = rid.rsplit("-", 1)[0] in prefixes
        if (known or bracketed) and rid not in seen:
            errors.append(f"requirement {rid} is cited but never defined")

    for e in dict.fromkeys(errors):
        print(e)
    if errors:
        print(f"\n{len(set(errors))} disagreement(s) between {spec_path.name} and the crate")
        return 1
    print(f"{spec_path.name} agrees with the crate: {len(written)} types, {len(seen)} requirements")
    return 0


if __name__ == "__main__":
    sys.exit(main())
