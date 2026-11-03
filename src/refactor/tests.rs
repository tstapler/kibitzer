#![allow(non_snake_case)]

use std::path::PathBuf;

use super::catalog::{list_operations_for_package, plan_move_files};
use super::feasibility::check;
use super::{BlockingReason, Edit, OperationKind, RefactorOperation, Warning};
use crate::arch_model::{ArchModel, PruneConfig, build_model_from_files};

const MOD: &str = "example.com/app";

const A1: &str = "package a\n\nfunc A1() {}\n";
const A2: &str = "package a\n\nfunc A2() int { return 1 }\n";
const A_TEST: &str = "package a\n\nfunc helper() {}\n";
const USER1: &str = "package user1\n\nimport \"example.com/app/a\"\n\nfunc U() { a.A1() }\n";
const USER2: &str =
    "package user2\n\nimport \"example.com/app/a\"\n\nfunc U() int { return a.A2() }\n";

fn fixture(label: &str, files: &[(&str, &str)]) -> (PathBuf, ArchModel) {
    let dir = crate::test_support::unique_temp_dir(label);
    std::fs::write(dir.join("go.mod"), format!("module {MOD}\n\ngo 1.21\n")).unwrap();
    let mut paths = Vec::new();
    for (rel, src) in files {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, src).unwrap();
        paths.push(path);
    }
    let model = build_model_from_files(
        &dir,
        &paths,
        &PruneConfig {
            include_private: true,
        },
    )
    .unwrap();
    (dir, model)
}

fn op(source: &str, target: &str, files: &[&str]) -> RefactorOperation {
    RefactorOperation {
        kind: OperationKind::MoveFiles,
        source_package: format!("{MOD}/{source}"),
        target_package: format!("{MOD}/{target}"),
        files: files.iter().map(PathBuf::from).collect(),
    }
}

fn base_files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("a/a1.go", A1),
        ("a/a2.go", A2),
        ("a/a_test.go", A_TEST),
        ("user1/user1.go", USER1),
        ("user2/user2.go", USER2),
    ]
}

fn names(edits: &[Edit]) -> Vec<&'static str> {
    edits
        .iter()
        .map(|e| match e {
            Edit::MoveFile { .. } => "move",
            Edit::ReplacePackageDecl { .. } => "decl",
            Edit::ReplaceImportPath { .. } => "import",
        })
        .collect()
}

fn assert_tree_unchanged(dir: &std::path::Path, files: &[(&str, &str)]) {
    for (rel, src) in files {
        assert_eq!(
            &std::fs::read_to_string(dir.join(rel)).unwrap(),
            src,
            "{rel}"
        );
    }
}

#[test]
fn check_should_BeFeasibleAndWriteNothing_When_SingleFileMoveIsAcyclic() {
    let files = base_files();
    let (dir, model) = fixture("refactor-single-feasible", &files);
    let report = check(&model, &op("a", "b", &["a/a1.go"]));
    assert!(report.feasible, "got: {report:?}");
    assert_eq!(report.importers_affected, vec![format!("{MOD}/user1")]);
    assert!(report.touched_files.iter().any(|f| f.ends_with("a/a1.go")));
    assert!(
        report
            .touched_files
            .iter()
            .any(|f| f.ends_with("user1/user1.go"))
    );
    assert!(
        !report
            .touched_files
            .iter()
            .any(|f| f.ends_with("user2/user2.go"))
    );
    assert!(
        !dir.join("b").exists(),
        "simulate must not create the target"
    );
    assert_tree_unchanged(&dir, &files);
}

#[test]
fn check_should_BlockOnCycle_When_TargetImportsSourceAndSourceCallsMovedSymbol() {
    let (_dir, model) = fixture(
        "refactor-cycle",
        &[
            ("a/a1.go", A1),
            ("a/a2.go", "package a\n\nfunc A2() int { A1(); return 1 }\n"),
            (
                "lib/lib.go",
                "package lib\n\nimport \"example.com/app/a\"\n\nfunc L() int { return a.A2() }\n",
            ),
        ],
    );
    let report = check(&model, &op("a", "lib", &["a/a1.go"]));
    assert!(!report.feasible);
    assert!(
        report
            .blocking
            .iter()
            .any(|b| matches!(b, BlockingReason::ImportCycle { cycle_path } if cycle_path.first() == cycle_path.last())),
        "got: {report:?}"
    );
}

