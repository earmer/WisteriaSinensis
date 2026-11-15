//! End-to-end tests: text through `compile_paragraph`, `layout_paragraph`,
//! the output gate and the §6 queries, shaped by the HarfRust backend with
//! the vendored test font (`tests/assets/README.md`).

mod acceptance;
mod common;
mod fonts;
mod frontend;
mod layout;
mod line_only;
mod queries;
mod shaper;
mod steps;
