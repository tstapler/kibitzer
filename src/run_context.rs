//! Everything one batch of checks shares, loaded once by the caller and threaded through
//! `run_checks_for_trigger`: the `accepted/` entries and how inline ignores apply.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;

use crate::accepted_findings::{AcceptedFindings, find_accepted_findings};
use crate::inline_ignores::{InlineIgnoreContext, InlineIgnoreMode, SuppressionCounts};

#[derive(Debug, Clone, Default)]
pub struct RunContext {
    pub accepted: AcceptedFindings,
    pub inline: InlineIgnoreContext,
    /// Skip the post-pass advisories for an unscoped (whole-file) run: `kibitzer run` and the
    /// LSP report suppressions through their own footer or not at all, and have no agent to tell.
    pub skip_whole_file_advisories: bool,
    /// Where the hook remembers which blocking suppressions it has already reported per file;
    /// `None` (the default) keeps every run stateless.
    pub advised_dir: Option<std::path::PathBuf>,
    /// The edit removed text whose position is unknown, so a finding may have slid under a directive.
    pub unlocated_deletion: bool,
}

impl RunContext {
    pub fn new(accepted: AcceptedFindings) -> Self {
        RunContext {
            accepted,
            inline: InlineIgnoreContext::default(),
            skip_whole_file_advisories: false,
            advised_dir: None,
            unlocated_deletion: false,
        }
    }

    /// Loads the `accepted/` entries above `start`; inline ignores start in their default mode.
    pub fn load(start: &Path) -> Result<Self> {
        let mut ctx = find_accepted_findings(start).map(Self::new)?;
        ctx.advised_dir = crate::cache::default_cache_path()
            .parent()
            .map(|dir| dir.join("advised"));
        Ok(ctx)
    }

    /// For one `kibitzer run` batch: a fresh suppression counter (never global, so parallel
    /// runs and tests cannot share a count) and the requested inline-ignore `mode`.
    pub fn for_batch(repo_root: &Path, mode: InlineIgnoreMode) -> Result<Self> {
        let mut ctx = Self::load(repo_root)?;
        ctx.inline = InlineIgnoreContext {
            mode,
            counter: Some(Arc::new(SuppressionCounts::default())),
            ..InlineIgnoreContext::default()
        };
        ctx.skip_whole_file_advisories = true;
        ctx.advised_dir = None;
        Ok(ctx)
    }
}
