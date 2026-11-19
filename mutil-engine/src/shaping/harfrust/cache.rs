//! Caching and thread-local scratch buffers for [`super::HarfRustShaper`]:
//! two-generation bounded caches for variation instances and compiled HarfRust
//! shape plans, plus reusable per-thread shaping buffers.

use crate::shaping::clusters::{GroupScratch, TaggedGlyph};
use crate::types::{FontFeature, FontId, FontVariation, GlyphOrientation, ShapingRun};
use core::cell::RefCell;
use core::hash::Hash;
use harfrust::{Buffer, Feature, Language, Script, ShapePlan, Tag};
use read_fonts::model::Font;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;

/// The most variation instances the shaper keeps parsed at once. A caller
/// animating an axis asks for a new instance each frame; generational eviction
/// keeps memory bounded while retaining hot instances across the boundary.
pub(super) const MAX_CACHED_INSTANCES: usize = 128;

/// The most compiled HarfRust shape plans the shaper keeps cached at once.
pub(super) const MAX_CACHED_PLANS: usize = 256;

/// Upper bound on element capacity retained in thread-local shaping scratch
/// buffers after a call finishes, so unusually large requests do not leave
/// peak allocations permanently resident on worker threads.
const MAX_RETAINED_SCRATCH_CAP: usize = 4096;

/// A variable instance, keyed by face and its sorted variation coordinates
/// (spec FNT-1 makes [`FontVariation`] a sound hash/equality key).
pub(super) type InstanceKey = (FontId, SmallVec<[FontVariation; 2]>);

/// The three shaping directions this backend emits to HarfRust.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum ShapeDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
}

impl ShapeDirection {
    pub(super) fn for_run(run: &ShapingRun) -> Self {
        if run.orientation == GlyphOrientation::VerticalUpright {
            Self::TopToBottom
        } else if run.bidi_level.direction().is_rtl() {
            Self::RightToLeft
        } else {
            Self::LeftToRight
        }
    }

    pub(super) fn is_vertical(self) -> bool {
        self == Self::TopToBottom
    }

    pub(super) fn is_horizontal_rtl(self) -> bool {
        self == Self::RightToLeft
    }

    pub(super) fn to_harfrust(self) -> harfrust::Direction {
        match self {
            Self::LeftToRight => harfrust::Direction::LeftToRight,
            Self::RightToLeft => harfrust::Direction::RightToLeft,
            Self::TopToBottom => harfrust::Direction::TopToBottom,
        }
    }
}

/// Key for a cached HarfRust [`ShapePlan`]: face, variation coordinates,
/// direction, script, language, and global OpenType features.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct PlanKey {
    pub(super) instance: InstanceKey,
    pub(super) direction: ShapeDirection,
    pub(super) script: Option<Script>,
    pub(super) language: Option<Language>,
    pub(super) features: SmallVec<[FontFeature; 8]>,
}

/// Two-generation bounded cache: new entries enter `current` until it reaches
/// half the capacity, at which point `current` rotates into `previous`
/// (dropping the older generation) while hits in `previous` promote back into
/// `current`.
pub(super) struct TwoGenCache<K, V> {
    current: HashMap<K, V>,
    previous: HashMap<K, V>,
}