#[test]
fn check_should_BlockOnInternalViolation_When_ImporterIsOutsideInternalParent() {
    let files = base_files();
    let (_dir, model) = fixture("refactor-internal", &files);
    let report = check(&model, &op("a", "x/internal/y", &["a/a1.go"]));
    assert!(!report.feasible);
    assert!(
        report.blocking.iter().any(|b| matches!(
            b,
            BlockingReason::InternalVisibilityViolation { importer, .. } if importer == &format!("{MOD}/user1")
        )),
        "got: {report:?}"
    );
}

#[test]
fn check_should_BlockOnCgo_When_MovedFileImportsC() {
    let (_dir, model) = fixture(
        "refactor-cgo",
        &[
            (
                "a/a1.go",
                "package a\n\n/*\n#include <stdio.h>\n*/\nimport \"C\"\n\nfunc A1() {}\n",
            ),
            ("a/a2.go", A2),
        ],
    );
    let report = check(&model, &op("a", "b", &["a/a1.go"]));
    assert!(
        report
            .blocking
            .iter()
            .any(|b| matches!(b, BlockingReason::CgoPackage { file } if file.ends_with("a/a1.go"))),
        "got: {report:?}"
    );
}

#[test]
fn check_should_WarnNotBlock_When_FilesAreBuildTaggedOrGenerated() {
    let (dir, model) = fixture(
        "refactor-warnings",
        &[
            ("a/a1.go", "//go:build linux\n\npackage a\n\nfunc A1() {}\n"),
            ("a/a2.go", A2),
        ],
    );
    let generated = "// Code generated by gen. DO NOT EDIT.\n\npackage a\n\nfunc G() {}\n";
    std::fs::write(dir.join("a/gen.go"), generated).unwrap();
    let report = check(&model, &op("a", "b", &["a/a1.go", "a/gen.go"]));
    assert!(report.feasible, "got: {report:?}");
    assert!(
        report
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::BuildTaggedFiles { .. }))
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|w| matches!(w, Warning::GeneratedFiles { .. }))
    );
}

#[test]
fn check_should_Block_When_FileIsNotInSourcePackage() {
    let files = base_files();
    let (_dir, model) = fixture("refactor-bad-file", &files);
    let report = check(&model, &op("a", "b", &["user1/user1.go"]));
    assert!(matches!(
        report.blocking.as_slice(),
        [BlockingReason::FileNotInPackage { .. }]
    ));
}

#[test]
fn plan_should_RewriteDeclImportsAndMove_When_SingleFileMove() {
    let files = base_files();
    let (dir, model) = fixture("refactor-plan-single", &files);
    let plan = plan_move_files(&model, &op("a", "b", &["a/a1.go"])).unwrap();
    assert_eq!(names(&plan.edits), vec!["decl", "import", "move"]);
    let import = plan
        .edits
        .iter()
        .find_map(|e| match e {
            Edit::ReplaceImportPath {
                file,
                new_path,
                alias,
                ..
            } => Some((file, new_path, alias)),
            _ => None,
        })
        .unwrap();
    assert!(import.0.ends_with("user1/user1.go"));
    assert_eq!(import.1, &format!("{MOD}/b"));
    assert_eq!(import.2.as_deref(), Some("a"), "alias keeps a.A1() valid");
    assert!(plan.touched.iter().all(|t| t.sha256.len() == 64));
    assert_eq!(
        plan.verification_cmds,
        vec!["go build ./...", "go vet ./..."]
    );
    assert_eq!(plan.id.len(), 64);
    assert_tree_unchanged(&dir, &files);
}

#[test]
fn plan_should_CoverAllImportersAndTestPackage_When_WholePackageMoveWithTestFile() {
    let mut files = base_files();
    files.push(("a/ext_test.go", "package a_test\n\nfunc ExtHelper() {}\n"));
    let (_dir, model) = fixture("refactor-plan-multi", &files);
    let plan = plan_move_files(&model, &op("a", "b", &[])).unwrap();
    let decls: Vec<(&str, &str)> = plan
        .edits
        .iter()
        .filter_map(|e| match e {
            Edit::ReplacePackageDecl {
                old_name, new_name, ..
            } => Some((old_name.as_str(), new_name.as_str())),
            _ => None,
        })
        .collect();
    assert!(decls.contains(&("a", "b")), "got: {decls:?}");
    assert!(decls.contains(&("a_test", "b_test")), "got: {decls:?}");
    let imports = names(&plan.edits)
        .iter()
        .filter(|n| **n == "import")
        .count();
    assert_eq!(imports, 2, "both importers rewritten");
    let moves = names(&plan.edits).iter().filter(|n| **n == "move").count();
    assert_eq!(moves, 4);
}

