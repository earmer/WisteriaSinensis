//! Hyphenation seams and the re-shape windows they imply (spec §5 Stage
//! 1.6.5).

use super::{BreakAction, DiscretionaryNode, ParagraphAst, ReshapeWindow, TextBoxView, WordNode};
use crate::types::HyphenOpportunity;
use core::iter::successors;
use smallvec::SmallVec;

/// A `Hyphenate` discretionary together with the two text boxes it joins
/// (see [`ParagraphAst::hyphen_seam`]).
#[derive(Clone, Copy, Debug)]
pub(crate) struct HyphenSeam<'a> {
    /// The seam's hyphenation opportunity.
    pub(crate) opportunity: &'a HyphenOpportunity,
    /// The text box ending at the seam (`items[index - 1]`).
    pub(crate) before: TextBoxView<'a>,
    /// The text box starting at the seam (`items[index + 1]`).
    pub(crate) after: TextBoxView<'a>,
}

impl ReshapeWindow {
    /// The window from the start of `first` to the end of `last`, two text
    /// boxes of one hyphen chain in text order.
    fn spanning(first: &TextBoxView<'_>, last: &TextBoxView<'_>) -> Self {
        Self {
            run_index: first.run_index,
            atom_range: first.atom_start..last.atom_start + last.atoms.len(),
            byte_range: first.byte_range.cover(last.byte_range),
        }
    }
}

