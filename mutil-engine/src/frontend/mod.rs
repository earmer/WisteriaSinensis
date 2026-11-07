//! Frontend: input abstraction and the paragraph AST (spec §3, §2.4, §2.5).
//!
//! Phase 0 defines the output and option types only; no analysis pass is
//! implemented.

mod ast;
mod options;
mod run;
mod span;

pub use ast::{
    BoxNode, BoxOverflowPolicy, BreakAction, DiscretionaryNode, GlueKind, GlueNode, GlueTier,
    HyphenOpportunity, ParagraphAst, ReplacementText, SpanEdge, WordNode,
};
pub use options::{AutospaceProfile, FrontendOptions, QuotePolicy};
pub use run::{ShapingRun, TextDecoration};
pub use span::{AtomicEditSpan, BoxDecorationBreak, InlineDecorationSpan};
