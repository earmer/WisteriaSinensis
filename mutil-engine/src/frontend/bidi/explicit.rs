//! UAX #9 paragraph level and explicit levels (P2, P3, X1 to X10), on a
//! sequence of bidi classes (spec FE-8, FE-30).

use crate::types::{BIDI_MAX_DEPTH, BidiClass, BidiLevel, Direction};

/// Whether X9 removes a character of `class`: the embedding and override
/// controls, PDF and BN.
pub(crate) const fn is_removed(class: BidiClass) -> bool {
    matches!(
        class,
        BidiClass::Lre
            | BidiClass::Rle
            | BidiClass::Lro
            | BidiClass::Rlo
            | BidiClass::Pdf
            | BidiClass::Bn
    )
}

/// Whether `class` is an isolate initiator.
pub(super) const fn is_isolate_initiator(class: BidiClass) -> bool {
    matches!(class, BidiClass::Lri | BidiClass::Rli | BidiClass::Fsi)
}

/// The matching PDI of every isolate initiator (BD9), or `None` when it has
/// none before the paragraph end.
pub(super) fn matching_pdis(classes: &[BidiClass]) -> Vec<Option<usize>> {
    let mut matches = vec![None; classes.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, &class) in classes.iter().enumerate() {
        if is_isolate_initiator(class) {
            open.push(i);
        } else if class == BidiClass::Pdi
            && let Some(initiator) = open.pop()
        {
            matches[initiator] = Some(i);
        } else if class == BidiClass::B {
            open.clear();
        }
    }
    matches
}

/// P2 over `classes[from..]`: the direction of the first strong character,
/// skipping the content of isolates. With `stop_at_pdi` (X5c, for an FSI) the
/// search ends at a PDI that closes the isolate it started in.
pub(super) fn first_strong(
    classes: &[BidiClass],
    pdis: &[Option<usize>],
    from: usize,
    stop_at_pdi: bool,
) -> Option<Direction> {
    let mut i = from;
    while let Some(&class) = classes.get(i) {
        match class {
            BidiClass::L => return Some(Direction::Ltr),
            BidiClass::R | BidiClass::Al => return Some(Direction::Rtl),
            BidiClass::Pdi if stop_at_pdi => return None,
            BidiClass::B => return None,
            c if is_isolate_initiator(c) => i = pdis[i]?,
            _ => {}
        }
        i += 1;
    }
    None
}

/// One entry of the directional status stack (X1).
#[derive(Clone, Copy)]
struct Status {
    level: u8,
    /// The class an override forces, if any.
    override_class: Option<BidiClass>,
    isolate: bool,
}

/// The directional status stack of X1 with its overflow counters.
struct StatusStack {
    entries: Vec<Status>,
    overflow_isolates: usize,
    overflow_embeddings: usize,
    valid_isolates: usize,
}

impl StatusStack {
    fn new(paragraph: u8) -> Self {
        let mut entries = Vec::with_capacity(usize::from(BIDI_MAX_DEPTH) + 2);
        entries.push(Status {
            level: paragraph,
            override_class: None,
            isolate: false,
        });
        Self {
            entries,
            overflow_isolates: 0,
            overflow_embeddings: 0,
            valid_isolates: 0,
        }
    }

    fn top(&self) -> Status {
        *self.entries.last().expect("the stack keeps its base entry")
    }

    /// Whether an entry at `level` may be pushed (X2 to X5c).
    fn admits(&self, level: u8) -> bool {
        level <= BIDI_MAX_DEPTH && self.overflow_isolates == 0 && self.overflow_embeddings == 0
    }

    /// X2 to X5: an embedding or override initiator.
    fn push_embedding(&mut self, class: BidiClass) {
        let rtl = matches!(class, BidiClass::Rle | BidiClass::Rlo);
        let level = next_level(self.top().level, rtl);
        if self.admits(level) {
            self.entries.push(Status {
                level,
                override_class: match class {
                    BidiClass::Rlo => Some(BidiClass::R),
                    BidiClass::Lro => Some(BidiClass::L),
                    _ => None,
                },
                isolate: false,
            });
        } else if self.overflow_isolates == 0 {
            self.overflow_embeddings += 1;
        }
    }

    /// X5a to X5c: an isolate initiator of direction `rtl`.
    fn push_isolate(&mut self, rtl: bool) {
        let level = next_level(self.top().level, rtl);
        if self.admits(level) {
            self.valid_isolates += 1;
            self.entries.push(Status {
                level,
                override_class: None,
                isolate: true,
            });
        } else {
            self.overflow_isolates += 1;
        }
    }

    /// X6a: a PDI closes the innermost valid isolate and what it holds.
    fn pop_isolate(&mut self) {
        if self.overflow_isolates > 0 {
            self.overflow_isolates -= 1;
        } else if self.valid_isolates > 0 {
            self.overflow_embeddings = 0;
            while self.entries.last().is_some_and(|s| !s.isolate) {
                self.entries.pop();
            }
            self.entries.pop();
            self.valid_isolates -= 1;
        }
    }

