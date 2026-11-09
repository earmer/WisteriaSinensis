# Engine contract

The crate currently provides the public data model, coordinate transforms, input
and output validators, checked geometry queries, and contract tests. It does not yet
implement Unicode analysis, actual line fitting, boundary reshaping,
justification, ruby placement, clamping, rendering, or interactive paragraph
queries. `WordNode` has four variants: Box / Glue / Discretionary / Tab.

`layout_paragraph` validates the AST and returns an empty skeleton,
carrying the AST's writing mode and direction. With a geometry provider it also
checks the strut probe, a conservative probe of input that can begin at block
start (up to the first break opportunity), and bounded clearance progress. It
never treats a later item's height as a first-line requirement. This is a
provider contract check, not the future line-fitting solver.
A skeleton is intentionally not a fully positioned paragraph and cannot pass
`PositionedParagraph::validate`.

## Coordinates and metrics

All layout coordinates use inline `x` and block-progressing `y`. Horizontal maps
to `(x, y)`, VerticalLr to `(y, x)`, VerticalRl to `(total_block_size - y, x)`.
Physical coordinates are paragraph-local; external container placement is a host
translation. Rectangle transforms normalize transformed opposite corners.

Line-over offsets are distinct from block coordinates: positive over offsets
map to negative block deltas horizontally and in VerticalRl, positive deltas in
VerticalLr. Use `WritingMode::over_to_block` at this boundary. A line's block
bounds are `[baseline - ascent, baseline + descent]`, except VerticalLr uses
`[baseline - descent, baseline + ascent]`. Thus both vertical modes place over
annotations physically to the right. Validators use the same boundary helper.

Raw font metrics require all 15 floating-point fields finite. Ascent, descent,
gap, their vertical equivalents, underline thickness, x-height and cap-height are
nonnegative. Horizontal and vertical ascent/descent sums are positive; derived
heights must remain finite. Baseline offsets are scaled layout units, positive
toward line-over. Font bridges supply missing vertical and BASE metrics;
`NoopShaper` remains a deliberately non-rendering zero-metric test double.

`LineHeight::resolve_strut_for_mode` copies the active raw metrics into
ascent/descent/gap. Fixed height splits leading equally, including negative
leading; it does not mutate raw metrics. `resolve_strut` defaults to horizontal.

Vertical upright and TCY orientations require even resolved levels; horizontal
orientation is forbidden vertically and vertical orientations horizontally.
Atoms and clusters must match their owning shaping run. UAX analysis and glyph
rotation/placement are future frontend/rendering responsibilities; accepting an
AST does not certify that it was produced by a conforming UAX implementation.

## Public inputs and frontend compilation

The AST is untrusted. Public structs are not `#[non_exhaustive]` and source
compatibility across field additions is not promised.

Future analysis order is graphemes, orientation, explicit-span Bidi, script and
locale, run splitting, shaping/spacing/ruby expansion, then break compilation.
Run splits include orientation, level, style, script and locale. TCY is one
cluster per text box; soft breaks between same-run TCY boxes are rejected.
Cluster boundaries are indivisible layout boundaries; editing atomicity remains
independent through `AtomicEditSpan`.

Run spacing and explicit baseline offsets must be finite. Nonzero letter spacing
is forbidden on runs containing `NO_LETTER_SPACING` atoms. Final text/glue widths
must be finite and nonnegative; negative spacing increments are allowed. Raw
shaping advances remain `RawShapedGlyph.x_advance`; `natural_width` is the final
layout advance, including spacing and ruby expansion. No raw/layout equality is
assumed for TCY.

Soft break `no_break_width == no_break_spacing`; hard breaks require both zero.
Cloned padding contributes `clone_pre = sum(end_padding)` and
`clone_post = sum(start_padding)`. Direct, hard, and unrepeated math breaks have
pre/post widths equal to these sums; punctuation and hyphenation carry their own
net deltas beyond those sums. Each break action is validated as well.

Explicit Bidi spans have unique IDs, nonempty concrete-boundary-aligned ranges,
and proper nesting ordered by ascending start / descending end. Equal ranges
are rejected; source nesting is capped at 63 independently of resolved levels,
which can reach 126. The validator does not rerun UAX #9.

Annotations have unique IDs, nonempty atom- and cluster-aligned text-only bases,
and nonoverlapping ranges in each side's ordered subsequence. Discretionaries,
glue, tabs and objects cannot occur inside a base. Opposite-side overlap is
allowed. Annotation text is nonempty, contains no line or paragraph separator,
and is partitioned exactly by local atoms/clusters. Run, font, scale, flags,
orientation, levels and safety checks share the body validation routines.
Emphasis requires one source character and one cluster. Ruby overhang and all
annotation block extents are finite and nonnegative.

Future ruby compilation uses indivisible grouped spans. Base capacity is
`max(base_width, ruby_width - 2 * max_overhang)`; expansion distributes in
proportion to raw cluster widths, equally when the raw total is zero. Opposite
sides use the maximum width requirement, not their sum. Placement must keep
ruby within its slot even where base-relative overhang is allowed. Emphasis
repeats over eligible non-space, non-opening/closing-punctuation atoms.

The input validator uses constant linear scans and boundary indexes. With
standard randomized hash maps/sets, time is expected O(total input records and
bytes), with O(input boundaries + spans) extra memory. It neither sorts incoming
spans nor copies annotation payloads to recursively validate a temporary AST.

## Geometry and future solving

