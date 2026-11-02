//! Mechanical, behavior-preserving refactoring engine.
//!
//! [`feasibility::check`] decides whether an operation is possible without writing anything;
//! [`catalog::plan_move_files`] turns a feasible one into a serializable [`RefactorPlan`];
//! [`apply::apply_plan`] is the only code here that writes, atomically and with a WAL.

pub mod apply;
pub mod catalog;
pub mod feasibility;
mod go_syntax;

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum OperationKind {
    MoveFiles,
    ExtractPackage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefactorOperation {
    pub kind: OperationKind,
    /// Module-qualified import path of the package the files leave.
    pub source_package: String,
    /// Module-qualified import path of the package the files join (created if absent).
    pub target_package: String,
    /// Files to move, absolute or repo-relative. Empty means every file in the package.
    #[serde(default)]
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum BlockingReason {
    UnknownSourcePackage {
        package: String,
    },
    FileNotInPackage {
        file: PathBuf,
    },
    ImportCycle {
        cycle_path: Vec<String>,
    },
    InternalVisibilityViolation {
        importer: String,
        target: String,
    },
    CgoPackage {
        file: PathBuf,
    },
    /// A file calls both moved and remaining symbols of the source package, so rewriting its
    /// import would need a second import rather than a path swap.
    MixedImporter {
        file: PathBuf,
    },
    /// Source and target are the same package.
    SamePackage {
        package: String,
    },
    /// A moved symbol shares a name with one already in the target package.
    NameCollision {
        symbol: String,
    },
    /// A file already in the target package imports the source and uses moved symbols; its
    /// import would become a self-import and its qualifiers would need removing.
    TargetPackageImporter {
        file: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "warning", rename_all = "snake_case")]
pub enum Warning {
    /// Build constraints are not evaluated by the model; verify on every target platform.
    BuildTaggedFiles {
        files: Vec<PathBuf>,
    },
    GeneratedFiles {
        files: Vec<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeasibilityReport {
    pub feasible: bool,
    pub blocking: Vec<BlockingReason>,
    pub warnings: Vec<Warning>,
    pub touched_files: Vec<PathBuf>,
    pub importers_affected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "edit", rename_all = "snake_case")]
pub enum Edit {
    MoveFile {
        from: PathBuf,
        to: PathBuf,
    },
    ReplacePackageDecl {
        file: PathBuf,
        old_name: String,
        new_name: String,
    },
    /// Swap one import path; `alias` keeps existing `pkg.Sym` selectors valid when the new
    /// package name differs from the old one.
    ReplaceImportPath {
        file: PathBuf,
        old_path: String,
        new_path: String,
        alias: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TouchedFile {
    pub path: PathBuf,
    /// Hex SHA-256 of the file's bytes at planning time; a later apply compares it to
    /// detect a stale plan.
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RefactorPlan {
    pub id: String,
    pub operation: RefactorOperation,
    pub edits: Vec<Edit>,
    pub verification_cmds: Vec<String>,
    pub touched: Vec<TouchedFile>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvailableOperation {
    pub kind: OperationKind,
    pub source_package: String,
    pub description: String,
}

#[cfg(test)]
mod tests;
