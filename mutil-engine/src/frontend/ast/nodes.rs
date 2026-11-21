//! The box-and-glue node algebra (spec §2.4).

use crate::types::{
    BaselineAlignment, BidiClass, BidiLevel, ByteIndex, HyphenOpportunity, TextRange,
};
use core::ops::Range;
use smallvec::SmallVec;

/// Kind of stretchable space (spec §2.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlueKind {
    /// A non-empty run of [`is_word_space`](crate::frontend::is_word_space)
    /// characters; the only kind that owns source bytes.
    WordSpace,
    /// Automatic CJK / non-CJK spacing, sized by the active profile.
    CjkAutospace,
    /// The punctuation compression pool of [`GlueTier::Tier0`]: the
    /// compressible margin of CJK full-width punctuation not already squeezed
    /// by `chws`. `natural_width == 0`, `max_stretch == 0`, `max_shrink` up to
    /// `0.5 * ic`.
    CjkPunctCompression,
}

impl GlueKind {
    /// The kind's tier. No kind maps to [`GlueTier::Tier2`]: that stretch is
    /// derived from `ALLOW_CJK_JUSTIFY_AFTER` on the clusters (spec §2.4).
    #[must_use]
    pub const fn tier(self) -> GlueTier {
        match self {
            Self::WordSpace | Self::CjkPunctCompression => GlueTier::Tier0,
            Self::CjkAutospace => GlueTier::Tier1,
        }
    }

    /// `true` for the engine-inserted kinds, which own no source bytes.
    #[must_use]
    pub const fn is_synthetic(self) -> bool {
        !matches!(self, Self::WordSpace)
    }
}

/// Priority level of a [`GlueNode`] in the JLReq / CLREQ ladder (spec §2.4,
/// §5 Stage 1.6.3): a lower tier absorbs slack before a higher one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum GlueTier {
    /// Punctuation compression pool and Western word spaces.
    #[default]
    Tier0 = 0,
    /// CJK / non-CJK autospace.
    Tier1 = 1,
    /// Inter-ideographic expansion: stretch only, and a tier of the
    /// middle-end's prefix-sum tables only (no [`GlueKind`] has it).
    Tier2 = 2,
}

/// Stretchable, shrinkable space (spec §2.4). Never an implicit break point:
/// see [`WordNode`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlueNode {
    /// Original Unicode bidi class for L1.
    pub original_bidi_class: BidiClass,
    /// Resolved level before line-local L1.
    pub bidi_level: BidiLevel,
    /// Whether trailing whitespace may disappear; always `true` for a
    /// `WordSpace` (spec WN-12).
    pub is_collapsible: bool,
    /// The spaces of a `WordSpace`; empty for the synthetic kinds.
    pub byte_range: TextRange,
    /// Space kind, which fixes the tier ([`GlueKind::tier`]).
    pub kind: GlueKind,
    /// Natural width.
    pub natural_width: f32,
    /// Maximum added width.
    pub max_stretch: f32,
    /// Maximum removed width.
    pub max_shrink: f32,
}

/// Which end of an [`InlineDecorationSpan`](crate::frontend::InlineDecorationSpan)
/// a [`BoxNode::SpanBoundary`] stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SpanEdge {
    /// The logical start of the span.
    Leading,
    /// The logical end of the span.
    Trailing,
}

/// What happens to a rigid inline object wider than the line (spec §2.4).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum BoxOverflowPolicy {
    /// Clamp the visual rendering to `wrap_width`.
    #[default]
    Clip,
    /// Let the object overflow when alone on an empty line.
    AllowOverflow,
    /// Scale width, ascent and descent uniformly by `wrap_width / width`.
    ScaleToFit,
}

/// Shaped text, naming slices of the paragraph-wide arrays (spec WN-5),
/// resolved by [`ParagraphAst::text_box`](super::ParagraphAst::text_box).
#[derive(Clone, Debug, PartialEq)]
pub struct TextBox {
    /// Index into [`ParagraphAst::runs`](super::ParagraphAst::runs).
    pub run_index: usize,
    /// First atom's start to last atom's end.
    pub byte_range: TextRange,
    /// Paragraph-level, non-empty, inside the run's `atom_range`.
    pub atom_range: Range<usize>,
    /// Paragraph-level, inside the run's `cluster_range`.
    pub cluster_range: Range<usize>,
    /// Sum of the clusters' widths: a validated cache (spec §1.5, AX-2).
    pub natural_width: f32,
}

