"""Builds `WisteriaVarTest.ttf`, the synthetic variable font of the
variable-font tests. See `README.md` beside this file for what it contains
and what the tests rely on.

Run from this directory with fontTools (4.60 or later):

    python3 -I build_variable_test_font.py

The output is deterministic: the same fontTools version writes the same
bytes.
"""

from __future__ import annotations

import sys
from io import BytesIO
from pathlib import Path

from fontTools.designspaceLib import (
    AxisDescriptor,
    DesignSpaceDocument,
    RuleDescriptor,
    SourceDescriptor,
)
from fontTools.feaLib.builder import addOpenTypeFeaturesFromString
from fontTools.fontBuilder import FontBuilder
from fontTools.pens.ttGlyphPen import TTGlyphPen
from fontTools.ttLib import TTFont
from fontTools.varLib import build as build_variable_font

OUT = Path(__file__).with_name("WisteriaVarTest.ttf")
UPEM = 1000
GLYPHS = [".notdef", "space", "H", "a", "a.serif", "f", "i", "f_i", "o", "x"]
CMAP = {0x20: "space", 0x48: "H", 0x61: "a", 0x66: "f", 0x69: "i", 0x6F: "o", 0x78: "x"}
# Default advances; a master scales them by `width` and adds `extra`.
ADVANCE = {
    ".notdef": 500, "space": 250, "H": 700, "a": 500, "a.serif": 560,
    "f": 300, "i": 250, "f_i": 500, "o": 520, "x": 480,
}
# The ink top of each glyph, from the baseline.
TOP = {"H": 700, "a": 500, "a.serif": 500, "f": 720, "i": 700, "f_i": 720, "o": 500, "x": 500}

# Line metrics of the default master. hhea repeats OS/2 typo, since MVAR's
# `hasc`, `hdsc` and `hlgp` vary both.
DEFAULT_METRICS = {
    "ascender": 800, "descender": -200, "line_gap": 0,
    "win_ascent": 900, "win_descent": 300,
    "underline_position": -100, "underline_thickness": 50,
    "strikeout_position": 300, "strikeout_size": 50,
    "x_height": 500, "cap_height": 700,
    "caret": 250,
}


class Master:
    """One source of the designspace: a location, an advance change and the
    metrics it overrides."""

    def __init__(self, name, location, width=1.0, extra=0, serif=0, **metrics):
        self.name = name
        self.location = location
        self.width = width
        self.extra = extra
        self.serif = serif
        self.metrics = {**DEFAULT_METRICS, **metrics}

    def advance(self, glyph: str) -> int:
        if glyph == "space":
            return round(ADVANCE[glyph] * self.width)
        return round(ADVANCE[glyph] * self.width) + self.extra


MASTERS = [
    Master("Regular", {"wght": 400, "wdth": 100, "SERF": 0}),
    Master(
        "Black", {"wght": 900, "wdth": 100, "SERF": 0}, extra=100,
        ascender=880, descender=-240, line_gap=20, underline_thickness=90,
        x_height=520, caret=300,
    ),
    Master(
        "Thin", {"wght": 100, "wdth": 100, "SERF": 0}, extra=-50,
        ascender=780, underline_thickness=30,
    ),
    Master("Expanded", {"wght": 400, "wdth": 125, "SERF": 0}, width=1.25),
    Master("Condensed", {"wght": 400, "wdth": 75, "SERF": 0}, width=0.75),
    Master(
        "Serif", {"wght": 400, "wdth": 100, "SERF": 100}, extra=40, serif=60,
        ascender=830, descender=-230, underline_position=-130,
    ),
    # A corner master: its ascender is 40 units above the sum of the Black
    # and Serif deltas, so MVAR needs a two-axis region to reach it.
    Master(
        "Black Serif", {"wght": 900, "wdth": 100, "SERF": 100}, extra=140, serif=60,
        ascender=950, descender=-270, line_gap=20, underline_position=-130,
        underline_thickness=90, x_height=520, caret=300,
    ),
]


