//! Integration tests for the public API: data model, validators, geometry
//! queries and hand-assembled pipeline fixtures.
//!
//! The whole suite is one test binary; each module covers one domain.

mod annotations;
mod api;
mod badness;
mod common;
mod error_messages;
mod geometry;
mod input_validation;
mod line_validation;
mod metrics;
mod pipeline_assembly;
mod pipeline_types;
mod rich_inline_spans;
mod source_coverage;
mod spans;
mod strategy_enums;
mod writing_modes;
