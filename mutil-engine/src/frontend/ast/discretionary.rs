//! Break opportunities: [`DiscretionaryNode`] and its [`BreakAction`]
//! (spec §2.4).

use crate::types::{ByteIndex, HyphenOpportunity, TextRange};
use smallvec::SmallVec;

/// Opportunity-specific work done when a [`DiscretionaryNode`] is chosen as a
/// line end (spec §2.4). The payload is applied lazily, at commit time
/// (§5 Stage 1.6.5): candidate evaluation uses the three widths only.
#[derive(Clone, Debug, PartialEq)]
pub enum BreakAction {
    /// Zero-cost break between words, Han ideographs or Thai words.
    Direct,
    /// Mandatory line termination (`penalty == MANDATORY_BREAK_PENALTY`).
    ///
    /// The node's `byte_range` says which of the two it is
    /// ([`DiscretionaryNode::is_paragraph_end`]), so the action stores
    /// nothing (spec §1.5, AX-2):
    ///
    /// * a **non-empty** range is an explicit line terminator present in the
    ///   text. Inside one paragraph that is **U+2028 only**: `\n`, `\r\n` and
    ///   the other paragraph separators end the paragraph and are handled by
    ///   [`split_paragraphs`](crate::frontend::split_paragraphs), and
    ///   `check_layout_inputs` rejects them in a node or in the text. The range
    ///   is exactly the three bytes of U+2028;
    /// * an **empty** range is the synthetic end-of-paragraph sentinel: the
    ///   empty range `text.len()..text.len()` on the **last** discretionary of
    ///   the paragraph. Text ending in U+2028 therefore ends with a real hard
    ///   break followed by the sentinel (the trailing blank line).
    HardBreak,
    /// Hyphenation: `TextShaper::reshape_window` runs at materialisation. Both
    /// neighbouring text boxes share one `run_index` (a word is split within a
    /// single `ShapingRun`). The widths minus the cloned padding are the net
    /// prefix and suffix shaping deltas; the cuts stay within the contiguous
    /// same-run chain of text boxes joined by hyphenation points
    /// (`check_layout_inputs`). [`ParagraphAst::break_window`](super::ParagraphAst::break_window) gives the window
    /// to re-shape when the break is taken. When it is **not** taken across a
    /// seam that split a cluster, the two boxes are re-shaped together; see
    /// [`ParagraphAst::rejoin_windows`](super::ParagraphAst::rejoin_windows).
    Hyphenate(HyphenOpportunity),
    /// Restore or trim `chws`-compressed CJK punctuation at the line edges.
    ///
    /// A unit variant: the data already lives in the node's widths. After the
    /// cloned padding is subtracted, `pre_break_width` is the signed delta added
    /// to the `natural_width` of the last cluster of the immediately preceding
    /// [`BoxNode::Text`](super::BoxNode::Text) (the end of line `k`); `post_break_width` the delta
    /// added to the first cluster of the immediately following one (the start
    /// of line `k + 1`). At least one delta is non-zero; a non-zero delta needs
    /// that text box, and the cluster's width plus the delta must stay
    /// non-negative.
    CjkPunctAdjust,
    /// Break after a top-level math relation (`=`) or binary operator (`+`).
    MathOperator {
        /// The operator glyph repeated at the start of the next line, if any.
        /// After the cloned padding is subtracted, `post_break_width` is
        /// exactly its advance (zero without a repeat) and `pre_break_width`
        /// is zero.
        repeat: Option<RepeatedOperator>,
    },
}

/// The glyph a [`BreakAction::MathOperator`] break repeats at the start of the
/// next line, with the advance it adds there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepeatedOperator {
    /// Backend glyph index in the font of the operator.
    pub glyph_id: u32,
    /// Advance of the repeated glyph. Finite and non-negative.
    pub advance: f32,
}

