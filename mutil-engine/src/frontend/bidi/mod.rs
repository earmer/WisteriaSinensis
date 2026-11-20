//! UAX #9 bidi resolution and the line-level rules L1 and L2 (spec §3 Step
//! 2, FE-8 to FE-12, FE-30, BID-3, ST2-1, ST2-2).
//!
//! The resolver works on bidi classes, so the conformance suites (RM-1) and
//! the frontend share it; [`resolve_text`] builds those classes from a
//! paragraph, its inline objects and its explicit bidi spans. The middle-end
//! reuses [`reset_line_levels`] and [`visual_order`] (spec SCO-6).

mod explicit;
mod implicit;

use super::{BidiControlKind, ExplicitBidiSpan};
use crate::types::{BidiClass, BidiLevel, Direction, TextRange};
use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_properties::props::{BidiClass as IcuClass, BidiMirroringGlyph, BidiPairedBracketType};
use icu_properties::{CodePointMapData, CodePointMapDataBorrowed};

pub(crate) use explicit::is_removed;

static CLASSES: CodePointMapDataBorrowed<'static, IcuClass> = CodePointMapData::<IcuClass>::new();
static MIRRORING: CodePointMapDataBorrowed<'static, BidiMirroringGlyph> =
    CodePointMapData::<BidiMirroringGlyph>::new();
static NFD: DecomposingNormalizerBorrowed<'static> = DecomposingNormalizerBorrowed::new_nfd();

/// A character's role in BD16 bracket pairing, with the key two brackets of
/// one pair share: the NFD form of the opener, and of the closer's paired
/// bracket (spec FE-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bracket {
    None,
    Open(u32),
    Close(u32),
}

/// The engine's bidi class of `c`.
pub(crate) fn char_class(c: char) -> BidiClass {
    match CLASSES.get(c) {
        IcuClass::LeftToRight => BidiClass::L,
        IcuClass::RightToLeft => BidiClass::R,
        IcuClass::ArabicLetter => BidiClass::Al,
        IcuClass::EuropeanNumber => BidiClass::En,
        IcuClass::EuropeanSeparator => BidiClass::Es,
        IcuClass::EuropeanTerminator => BidiClass::Et,
        IcuClass::ArabicNumber => BidiClass::An,
        IcuClass::CommonSeparator => BidiClass::Cs,
        IcuClass::NonspacingMark => BidiClass::Nsm,
        IcuClass::BoundaryNeutral => BidiClass::Bn,
        IcuClass::ParagraphSeparator => BidiClass::B,
        IcuClass::SegmentSeparator => BidiClass::S,
        IcuClass::WhiteSpace => BidiClass::Ws,
        IcuClass::LeftToRightEmbedding => BidiClass::Lre,
        IcuClass::LeftToRightOverride => BidiClass::Lro,
        IcuClass::RightToLeftEmbedding => BidiClass::Rle,
        IcuClass::RightToLeftOverride => BidiClass::Rlo,
        IcuClass::PopDirectionalFormat => BidiClass::Pdf,
        IcuClass::LeftToRightIsolate => BidiClass::Lri,
        IcuClass::RightToLeftIsolate => BidiClass::Rli,
        IcuClass::FirstStrongIsolate => BidiClass::Fsi,
        IcuClass::PopDirectionalIsolate => BidiClass::Pdi,
        _ => BidiClass::On,
    }
}

/// The canonical form BD16 compares brackets by.
fn canonical(c: char) -> u32 {
    let mut decomposed = NFD.normalize_iter(core::iter::once(c));
    match (decomposed.next(), decomposed.next()) {
        (Some(single), None) => u32::from(single),
        _ => u32::from(c),
    }
}

/// The BD16 role of `c` (spec FE-9): `Bidi_Paired_Bracket` is its
/// `Bidi_Mirroring_Glyph`.
pub(crate) fn bracket(c: char) -> Bracket {
    let glyph = MIRRORING.get(c);
    match (glyph.paired_bracket_type, glyph.mirroring_glyph) {
        (BidiPairedBracketType::Open, _) => Bracket::Open(canonical(c)),
        (BidiPairedBracketType::Close, Some(pair)) => Bracket::Close(canonical(pair)),
        _ => Bracket::None,
    }
}

/// The resolved paragraph: its direction and a level per input position.
pub(crate) struct Resolution {
    pub(crate) direction: Direction,
    pub(crate) levels: Vec<BidiLevel>,
}

