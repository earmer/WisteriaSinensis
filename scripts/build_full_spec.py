#!/usr/bin/env python3
"""Composes one self-contained Markdown file from the specification sources.

Sources
-------
* SPECIFICATION.md : the normative contract (sections 0 to 7).
* RESEARCH.md      : the informative evidence; its sections carry the
                     specification's numbers.
* Generated        : a table of contents, a requirement index and a type index.

Composition
-----------
Every specification section is kept verbatim. The matching research text is
placed after the section (or sub-section) it explains, under a heading named
"Background and evidence (informative)", so that normative and informative
text never mix. Research sections without a specification counterpart (the
design history, the open questions, the references) become appendices.

The script then proves that nothing was lost: every non-blank content line of
both sources must occur in the output, every requirement id and every fenced
block must survive, and no dangling reference to the two source files may
remain. A failed check exits with status 1 and writes nothing.

Standard library only. usage: scripts/build_full_spec.py [-o OUTPUT]
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC_PATH = ROOT / "SPECIFICATION.md"
RESEARCH_PATH = ROOT / "RESEARCH.md"
DEFAULT_OUT = ROOT / "dist" / "FULL_SPECIFICATION.md"

IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
REQ_ID = r"[A-Z][A-Z0-9]*-\d+"
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")

BACKGROUND = "Background and evidence (informative)"
BACKGROUND_NOTE = "*Informative. Not part of the contract; where this disagrees with the text above, the text above wins.*"

APPENDICES = {
    "History": "Appendix A. Design history",
    "Open questions and unverified items": "Appendix B. Open questions and unverified items",
    "References": "Appendix C. References",
}
REQ_INDEX_TITLE = "Appendix D. Requirement index"
TYPE_INDEX_TITLE = "Appendix E. Type index"


class BuildError(Exception):
    pass


# --------------------------------------------------------------------------
# Markdown structure
# --------------------------------------------------------------------------


def code_flags(lines: list[str]) -> list[bool]:
    """True for every line inside a fenced block, delimiters included."""
    flags: list[bool] = []
    opened: tuple[str, int] | None = None
    for line in lines:
        m = FENCE.match(line)
        if opened is None:
            if m:
                opened = (m.group(1)[0], len(m.group(1)))
            flags.append(opened is not None)
        else:
            flags.append(True)
            if m and m.group(1)[0] == opened[0] and len(m.group(1)) >= opened[1] and not m.group(2).strip():
                opened = None
    if opened is not None:
        raise BuildError("a fenced block is never closed")
    return flags


@dataclass
class Sub:
    title: str
    lines: list[str] = field(default_factory=list)


@dataclass
class Section:
    title: str
    intro: list[str] = field(default_factory=list)
    subs: list[Sub] = field(default_factory=list)


@dataclass
class Doc:
    title: str
    preamble: list[str]
    sections: list[Section]


def trim(lines: list[str]) -> list[str]:
    """Drops blank lines and horizontal rules at both ends."""
    lines = list(lines)
    while lines and lines[0].strip() in ("", "---"):
        lines.pop(0)
    while lines and lines[-1].strip() in ("", "---"):
        lines.pop()
    return lines


def parse(text: str) -> Doc:
    """Splits a document at its level-2 and level-3 headings (fence-aware)."""
    lines = text.rstrip("\n").split("\n")
    flags = code_flags(lines)
    title = ""
    preamble: list[str] = []
    sections: list[Section] = []
    target = preamble
    for line, in_code in zip(lines, flags):
        m = None if in_code else HEADING.match(line)
        if m and len(m.group(1)) == 1 and not title:
            title = m.group(2)
        elif m and len(m.group(1)) == 2:
            sections.append(Section(m.group(2)))
            target = sections[-1].intro
        elif m and len(m.group(1)) == 3 and sections:
            sections[-1].subs.append(Sub(m.group(2)))
            target = sections[-1].subs[-1].lines
        else:
            target.append(line)
    for s in sections:
        s.intro = trim(s.intro)
        for sub in s.subs:
            sub.lines = trim(sub.lines)
    return Doc(title, trim(preamble), sections)


def key_of(title: str) -> str | None:
    """'1.' -> '1', '2.3 Shaped...' -> '2.3', 'Stage 4: ...' -> 'Stage 4'."""
    m = re.match(r"(\d+(?:\.\d+)*)\.?(?:\s|$)", title)
    if m:
        return m.group(1)
    m = re.match(r"(Step|Stage)\s+(\d+)\b", title)
    return f"{m.group(1)} {m.group(2)}" if m else None


def slugify(title: str) -> str:
    """GitHub's heading anchor, before de-duplication."""
    title = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", title)
    title = title.replace("`", "").replace("*", "").lower()
    return re.sub(r"[^\w\- ]", "", title).replace(" ", "-")


