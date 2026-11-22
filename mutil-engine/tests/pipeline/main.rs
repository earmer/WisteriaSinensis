//! End-to-end tests: text through `compile_paragraph`, `layout_paragraph`,
//! the output gate and the §6 queries, shaped by the HarfRust backend with
//! the vendored test font (`tests/assets/README.md`).

mod acceptance;
mod acceptance_phase2;
mod bidi_frontend;
mod bidi_queries;
mod canonical_features;
mod cjk_punctuation_trim;
mod cjk_routing;
mod clearance;
mod common;
mod decoration_spans;
mod emergency_refit;
mod f64_domain;
mod font_table_gates;
mod fonts;
mod frontend;
mod glue_width;
mod hanging;
mod hanging_regressions;
mod hanging_roles;
mod hit_test_coincident;
mod justification;
mod layout;
mod line_only;
mod long_word;
mod motion_modes;
mod queries;
mod seam_padding;
mod shape_buffers;
mod shaper;
mod shaper_numbers;
mod spacing_frontend;
mod span_topology;
mod steps;
mod stopless_lines;
mod variable_fonts;