    /// X7: a PDF closes the innermost embedding or override, unless an
    /// isolate is innermost.
    fn pop_embedding(&mut self) {
        if self.overflow_isolates > 0 {
            // An overflowed isolate holds it: nothing to close.
        } else if self.overflow_embeddings > 0 {
            self.overflow_embeddings -= 1;
        } else if !self.top().isolate && self.entries.len() >= 2 {
            self.entries.pop();
        }
    }
}

/// The explicit levels of X1 to X8, with the classes an override forces
/// written back into `classes`. Removed characters (X9) keep the level of
/// the stack's top at their position; their final level is assigned later
/// (FE-10).
pub(super) fn explicit_levels(
    classes: &mut [BidiClass],
    pdis: &[Option<usize>],
    paragraph: u8,
) -> Vec<u8> {
    let mut levels = vec![paragraph; classes.len()];
    let mut stack = StatusStack::new(paragraph);
    for i in 0..classes.len() {
        let top = stack.top();
        let class = classes[i];
        levels[i] = top.level;
        match class {
            BidiClass::Rle | BidiClass::Lre | BidiClass::Rlo | BidiClass::Lro => {
                stack.push_embedding(class);
            }
            BidiClass::Rli | BidiClass::Lri | BidiClass::Fsi => {
                if let Some(forced) = top.override_class {
                    classes[i] = forced;
                }
                stack.push_isolate(match class {
                    BidiClass::Rli => true,
                    BidiClass::Lri => false,
                    _ => first_strong(classes, pdis, i + 1, true) == Some(Direction::Rtl),
                });
            }
            BidiClass::Pdi => {
                stack.pop_isolate();
                let top = stack.top();
                levels[i] = top.level;
                if let Some(forced) = top.override_class {
                    classes[i] = forced;
                }
            }
            BidiClass::Pdf => stack.pop_embedding(),
            BidiClass::B => levels[i] = paragraph,
            BidiClass::Bn => {}
            _ => {
                if let Some(forced) = top.override_class {
                    classes[i] = forced;
                }
            }
        }
    }
    levels
}

/// The least odd (`rtl`) or even level above `level`.
const fn next_level(level: u8, rtl: bool) -> u8 {
    if rtl {
        (level + 1) | 1
    } else {
        (level + 2) & !1
    }
}

/// The strong class of the direction of `level`: R when odd, else L.
pub(super) const fn direction_class(level: u8) -> BidiClass {
    if level % 2 == 1 {
        BidiClass::R
    } else {
        BidiClass::L
    }
}

/// An isolating run sequence (BD13): the indices of its characters in order,
/// its level, and the classes before and after it (X10).
pub(super) struct RunSequence {
    pub(super) indices: Vec<usize>,
    pub(super) level: u8,
    pub(super) sos: BidiClass,
    pub(super) eos: BidiClass,
}

/// X10: the isolating run sequences of the characters X9 keeps.
pub(super) fn run_sequences(
    classes: &[BidiClass],
    original: &[BidiClass],
    levels: &[u8],
    pdis: &[Option<usize>],
    paragraph: u8,
) -> Vec<RunSequence> {
    let kept: Vec<usize> = (0..classes.len())
        .filter(|&i| !is_removed(original[i]))
        .collect();
    // Level runs over the kept characters.
    let mut runs: Vec<Vec<usize>> = Vec::new();
    for &i in &kept {
        match runs.last_mut() {
            Some(run) if levels[run[0]] == levels[i] => run.push(i),
            _ => runs.push(vec![i]),
        }
    }
    // Chain runs across matched isolates: a run that starts with a PDI
    // continues the sequence whose run ended with its initiator.
    let mut sequences: Vec<Vec<usize>> = Vec::new();
    let mut awaiting: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for run in runs {
        let first = run[0];
        let last = *run.last().expect("a level run is not empty");
        let sequence = match awaiting.remove(&first) {
            Some(s) => {
                sequences[s].extend_from_slice(&run);
                s
            }
            _ => {
                sequences.push(run);
                sequences.len() - 1
            }
        };
        if is_isolate_initiator(original[last])
            && let Some(pdi) = pdis[last]
        {
            awaiting.insert(pdi, sequence);
        }
    }
    let level_before = |i: usize| {
        kept[..kept.partition_point(|&k| k < i)]
            .last()
            .map_or(paragraph, |&k| levels[k])
    };
    let level_after = |i: usize| {
        kept.get(kept.partition_point(|&k| k <= i))
            .map_or(paragraph, |&k| levels[k])
    };
    sequences
        .into_iter()
        .map(|indices| {
            let first = indices[0];
            let last = *indices.last().expect("a sequence is not empty");
            let level = levels[first];
            // A sequence that ends with an isolate initiator has no
            // matching PDI after it on the paragraph's level.
            let after = if is_isolate_initiator(original[last]) {
                paragraph
            } else {
                level_after(last)
            };
            RunSequence {
                sos: direction_class(level.max(level_before(first))),
                eos: direction_class(level.max(after)),
                level,
                indices,
            }
        })
        .collect()
}

/// The level `level` as a [`BidiLevel`]; UAX #9 never exceeds its maximum.
pub(super) fn checked_level(level: u8) -> BidiLevel {
    BidiLevel::try_new(level).expect("UAX #9 levels never exceed BIDI_MAX_DEPTH + 1")
}
