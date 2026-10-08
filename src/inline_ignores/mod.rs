//! Inline `kibitzer:ignore <rule>[,<rule>...] -- <reason>` directives: parsing, scanning,
//! matching, and application to findings.

mod apply;
mod hint;
mod parse;
mod rules;
mod scan;
mod types;
mod unused;

#[cfg(test)]
mod anchor_tests;
#[cfg(test)]
mod tests;

pub use apply::{InlineIgnoreContext, InlineIgnoreMode, SuppressionCounts};
pub(crate) use apply::{apply_inline_ignores, capped_anchors};
pub(crate) use hint::{HINT_RULE_LIMIT, batch_syntax_hint, syntax_hint, syntax_hint_limited};
pub use parse::is_directive_comment;
pub(crate) use parse::{echo_parts, near_miss_text};
pub(crate) use rules::{anchor_rule, did_you_mean, known_rule, owned_by};
pub use scan::scan_directives;
pub use types::{
    Directive, DirectiveParse, InlineOutcome, Line, MalformedReason, Reason, RuleId, Scanned,
    WeakReason,
};
pub(crate) use types::{LineSpan, RawFinding, rows_intersect, valid_directives};
pub(crate) use unused::{FirstPass, UnusedKind, unowned_verdicts, unused_ignores};

// Reached through this module only by tests; production code names the submodule item itself.
#[cfg(test)]
pub(crate) use parse::parse_comment_line;
#[cfg(test)]
pub(crate) use types::{Anchor, DroppedFinding, FILE_SCOPE_RULES, ReasonError};
#[cfg(test)]
pub(crate) use unused::UnusedVerdict;