Slots have finite starts, ends strictly beyond starts, and finite widths except
that positive-infinite ends/capacities explicitly represent no wrapping. Finite
endpoint subtraction must not overflow. Slots are ascending and disjoint.
Visual content coordinates, advances and dimensions always remain finite.
Finite and infinite capacities never compare approximately equal.

Unobstructed width is the original container width, before indentation. Both
indents must be finite; without a custom provider each must be below wrap width.
Negative indentation is allowed. Default geometry is one indented slot.

Each provider response is validated at query time. For a fixed line index and
block start, an expanded query envelope must return slots each contained in an
old slot, with the same unobstructed reference width. An empty *strut* response
requires a finite forward feature offset; clearance progress below 1e-4 layout
units or more than 1024 attempts fails with `InvalidLineGeometry`. An expanded
probe that happens to be empty is a contract datapoint, not a rejection of the
initial position, because the taller envelope is not known to occur there.

The future fitter keeps a nondecreasing envelope and never shrinks a committed
line height. **Monotone available space alone does not prove that two iterations
suffice**: strategy changes, slot fragmentation, and reshaping can change the
chosen entity set. A bounded implementation must return an explicit failure or
use a specified fallback; it must not silently clip source to force convergence.
No two-iteration convergence claim is made by the current implementation.

Future multi-slot fitting must establish legal break boundaries at obstacle
crossings. Slot storage is always inline-ascending; logical consumption follows
base direction. Justification fits each occupied slot independently. Whole-line
Bidi reordering and subsequent slot assignment must still prove that reordered
indivisible clusters fit their assigned slots: a logical capacity partition is
not by itself such a proof. These algorithms are not implemented yet.

Tabs use the flow-space origin as the stop reference. Actual obstacle-edge tab
consumption and slot transitions belong to the future fitter.

## Positioned output

Every slot reports its start, content advance and capacity. Their content widths
sum to line visual width. Nonempty slot output must match the context and final
geometry snapshot. Legacy empty slot output is normalized only for a single
context slot starting at zero. Snapshots are validated directly; paragraph
validation never requeries a provider.

Body glyph advances, objects, tabs, decoration fragments and atomic fragments
must each fit wholly in one active slot interval. Capacity overflow is allowed
only for a single slot at full unobstructed width and one AllowOverflow object
or indivisible cluster, optionally with a synthesized clamp suffix. Clip and
ScaleToFit objects do not receive that exemption. Synthesized-only ellipses also
receive the narrow-column exemption.

Hanging extents must equal actual outer overflow and be permitted by HangMode.
Unused slots and internal obstacles cannot be occupied by hanging or preserved
trailing whitespace. Zero-advance marks retain the existing advance-geometry
exemption; ink extents are not available in this API.

Synthesized iff source byte range is absent. Body source ranges belong to the
line. Annotation glyphs are synthesized, finite, and attached to a nonempty
visible base in exactly one slot. Every non-synthesized body glyph intersecting
the base must lie wholly inside it and in that slot; at least one such glyph is
required. Glyph-less source nodes (notably glue and zero-width source bytes) may
sit inside the base, so contiguous byte coverage is not required: the base
envelope is the union of the intersecting body glyph intervals. Annotation
advance intervals remain within both the base-relative overhang allowance and
the active slot. Positioned annotations include `block_extent`, since a
standalone output validator otherwise cannot verify declared annotation height.
Their side's line extent must include the strut plus that declaration.

An ordinary empty line has zero content and no visual entities or ellipsis.
The explicit clamp exception permits an empty byte range with only synthesized
ellipsis output, or no glyphs for an empty suffix. This avoids falsely consuming
an invisible source atom and preserves `truncated_range = 0..text.len()`.

Paragraph `validate(text, options, context)` checks nonempty lines, per-line
geometry, forward/nonoverlapping block bounds, finite total block size, UTF-8
coverage from zero, adjacent range continuity, and the exact remaining tail.
Unclamped output ends at text length with no ellipsized lines. Clamped output
requires a valid clamp policy, a nonempty tail starting at the final line end,
and exactly the final line ellipsized. `validate_geometry(context)` is available
for geometry-only fixtures; it does not certify source coverage. The
`base_direction` and `size` fields are informational; `total_block_size` is the
coordinate-transform authority.

Future truncation respects cluster, TCY, atomic-edit and ruby boundaries; a
multi-glyph suffix is one synthesized sequence. At least one line is retained
even when the block limit is smaller than that line. Line count includes hard
break empty lines, but not clearance jumps. VerticalLr block-limit measurement
uses the same mode-correct line bounds as validation.

## Interaction boundary

Physical arrow requests resolve through `Motion::flow_direction` to inline
previous/next or block previous/next. Logical/word requests remain distinct.
The actual hit-test, caret, selection and navigation queries remain future
work. They must transform at their physical boundary, locate nonoverlapping
flow line boxes, snap obstacle hits to the nearer caret (ties toward positive
inline), keep selections split by slot, and carry preferred inline position
across block motion. Ruby hits map to bases; ellipsis hits and hidden-tail caret
requests map upstream to the truncation boundary. TCY editing is atomic only
when independently marked as an atomic editing span.

## Verification

The contract tests in `tests/contracts/` cover writing modes and orientation,
font metrics, explicit spacing, bidi spans, annotations, slots, line geometry,
clamping and truncation, plus empty/synthesized-only clamping, infinity mismatch,
malformed provider intervals, absent clearance progress, glyph-less annotation
bases, late tall input outside the initial probe, and asymmetric VerticalLr
paragraph bounds (including an under-side annotation). The manual
release-mode scaling benchmark is `#[ignore]`d and can be run explicitly.
