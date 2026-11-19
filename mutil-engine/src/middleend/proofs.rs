//! The proof kernel (ADR-0004 rule 5): views that carry a fact a gate proved,
//! so code behind the gate reads it without indexing or falling back.
//!
//! Every view wraps a borrow and costs nothing at run time. Each slice access
//! names the fact it relies on in a `PROOF(...)` comment, which
//! `scripts/check_pipeline.py` requires.

use super::output::{CaretStop, HangSide};
use crate::middleend::tolerance::approx_eq;

/// A slice that is not empty.
#[derive(Debug)]
#[repr(transparent)]
pub(crate) struct NonEmptySlice<'a, T>(&'a [T]);

impl<T> Clone for NonEmptySlice<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for NonEmptySlice<'_, T> {}

impl<'a, T> NonEmptySlice<'a, T> {
    /// `slice`, unless it is empty.
    pub(crate) fn try_from_slice(slice: &'a [T]) -> Option<Self> {
        (!slice.is_empty()).then_some(Self(slice))
    }

    pub(crate) fn first(self) -> &'a T {
        // PROOF(NonEmptySlice::try_from_slice): the slice is not empty.
        &self.0[0]
    }

    pub(crate) fn last(self) -> &'a T {
        // PROOF(NonEmptySlice::try_from_slice): the slice is not empty.
        &self.0[self.last_index()]
    }

    pub(crate) fn last_index(self) -> usize {
        self.0.len() - 1
    }

    pub(crate) fn iter(self) -> core::slice::Iter<'a, T> {
        self.0.iter()
    }

    pub(crate) fn get(self, index: usize) -> Option<&'a T> {
        self.0.get(index)
    }

    /// The element at `index`, or the last one past the end.
    pub(crate) fn get_clamped(self, index: usize) -> &'a T {
        // PROOF(NonEmptySlice::try_from_slice): `last_index` is an index.
        &self.0[index.min(self.last_index())]
    }

    pub(crate) fn partition_point(self, pred: impl FnMut(&T) -> bool) -> usize {
        self.0.partition_point(pred)
    }

    /// The index of the first element for which `pred` fails, else the last.
    pub(crate) fn partition_point_clamped(self, pred: impl FnMut(&T) -> bool) -> usize {
        self.partition_point(pred).min(self.last_index())
    }

    /// The element with the least key, the first of ties, with its index.
    pub(crate) fn min_by_key_indexed<K: Ord>(self, mut key: impl FnMut(&T) -> K) -> (usize, &'a T) {
        let mut best = (0, self.first());
        for item in self.iter().enumerate().skip(1) {
            if key(item.1) < key(best.1) {
                best = item;
            }
        }
        best
    }
}

/// A string that is not empty.
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub(crate) struct NonEmptyStr<'a>(&'a str);

impl<'a> NonEmptyStr<'a> {
    /// `s`, unless it is empty.
    pub(crate) fn try_new(s: &'a str) -> Option<Self> {
        (!s.is_empty()).then_some(Self(s))
    }

    pub(crate) fn first_char(self) -> char {
        let mut chars = self.0.chars();
        // PROOF(NonEmptyStr::try_new): a non-empty `str` has a first char.
        chars.next().expect("non-empty str")
    }

    pub(crate) fn last_char(self) -> char {
        let mut chars = self.0.chars();
        // PROOF(NonEmptyStr::try_new): a non-empty `str` has a last char.
        chars.next_back().expect("non-empty str")
    }
}

/// The index of a line of a [`ValidatedLayout`](super::ValidatedLayout).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LineIdx(pub(super) usize);

impl LineIdx {
    pub(crate) fn get(self) -> usize {
        self.0
    }
}

/// The index of a stop in its line's [`SortedCaretStops`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct StopIdx(usize);

/// The caret stops of a line that passed the output gate, when it has any:
/// G4 (`InvalidCaretStop`) proved them finite and in non-decreasing `x`, and
/// rejected two stops of one offset at one position (`DuplicateCaretStop`).
#[derive(Clone, Copy, Debug)]
#[repr(transparent)]
pub(crate) struct SortedCaretStops<'a>(NonEmptySlice<'a, CaretStop>);

impl<'a> SortedCaretStops<'a> {
    /// The stops of a validated line; `None` for a line without stops
    /// (CRT-1). Only `ValidatedLayout` calls it, on lines G4 accepted.
    pub(super) fn try_from_validated(stops: &'a [CaretStop]) -> Option<Self> {
        NonEmptySlice::try_from_slice(stops).map(Self)
    }

    pub(crate) fn first(self) -> &'a CaretStop {
        self.0.first()
    }

    pub(crate) fn last(self) -> &'a CaretStop {
        self.0.last()
    }

    pub(crate) fn iter(self) -> core::slice::Iter<'a, CaretStop> {
        self.0.iter()
    }

    /// The stops in visual order, with their indices.
    pub(crate) fn indexed(self) -> impl Iterator<Item = (StopIdx, &'a CaretStop)> {
        self.iter().enumerate().map(|(i, s)| (StopIdx(i), s))
    }

    /// The stop with the least offset, the first of ties.
    pub(crate) fn min_by_offset(self) -> (StopIdx, &'a CaretStop) {
        let (index, stop) = self.0.min_by_key_indexed(|s| s.offset);
        (StopIdx(index), stop)
    }

    /// The extreme stop towards `side` (spec §6.3 rule 3), as
    /// [`PositionedLine::extreme_caret_stop`](super::PositionedLine::extreme_caret_stop)
    /// finds it: sorted stops put the first of the leftmost ties first and
    /// the last of the rightmost ties last.
    pub(crate) fn edge(self, side: HangSide) -> (StopIdx, &'a CaretStop) {
        match side {
            HangSide::Left => (StopIdx(0), self.first()),
            HangSide::Right => (StopIdx(self.0.last_index()), self.last()),
        }
    }

    /// The stop at `index`, clamped to the last one.
    pub(crate) fn stop(self, index: StopIdx) -> &'a CaretStop {
        self.0.get_clamped(index.0)
    }

    /// The nearest stop at another position than the one at `from`, one
    /// visual step right or left: the first at it moving right, the last
    /// moving left (spec INT-13).
    pub(crate) fn step_visual(self, from: StopIdx, rightwards: bool) -> Option<&'a CaretStop> {
        let x = self.stop(from).x;
        let elsewhere = |s: &&CaretStop| !approx_eq(s.x, x);
        if rightwards {
            self.iter().skip(from.0 + 1).find(elsewhere)
        } else {
            self.iter().take(from.0).rev().find(elsewhere)
        }
    }

    /// The stop nearest `x` for `first.x < x < last.x` (spec INT-8, INT-20):
    /// of the stops either side of `x`, the nearer, the right one at the
    /// midpoint. The right one is the first of its coincident group and the
    /// left one the last of its own. Outside the bounds it is still total.
    pub(crate) fn nearest_in_bounds(self, x: f32) -> &'a CaretStop {
        let right = self.0.partition_point(|s| s.x <= x);
        let r = self.0.get_clamped(right);
        let l = self.0.get_clamped(right.saturating_sub(1));
        if r.x - x <= x - l.x { r } else { l }
    }
}
