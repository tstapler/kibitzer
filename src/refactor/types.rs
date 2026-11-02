//! Serializable plan/result types for the `refactor go` operations.
//!
//! Paths inside a plan are repo-relative with `/` separators so a plan is portable and
//! golden-testable; only `repo_root` is machine-specific.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    MoveFiles,
    RenamePkg,
    ExtractDecls,
    SplitPkg,
    SyncPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepKind {
    RewriteFile,
    MoveFile,
    CreateFile,
    DeleteFile,
    PolicyUpdate,
}

/// One file's before/after state. `old_sha256` pins the pre-apply content so a stale or
/// half-applied plan is detected instead of overwriting unrelated edits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileTransform {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStep {
    pub kind: PlanStepKind,
    pub transform: FileTransform,
    pub reason: String,
}

/// A pre-flight finding that forbids applying the plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockingDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

impl BlockingDiagnostic {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
            file: None,
            line: None,
        }
    }

    pub fn at(mut self, file: &str, line: Option<usize>) -> Self {
        self.file = Some(file.to_string());
        self.line = line;
        self
    }
}

/// A directory, file, or import path that changed; `sync-policy` rewrites repo policy by it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PathMapping {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovePlan {
    pub operation: OperationKind,
    pub repo_root: String,
    pub module: String,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub steps: Vec<PlanStep>,
    pub blockers: Vec<BlockingDiagnostic>,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Exported-surface changes (symbol now lives at a different import path).
    #[serde(default)]
    pub surface_changes: Vec<String>,
    #[serde(default)]
    pub path_map: Vec<PathMapping>,
    /// Repo-relative package directories (`./pkg/a`) to build/vet/test after apply.
    #[serde(default)]
    pub affected_packages: Vec<String>,
    /// Feasibility-simulation output (#153), embedded verbatim; a reported blocker
    /// becomes a `simulation_blocker` diagnostic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulation_result: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandOutcome {
    pub command: String,
    pub exit_code: Option<i32>,
    /// stdout then stderr, verbatim.
    pub output: String,
}

impl CommandOutcome {
    pub fn ok(&self) -> bool {
        self.exit_code == Some(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_apply_hook: Option<CommandOutcome>,
    pub build: CommandOutcome,
    pub vet: CommandOutcome,
    pub test: CommandOutcome,
}

impl VerifyResult {
    pub fn ok(&self) -> bool {
        self.post_apply_hook.as_ref().is_none_or(CommandOutcome::ok)
            && self.build.ok()
            && self.vet.ok()
            && self.test.ok()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Applied,
    /// The tree already matched the plan's target; nothing was written.
    AlreadyApplied,
    /// Verification failed; every touched path was restored.
    RolledBack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyResult {
    pub status: ApplyStatus,
    pub applied_steps: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verify: Option<VerifyResult>,
    /// Paths that could not be restored after a rollback (empty when the rollback was clean).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub restore_errors: Vec<String>,
    pub plan: MovePlan,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_round_trips_through_json() {
        let plan = MovePlan {
            operation: OperationKind::MoveFiles,
            repo_root: ".".into(),
            module: "example.com/m".into(),
            from: vec!["example.com/m/a".into()],
            to: vec!["example.com/m/b".into()],
            steps: vec![PlanStep {
                kind: PlanStepKind::MoveFile,
                transform: FileTransform {
                    old_path: Some("a/x.go".into()),
                    old_sha256: Some("00".into()),
                    new_path: Some("b/x.go".into()),
                    new_content: Some("package b\n".into()),
                },
                reason: "r".into(),
            }],
            blockers: vec![BlockingDiagnostic::new("c", "m").at("a/x.go", Some(3))],
            warnings: vec![],
            surface_changes: vec![],
            path_map: vec![],
            affected_packages: vec!["./b".into()],
            simulation_result: None,
        };
        let json = serde_json::to_string_pretty(&plan).unwrap();
        let back: MovePlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back.steps, plan.steps);
        assert_eq!(back.blockers, plan.blockers);
        assert!(!json.contains("simulation_result"));
    }
}
