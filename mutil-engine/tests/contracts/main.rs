//! Integration tests for the public API: data model, validators, geometry
//! queries and hand-assembled pipeline fixtures.
//!
//! The whole suite is one test binary; each module covers one domain.

mod annotations;
mod api;
mod badness;
mod bidi_gates;
mod common;
mod data_versions;
mod error_causes;
mod error_messages;
mod gates;
mod geometry;
mod hanging_gates;
mod host_contract;
mod input_precedence;
mod input_validation;
mod line_validation;
mod metrics;
mod paragraph_validation;
mod pipeline_assembly;
mod pipeline_types;
mod rich_inline_spans;
mod source_coverage;
mod spans;
mod storage;
mod strategy_enums;
mod writing_modes;