def scan_headings(text: str) -> list[tuple[int, str, str]]:
    """(level, title, anchor) of every heading outside fences, in order."""
    lines = text.split("\n")
    seen: Counter[str] = Counter()
    out = []
    for line, in_code in zip(lines, code_flags(lines)):
        m = None if in_code else HEADING.match(line)
        if m:
            slug = slugify(m.group(2))
            n = seen[slug]
            seen[slug] += 1
            out.append((len(m.group(1)), m.group(2), slug if n == 0 else f"{slug}-{n}"))
    return out


# --------------------------------------------------------------------------
# Wording that only made sense while the two files were separate
# --------------------------------------------------------------------------

SPEC_INTRO_OLD = (
    "Background, sources, design history and open questions are in [RESEARCH.md](RESEARCH.md), "
    "whose sections carry the same numbers as the sections here: the evidence for §*n* is in RESEARCH §*n*."
)
SPEC_INTRO_NEW = (
    "Background, sources, design history and open questions are part of this edition: "
    "the informative *Background and evidence* subsections follow the sections they concern, "
    "and Appendices A to C collect the design history, the open questions and the references."
)


def rewrite(text: str, exact: list[tuple[str, str]], loose: list[tuple[str, str]]) -> str:
    for old, new in exact:
        if old not in text:
            raise BuildError(f"expected wording not found, update the script: {old[:70]}...")
        text = text.replace(old, new)
    for pattern, new in loose:
        text = re.sub(pattern, new, text)
    return text


def load_sources() -> tuple[str, str]:
    spec = rewrite(
        SPEC_PATH.read_text(encoding="utf-8"),
        exact=[(SPEC_INTRO_OLD, SPEC_INTRO_NEW)],
        loose=[(r"\bRESEARCH §", "the background notes to §")],
    )
    research = rewrite(
        RESEARCH_PATH.read_text(encoding="utf-8"),
        exact=[],
        loose=[(r"\bSPECIFICATION §", "§"), (r"\bthe References section\b", "Appendix C"), (r"flagged in the References\.", "flagged in Appendix C.")],
    )
    return spec, research


# --------------------------------------------------------------------------
# Composition
# --------------------------------------------------------------------------


def git(*args: str) -> str:
    try:
        r = subprocess.run(["git", "-C", str(ROOT), *args], capture_output=True, text=True, check=True)
        return r.stdout.strip()
    except (OSError, subprocess.CalledProcessError):
        return ""


def provenance() -> str:
    rev, date = git("rev-parse", "--short", "HEAD"), git("log", "-1", "--format=%cs")
    dirty = bool(git("status", "--porcelain", "--", "SPECIFICATION.md", "RESEARCH.md"))
    where = f"revision `{rev}` ({date})" if rev else "an unknown revision"
    if dirty:
        where += " plus uncommitted changes"
    return (
        f"> Composed from `SPECIFICATION.md` (normative) and `RESEARCH.md` (informative) at {where} "
        "by `scripts/build_full_spec.py`. Do not edit this file: edit the sources and rebuild."
    )


def research_split(research: Doc) -> tuple[list[str], dict[str, Section], list[Section]]:
    """Labels preamble, numbered sections by key, and the appendix sections."""
    pre = research.preamble
    first_para = []
    for line in pre:
        if not line.strip():
            break
        first_para.append(line)
    if not first_para or not first_para[0].startswith("Informative companion to"):
        raise BuildError("RESEARCH.md no longer opens with its 'Informative companion' paragraph")
    labels = trim(pre[len(first_para):])

    numbered: dict[str, Section] = {}
    appendices: list[Section] = []
    for s in research.sections:
        k = key_of(s.title)
        if k is not None:
            numbered[k] = s
        elif s.title in APPENDICES:
            appendices.append(s)
        else:
            raise BuildError(f"RESEARCH.md section '{s.title}' is neither numbered nor a known appendix")
    missing = [t for t in APPENDICES if t not in {a.title for a in appendices}]
    if missing:
        raise BuildError(f"RESEARCH.md lacks the section(s): {missing}")
    appendices.sort(key=lambda a: list(APPENDICES).index(a.title))
    return labels, numbered, appendices


def background(level: int, lines: list[str]) -> list[str]:
    return [f"{'#' * level} {BACKGROUND}", "", BACKGROUND_NOTE, "", *lines, ""]


def emit_section(out: list[str], s: Section, heading: str | None = None) -> None:
    out += [f"## {heading or s.title}", ""]
    if s.intro:
        out += [*s.intro, ""]
    for sub in s.subs:
        out += [f"### {sub.title}", ""]
        if sub.lines:
            out += [*sub.lines, ""]


