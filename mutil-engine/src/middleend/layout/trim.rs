//! Layout-time side-slot trimming of full-width CJK punctuation (spec §5).

use super::{Layout, Piece, PieceKind, Reshapes};
use crate::frontend::GlueKind;
use crate::frontend::spacing::{COMPRESSED_PAIR_FRACTION, turns_compression_off};
use crate::middleend::CjkPunctuationTrim;
use crate::middleend::validated::{ClusterId, RunId};
use crate::types::{AtomFlags, LocaleTag};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PunctTrimClass {
    Open,
    Close,
    MiddleDot,
}

impl PunctTrimClass {
    pub(super) fn from_flags(flags: AtomFlags) -> Option<Self> {
        if flags.contains(AtomFlags::CJK_TRIM_OPEN) {
            Some(Self::Open)
        } else if flags.contains(AtomFlags::CJK_TRIM_CLOSE) {
            Some(Self::Close)
        } else if flags.contains(AtomFlags::CJK_TRIM_MIDDLE_DOT) {
            Some(Self::MiddleDot)
        } else {
            None
        }
    }

    pub(super) fn max_lead(self, font_size: f64) -> f64 {
        match self {
            Self::Open => 0.5 * font_size,
            Self::MiddleDot => 0.25 * font_size,
            Self::Close => 0.0,
        }
    }

    pub(super) fn max_trail(self, font_size: f64) -> f64 {
        match self {
            Self::Close => 0.5 * font_size,
            Self::MiddleDot => 0.25 * font_size,
            Self::Open => 0.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct TrimInfo {
    pub(super) class: PunctTrimClass,
    pub(super) policy: CjkPunctuationTrim,
    pub(super) font_size: f64,
}

pub(super) fn effective_trim_policy(
    policy: CjkPunctuationTrim,
    locale: &LocaleTag,
) -> CjkPunctuationTrim {
    match policy {
        CjkPunctuationTrim::Auto => {
            if turns_compression_off(locale) {
                CjkPunctuationTrim::SpaceAll
            } else {
                CjkPunctuationTrim::SpaceFirst
            }
        }
        other => other,
    }
}

pub(super) fn is_trim_transparent(piece: &Piece<'_>) -> bool {
    piece.is_edge_transparent()
        || matches!(
            piece.kind,
            PieceKind::Glue(glue) if glue.kind == GlueKind::CjkPunctCompression
        )
}

pub(super) fn line_start_lead_trim(info: TrimInfo, is_soft_wrap: bool) -> f64 {
    if info.class != PunctTrimClass::Open {
        return 0.0;
    }
    match info.policy {
        CjkPunctuationTrim::TrimStart => 0.5 * info.font_size,
        CjkPunctuationTrim::SpaceFirst if is_soft_wrap => 0.5 * info.font_size,
        _ => 0.0,
    }
}

pub(super) fn pair_trim(a: TrimInfo, b: TrimInfo) -> (f64, f64) {
    use PunctTrimClass::{Close, MiddleDot, Open};
    match (a.class, b.class) {
        (Close, Close) | (Close, Open) => (0.5 * a.font_size, 0.0),
        (Open, Open) => (0.0, 0.5 * b.font_size),
        (Close, MiddleDot) | (MiddleDot, Open) | (MiddleDot, MiddleDot) => {
            (0.25 * a.font_size, 0.0)
        }
        (Open, MiddleDot) | (MiddleDot, Close) => (0.0, 0.25 * b.font_size),
        _ => (0.0, 0.0),
    }
}

impl Layout<'_, '_> {
    pub(super) fn cluster_trim_info(
        &self,
        run: RunId,
        cluster: ClusterId,
        raw_width: f64,
    ) -> Option<TrimInfo> {
        let r = self.inputs.run(run);
        let policy = effective_trim_policy(self.options.cjk_punctuation_trim, &r.locale);
        if policy == CjkPunctuationTrim::SpaceAll {
            return None;
        }
        let c = self.inputs.cluster(cluster);
        let class = PunctTrimClass::from_flags(c.flags)?;
        let font_size = f64::from(r.font_size);
        if raw_width <= f64::from(COMPRESSED_PAIR_FRACTION) * font_size {
            return None;
        }
        Some(TrimInfo {
            class,
            policy,
            font_size,
        })
    }

    pub(super) fn start_edge_trim(
        &self,
        piece: &Piece<'_>,
        is_soft_wrap: bool,
        pending: Option<&Reshapes>,
    ) -> f64 {
        let PieceKind::Cluster { run, cluster } = piece.kind else {
            return 0.0;
        };
        let (_, raw_width) = self.cluster_glyphs_over(pending, cluster);
        self.cluster_trim_info(run, cluster, raw_width)
            .map_or(0.0, |info| line_start_lead_trim(info, is_soft_wrap))
    }
}