/// UAX #9 on one paragraph of classes (P2 to I2), `base` overriding P2 and
/// P3 (HL1). A character X9 removes takes the level of the one before it,
/// else the paragraph's (spec FE-10, UAX #9 §5.2).
pub(crate) fn resolve(
    classes: &[BidiClass],
    brackets: &[Bracket],
    base: Option<Direction>,
) -> Resolution {
    let pdis = explicit::matching_pdis(classes);
    let direction = base.unwrap_or_else(|| {
        explicit::first_strong(classes, &pdis, 0, false).unwrap_or(Direction::Ltr)
    });
    let paragraph = direction.base_level().get();
    let mut work = classes.to_vec();
    let mut levels = explicit::explicit_levels(&mut work, &pdis, paragraph);
    let ctx = implicit::Context {
        classes: &work,
        original: classes,
        brackets,
    };
    for sequence in explicit::run_sequences(&work, classes, &levels, &pdis, paragraph) {
        implicit::resolve_sequence(&ctx, &sequence, &mut levels);
    }
    for i in 0..levels.len() {
        if is_removed(classes[i]) {
            levels[i] = i.checked_sub(1).map_or(paragraph, |p| levels[p]);
        }
    }
    Resolution {
        direction,
        levels: levels.into_iter().map(explicit::checked_level).collect(),
    }
}

/// A paragraph's levels by source position (spec FE-30).
pub(crate) struct TextLevels {
    pub(crate) direction: Direction,
    /// When `None`, every position has `direction.base_level()` (uniform LTR
    /// fast path with zero allocations or lookups). Otherwise, each resolved
    /// position's starting byte offset, ascending, and level.
    positions: Option<Vec<(usize, BidiLevel)>>,
}

impl TextLevels {
    /// The level of the code point or inline object starting at `byte`.
    pub(crate) fn at(&self, byte: usize) -> BidiLevel {
        let Some(positions) = &self.positions else {
            return self.direction.base_level();
        };
        let at = positions.partition_point(|&(s, _)| s < byte);
        debug_assert_eq!(
            positions.get(at).map(|p| p.0),
            Some(byte),
            "a unit starts at {byte}"
        );
        positions
            .get(at)
            .map_or(self.direction.base_level(), |p| p.1)
    }
}

/// The control characters a span stands for at its edges (spec BID-3).
fn span_controls(kind: BidiControlKind) -> (BidiClass, BidiClass) {
    match kind {
        BidiControlKind::Isolate(None) => (BidiClass::Fsi, BidiClass::Pdi),
        BidiControlKind::Isolate(Some(Direction::Ltr)) => (BidiClass::Lri, BidiClass::Pdi),
        BidiControlKind::Isolate(Some(Direction::Rtl)) => (BidiClass::Rli, BidiClass::Pdi),
        BidiControlKind::Override(Direction::Ltr) => (BidiClass::Lro, BidiClass::Pdf),
        BidiControlKind::Override(Direction::Rtl) => (BidiClass::Rlo, BidiClass::Pdf),
    }
}

/// Collects the resolver's input: the text's own classes, one ON position
/// per inline object (spec FE-12), and the spans' controls merged at their
/// edges (BID-3), which take no source position.
struct Builder<'s> {
    spans: &'s [ExplicitBidiSpan],
    next_span: usize,
    open: Vec<&'s ExplicitBidiSpan>,
    classes: Vec<BidiClass>,
    brackets: Vec<Bracket>,
    /// The byte start of each position, `None` for a span's control.
    starts: Vec<Option<usize>>,
}

impl Builder<'_> {
    fn push(&mut self, class: BidiClass, bracket: Bracket, start: Option<usize>) {
        self.classes.push(class);
        self.brackets.push(bracket);
        self.starts.push(start);
    }

    /// The span edges at `offset`: closing ones inner first, then opening
    /// ones outer first.
    fn edges_at(&mut self, offset: usize) {
        while let Some(span) = self.open.last().filter(|s| s.byte_range.end() == offset) {
            let close = span_controls(span.control).1;
            self.open.pop();
            self.push(close, Bracket::None, None);
        }
        while let Some(span) = self
            .spans
            .get(self.next_span)
            .filter(|s| s.byte_range.start() == offset)
        {
            self.push(span_controls(span.control).0, Bracket::None, None);
            self.open.push(span);
            self.next_span += 1;
        }
    }
}

/// Whether `c` cannot trigger any non-zero bidi level under an LTR paragraph
/// base with no explicit bidi spans: no RTL/Arabic/Arabic-number class and no
/// explicit embedding, override, or isolate control.
fn is_ltr_only_char(c: char) -> bool {
    c.is_ascii()
        || !matches!(
            char_class(c),
            BidiClass::R
                | BidiClass::Al
                | BidiClass::An
                | BidiClass::Lre
                | BidiClass::Lro
                | BidiClass::Rle
                | BidiClass::Rlo
                | BidiClass::Pdf
                | BidiClass::Lri
                | BidiClass::Rli
                | BidiClass::Fsi
                | BidiClass::Pdi
        )
}

