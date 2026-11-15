//! Hyphenation opportunities (spec §2.4, WN-22).
//!
//! Defined in `types` because both the item sequence and the shaping bridge's
//! boundary edits read them (spec SCO-6).

/// Replacement text for spelling-change hyphenation.
///
/// Replacements are rare (`Option`, usually `None`), so `Box<str>` costs 16
/// bytes inline (niche-optimised) and one allocation only when present.
pub type ReplacementText = Box<str>;

/// Hyphenation opportunity mirroring libhyphen's `(rep, pos, cut)` `[H3]`
/// (spec §2.4). Example: Dutch "omaatje" becomes "oma-tje".
///
/// The break position is that of the `DiscretionaryNode` carrying the
/// opportunity (`DiscretionaryNode::byte_offset`), in the **original** text:
/// the prefix ends and the suffix begins there, before any replacement is
/// applied.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HyphenOpportunity {
    /// Text replacing the bytes cut from the end of the prefix, if the
    /// spelling changes.
    pub prefix_replacement: Option<ReplacementText>,
    /// Text replacing the bytes cut from the start of the suffix, if the
    /// spelling changes.
    pub suffix_replacement: Option<ReplacementText>,
    /// Original bytes removed immediately **before** the break position. A cut
    /// that straddles the break is split between the two counts. Each cut ends
    /// on an atom boundary within the word's contiguous same-run hyphen chain,
    /// so it may cross an unchosen hyphenation point of the same word but never
    /// a run or span boundary.
    pub prefix_cut_bytes: u32,
    /// Original bytes removed immediately **after** the break position; the
    /// same rules apply.
    pub suffix_cut_bytes: u32,
}
