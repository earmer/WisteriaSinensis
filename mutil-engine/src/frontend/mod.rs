//! Frontend: input abstraction and the paragraph AST (spec §3, §2.4, §2.5).
//!
//! Phase 0 defines the output types only; no analysis pass is implemented.

mod ast;
mod run;

pub use ast::{GlueKind, HyphenOpportunity, ParagraphAst, PunctBind, ReplacementText, WordNode};
pub use run::{ShapingRun, TextDecoration};
