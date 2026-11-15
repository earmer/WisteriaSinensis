//! End-to-end tests: text through `compile_paragraph`, `layout_paragraph`,
//! the output gate and the §6 queries, shaped by the HarfRust backend with
//! the vendored test font (`tests/assets/README.md`).

mod common;
mod fonts;
mod frontend;
mod layout;
mod queries;
mod shaper;