#[test]
fn plan_should_Error_When_OperationIsInfeasible() {
    let files = base_files();
    let (_dir, model) = fixture("refactor-plan-infeasible", &files);
    let err = plan_move_files(&model, &op("a", "x/internal/y", &["a/a1.go"])).unwrap_err();
    assert!(err.to_string().contains("not feasible"), "got: {err}");
}

#[test]
fn plan_should_Error_When_MovedAndRemainingSymbolsCallEachOther() {
    let (_dir, model) = fixture(
        "refactor-plan-entangled",
        &[
            ("a/a1.go", A1),
            ("a/a2.go", "package a\n\nfunc A2() int { A1(); return 1 }\n"),
        ],
    );
    let err = plan_move_files(&model, &op("a", "b", &["a/a1.go"])).unwrap_err();
    assert!(err.to_string().contains("call each other"), "got: {err}");
}

#[test]
fn list_operations_should_ReturnEmptyNotError_When_NothingIsSupported() {
    let (_dir, model) = fixture(
        "refactor-list",
        &[
            ("a/a1.go", A1),
            ("a/a2.go", A2),
            ("solo/solo.go", "package solo\n\nfunc S() {}\n"),
        ],
    );
    assert!(list_operations_for_package(&model, &format!("{MOD}/solo")).is_empty());
    assert!(list_operations_for_package(&model, "nope").is_empty());
    let ops = list_operations_for_package(&model, &format!("{MOD}/a"));
    assert_eq!(ops.len(), 2);
}

#[test]
fn check_should_Block_When_TargetEqualsSource() {
    let (_dir, model) = fixture("refactor-same", &[("a/a1.go", A1), ("a/a2.go", A2)]);
    let same = check(&model, &op("a", "a", &["a/a1.go"]));
    assert!(matches!(
        same.blocking.as_slice(),
        [BlockingReason::SamePackage { .. }]
    ));
}

#[test]
fn check_should_Block_When_MovedSymbolCollidesWithTargetSymbol() {
    let (_dir, model) = fixture(
        "refactor-collision",
        &[
            ("a/a1.go", A1),
            ("a/a2.go", A2),
            ("b/b.go", "package b\n\nfunc A1() {}\n"),
        ],
    );
    let report = check(&model, &op("a", "b", &["a/a1.go"]));
    assert!(
        report
            .blocking
            .iter()
            .any(|b| matches!(b, BlockingReason::NameCollision { symbol } if symbol == "A1"))
    );
}

#[test]
fn check_should_Block_When_TargetPackageFileImportsSource() {
    let (_dir, model) = fixture(
        "refactor-target-importer",
        &[
            ("a/a1.go", A1),
            ("a/a2.go", A2),
            (
                "c/c.go",
                "package c\n\nimport \"example.com/app/a\"\n\nfunc C() { a.A1() }\n",
            ),
        ],
    );
    let report = check(&model, &op("a", "c", &["a/a1.go"]));
    assert!(
        report
            .blocking
            .iter()
            .any(|b| matches!(b, BlockingReason::TargetPackageImporter { .. })),
        "got: {report:?}"
    );
}

mod apply_tests {
    use std::cell::Cell;
    use std::path::Path;

    use super::*;
    use crate::refactor::RefactorPlan;
    use crate::refactor::apply::{ApplyIo, RealIo, apply_plan, recover};

    /// Fails the Nth rename and/or every verification command, otherwise behaves like the real thing.
    struct FakeIo {
        fail_rename_at: Option<usize>,
        fail_verify: bool,
        renames: Cell<usize>,
    }

    impl FakeIo {
        fn new(fail_rename_at: Option<usize>, fail_verify: bool) -> Self {
            Self {
                fail_rename_at,
                fail_verify,
                renames: Cell::new(0),
            }
        }
    }

    impl ApplyIo for FakeIo {
        fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
            let n = self.renames.get();
            self.renames.set(n + 1);
            if self.fail_rename_at == Some(n) {
                return Err(std::io::Error::other("injected rename failure"));
            }
            std::fs::rename(from, to)
        }