impl<K, V> Default for TwoGenCache<K, V> {
    fn default() -> Self {
        Self {
            current: HashMap::new(),
            previous: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash, V> TwoGenCache<K, V> {
    /// Returns the entry for `key`, promoting from `previous` on a warm hit or
    /// building via `build` on a miss, while keeping total entries `<= cap`.
    pub(super) fn get_or_insert_with(
        &mut self,
        key: K,
        cap: usize,
        build: impl FnOnce() -> V,
    ) -> &mut V {
        if self.current.contains_key(&key) {
            return self.current.entry(key).or_insert_with(build);
        }
        let promoted = self.previous.remove_entry(&key);
        let half = (cap / 2).max(1);
        if self.current.len() >= half {
            if cap <= 1 {
                self.current.clear();
                self.previous.clear();
            } else {
                core::mem::swap(&mut self.current, &mut self.previous);
                self.current.clear();
            }
        }
        if let Some((k, v)) = promoted {
            self.current.entry(k).or_insert(v)
        } else {
            self.current.entry(key).or_insert_with(build)
        }
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.current.len() + self.previous.len()
    }

    #[cfg(test)]
    pub(super) fn keys(&self) -> impl Iterator<Item = &K> {
        self.current.keys().chain(self.previous.keys())
    }
}

/// Thread-local scratch allocations reused across shaping calls to avoid
/// per-run `Buffer`, `TaggedGlyph`, and cluster-grouping heap churn.
pub(super) struct ShaperScratch {
    pub(super) buffer: Buffer,
    pub(super) tagged: Vec<TaggedGlyph>,
    pub(super) groups: GroupScratch,
}

impl Default for ShaperScratch {
    fn default() -> Self {
        Self {
            buffer: Buffer::new(),
            tagged: Vec::new(),
            groups: GroupScratch::default(),
        }
    }
}

impl ShaperScratch {
    /// Resets `buffer` for reuse, replacing it instead if the previous segment
    /// exceeded [`MAX_RETAINED_SCRATCH_CAP`].
    pub(super) fn reset_buffer(&mut self) {
        if self.buffer.len() > MAX_RETAINED_SCRATCH_CAP {
            self.buffer = Buffer::new();
        } else {
            self.buffer.reset();
        }
    }

    /// Releases oversized capacities so an outlier segment does not stay
    /// permanently resident in thread-local storage.
    fn trim(&mut self) {
        if self.buffer.len() > MAX_RETAINED_SCRATCH_CAP
            || self.tagged.capacity() > MAX_RETAINED_SCRATCH_CAP
        {
            self.buffer = Buffer::new();
        }
        self.tagged.clear();
        if self.tagged.capacity() > MAX_RETAINED_SCRATCH_CAP {
            self.tagged.shrink_to(MAX_RETAINED_SCRATCH_CAP);
        }
        self.groups.trim(MAX_RETAINED_SCRATCH_CAP);
    }
}

std::thread_local! {
    static SHAPER_SCRATCH: RefCell<ShaperScratch> = RefCell::new(ShaperScratch::default());
}

pub(super) fn with_scratch<R>(f: impl FnOnce(&mut ShaperScratch) -> R) -> R {
    SHAPER_SCRATCH.with(|cell| match cell.try_borrow_mut() {
        Ok(mut scratch) => {
            let result = f(&mut scratch);
            scratch.trim();
            result
        }
        Err(_) => f(&mut ShaperScratch::default()),
    })
}

/// Per-run HarfRust configuration computed once for all segments of a run.
pub(super) struct RunShapeSetup {
    pub(super) direction: ShapeDirection,
    pub(super) script: Option<Script>,
    pub(super) language: Option<Language>,
    pub(super) features: SmallVec<[Feature; 8]>,
}

impl RunShapeSetup {
    pub(super) fn new(run: &ShapingRun) -> Self {
        let direction = ShapeDirection::for_run(run);
        let script = Script::from_iso15924_tag(Tag::new(&run.script.as_bytes()));
        let language = (run.locale.as_str() != "und")
            .then(|| Language::new(run.locale.as_str()))
            .flatten();
        let features = run
            .features
            .as_slice()
            .iter()
            .map(|f| Feature::new(Tag::new(&f.tag), f.value, ..))
            .collect();
        Self {
            direction,
            script,
            language,
            features,
        }
    }
}

/// One resolved font instance and its compiled [`ShapePlan`] for the current run.
pub(super) struct PreparedFont {
    pub(super) id: FontId,
    pub(super) font: Font,
    pub(super) scale: f32,
    pub(super) plan: Arc<ShapePlan>,
}