/// An embedded object with fixed extents (spec §2.4, WN-7).
#[derive(Clone, Debug, PartialEq)]
pub struct InlineObjectBox {
    /// Alignment relative to the line baseline.
    pub baseline_align: BaselineAlignment,
    /// Non-empty source bytes, usually one U+FFFC.
    pub byte_range: TextRange,
    /// Caller-assigned identifier.
    pub box_id: u64,
    /// Width.
    pub width: f32,
    /// Extent above the baseline.
    pub ascent: f32,
    /// Extent below the baseline.
    pub descent: f32,
    /// Behaviour when wider than the line.
    pub overflow_policy: BoxOverflowPolicy,
    /// Resolved UAX #9 level of the object's source.
    pub bidi_level: BidiLevel,
}

/// A rigid item of the line-breaking sequence (spec §2.4).
#[derive(Clone, Debug, PartialEq)]
pub enum BoxNode {
    /// Shaped text.
    Text(TextBox),
    /// An embedded object with fixed extents.
    InlineObject(InlineObjectBox),
    /// The padding-plus-border advance at one end of a decoration span.
    SpanBoundary {
        /// Empty range at the span's start or end.
        byte_range: TextRange,
        /// Identifier of the span.
        span_id: u64,
        /// Which end of the span.
        edge: SpanEdge,
        /// `start_padding` or `end_padding` of the span.
        width: f32,
        /// Level of the span's first (`Leading`) or last (`Trailing`) inline
        /// entity, a text atom or `InlineObject`, else the base level (WN-8).
        bidi_level: BidiLevel,
    },
}

/// One item of the line-breaking sequence (spec §2.4).
///
/// **Invariant:** a line may end only at a [`WordNode::Discretionary`] (bar
/// the emergency cluster break). A breakable space is `[Box, Glue,
/// Discretionary]`: the discretionary follows the glue, so on a break the
/// spaces stay on line `k` (spec §2.6: `byte_range` includes trailing
/// whitespace, `visible_byte_end` does not). A space bound to the next word is
/// `[Box, Glue]`.
#[derive(Clone, Debug, PartialEq)]
pub enum WordNode {
    /// A rigid item.
    Box(BoxNode),
    /// Stretchable, shrinkable space.
    Glue(GlueNode),
    /// A break opportunity.
    Discretionary(DiscretionaryNode),
    /// A tab; width is `TabStops::next_stop(x) - x`.
    Tab {
        /// Source bytes. A tab stores no level: UAX #9 L1 resets it to the
        /// paragraph level (spec §1.5, AX-2).
        byte_range: TextRange,
    },
}

impl WordNode {
    /// The source bytes this node covers; empty for zero-width nodes.
    #[must_use]
    pub fn byte_span(&self) -> TextRange {
        match self {
            Self::Box(
                BoxNode::Text(TextBox { byte_range, .. })
                | BoxNode::InlineObject(InlineObjectBox { byte_range, .. })
                | BoxNode::SpanBoundary { byte_range, .. },
            )
            | Self::Glue(GlueNode { byte_range, .. })
            | Self::Discretionary(DiscretionaryNode { byte_range, .. })
            | Self::Tab { byte_range } => *byte_range,
        }
    }

    /// `true` for the only node a line may end at.
    #[must_use]
    pub const fn is_break_opportunity(&self) -> bool {
        matches!(self, Self::Discretionary(_))
    }
}

/// Work done when a [`DiscretionaryNode`] is chosen as a line end, applied at
/// commit time (spec §2.4, §5 Stage 1.6.5).
#[derive(Clone, Debug, PartialEq)]
pub enum BreakAction {
    /// Zero-cost break between words, Han ideographs or Thai words.
    Direct,
    /// Mandatory line termination; the node's `byte_range` tells which, so the
    /// action stores nothing (spec §1.5, AX-2). Non-empty: exactly the three
    /// bytes of U+2028 (`\n`, `\r\n` end the paragraph and are rejected
    /// inside one). Empty: the end-of-paragraph sentinel
    /// ([`DiscretionaryNode::is_paragraph_end`]); text ending in U+2028 ends
    /// with a real hard break followed by the sentinel.
    HardBreak,
    /// Hyphenation; both neighbouring text boxes share one run. See
    /// [`ParagraphAst::break_window`](super::ParagraphAst::break_window) and
    /// [`ParagraphAst::rejoin_windows`](super::ParagraphAst::rejoin_windows).
    Hyphenate(HyphenOpportunity),
    /// Break after a top-level math relation or binary operator.
    MathOperator {
        /// The operator repeated at the start of the next line, if any.
        repeat: Option<RepeatedOperator>,
    },
}

