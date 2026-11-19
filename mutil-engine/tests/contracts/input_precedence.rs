//! G1 multi-fault precedence (AST-11): the input gate returns the first
//! violation. The specification fixes no order between unrelated checks, so
//! these tests pin the order the gate documents for its phases: policy
//! (constraints, options, strut), paragraph separators, storage,
//! presentation, bidi and annotation spans, shaping runs, decoration and
//! atomic spans, item extents, item order, per-item errors, the paragraph end,
//! span boundary order, span enclosure, and source coverage. Within storage,
//! run tiling (`InvalidRunStorage`) precedes `RunIndexOutOfRange`, as in the
//! AST-12 table. Per-item errors include a boundary's own width and level.
//!
//! Each test builds a paragraph with exactly two faults and asserts that the
//! error of the earlier phase wins; it also checks that each fault alone
//! raises its own error, so a fixture that stops producing its fault fails
//! loudly instead of passing vacuously.

use crate::common::*;
use mutil_engine::*;

struct Case {
    ast: ParagraphAst,
    text: String,
    constraints: LayoutConstraints,
    options: LayoutOptions,
}

type Fault = fn(&mut Case);

/// "abcd" as two text boxes `0..2` and `2..4` and the sentinel.
fn base() -> Case {
    let text = "abcd";
    let mut ast = ast(text);
    rebuild_items(&mut ast, |a| vec![boxed(a, 0, 2), boxed(a, 2, 4), end(4)]);
    Case {
        ast,
        text: text.into(),
        constraints: constraints(),
        options: LayoutOptions::default(),
    }
}

impl Case {
    fn with(faults: &[Fault]) -> Self {
        let mut case = base();
        for fault in faults {
            fault(&mut case);
        }
        case
    }

    fn run(&self) -> Result<(), LayoutInputError> {
        check_layout_inputs(&self.ast, &self.text, &self.constraints, &self.options)
    }
}

/// Asserts that `early` wins over `late` and that each fault alone raises its
/// own error. `early_first` is the order the faults are applied in, for the
/// faults that rewrite the items.
fn precedence(
    early: (Fault, LayoutInputError),
    late: (Fault, LayoutInputError),
    early_first: bool,
) {
    assert_eq!(Case::with(&[]).run(), Ok(()), "the base is valid");
    assert_eq!(
        Case::with(&[early.0]).run(),
        Err(early.1.clone()),
        "early alone"
    );
    assert_eq!(Case::with(&[late.0]).run(), Err(late.1), "late alone");
    let both = if early_first {
        [early.0, late.0]
    } else {
        [late.0, early.0]
    };
    assert_eq!(Case::with(&both).run(), Err(early.1), "both faults");
}

fn bidi_span(id: u64, start: usize, end: usize) -> ExplicitBidiSpan {
    ExplicitBidiSpan {
        span_id: id,
        byte_range: TextRange::new(start, end),
        control: BidiControlKind::Isolate(None),
    }
}

fn boundary(span_id: u64, offset: usize, edge: SpanEdge, width: f32) -> WordNode {
    WordNode::Box(BoxNode::SpanBoundary {
        byte_range: TextRange::empty_at(offset),
        span_id,
        edge,
        width,
        bidi_level: BidiLevel::LTR,
    })
}

fn decoration(span_id: u64, start: usize, end: usize, start_padding: f32) -> InlineDecorationSpan {
    InlineDecorationSpan {
        span_id,
        byte_range: TextRange::new(start, end),
        start_padding,
        end_padding: 0.0,
        break_mode: BoxDecorationBreak::Slice,
    }
}

// --- policy ---------------------------------------------------------------

fn bad_constraints(c: &mut Case) {
    c.constraints.wrap_width = 0.0;
}

fn bad_options(c: &mut Case) {
    c.options.clamp = Some(LineClampPolicy {
        max_lines: Some(0),
        max_block_size: None,
        ellipsis: "".into(),
    });
}

fn bad_strut(c: &mut Case) {
    c.ast.strut_font_size = 0.0;
}

// --- text and storage -----------------------------------------------------

fn separator(c: &mut Case) {
    c.text = "abc\n".into();
}

fn run_index_out_of_range(c: &mut Case) {
    if let WordNode::Box(BoxNode::Text(TextBox { run_index, .. })) = &mut c.ast.items[0] {
        *run_index = 5;
    }
}

fn run_storage(c: &mut Case) {
    c.ast.atoms.push(atom(3));
}

fn orientation(c: &mut Case) {
    c.ast.runs[0].orientation = GlyphOrientation::VerticalUpright;
}

fn bad_bidi_span(c: &mut Case) {
    c.ast.bidi_spans = vec![bidi_span(1, 0, 9)];
}

fn bad_annotation(c: &mut Case) {
    let mut span = annotation(TextRange::new(0, 4), AnnotationSide::Over);
    span.clusters.clear();
    c.ast.annotations = vec![span];
}

fn bad_shaping_run(c: &mut Case) {
    c.ast.runs[0].font_size = 0.0;
}

