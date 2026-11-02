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