/// Resolves a paragraph's text (spec FE-30). `objects` are the ranges of its
/// inline objects, ascending; `spans` its validated explicit bidi spans.
pub(crate) fn resolve_text(
    text: &str,
    objects: &[TextRange],
    spans: &[ExplicitBidiSpan],
    base: Option<Direction>,
) -> TextLevels {
    // Under UAX #9 with an LTR (or auto) base and no explicit spans, if no
    // character has an R, AL, AN, or explicit control class then W1-W7 resolve
    // all EN to L (sos = L), N0-N2 resolve all neutrals to L, and I1 keeps
    // every level at 0.
    if spans.is_empty()
        && base != Some(Direction::Rtl)
        && (text.is_ascii() || text.chars().all(is_ltr_only_char))
    {
        return TextLevels {
            direction: Direction::Ltr,
            positions: None,
        };
    }
    let mut builder = Builder {
        spans,
        next_span: 0,
        open: Vec::new(),
        classes: Vec::with_capacity(text.len()),
        brackets: Vec::with_capacity(text.len()),
        starts: Vec::with_capacity(text.len()),
    };
    let mut next_object = 0;
    for (offset, c) in text.char_indices() {
        while objects.get(next_object).is_some_and(|o| o.end() <= offset) {
            next_object += 1;
        }
        match objects.get(next_object) {
            Some(o) if o.start() == offset => {
                builder.edges_at(offset);
                builder.push(BidiClass::On, Bracket::None, Some(offset));
            }
            Some(o) if o.start() < offset => {}
            _ => {
                builder.edges_at(offset);
                builder.push(char_class(c), bracket(c), Some(offset));
            }
        }
    }
    builder.edges_at(text.len());
    let resolution = resolve(&builder.classes, &builder.brackets, base);
    let positions = builder
        .starts
        .iter()
        .zip(resolution.levels)
        .filter_map(|(start, level)| Some(((*start)?, level)))
        .collect();
    TextLevels {
        direction: resolution.direction,
        positions: Some(positions),
    }
}

/// How L1 treats an item of a line (spec ST2-1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineClass {
    /// A segment or paragraph separator: reset, with the whitespace before it.
    Separator,
    /// Whitespace, an isolate control, or (UAX #9 §5.2) a removed character:
    /// reset before a separator and at the line end.
    Whitespace,
    /// Anything else.
    Other,
}

/// The L1 role of an original class.
pub(crate) const fn line_class(class: BidiClass) -> LineClass {
    match class {
        BidiClass::S | BidiClass::B => LineClass::Separator,
        BidiClass::Ws
        | BidiClass::Fsi
        | BidiClass::Lri
        | BidiClass::Rli
        | BidiClass::Pdi
        | BidiClass::Lre
        | BidiClass::Rle
        | BidiClass::Lro
        | BidiClass::Rlo
        | BidiClass::Pdf
        | BidiClass::Bn => LineClass::Whitespace,
        _ => LineClass::Other,
    }
}

/// L1 on one line: separators, the whitespace before them and the
/// whitespace at the line's end take the paragraph level `base`.
pub(crate) fn reset_line_levels(
    classes: impl DoubleEndedIterator<Item = LineClass> + ExactSizeIterator,
    levels: &mut [BidiLevel],
    base: BidiLevel,
) {
    let mut resetting = true;
    for (class, level) in classes.rev().zip(levels.iter_mut().rev()) {
        resetting = match class {
            LineClass::Separator => true,
            LineClass::Whitespace => resetting,
            LineClass::Other => false,
        };
        if resetting {
            *level = base;
        }
    }
}

/// L2: writes the logical indices of a line in visual order, left to right,
/// into `order`. From the highest level to the lowest odd one, every maximal
/// sequence at that level or above is reversed (spec ST2-2).
pub(crate) fn visual_order_into(levels: &[BidiLevel], order: &mut Vec<usize>) {
    order.clear();
    order.extend(0..levels.len());
    let Some(highest) = levels.iter().map(|l| l.get()).max() else {
        return;
    };
    let lowest_odd = levels
        .iter()
        .map(|l| l.get())
        .filter(|l| l % 2 == 1)
        .min()
        .unwrap_or(highest + 1);
    for level in (lowest_odd.max(1)..=highest).rev() {
        for run in
            order.chunk_by_mut(|&a, &b| (levels[a].get() >= level) == (levels[b].get() >= level))
        {
            if levels[run[0]].get() >= level {
                run.reverse();
            }
        }
    }
}

/// L2: the logical indices of a line in visual order, left to right. From
/// the highest level to the lowest odd one, every maximal sequence at that
/// level or above is reversed (spec ST2-2).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn visual_order(levels: &[BidiLevel]) -> Vec<usize> {
    let mut order = Vec::with_capacity(levels.len());
    visual_order_into(levels, &mut order);
    order
}
