//! Inline `kibitzer:ignore <rule>[,<rule>...] -- <reason>` directives: parsing, scanning,
//! matching, and application to findings.

mod apply;
mod hint;
mod parse;
mod rules;
pub(crate) mod sanitize;
mod scan;
mod types;
mod unused;

#[cfg(test)]
mod anchor_tests;
#[cfg(test)]
mod tests;

pub(crate) use apply::{IgnoreTarget, apply_inline_ignores, capped_anchors};
pub(crate) use hint::{HINT_RULE_LIMIT, batch_syntax_hint, syntax_hint, syntax_hint_limited};
pub(crate) use parse::{echo_parts, near_miss_text};
pub use parse::{is_directive_only_comment, without_directive_lines};
pub(crate) use rules::{anchor_rule, did_you_mean, is_file_scope, known_rule, owned_by};
pub use scan::ScanMemo;
pub use types::{
    Directive, DirectiveParse, InlineIgnoreContext, InlineIgnoreMode, InlineOutcome, Line,
    MalformedReason, Reason, RuleId, SuppressionCounts, WeakReason,
};
pub(crate) use types::{LineSpan, RawFinding, has_marker, rows_intersect, valid_directives};
pub(crate) use unused::{FirstPass, UnusedKind, unowned_verdicts, unused_ignores};

// Reached through this module only by tests; production code names the submodule item itself.
#[cfg(test)]
pub(crate) use parse::{is_directive_comment, parse_comment_line};
#[cfg(test)]
pub(crate) use scan::scan_directives;
pub(crate) use types::DroppedFinding;
#[cfg(test)]
pub(crate) use types::FILE_SCOPE_RULES;
#[cfg(test)]
pub use types::Scanned;
#[cfg(test)]
pub(crate) use types::{Anchor, ReasonError};
#[cfg(test)]
pub(crate) use unused::UnusedVerdict;