fn bad_atomic_span(c: &mut Case) {
    c.ast.atomic_spans = vec![AtomicEditSpan {
        span_id: 1,
        byte_range: TextRange::new(0, 9),
    }];
}

/// A decoration span over the whole text, whose start padding is not a
/// measurement, with correct boundaries.
fn bad_decoration_span(c: &mut Case) {
    c.ast.decoration_spans = vec![decoration(0, 0, 4, f32::NAN)];
    c.ast
        .items
        .insert(0, boundary(0, 0, SpanEdge::Leading, 0.0));
    let at = c.ast.items.len() - 1;
    c.ast
        .items
        .insert(at, boundary(0, 4, SpanEdge::Trailing, 0.0));
}

// --- items ----------------------------------------------------------------

fn item_outside_text(c: &mut Case) {
    if let WordNode::Box(BoxNode::Text(TextBox { byte_range, .. })) = &mut c.ast.items[0] {
        *byte_range = TextRange::new(0, 9);
    }
}

fn items_out_of_order(c: &mut Case) {
    c.ast
        .items
        .insert(1, WordNode::Discretionary(DiscretionaryNode::direct(1)));
}

fn bad_item(c: &mut Case) {
    c.ast.clusters[0].font.scale = f32::NAN;
}

fn missing_paragraph_end(c: &mut Case) {
    c.ast.items.pop();
}

/// Two nested decoration spans over `0..4` whose trailing boundaries are in
/// the wrong order (AST-8: LIFO). If the items already hold two trailing
/// boundaries, only swaps them; otherwise builds the spans around the boxes.
fn bad_span_order(c: &mut Case) {
    let trailing: Vec<usize> = c
        .ast
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, n)| match n {
            WordNode::Box(BoxNode::SpanBoundary {
                edge: SpanEdge::Trailing,
                ..
            }) => Some(i),
            _ => None,
        })
        .collect();
    if let [first, second] = trailing[..] {
        c.ast.items.swap(first, second);
        return;
    }
    c.ast.decoration_spans = vec![decoration(0, 0, 4, 0.0), decoration(1, 0, 4, 0.0)];
    let at = c.ast.items.len() - 1;
    c.ast
        .items
        .insert(at, boundary(0, 4, SpanEdge::Trailing, 0.0));
    c.ast
        .items
        .insert(at + 1, boundary(1, 4, SpanEdge::Trailing, 0.0));
    c.ast
        .items
        .insert(0, boundary(1, 0, SpanEdge::Leading, 0.0));
    c.ast
        .items
        .insert(0, boundary(0, 0, SpanEdge::Leading, 0.0));
}

/// Two nested spans over `0..2`; the break opportunity at their end sits
/// before the trailing boundaries instead of after them (AST-8).
fn span_enclosure(c: &mut Case) {
    c.ast.decoration_spans = vec![decoration(0, 0, 2, 0.0), decoration(1, 0, 2, 0.0)];
    let mut items = std::mem::take(&mut c.ast.items);
    let sentinel = items.pop().unwrap();
    let second = items.pop().unwrap();
    let first = items.pop().unwrap();
    c.ast.items = vec![
        boundary(0, 0, SpanEdge::Leading, 0.0),
        boundary(1, 0, SpanEdge::Leading, 0.0),
        first,
        WordNode::Discretionary(DiscretionaryNode::direct(2)),
        boundary(1, 2, SpanEdge::Trailing, 0.0),
        boundary(0, 2, SpanEdge::Trailing, 0.0),
        second,
        sentinel,
    ];
}

/// Shrinks the last text box to its first byte: its other byte is owned by
/// nobody, while the storage still tiles.
fn uncovered_source(c: &mut Case) {
    let last = c
        .ast
        .items
        .iter()
        .rposition(|i| matches!(i, WordNode::Box(BoxNode::Text(TextBox { .. }))))
        .unwrap();
    edit_text_box(&mut c.ast, last, |atoms, clusters| {
        atoms.truncate(1);
        clusters.truncate(1);
    });
    if let WordNode::Box(BoxNode::Text(TextBox {
        byte_range,
        natural_width,
        ..
    })) = &mut c.ast.items[last]
    {
        *byte_range = TextRange::new(2, 3);
        *natural_width = 1.0;
    }
}

// --- pairs ----------------------------------------------------------------

#[test]
fn ast11_geo2_constraints_before_opt3_options() {
    precedence(
        (bad_constraints, LayoutInputError::InvalidConstraints),
        (bad_options, LayoutInputError::InvalidLayoutOptions),
        true,
    );
}

#[test]
fn ast11_opt3_options_before_ast6_strut() {
    precedence(
        (bad_options, LayoutInputError::InvalidLayoutOptions),
        (bad_strut, LayoutInputError::InvalidStrut),
        true,
    );
}

#[test]
fn ast11_ast6_strut_before_ast2_paragraph_separator() {
    precedence(
        (bad_strut, LayoutInputError::InvalidStrut),
        (separator, LayoutInputError::ParagraphSeparatorInText),
        true,
    );
}

