//! The shaper gate (spec AX-3, G2): every answer a [`TextShaper`] gives at
//! layout time is checked here, once, before layout uses it.
//!
//! Shaping output that the frontend stores in the AST is checked by the input
//! gate instead. Ligature carets are checked by
//! [`resolve_ligature_carets`](super::resolve_ligature_carets) (spec INT-5).

use super::{LayoutInputError, LineHeight, StrutMetrics};
use crate::shaping::TextShaper;
use crate::types::{FontInstanceRef, WritingMode};

/// The line box of one font instance under `line_height`: the shaper's
/// metrics for `instance`, checked (spec FNT-4, MSR-1), resolved like the
/// paragraph strut and checked again as a strut.
///
/// # Errors
/// `InvalidFontMetrics` when the shaper's metrics are invalid or resolve to an
/// invalid line box.
pub(crate) fn checked_line_box(
    shaper: &dyn TextShaper,
    instance: FontInstanceRef<'_>,
    line_height: LineHeight,
    mode: WritingMode,
) -> Result<StrutMetrics, LayoutInputError> {
    let font_size = instance.font_size;
    let metrics = shaper.font_metrics(instance);
    if !metrics.is_valid() {
        return Err(LayoutInputError::InvalidFontMetrics);
    }
    let line_box = line_height.resolve_strut(&metrics, font_size, mode);
    if line_box.is_valid() {
        Ok(line_box)
    } else {
        Err(LayoutInputError::InvalidFontMetrics)
    }
}
