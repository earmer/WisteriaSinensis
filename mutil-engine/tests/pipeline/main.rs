//! End-to-end tests: text through `compile_paragraph`, `layout_paragraph`,
//! the output gate and the §6 queries, shaped by the HarfRust backend with
//! the vendored test font (`tests/assets/README.md`).

mod acceptance;
mod acceptance_phase2;
mod bidi_frontend;
mod bidi_queries;
mod cjk_routing;
mod common;
mod fonts;
mod frontend;
mod hanging;
mod hanging_regressions;
mod justification;
mod layout;
mod line_only;
mod queries;
mod shaper;
mod spacing_frontend;
mod steps;