def assemble(spec: Doc, research: Doc, toc: str, req_index: str, type_index: str) -> str:
    labels, numbered, appendices = research_split(research)
    unused = dict(numbered)
    title = spec.title.rsplit(" — ", 1)[0] + " — Full Specification"

    out: list[str] = [f"# {title}", "", provenance(), "", "## Contents", "", toc, ""]
    out += ["## About this edition", "", *spec.preamble, ""]
    out += [
        "The text of Sections 0 to 7 is the normative contract. Where the research notes have something to say about a "
        f"section or sub-section, it follows that part under a heading *{BACKGROUND}*; none of it is normative. "
        "Appendices A to C reproduce the design history, the open questions and the references. Appendices D and E "
        "are generated indexes. If an informative passage disagrees with the specification, the specification wins.",
        "",
        *labels,
        "",
    ]

    for s in spec.sections:
        out += ["---", ""]
        r = unused.pop(key_of(s.title) or "", None) if key_of(s.title) else None
        out += [f"## {s.title}", ""]
        if s.intro:
            out += [*s.intro, ""]
        if r and r.intro:
            out += background(3, r.intro)
        research_subs = {key_of(x.title): x for x in (r.subs if r else []) if key_of(x.title)}
        for sub in s.subs:
            out += [f"### {sub.title}", ""]
            if sub.lines:
                out += [*sub.lines, ""]
            rs = research_subs.pop(key_of(sub.title), None) if key_of(sub.title) else None
            if rs:
                out += background(4, rs.lines)
        leftovers = [x for x in (r.subs if r else []) if x.title and key_of(x.title) in research_subs]
        leftovers += [x for x in (r.subs if r else []) if not key_of(x.title)]
        if leftovers:
            out += [f"### Further background (informative)", "", BACKGROUND_NOTE, ""]
            for x in leftovers:
                out += [f"#### Background: {x.title}", "", *x.lines, ""]

    if unused:
        raise BuildError(f"RESEARCH.md sections without a counterpart in the specification: {sorted(unused)}")

    for a in appendices:
        out += ["---", ""]
        emit_section(out, a, APPENDICES[a.title])
    out += ["---", "", f"## {REQ_INDEX_TITLE}", "", req_index, ""]
    out += ["---", "", f"## {TYPE_INDEX_TITLE}", "", type_index, ""]

    text = "\n".join(out)
    return re.sub(r"\n{3,}", "\n\n", text).rstrip("\n") + "\n"


# --------------------------------------------------------------------------
# Generated parts
# --------------------------------------------------------------------------


def make_toc(headings: list[tuple[int, str, str]]) -> str:
    rows = []
    for level, title, slug in headings:
        if level == 2 and title not in ("Contents",) or level == 3 and title != BACKGROUND and not title.startswith("Further"):
            rows.append(f"{'  ' * (level - 2)}- [{title}](#{slug})")
    return "\n".join(rows)


def spec_parts(spec: Doc) -> list[tuple[str, list[str]]]:
    parts = []
    for s in spec.sections:
        parts.append((s.title, s.intro))
        parts += [(sub.title, sub.lines) for sub in s.subs]
    return parts


def make_req_index(spec: Doc, anchors: dict[str, str]) -> str:
    rows = ["| Section | Requirements |", "| --- | --- |"]
    total = 0
    for title, lines in spec_parts(spec):
        ids = re.findall(rf"\*\*\[({REQ_ID})\]", "\n".join(lines))
        if ids:
            total += len(ids)
            cell = ", ".join(f"`{i}`" for i in ids)
            rows.append(f"| [{title.replace('|', chr(92) + '|')}](#{anchors[title]}) | {cell} |")
    return f"{total} requirements, in document order.\n\n" + "\n".join(rows)


def make_type_index(spec: Doc, anchors: dict[str, str]) -> str:
    found = []
    for title, lines in spec_parts(spec):
        flags = code_flags(lines)
        i = 0
        while i < len(lines):
            m = FENCE.match(lines[i]) if flags[i] else None
            if m and m.group(2).strip().startswith("rust"):
                planned = "planned" in m.group(2)
                j = i + 1
                while j < len(lines) and not (FENCE.match(lines[j]) and not FENCE.match(lines[j]).group(2).strip()):
                    d = re.match(rf"\s*pub\s+(struct|enum|trait|type)\s+({IDENT})", lines[j])
                    if d:
                        found.append((d.group(2), d.group(1), "planned" if planned else "implemented", title))
                    j += 1
                i = j
            i += 1
    rows = ["| Type | Kind | Status | Defined in |", "| --- | --- | --- | --- |"]
    for name, kind, status, title in sorted(found, key=lambda r: (r[0].lower(), r[3])):
        rows.append(f"| `{name}` | {kind} | {status} | [{title.replace('|', chr(92) + '|')}](#{anchors[title]}) |")
    return f"{len(found)} public types written in the specification's Rust blocks.\n\n" + "\n".join(rows)


