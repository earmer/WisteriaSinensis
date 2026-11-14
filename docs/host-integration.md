# Host integration notes

These notes are **not normative**. The rules a renderer must follow to draw
validated output correctly are in `SPECIFICATION.md` §8 (HST-1 to HST-8).
What follows is integration practice that depends on the host: an editor, a
document viewer, an exporter, a test rasteriser. Nothing here makes a host
conforming or non-conforming.

## Sharing fonts

Build one font registry, implement `FontSource` on it, and give it to both the
shaper (layout thread) and the renderer (render thread). The render thread then
needs no access to the shaper.

Load large fonts by memory-mapping the file and wrapping the map with
`FontBytes::new`: the bytes stay in the shared, read-only pages of the map.
Embedded fonts use `FontBytes::from_static`.

A long-running host that opens and closes documents with embedded fonts can
unload a face once none of its cached output names it (HST-2). A host that does
not track its output can simply keep every face for the session.

## Drawing order

The engine reports geometry; the order of layers is the host's design. Common
practice within one paragraph, which hosts adjust to their own styling:

- selection and IME target-clause backgrounds below the glyphs;
- underlines below or above the glyphs depending on the style, strike-through
  above them;
- inline-object content in its `PositionedInlineBox` rectangle, with any
  selection overlay drawn above it;
- composition underline and caret on top.

## Caching

A `PositionedParagraph` is immutable and self-contained, with paragraph-local
coordinates, so it is a natural unit to cache. A host that pans or zooms large
amounts of text can keep each unchanged paragraph as a retained draw object and
move it by transform, re-laying out a paragraph only when its text, style or
width changes. Versioning, spatial indexing and viewport culling belong to the
host. A headless rasteriser or an exporter can draw each paragraph once.

## Continuous zoom

HST-5 and HST-6 fix what may change under scale (nothing in the geometry), not
how outlines are rasterised. Options include bitmap atlases bucketed by scale
(for example powers of two, with mipmapping between buckets), signed distance
fields, or direct outline rendering with stem darkening at small sizes. Each
has trade-offs for CJK fonts with tens of thousands of glyphs, notably
distance-field artefacts at dense stroke junctions and the cost of generating
fields for arbitrary user fonts at run time.

## Input methods and accessibility

The §6 queries return rectangles in paragraph-local physical coordinates
(HST-8). A host passes the platform the rectangle after applying, in order, the
paragraph's position in its container and the view transform. Some platform
APIs accept the transform itself (for example an editable region's size and
transform), and take the local rectangles directly. A composition range that
wraps gives several rectangles.

Platforms that address text by UTF-16 offsets across a whole editable region
need offsets that span paragraphs; that conversion belongs to whatever layer
owns the paragraph sequence.
