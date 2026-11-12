//! Paragraph text and AST items must tile the source exactly; separators are rejected.

use crate::common::*;
use mutil_engine::*;

#[test]
fn valid_control() {
    assert!(check(&ast("abc"), "abc").is_ok());
}

#[test]
fn sentinel_cannot_replace_source() {
    let mut a = ast("abc");
    rebuild_items(&mut a, |_| vec![end(3)]);
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::UncoveredSource { byte: 0 }),
        "all bytes unowned"
    );
}

#[test]
fn source_cannot_have_gaps() {
    let mut a = ast("abc");
    rebuild_items(&mut a, |b| vec![boxed(b, 0, 2), end(3)]);
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::UncoveredSource { byte: 2 }),
        "2..3 unowned"
    );
}

#[test]
fn source_cannot_overlap() {
    let mut a = ast("abc");
    rebuild_items(&mut a, |b| vec![boxed(b, 0, 2), boxed(b, 1, 3), end(3)]);
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
}

fn reject_separator_in_source(sep: &str) {
    let t = format!("a{sep}b");
    let mut a = ast(&t);
    a.items[0] = WordNode::Box(BoxNode::InlineObject {
        bidi_level: 0,
        baseline_align: BaselineAlignment::Auto,
        byte_range: TextRange::new(0, t.len()),
        box_id: 1,
        width: 1.,
        ascent: 8.,
        descent: 2.,
        overflow_policy: BoxOverflowPolicy::Clip,
    });
    clear_text(&mut a);
    assert_eq!(
        check(&a, &t),
        Err(LayoutInputError::ParagraphSeparatorInText),
        "separator {sep:?}"
    );
}

fn reject_hard_break(sep: &str) {
    let mut a = ast(sep);
    rebuild_items(&mut a, |_| {
        vec![
            WordNode::Discretionary(DiscretionaryNode::hard_break(
                TextRange::new(0, sep.len()),
                false,
            )),
            end(sep.len()),
        ]
    });
    assert_eq!(
        check(&a, sep),
        Err(LayoutInputError::ParagraphSeparatorInText),
        "hard break {sep:?}"
    );
}

#[test]
fn line_separator_remains_allowed() {
    let t = "a\u{2028}b";
    let mut a = ast(t);
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1),
            WordNode::Discretionary(DiscretionaryNode::hard_break(TextRange::new(1, 4), false)),
            boxed(b, 4, 5),
            end(5),
        ]
    });
    assert!(check(&a, t).is_ok());
}

macro_rules! separator_cases {
    ($source:ident, $node:ident, $separator:expr) => {
        #[test]
        fn $source() {
            reject_separator_in_source($separator);
        }
        #[test]
        fn $node() {
            reject_hard_break($separator);
        }
    };
}

separator_cases!(reject_lf_source, reject_lf_node, "\n");

separator_cases!(reject_cr_source, reject_cr_node, "\r");

separator_cases!(reject_crlf_source, reject_crlf_node, "\r\n");

separator_cases!(reject_nel_source, reject_nel_node, "\u{85}");

separator_cases!(reject_ps_source, reject_ps_node, "\u{2029}");

separator_cases!(reject_vt_source, reject_vt_node, "\u{b}");

separator_cases!(reject_ff_source, reject_ff_node, "\u{c}");

#[test]
fn zero_width_node_cannot_be_anchored_inside_owned_content() {
    let mut a = ast("abc");
    a.items
        .insert(1, WordNode::Discretionary(DiscretionaryNode::direct(1)));
    // The box owns `0..3`, so a zero-width node at offset 1 is inside owned
    // content and breaks monotone item order.
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemsOutOfOrder { item: 1 })
    );
}

#[test]
fn zero_width_node_may_sit_on_a_content_boundary() {
    let mut a = ast("abc");
    rebuild_items(&mut a, |b| {
        vec![
            boxed(b, 0, 1),
            WordNode::Discretionary(DiscretionaryNode::direct(1)),
            boxed(b, 1, 3),
            end(3),
        ]
    });
    assert!(check(&a, "abc").is_ok());
}

#[test]
fn text_box_atoms_must_tile_their_byte_range() {
    let mut a = ast("abc");
    // Drop the middle atom (and its cluster): the clusters still partition
    // the two remaining atoms, but `0..1` + `2..3` leaves byte 1 unrepresented.
    edit_text_box(&mut a, 0, |atoms, clusters| {
        atoms.remove(1);
        clusters.remove(1);
        clusters[1].atom_range = 1..2;
    });
    if let WordNode::Box(BoxNode::Text { natural_width, .. }) = &mut a.items[0] {
        *natural_width = 2.;
    }
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::ItemNotSelfContained { item: 0 })
    );
}

#[test]
fn every_paragraph_separator_is_rejected_in_paragraph_text() {
    for text in [
        "a\nb",
        "a\rb",
        "a\u{85}b",
        "a\u{2029}b",
        "a\u{1c}b",
        "a\u{1e}b",
    ] {
        let a = ast(text);
        assert_eq!(
            check(&a, text),
            Err(LayoutInputError::ParagraphSeparatorInText),
            "{text:?}"
        );
    }
}

#[test]
fn source_coverage_cannot_skip_an_internal_byte() {
    let mut a = ast("abc");
    rebuild_items(&mut a, |b| vec![boxed(b, 0, 1), boxed(b, 2, 3), end(3)]);
    assert_eq!(
        check(&a, "abc"),
        Err(LayoutInputError::UncoveredSource { byte: 1 })
    );
}

#[test]
fn glue_only_source_must_cover_every_byte() {
    let mut a = ast("   ");
    rebuild_items(&mut a, |_| {
        vec![
            WordNode::Glue(GlueNode {
                original_bidi_class: BidiClass::Ws,
                bidi_level: 0,
                is_collapsible: true,
                byte_range: TextRange::new(0, 1),
                kind: GlueKind::WordSpace,
                natural_width: 1.,
                max_stretch: 0.,
                max_shrink: 0.,
            }),
            end(3),
        ]
    });
    assert_eq!(
        check(&a, "   "),
        Err(LayoutInputError::UncoveredSource { byte: 1 })
    );
    if let WordNode::Glue(g) = &mut a.items[0] {
        g.byte_range.end = 3;
    }
    assert!(check(&a, "   ").is_ok());
}