/// Generalised three-state penalty node after Omega / LuaTeX (spec §2.4): the
/// **only** place a line may end (except for an emergency cluster break on an
/// overflowing empty line).
///
/// If the line breaks here, line `k` gains `pre_break_width` and line `k + 1`
/// gains `post_break_width`; if not, the current line gains `no_break_width`.
/// Widths are signed deltas (a trim or restore may be negative) but finite.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscretionaryNode {
    /// Bytes this opportunity owns. An **empty** range at the break position
    /// for every soft opportunity (after a space, between ideographs,
    /// hyphenation, a math operator); for a [`BreakAction::HardBreak`] the exact
    /// U+2028 line separator (3 bytes; `\n` and `\r\n` are paragraph separators
    /// that [`split_paragraphs`](crate::frontend::split_paragraphs) removes
    /// before a paragraph exists), or an empty range at the end of the
    /// paragraph. The next line starts at `byte_range.end`, so no byte is left
    /// unowned.
    pub byte_range: TextRange,
    /// Knuth-Plass penalty; see the associated constants for the canonical scale.
    pub penalty: i32,
    /// `true` for hyphenation breaks (triggers the alpha demerits).
    pub flagged: bool,
    /// Width added to the end of line `k` if broken here: the `end_padding` of
    /// every span in `cloned_spans` plus the action's own delta (see
    /// [`BreakAction`]); for `Direct`, `HardBreak` and `MathOperator` the
    /// padding sum alone.
    pub pre_break_width: f32,
    /// Width added to the start of line `k + 1` if broken here: the
    /// `start_padding` of every span in `cloned_spans` plus the action's own
    /// delta; for `Direct` and `HardBreak` the padding sum alone, for
    /// `MathOperator` the padding sum plus the repeated operator's advance.
    ///
    /// Both widths are the only home of the shaping deltas of `Hyphenate` and
    /// `CjkPunctAdjust`. The padding and the repeated advance folded into them
    /// are a validated cache (spec §1.5, AX-2): the breaker reads them for every
    /// candidate, and `check_layout_inputs` requires exact agreement with the
    /// spans and the action.
    pub post_break_width: f32,
    /// Width contributed to the current line if **not** broken here (for
    /// example the letter-spacing of the skipped opportunity). Finite; zero for
    /// a hard break.
    pub no_break_width: f32,
    /// Work done when the break is chosen.
    pub action: BreakAction,
    /// Ids of the [`BoxDecorationBreak::Clone`](crate::frontend::BoxDecorationBreak::Clone)
    /// spans this break falls strictly inside. Cloning padding is orthogonal to
    /// `action` (a hyphenation break inside a chip is both): when the line
    /// breaks here, each listed span gets its cloned `end_padding` on line `k`
    /// (inside `pre_break_width`) and its cloned `start_padding` on line `k + 1`
    /// (inside `post_break_width`).
    pub cloned_spans: SmallVec<[u64; 2]>,
}

impl DiscretionaryNode {
    /// Mandatory break (U+2028 inside a paragraph, or the end of paragraph): `-INF_PENALTY`.
    pub const MANDATORY_BREAK_PENALTY: i32 = -10_000;
    /// An unpenalised natural break.
    pub const NATURAL_PENALTY: i32 = 0;
    /// Lowest URL interior break penalty (slash and punctuation breaks).
    pub const URL_PENALTY_MIN: i32 = 25;
    /// Highest URL interior break penalty.
    pub const URL_PENALTY_MAX: i32 = 50;
    /// Standard Liang-pattern hyphenation (`hypher`); flagged.
    pub const HYPHEN_PENALTY: i32 = 135;
    /// Spelling-change hyphenation (libhyphen Dutch, Catalan, Hungarian); flagged.
    pub const SPELLING_CHANGE_HYPHEN_PENALTY: i32 = 250;
    /// Added to every break inside a `BoxDecorationBreak::Clone` span.
    pub const CLONED_SPAN_PENALTY: i32 = 300;
    /// Break after a top-level inline math relation (`\relpenalty`).
    pub const MATH_RELATION_PENALTY: i32 = 500;
    /// Break after a top-level inline math binary operator (`\binoppenalty`).
    pub const MATH_BINARY_OPERATOR_PENALTY: i32 = 700;
    /// A forbidden break is represented by **omitting** the node; a node with
    /// this penalty or more is rejected by `check_layout_inputs`.
    pub const FORBIDDEN_BREAK_PENALTY: i32 = 10_000;

    /// A zero-cost, zero-width natural break at `byte_offset` (an empty range).
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

    /// A mandatory break at the line separator U+2028 that `newline` covers
    /// exactly. Paragraph separators (`\n`, `\r`, `\r\n`, U+0085, U+2029,
    /// U+001C–U+001E) are not allowed here because a paragraph AST covers one
    /// paragraph only: split document text with
    /// [`split_paragraphs`](crate::frontend::split_paragraphs) first.
    #[must_use]
    pub fn hard_break(newline: TextRange) -> Self {
        Self {
            byte_range: newline,
            penalty: Self::MANDATORY_BREAK_PENALTY,
            action: BreakAction::HardBreak,
            ..Self::direct(newline.start())
        }
    }

    /// The paragraph-end sentinel of a paragraph whose text is `text_len`
    /// bytes long: the mandatory break owning the empty range at its end.
    #[must_use]
    pub fn paragraph_end(text_len: ByteIndex) -> Self {
        Self::hard_break(TextRange::empty_at(text_len))
    }

    /// Whether this is the paragraph-end sentinel: a hard break that owns no
    /// bytes.
    #[must_use]
    pub fn is_paragraph_end(&self) -> bool {
        matches!(self.action, BreakAction::HardBreak) && self.byte_range.is_empty()
    }

    /// The break position: where line `k` ends and, for a soft opportunity,
    /// line `k + 1` begins (`byte_range.start`).
    #[must_use]
    pub const fn byte_offset(&self) -> ByteIndex {
        self.byte_range.start()
    }

    /// `true` when the break is mandatory (`penalty == MANDATORY_BREAK_PENALTY`).
    ///
    /// The mandatory penalty is the only one; a penalty below it is rejected by
    /// the input validator (spec WN-18).
    #[must_use]
    pub const fn is_mandatory(&self) -> bool {
        self.penalty == Self::MANDATORY_BREAK_PENALTY
    }
}