        fn run(&self, cmd: &str, _dir: &Path) -> (bool, String) {
            if self.fail_verify {
                (false, format!("{cmd}: ./b.go:3: injected build error"))
            } else {
                (true, String::new())
            }
        }
    }

    fn plan_for(
        label: &str,
        files: &[(&str, &str)],
        o: &RefactorOperation,
    ) -> (PathBuf, RefactorPlan) {
        let (dir, model) = fixture(label, files);
        let plan = plan_move_files(&model, o).unwrap();
        (dir, plan)
    }

    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join(rel)).unwrap()
    }

    #[test]
    fn apply_should_MoveFileAndRewriteImporter_When_SingleFileMove() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-single", &files, &op("a", "b", &["a/a1.go"]));
        let result = apply_plan(&dir, &plan, &RealIo).unwrap();
        assert_eq!(result.verification.len(), 2, "go build and go vet both ran");
        assert!(!dir.join("a/a1.go").exists());
        assert_eq!(read(&dir, "b/a1.go"), "package b\n\nfunc A1() {}\n");
        assert!(read(&dir, "user1/user1.go").contains("a \"example.com/app/b\""));
        assert_eq!(read(&dir, "user2/user2.go"), USER2, "untouched importer");
        assert!(
            !dir.join(".kibitzer/plans/.wal").exists(),
            "WAL removed on success"
        );
    }

    #[test]
    fn apply_should_RenameTestPackages_When_WholePackageMoveWithTestFiles() {
        let mut files = base_files();
        files.push(("a/ext_test.go", "package a_test\n\nfunc ExtHelper() {}\n"));
        let (dir, plan) = plan_for("apply-multi", &files, &op("a", "b", &[]));
        apply_plan(&dir, &plan, &RealIo).unwrap();
        for gone in ["a/a1.go", "a/a2.go", "a/a_test.go", "a/ext_test.go"] {
            assert!(!dir.join(gone).exists(), "{gone}");
        }
        assert!(read(&dir, "b/a_test.go").starts_with("package b\n"));
        assert!(read(&dir, "b/ext_test.go").starts_with("package b_test\n"));
        assert!(read(&dir, "user1/user1.go").contains("\"example.com/app/b\""));
        assert!(read(&dir, "user2/user2.go").contains("\"example.com/app/b\""));
    }

    #[test]
    fn apply_should_RejectBeforeAnyWrite_When_TouchedFileChangedSincePlanning() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-stale", &files, &op("a", "b", &["a/a1.go"]));
        std::fs::write(dir.join("user1/user1.go"), format!("{USER1}// edited\n")).unwrap();
        let err = apply_plan(&dir, &plan, &RealIo).unwrap_err();
        assert!(format!("{err:#}").contains("stale plan"), "got: {err:#}");
        assert!(dir.join("a/a1.go").exists());
        assert!(!dir.join("b").exists());
        assert!(
            !dir.join(".kibitzer/plans/.wal").exists(),
            "rejected before the WAL"
        );
    }

    #[test]
    fn apply_should_RestoreTree_When_BuildFails() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-build-fail", &files, &op("a", "b", &["a/a1.go"]));
        let err = apply_plan(&dir, &plan, &FakeIo::new(None, true)).unwrap_err();
        assert!(
            format!("{err:#}").contains("injected build error"),
            "output is the error"
        );
        assert_tree_unchanged(&dir, &files);
        assert!(!dir.join("b").exists(), "created directory removed");
        assert!(!dir.join(".kibitzer/plans/.wal").exists());
    }

    #[test]
    fn apply_should_RestoreTree_When_RenameFailsMidLoop() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-rename-fail", &files, &op("a", "b", &["a/a1.go"]));
        let err = apply_plan(&dir, &plan, &FakeIo::new(Some(1), false)).unwrap_err();
        assert!(format!("{err:#}").contains("injected rename failure"));
        assert_tree_unchanged(&dir, &files);
        assert!(!dir.join("b").exists());
    }

    #[test]
    fn recover_should_RollBackPartialApply_When_WalIsOrphaned() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-orphan", &files, &op("a", "b", &["a/a1.go"]));
        // Simulate a kill after the first rename: the process dies without cleanup.
        let wal_dir = dir.join(".kibitzer/plans");
        crate::refactor_core::txn::simulate_kill(true);
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            struct Crash;
            impl ApplyIo for Crash {
                fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
                    std::fs::rename(from, to)?;
                    panic!("simulated crash");
                }
                fn run(&self, _: &str, _: &Path) -> (bool, String) {
                    (true, String::new())
                }
            }
            let _ = apply_plan(&dir, &plan, &Crash);
        }));
        crate::refactor_core::txn::simulate_kill(false);
        assert!(crashed.is_err());
        assert!(wal_dir.join(".wal").exists(), "WAL survives the crash");
        assert!(dir.join("b/a1.go").exists(), "tree is partially applied");

        assert!(recover(&dir).unwrap().is_some());
        assert_tree_unchanged(&dir, &files);
        assert!(!dir.join("b").exists());
        assert!(!wal_dir.join(".wal").exists());
        assert_eq!(recover(&dir).unwrap(), None, "idempotent");
    }

    #[test]
    fn apply_should_Reject_When_PlanWasTamperedWith() {
        let files = base_files();
        let (dir, mut plan) = plan_for("apply-tamper", &files, &op("a", "b", &["a/a1.go"]));
        plan.verification_cmds.push("rm -rf /".into());
        let err = apply_plan(&dir, &plan, &RealIo).unwrap_err();
        assert!(
            format!("{err:#}").contains("plan id does not match"),
            "got: {err:#}"
        );
        assert_tree_unchanged(&dir, &files);
    }

    fn resign(plan: &mut RefactorPlan) {
        use sha2::{Digest, Sha256};
        plan.id = String::new();
        let json = serde_json::to_string(&*plan).unwrap();
        plan.id = Sha256::digest(json.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
    }

    type Forge = fn(&mut RefactorPlan, &Path);

    /// A plan an MCP client could forge: valid id, hostile content. Each must be refused before
    /// any write, with the tree untouched.
    #[test]
    fn apply_should_Refuse_When_ForgedPlanIsValidlySigned() {
        let cases: Vec<(&str, &str, Forge)> = vec![
            ("disallowed command", "not allowed", |p, _| {
                p.verification_cmds.push("rm -rf /".into());
            }),
            ("bad package name", "not a valid Go package name", |p, _| {
                for e in &mut p.edits {
                    if let Edit::ReplacePackageDecl { new_name, .. } = e {
                        *new_name = "a b".into();
                    }
                }
            }),
            (
                "import path injection",
                "not a valid import path",
                |p, _| {
                    for e in &mut p.edits {
                        if let Edit::ReplaceImportPath { new_path, .. } = e {
                            *new_path = "x\"\nimport \"os".into();
                        }
                    }
                },
            ),
            ("unrecorded edit target", "did not record", |p, _| {
                p.touched.remove(0);
            }),
            (
                "destination escapes via ..",
                "outside the repository",
                |p, dir| {
                    for e in &mut p.edits {
                        if let Edit::MoveFile { to, .. } = e {
                            *to = dir.join("../../escaped.go");
                        }
                    }
                },
            ),
            (
                "touched path outside root",
                "outside the repository",
                |p, _| {
                    let mut extra = p.touched[0].clone();
                    extra.path = PathBuf::from("/etc/x.go");
                    p.touched.push(extra);
                },
            ),
        ];
        for (name, want, forge) in cases {
            let files = base_files();
            let (dir, mut plan) = plan_for("apply-forged", &files, &op("a", "b", &["a/a1.go"]));
            forge(&mut plan, &dir);
            resign(&mut plan);
            let err = format!(
                "{:#}",
                apply_plan(&dir, &plan, &FakeIo::new(None, false)).unwrap_err()
            );
            assert!(err.contains(want), "{name}: got {err}");
            assert_tree_unchanged(&dir, &files);
            assert!(
                !dir.join(".kibitzer/plans/.wal").exists(),
                "{name}: WAL written"
            );
        }
    }

    #[test]
    fn is_inside_should_Reject_When_PathUsesDotDot() {
        let root = crate::test_support::unique_temp_dir("inside-dotdot");
        assert!(crate::refactor_core::guard::is_inside(
            &root,
            &root.join("a/b.go")
        ));
        assert!(!crate::refactor_core::guard::is_inside(
            &root,
            &root.join("../../etc/x")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn is_inside_should_Reject_When_SymlinkLeavesRoot() {
        let root = crate::test_support::unique_temp_dir("inside-link");
        let outside = crate::test_support::unique_temp_dir("inside-outside");
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        assert!(!crate::refactor_core::guard::is_inside(
            &root,
            &root.join("link/x.go")
        ));
    }

    #[test]
    fn recover_should_Refuse_When_WalNamesPathOutsideRepo() {
        let dir = crate::test_support::unique_temp_dir("hostile-wal");
        let plans = dir.join(".kibitzer/plans");
        std::fs::create_dir_all(plans.join(".wal-backup")).unwrap();
        std::fs::write(plans.join(".wal-backup/0"), "owned").unwrap();
        let victim = crate::test_support::unique_temp_dir("hostile-victim").join("bashrc");
        std::fs::write(&victim, "original").unwrap();
        let wal = serde_json::json!({
            "plan_id": "x",
            "created_dirs": [],
            "entries": [{
                "path": victim, "backup": plans.join(".wal-backup/0"),
                "temp": dir.join("t.go.tmp")
            }]
        });
        std::fs::write(plans.join(".wal"), wal.to_string()).unwrap();
        let err = recover(&dir).unwrap_err();
        assert!(
            format!("{err:#}").contains("outside the repository"),
            "got {err:#}"
        );
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original");
    }

    #[test]
    fn apply_should_Refuse_The_Second_Apply_Of_The_Same_Plan_As_Stale() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-twice", &files, &op("a", "b", &["a/a1.go"]));
        apply_plan(&dir, &plan, &RealIo).unwrap();
        let after_first = std::fs::read_to_string(dir.join("b/a1.go")).unwrap();
        let err = apply_plan(&dir, &plan, &RealIo).unwrap_err();
        assert!(
            format!("{err:#}").contains("stale plan")
                || format!("{err:#}").contains("already exists"),
            "second apply must be refused, got: {err:#}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("b/a1.go")).unwrap(),
            after_first
        );
    }

    #[test]
    fn apply_should_Recover_An_Orphan_Before_Judging_Staleness() {
        let files = base_files();
        let (dir, plan) = plan_for("apply-orphan-first", &files, &op("a", "b", &["a/a1.go"]));
        // A killed apply leaves the tree half-moved and the log behind.
        crate::refactor_core::txn::simulate_kill(true);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            struct Crash;
            impl ApplyIo for Crash {
                fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
                    std::fs::rename(from, to)?;
                    panic!("simulated crash");
                }
                fn run(&self, _: &str, _: &Path) -> (bool, String) {
                    (true, String::new())
                }
            }
            let _ = apply_plan(&dir, &plan, &Crash);
        }));
        crate::refactor_core::txn::simulate_kill(false);
        // The same plan, applied again: recovery must run first, so it is judged against the
        // restored tree and succeeds instead of failing "stale" against the half-applied one.
        apply_plan(&dir, &plan, &RealIo).unwrap();
        assert!(dir.join("b/a1.go").exists() && !dir.join("a/a1.go").exists());
    }

    #[test]
    fn apply_should_Refuse_A_Package_Name_The_File_Cannot_Take() {
        let files = base_files();
        let (dir, mut plan) = plan_for("apply-name-mismatch", &files, &op("a", "b", &["a/a1.go"]));
        for e in &mut plan.edits {
            if let Edit::ReplacePackageDecl { new_name, .. } = e {
                // The file's clause is not `_test`, so the shared rewriter cannot produce this.
                *new_name = "bee_test".into();
            }
        }
        resign(&mut plan);
        let err = apply_plan(&dir, &plan, &RealIo).unwrap_err();
        assert!(
            format!("{err:#}").contains("would not become"),
            "got: {err:#}"
        );
        assert_tree_unchanged(&dir, &files);
    }

    #[test]
    fn verification_failure_output_returned_to_clients_is_capped() {
        let big = "x".repeat(200 * 1024);
        let capped = crate::refactor::apply::cap_output(&big);
        assert!(capped.len() < 70 * 1024, "{}", capped.len());
        assert!(capped.contains("output truncated"));
        assert_eq!(crate::refactor::apply::cap_output("short"), "short");
        // Multi-byte text must be cut on a character boundary.
        let multi = "é".repeat(100 * 1024);
        assert!(crate::refactor::apply::cap_output(&multi).is_char_boundary(0));
    }
}