def outline(glyph: str, master: Master):
    """A stem and, for the serif axis, a foot whose width grows from 0."""
    pen = TTGlyphPen(None)
    if glyph in ("space",):
        return pen.glyph()
    advance = master.advance(glyph)
    top = TOP.get(glyph, 700)
    left, right = 50, max(advance - 50, 60)
    pen.moveTo((left, 0))
    pen.lineTo((left, top))
    pen.lineTo((right, top))
    pen.lineTo((right, 0))
    pen.closePath()
    # The serif foot: present in every master so the outlines stay
    # compatible, zero-width where `serif` is 0.
    pen.moveTo((left - master.serif // 2, 0))
    pen.lineTo((left - master.serif // 2, 20))
    pen.lineTo((right + master.serif // 2, 20))
    pen.lineTo((right + master.serif // 2, 0))
    pen.closePath()
    return pen.glyph()


FEATURES = """
languagesystem DFLT dflt;
languagesystem latn dflt;
feature liga { sub f i by f_i; } liga;
table GDEF {
  GlyphClassDef [H a a.serif f i o x], [f_i], , ;
  LigatureCaretByPos f_i CARET;
} GDEF;
"""


def build_master(master: Master) -> TTFont:
    m = master.metrics
    fb = FontBuilder(UPEM, isTTF=True)
    fb.setupGlyphOrder(GLYPHS)
    fb.setupCharacterMap(CMAP)
    fb.setupGlyf({g: outline(g, master) for g in GLYPHS})
    fb.setupHorizontalMetrics({g: (master.advance(g), 50) for g in GLYPHS})
    fb.setupHorizontalHeader(
        ascent=m["ascender"], descent=m["descender"], lineGap=m["line_gap"]
    )
    fb.setupNameTable({"familyName": "Wisteria Var Test", "styleName": master.name})
    fb.setupOS2(
        sTypoAscender=m["ascender"], sTypoDescender=m["descender"],
        sTypoLineGap=m["line_gap"],
        usWinAscent=m["win_ascent"], usWinDescent=m["win_descent"],
        sxHeight=m["x_height"], sCapHeight=m["cap_height"],
        yStrikeoutPosition=m["strikeout_position"], yStrikeoutSize=m["strikeout_size"],
        fsSelection=0x80 | 0x40,  # USE_TYPO_METRICS, REGULAR
        achVendID="NONE", version=4,
    )
    fb.setupPost(
        underlinePosition=m["underline_position"],
        underlineThickness=m["underline_thickness"],
    )
    addOpenTypeFeaturesFromString(fb.font, FEATURES.replace("CARET", str(m["caret"])))
    buffer = BytesIO()
    fb.save(buffer)
    buffer.seek(0)
    return TTFont(buffer)


def designspace() -> DesignSpaceDocument:
    doc = DesignSpaceDocument()
    for tag, name, minimum, default, maximum in [
        ("wght", "Weight", 100, 400, 900),
        ("wdth", "Width", 75, 100, 125),
        ("SERF", "Serif", 0, 0, 100),
    ]:
        axis = AxisDescriptor()
        axis.tag, axis.name = tag, name
        axis.minimum, axis.default, axis.maximum = minimum, default, maximum
        if tag == "wght":
            # avar: the upper half of the axis is eased, so wght 650 sits a
            # quarter of the way to the Black master, not half.
            axis.map = [(100, 100), (400, 400), (650, 525), (900, 900)]
        doc.addAxis(axis)
    for master in MASTERS:
        source = SourceDescriptor()
        source.font = build_master(master)
        source.name = master.name
        source.location = {
            {"wght": "Weight", "wdth": "Width", "SERF": "Serif"}[k]: v
            for k, v in master.location.items()
        }
        doc.addSource(source)
    # A structural change: from Serif 50 the `a` is a different glyph.
    rule = RuleDescriptor()
    rule.name = "serif-a"
    rule.conditionSets = [[{"name": "Serif", "minimum": 50, "maximum": 100}]]
    rule.subs = [("a", "a.serif")]
    doc.addRule(rule)
    return doc


def main() -> int:
    font, _, _ = build_variable_font(designspace(), exclude=["STAT"])
    # A fixed date, 2026-11-19, in seconds since 1904, so a rebuild writes
    # the same bytes.
    font["head"].created = font["head"].modified = 3846441600
    font.recalcTimestamp = False
    font.save(OUT)
    print(f"wrote {OUT.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