impl ParagraphAst {
    /// The hyphenation seam at `items[index]`: a `Hyphenate` discretionary
    /// whose neighbours `items[index - 1]` and `items[index + 1]` are text
    /// boxes of one run, the three meeting end to end. `None` otherwise.
    ///
    /// Seams link text boxes into **hyphen chains**: maximal sequences of text
    /// boxes of one run joined solely by such seams. A chain stops at glue, a
    /// tab, an inline object, a span boundary, a run change or any other
    /// discretionary.
    pub(crate) fn hyphen_seam(&self, index: usize) -> Option<HyphenSeam<'_>> {
        let WordNode::Discretionary(DiscretionaryNode {
            action: BreakAction::Hyphenate(opportunity),
            byte_range,
            ..
        }) = self.items.get(index)?
        else {
            return None;
        };
        let before = self.text_box(index.checked_sub(1)?)?;
        let after = self.text_box(index + 1)?;
        (before.run_index == after.run_index
            && before.byte_range.end() == byte_range.start()
            && byte_range.end() == after.byte_range.start())
        .then_some(HyphenSeam {
            opportunity,
            before,
            after,
        })
    }

    /// The text box before `items[item]` in its hyphen chain, with its index.
    fn chain_predecessor(&self, item: usize) -> Option<(usize, TextBoxView<'_>)> {
        let seam = item.checked_sub(1)?;
        // `hyphen_seam` rejects `seam == 0`, so `seam - 1` cannot underflow.
        let before = self.hyphen_seam(seam)?.before;
        Some((seam - 1, before))
    }

    /// The text box after `items[item]` in its hyphen chain, with its index.
    fn chain_successor(&self, item: usize) -> Option<(usize, TextBoxView<'_>)> {
        let seam = item + 1;
        Some((seam + 1, self.hyphen_seam(seam)?.after))
    }

    /// The window to re-shape when the `Hyphenate` discretionary at
    /// `items[index]` **is taken** as a line end (spec §5 Stage 1.6.5).
    ///
    /// The window covers the text boxes the break touches: at least the box on
    /// each side of it, extended along the hyphen chain as far as a spelling
    /// change reaches (a cut may cross an unchosen hyphenation point of the
    /// same word, so it can end in an earlier or later box). The break offset
    /// ([`DiscretionaryNode::byte_offset`]) lies strictly inside it: line `k`
    /// re-shapes `byte_range.start..offset` with the opportunity's
    /// `TrailingEdit`, line `k + 1` re-shapes `offset..byte_range.end` with its
    /// `LeadingEdit`.
    ///
    /// `None` when `items[index]` is not a hyphenation seam, or when a cut
    /// leaves the seam's hyphen chain (which `check_layout_inputs` rejects).
    #[must_use]
    pub fn break_window(&self, index: usize) -> Option<ReshapeWindow> {
        let HyphenSeam {
            opportunity,
            before,
            after,
        } = self.hyphen_seam(index)?;
        let offset = before.byte_range.end();
        // The first and the last original byte the break's edits touch. An
        // edit without a cut still touches the neighbouring box: the hyphen
        // attaches to the last cluster before the break.
        let first = offset.checked_sub((opportunity.prefix_cut_bytes as usize).max(1))?;
        let last = offset.checked_add((opportunity.suffix_cut_bytes as usize).max(1))? - 1;

        let (_, start) = successors(Some((index - 1, before)), |&(item, _)| {
            self.chain_predecessor(item)
        })
        .find(|(_, view)| view.byte_range.start() <= first)?;
        let (_, end) = successors(Some((index + 1, after)), |&(item, _)| {
            self.chain_successor(item)
        })
        .find(|(_, view)| last < view.byte_range.end())?;
        Some(ReshapeWindow::spanning(&start, &end))
    }

    /// The windows to re-shape as **one** piece each because a `Hyphenate`
    /// discretionary was *not* taken on a committed line (spec §5 Stage 1.6.5,
    /// rejoin rule). `untaken` are the indices into `items` of the untaken
    /// discretionaries, in any order.
    ///
    /// A hyphenation opportunity inside a multi-atom cluster (the `of-fice`
    /// split of an `ffi` ligature) is resolved early by re-shaping the two
    /// sides separately (§2.3 note 3). The frontend marks such a seam by giving
    /// the first cluster after it `safe_to_concat_before == false`. If the
    /// break is then not taken, concatenating the two separately shaped boxes
    /// would lose the ligature and any kerning across the seam, so the
    /// middle-end calls `TextShaper::reshape_window` once per returned window
    /// with `LeadingEdit::None` and `TrailingEdit::None`.
    ///
    /// Each unsafe seam contributes the stretch from the start of the text box
    /// before it to the end of the one after it; an index that is not a
    /// hyphenation seam, or whose seam is safe to concatenate, contributes
    /// nothing. Consecutive unsafe seams share a box, so their stretches
    /// overlap and are merged: the result is in text order, pairwise disjoint,
    /// and shapes every atom at most once.
    #[must_use]
    pub fn rejoin_windows(
        &self,
        untaken: impl IntoIterator<Item = usize>,
    ) -> SmallVec<[ReshapeWindow; 4]> {
        let mut windows: SmallVec<[ReshapeWindow; 4]> = untaken
            .into_iter()
            .filter_map(|index| self.rejoin_window(index))
            .collect();
        windows.sort_by_key(|window| window.byte_range.start());
        let mut merged: SmallVec<[ReshapeWindow; 4]> = SmallVec::new();
        for window in windows {
            match merged.last_mut() {
                Some(last) if window.byte_range.start() < last.byte_range.end() => {
                    last.byte_range = last.byte_range.cover(window.byte_range);
                    last.atom_range.end = last.atom_range.end.max(window.atom_range.end);
                }
                _ => merged.push(window),
            }
        }
        merged
    }

    /// The stretch one untaken, unsafe hyphenation seam needs re-shaped: the
    /// two text boxes it joins. `None` when `items[index]` is not a seam or the
    /// seam is safe to concatenate.
    fn rejoin_window(&self, index: usize) -> Option<ReshapeWindow> {
        let HyphenSeam { before, after, .. } = self.hyphen_seam(index)?;
        after
            .clusters
            .first()
            .is_some_and(|c| !c.safe_to_concat_before)
            .then(|| ReshapeWindow::spanning(&before, &after))
    }
}