/// The glyph a [`BreakAction::MathOperator`] break repeats on the next line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepeatedOperator {
    /// Backend glyph index in the operator's font.
    pub glyph_id: u32,
    /// Advance of the repeated glyph; finite and non-negative.
    pub advance: f32,
}

/// Three-state penalty node after Omega / LuaTeX (spec §2.4): the only place a
/// line may end. Widths are finite signed deltas.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscretionaryNode {
    /// Empty at the break position for a soft opportunity; U+2028, or empty
    /// at the paragraph end, for a hard break. The next line starts at its end.
    pub byte_range: TextRange,
    /// Knuth-Plass penalty; see the associated constants.
    pub penalty: i32,
    /// `true` for hyphenation breaks (alpha demerits).
    pub flagged: bool,
    /// Added to line `k` if broken here: cloned `end_padding` plus the
    /// action's delta (a validated cache, spec §1.5, AX-2).
    pub pre_break_width: f32,
    /// Added to line `k + 1` if broken here: cloned `start_padding` plus the
    /// action's delta or the repeated operator's advance.
    pub post_break_width: f32,
    /// Added to the current line if not broken here; zero for a hard break.
    pub no_break_width: f32,
    /// Work done when the break is chosen.
    pub action: BreakAction,
    /// Ids of the [`BoxDecorationBreak::Clone`](crate::frontend::BoxDecorationBreak::Clone)
    /// spans this break falls strictly inside.
    pub cloned_spans: SmallVec<[u64; 2]>,
}

impl DiscretionaryNode {
    /// Mandatory break: `-INF_PENALTY`.
    pub const MANDATORY_BREAK_PENALTY: i32 = -10_000;
    /// An unpenalised natural break.
    pub const NATURAL_PENALTY: i32 = 0;
    /// Lowest URL interior break penalty.
    pub const URL_PENALTY_MIN: i32 = 25;
    /// Highest URL interior break penalty.
    pub const URL_PENALTY_MAX: i32 = 50;
    /// Standard Liang-pattern hyphenation; flagged.
    pub const HYPHEN_PENALTY: i32 = 135;
    /// Spelling-change hyphenation; flagged.
    pub const SPELLING_CHANGE_HYPHEN_PENALTY: i32 = 250;
    /// Added to every break inside a `BoxDecorationBreak::Clone` span.
    pub const CLONED_SPAN_PENALTY: i32 = 300;
    /// Break after a top-level math relation (`\relpenalty`).
    pub const MATH_RELATION_PENALTY: i32 = 500;
    /// Break after a top-level math binary operator (`\binoppenalty`).
    pub const MATH_BINARY_OPERATOR_PENALTY: i32 = 700;
    /// A forbidden break omits the node; this penalty or more is rejected.
    pub const FORBIDDEN_BREAK_PENALTY: i32 = 10_000;

    /// A zero-cost, zero-width natural break at `byte_offset`.
    #[must_use]
    pub fn direct(byte_offset: ByteIndex) -> Self {
        Self {
            byte_range: TextRange::empty_at(byte_offset),
            penalty: Self::NATURAL_PENALTY,
            flagged: false,
            pre_break_width: 0.0,
            post_break_width: 0.0,
            no_break_width: 0.0,
            action: BreakAction::Direct,
            cloned_spans: SmallVec::new(),
        }
    }

    /// A mandatory break owning `newline` (U+2028; paragraph separators are
    /// removed by [`split_paragraphs`](crate::frontend::split_paragraphs)).
    #[must_use]
    pub fn hard_break(newline: TextRange) -> Self {
        Self {
            byte_range: newline,
            penalty: Self::MANDATORY_BREAK_PENALTY,
            action: BreakAction::HardBreak,
            ..Self::direct(newline.start())
        }
    }

    /// The paragraph-end sentinel: a hard break owning the empty range at
    /// `text_len`.
    #[must_use]
    pub fn paragraph_end(text_len: ByteIndex) -> Self {
        Self::hard_break(TextRange::empty_at(text_len))
    }

    /// Whether this is the paragraph-end sentinel.
    #[must_use]
    pub fn is_paragraph_end(&self) -> bool {
        matches!(self.action, BreakAction::HardBreak) && self.byte_range.is_empty()
    }

    /// The break position (`byte_range.start`).
    #[must_use]
    pub const fn byte_offset(&self) -> ByteIndex {
        self.byte_range.start()
    }

    /// `true` when `penalty == MANDATORY_BREAK_PENALTY` (spec WN-18).
    #[must_use]
    pub const fn is_mandatory(&self) -> bool {
        self.penalty == Self::MANDATORY_BREAK_PENALTY
    }
}
