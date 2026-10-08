//! Everything one batch of checks shares, loaded once by the caller and threaded through
//! `run_checks_for_trigger`: the `accepted/` entries and how inline ignores apply.

use std::path::Path;

use anyhow::Result;

use crate::accepted_findings::{AcceptedFindings, find_accepted_findings};
use crate::inline_ignores::InlineIgnoreContext;

#[derive(Debug, Clone, Default)]
pub struct RunContext {
    pub accepted: AcceptedFindings,
    pub inline: InlineIgnoreContext,
}

impl RunContext {
    pub fn new(accepted: AcceptedFindings) -> Self {
        RunContext {
            accepted,
            inline: InlineIgnoreContext::default(),
        }
    }

    /// Loads the `accepted/` entries above `start`; inline ignores start in their default mode.
    pub fn load(start: &Path) -> Result<Self> {
        find_accepted_findings(start).map(Self::new)
    }
}