# --------------------------------------------------------------------------
# Verification
# --------------------------------------------------------------------------


def content_lines(text: str) -> Counter[str]:
    """Every non-blank line that is not a heading or a rule, fenced lines included."""
    lines = text.rstrip("\n").split("\n")
    out: Counter[str] = Counter()
    for line, in_code in zip(lines, code_flags(lines)):
        if not line.strip() or (not in_code and (HEADING.match(line) or line.strip() == "---")):
            continue
        out[line.rstrip()] += 1
    return out


def fenced_blocks(text: str) -> Counter[str]:
    lines = text.rstrip("\n").split("\n")
    blocks: Counter[str] = Counter()
    cur: list[str] = []
    prev = False
    for line, in_code in zip(lines, code_flags(lines)):
        if in_code:
            cur.append(line)
        if prev and not in_code and cur:
            blocks["\n".join(cur)] += 1
            cur = []
        prev = in_code
    if cur:
        blocks["\n".join(cur)] += 1
    return blocks


def verify(spec_text: str, research_text: str, research: Doc, result: str) -> None:
    problems: list[str] = []

    # The research text minus its opening paragraph, which the new front matter replaces.
    _, _, _ = research_split(research)
    dropped = research.preamble[: next((i for i, l in enumerate(research.preamble) if not l.strip()), len(research.preamble))]
    expected = content_lines(spec_text) + content_lines(research_text)
    for line in dropped:
        expected[line.rstrip()] -= 1
    have = content_lines(result)
    lost = expected - have
    if lost:
        problems.append(f"{sum(lost.values())} source line(s) missing from the output, e.g. {next(iter(lost))[:90]!r}")

    ids_src = Counter(re.findall(rf"\*\*\[({REQ_ID})\]", spec_text))
    ids_out = Counter(re.findall(rf"\*\*\[({REQ_ID})\]", result))
    if ids_src != ids_out:
        problems.append(f"requirement ids differ: {sorted((ids_src - ids_out) + (ids_out - ids_src))[:5]}")
    dup = [i for i, n in ids_src.items() if n > 1]
    if dup:
        problems.append(f"requirement ids defined more than once in the source: {dup[:5]}")

    blocks_src = fenced_blocks(spec_text) + fenced_blocks(research_text)
    if blocks_src != fenced_blocks(result):
        problems.append("fenced blocks differ between the sources and the output")

    body = "\n".join(l for l in result.split("\n") if not l.startswith("> Composed from"))
    left = re.findall(r"RESEARCH(?:\.md)?|SPECIFICATION\.md", body)
    if left:
        problems.append(f"the output still refers to a separate source file ({left[0]}); add a rewrite")

    anchors = [a for _, _, a in scan_headings(result)]
    for target in re.findall(r"\]\(#([^)]+)\)", result):
        if target not in anchors:
            problems.append(f"link to a missing anchor: #{target}")
            break

    if problems:
        raise BuildError("verification failed:\n  - " + "\n  - ".join(problems))


# --------------------------------------------------------------------------


def build() -> tuple[str, dict[str, int]]:
    spec_text, research_text = load_sources()
    spec, research = parse(spec_text), parse(research_text)

    # Pass 1 fixes the heading structure, pass 2 fills the generated parts.
    draft = assemble(spec, research, "", "", "")
    anchors = {title: slug for _, title, slug in scan_headings(draft)}
    toc = make_toc(scan_headings(draft))
    text = assemble(spec, research, toc, make_req_index(spec, anchors), make_type_index(spec, anchors))
    if [h[:2] for h in scan_headings(text)] != [h[:2] for h in scan_headings(draft)]:
        raise BuildError("the generated parts changed the heading structure")

    verify(spec_text, research_text, research, text)
    stats = {
        "sections": len(spec.sections),
        "requirements": len(re.findall(rf"\*\*\[{REQ_ID}\]", spec_text)),
        "lines": text.count("\n"),
    }
    return text, stats


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("-o", "--output", type=Path, default=DEFAULT_OUT)
    args = ap.parse_args()
    try:
        text, stats = build()
    except BuildError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(text, encoding="utf-8", newline="\n")
    print(
        f"wrote {args.output}: {stats['lines']} lines, {stats['sections']} specification sections, "
        f"{stats['requirements']} requirements"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