#[test]
fn ast11_ast2_paragraph_separator_before_ast4_run_index() {
    precedence(
        (separator, LayoutInputError::ParagraphSeparatorInText),
        (
            run_index_out_of_range,
            LayoutInputError::RunIndexOutOfRange { item: 0 },
        ),
        true,
    );
}

#[test]
fn ast11_ast4_run_storage_before_ast4_run_index() {
    precedence(
        (run_storage, LayoutInputError::InvalidRunStorage { run: 0 }),
        (
            run_index_out_of_range,
            LayoutInputError::RunIndexOutOfRange { item: 0 },
        ),
        true,
    );
}

#[test]
fn ast11_ast4_run_storage_before_ast5_orientation() {
    precedence(
        (run_storage, LayoutInputError::InvalidRunStorage { run: 0 }),
        (orientation, LayoutInputError::OrientationMismatch),
        true,
    );
}

#[test]
fn ast11_ast5_orientation_before_bid2_bidi_span() {
    precedence(
        (orientation, LayoutInputError::OrientationMismatch),
        (bad_bidi_span, LayoutInputError::InvalidBidiSpan { span: 0 }),
        true,
    );
}

#[test]
fn ast11_bid2_bidi_span_before_ast5_shaping_run() {
    precedence(
        (bad_bidi_span, LayoutInputError::InvalidBidiSpan { span: 0 }),
        (
            bad_shaping_run,
            LayoutInputError::InvalidShapingRun { run: 0 },
        ),
        true,
    );
}

#[test]
fn ast11_ann2_annotation_span_before_ast5_shaping_run() {
    precedence(
        (
            bad_annotation,
            LayoutInputError::InvalidAnnotationSpan { annotation: 0 },
        ),
        (
            bad_shaping_run,
            LayoutInputError::InvalidShapingRun { run: 0 },
        ),
        true,
    );
}

#[test]
fn ast11_ast5_shaping_run_before_ast9_atomic_span() {
    precedence(
        (
            bad_shaping_run,
            LayoutInputError::InvalidShapingRun { run: 0 },
        ),
        (
            bad_atomic_span,
            LayoutInputError::InvalidAtomicSpan { span: 0 },
        ),
        true,
    );
}

#[test]
fn ast11_ast7_decoration_span_before_ast3_item_outside_text() {
    precedence(
        (
            bad_decoration_span,
            LayoutInputError::InvalidDecorationSpan { span: 0 },
        ),
        (
            item_outside_text,
            LayoutInputError::ItemOutsideText { item: 0 },
        ),
        false,
    );
}

#[test]
fn ast11_ast9_atomic_span_before_ast3_item_outside_text() {
    precedence(
        (
            bad_atomic_span,
            LayoutInputError::InvalidAtomicSpan { span: 0 },
        ),
        (
            item_outside_text,
            LayoutInputError::ItemOutsideText { item: 0 },
        ),
        true,
    );
}

#[test]
fn ast11_ast3_item_outside_text_before_item_order() {
    precedence(
        (
            item_outside_text,
            LayoutInputError::ItemOutsideText { item: 0 },
        ),
        (
            items_out_of_order,
            LayoutInputError::ItemsOutOfOrder { item: 1 },
        ),
        true,
    );
}

#[test]
fn ast11_ast3_item_order_before_wn6_per_item_error() {
    precedence(
        (
            items_out_of_order,
            LayoutInputError::ItemsOutOfOrder { item: 1 },
        ),
        (bad_item, LayoutInputError::InvalidTextWidth { item: 0 }),
        true,
    );
}

#[test]
fn ast11_wn6_per_item_error_before_wn17_missing_paragraph_end() {
    precedence(
        (bad_item, LayoutInputError::InvalidTextWidth { item: 0 }),
        (missing_paragraph_end, LayoutInputError::MissingParagraphEnd),
        true,
    );
}

#[test]
fn ast11_wn17_missing_paragraph_end_before_ast8_span_boundary_order() {
    precedence(
        (missing_paragraph_end, LayoutInputError::MissingParagraphEnd),
        (
            bad_span_order,
            LayoutInputError::InvalidSpanBoundary { item: 4 },
        ),
        false,
    );
}

#[test]
fn ast11_ast8_span_boundary_order_before_ast8_span_enclosure() {
    precedence(
        (
            bad_span_order,
            LayoutInputError::InvalidSpanBoundary { item: 4 },
        ),
        (
            span_enclosure,
            LayoutInputError::SpanEnclosure { span: 0, item: 3 },
        ),
        false,
    );
}

#[test]
fn ast11_ast8_span_enclosure_before_ast3_uncovered_source() {
    precedence(
        (
            span_enclosure,
            LayoutInputError::SpanEnclosure { span: 0, item: 3 },
        ),
        (
            uncovered_source,
            LayoutInputError::UncoveredSource { byte: 3 },
        ),
        true,
    );
}
