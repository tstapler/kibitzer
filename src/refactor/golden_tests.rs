//! Golden-file harness: `testdata/refactor/<operation>/<case>/{op.json, plan.json, after.txt}`
//! with the pre-apply tree in `testdata/refactor/_inputs/<op.json "input">/`.
//! `KIBITZER_BLESS=1 cargo test refactor::golden` rewrites the expected files; review the diff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use super::atomic_apply::{ApplyOptions, apply};
use super::plan::sha256_hex;
use super::types::{ApplyStatus, MovePlan, PlanStepKind};
use super::*;

fn data_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/refactor")
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!(
        "kibitzer-refactor-{}-{}-{tag}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let e = e.unwrap();
        let to = dst.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &to);
        } else {
            std::fs::copy(e.path(), to).unwrap();
        }
    }
}

fn input_copy(name: &str) -> PathBuf {
    let d = temp_dir(name);
    copy_dir(&data_root().join("_inputs").join(name), &d);
    d
}

fn checksums(root: &Path) -> BTreeMap<String, String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                walk(root, &e.path(), out);
            } else {
                let rel = e
                    .path()
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, sha256_hex(&std::fs::read(e.path()).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn snapshot(root: &Path) -> String {
    checksums(root)
        .keys()
        .map(|rel| {
            format!(
                "=== {rel} ===\n{}\n",
                std::fs::read_to_string(root.join(rel)).unwrap()
            )
        })
        .collect()
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect()
}

fn run_plan(root: &Path, op: &Value) -> MovePlan {
    let sim = op.get("simulation").cloned();
    let s = |k: &str| op[k].as_str().unwrap().to_string();
    match op["operation"].as_str().unwrap() {
        "move_files" => plan_move_files(root, &strs(&op["src"]), &s("dest_pkg"), sim).unwrap(),
        "rename_pkg" => {
            plan_rename_pkg(root, &s("from"), &s("to"), op["name"].as_str(), sim).unwrap()
        }
        "extract_decls" => {
            plan_extract_decls(root, &s("from"), &strs(&op["decls"]), &s("to"), sim).unwrap()
        }
        "split_pkg" => {
            let clusters: Vec<(String, Vec<String>)> = op["clusters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| (c["to"].as_str().unwrap().to_string(), strs(&c["files"])))
                .collect();
            plan_split_pkg(root, &s("pkg"), &clusters, sim).unwrap()
        }
        "sync_policy" => plan_sync_policy(root, &run_plan(root, &op["source"])).unwrap(),
        other => panic!("unknown operation {other}"),
    }
}

fn go_available() -> bool {
    std::process::Command::new("go")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success())
}

fn run_case(case_dir: &Path) {
    let op: Value =
        serde_json::from_str(&std::fs::read_to_string(case_dir.join("op.json")).unwrap()).unwrap();
    let name = case_dir
        .strip_prefix(data_root())
        .unwrap()
        .display()
        .to_string();
    let bless = std::env::var_os("KIBITZER_BLESS").is_some();
    let root = input_copy(op["input"].as_str().unwrap());
    let before = checksums(&root);

    let mut plan = run_plan(&root, &op);
    assert_eq!(
        checksums(&root),
        before,
        "{name}: planning must not touch the tree"
    );
    plan.repo_root = "<repo>".into();
    let plan_json = format!("{}\n", serde_json::to_string_pretty(&plan).unwrap());
    let expected_path = case_dir.join("plan.json");
    if bless {
        std::fs::write(&expected_path, &plan_json).unwrap();
    }
    let expected = std::fs::read_to_string(&expected_path)
        .unwrap_or_else(|_| panic!("{name}: missing plan.json (run with KIBITZER_BLESS=1)"));
    assert_eq!(plan_json, expected, "{name}: plan differs from golden");

    let after_path = case_dir.join("after.txt");
    if !plan.blockers.is_empty() {
        let err =
            apply(&plan, &root, &ApplyOptions::default()).expect_err("blocked plan must not apply");
        assert!(err.to_string().contains("blocker"), "{name}: {err}");
        assert_eq!(
            checksums(&root),
            before,
            "{name}: refused apply must not touch the tree"
        );
        assert!(
            !after_path.exists(),
            "{name}: blocked case must not have after.txt"
        );
    } else {
        let policy_only = plan.operation == types::OperationKind::SyncPolicy;
        let opts = ApplyOptions {
            verify: go_available() && !policy_only,
            ..Default::default()
        };
        let result =
            apply(&plan, &root, &opts).unwrap_or_else(|e| panic!("{name}: apply failed: {e:#}"));
        assert_eq!(result.status, ApplyStatus::Applied, "{name}");
        if let Some(v) = &result.verify {
            assert!(
                v.ok(),
                "{name}: verification failed:\n{}\n{}\n{}",
                v.build.output,
                v.vet.output,
                v.test.output
            );
        }
        let tree = snapshot(&root);
        if bless {
            std::fs::write(&after_path, &tree).unwrap();
        }
        assert_eq!(
            tree,
            std::fs::read_to_string(&after_path).unwrap_or_default(),
            "{name}: after-tree differs from golden"
        );
        // Idempotence: the second apply is a no-op, not a mutation.
        let again = apply(&plan, &root, &opts)
            .unwrap_or_else(|e| panic!("{name}: second apply errored: {e:#}"));
        assert_eq!(again.status, ApplyStatus::AlreadyApplied, "{name}");
        assert_eq!(
            snapshot(&root),
            tree,
            "{name}: second apply changed the tree"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn golden_cases() {
    let mut cases = vec![];
    for op in std::fs::read_dir(data_root()).unwrap() {
        let op = op.unwrap().path();
        if !op.is_dir() || op.file_name().unwrap().to_string_lossy().starts_with('_') {
            continue;
        }
        for c in std::fs::read_dir(&op).unwrap() {
            let c = c.unwrap().path();
            if c.join("op.json").exists() {
                cases.push(c);
            }
        }
    }
    cases.sort();
    assert!(
        cases.len() >= 25,
        "expected the full fixture matrix, found {}",
        cases.len()
    );
    for c in cases {
        run_case(&c);
    }
}

/// Every operation must have the five AC-1 shapes.
#[test]
fn every_operation_covers_the_required_shapes() {
    for op in [
        "move-files",
        "rename-pkg",
        "extract-decls",
        "split-pkg",
        "sync-policy",
    ] {
        for case in [
            "clean",
            "test-package",
            "internal-boundary",
            "cycle-rejection",
            "simulator-negative",
        ] {
            assert!(
                data_root().join(op).join(case).join("op.json").exists(),
                "missing fixture {op}/{case}"
            );
        }
    }
}

fn blocker_codes(case: &str) -> Vec<String> {
    let p: MovePlan = serde_json::from_str(
        &std::fs::read_to_string(data_root().join(case).join("plan.json")).unwrap(),
    )
    .unwrap();
    p.blockers.into_iter().map(|b| b.code).collect()
}

#[test]
fn negative_fixtures_report_the_expected_blocker() {
    for op in [
        "move-files",
        "rename-pkg",
        "extract-decls",
        "split-pkg",
        "sync-policy",
    ] {
        assert!(
            blocker_codes(&format!("{op}/cycle-rejection")).contains(&"cycle_detected".to_string()),
            "{op}"
        );
        assert!(
            blocker_codes(&format!("{op}/simulator-negative"))
                .contains(&"simulation_blocker".to_string()),
            "{op}"
        );
        // rename's internal fixture and the others all surface the boundary violation
        assert!(
            blocker_codes(&format!("{op}/internal-boundary"))
                .contains(&"internal_visibility".to_string()),
            "{op}"
        );
    }
    assert!(
        blocker_codes("move-files/symbol-conflict-external-test")
            .contains(&"symbol_conflict_external_test".to_string())
    );
    assert!(
        blocker_codes("extract-decls/symbol-conflict-external-test")
            .contains(&"symbol_conflict_external_test".to_string())
    );
    for (case, code) in [
        ("move-files/hazard-build-tag", "build_constraint"),
        ("move-files/hazard-cgo", "cgo"),
        ("move-files/hazard-generated", "generated_file"),
        ("move-files/hazard-init-registration", "init_registration"),
    ] {
        assert!(blocker_codes(case).contains(&code.to_string()), "{case}");
    }
}

// ---------------------------------------------------------------------------------------
// Atomicity, idempotence, verification
// ---------------------------------------------------------------------------------------

fn base_move_plan(root: &Path) -> MovePlan {
    plan_move_files(
        root,
        &["lib/util.go".into(), "lib/util_test.go".into()],
        "example.com/m/util",
        None,
    )
    .unwrap()
}

#[test]
fn failure_at_every_step_restores_the_tree_byte_for_byte() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    let kinds: Vec<PlanStepKind> = plan.steps.iter().map(|s| s.kind).collect();
    assert!(kinds.contains(&PlanStepKind::RewriteFile) && kinds.contains(&PlanStepKind::MoveFile));
    let before = checksums(&root);
    for i in 0..plan.steps.len() {
        let err = apply(
            &plan,
            &root,
            &ApplyOptions {
                inject_failure_after_step: Some(i),
                ..Default::default()
            },
        )
        .expect_err("injected failure must surface");
        assert!(
            format!("{err:#}").contains("working tree restored"),
            "{err:#}"
        );
        assert_eq!(
            checksums(&root),
            before,
            "tree changed after failure at step {i}"
        );
    }
    // …and a clean apply still works afterwards (no leftover temp files or dirs in the way).
    let ok = apply(&plan, &root, &ApplyOptions::default()).unwrap();
    assert_eq!(ok.status, ApplyStatus::Applied);
}

#[test]
fn policy_update_failure_restores_the_tree() {
    let root = input_copy("base");
    let source =
        plan_rename_pkg(&root, "example.com/m/lib", "example.com/m/core", None, None).unwrap();
    let policy = plan_sync_policy(&root, &source).unwrap();
    assert!(
        policy
            .steps
            .iter()
            .all(|s| s.kind == PlanStepKind::PolicyUpdate)
            && !policy.steps.is_empty()
    );
    let before = checksums(&root);
    apply(
        &policy,
        &root,
        &ApplyOptions {
            inject_failure_after_step: Some(0),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert_eq!(checksums(&root), before);
}

#[test]
fn real_write_failure_mid_apply_restores_earlier_writes() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    // A regular file where the destination directory must go makes the last writes fail
    // after earlier rewrites already landed.
    std::fs::write(root.join("util"), "not a directory").unwrap();
    let before = checksums(&root);
    apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert_eq!(checksums(&root), before);
}

#[test]
fn stale_or_partial_plan_is_a_clear_error_without_mutation() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    // Stale: an importer changed after planning.
    std::fs::write(root.join("app/app.go"), "package app\n\nfunc Run() {}\n").unwrap();
    let before = checksums(&root);
    let err = apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert!(
        err.to_string().contains("stale plan") && err.to_string().contains("nothing was modified"),
        "{err}"
    );
    assert_eq!(checksums(&root), before);

    // Partially applied: one step's target state already present, the rest not.
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    let first = &plan.steps[0].transform;
    std::fs::write(
        root.join(first.new_path.as_ref().unwrap()),
        first.new_content.as_ref().unwrap(),
    )
    .unwrap();
    let before = checksums(&root);
    let err = apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert!(err.to_string().contains("partially applied"), "{err}");
    assert_eq!(checksums(&root), before);
}

#[test]
fn failed_verification_reports_output_verbatim_and_rolls_back() {
    if !go_available() {
        return;
    }
    let root = input_copy("base");
    let mut plan = base_move_plan(&root);
    // Sabotage the planned content so `go build` must fail.
    let step = plan
        .steps
        .iter_mut()
        .find(|s| s.transform.new_path.as_deref() == Some("app/app.go"))
        .unwrap();
    step.transform.new_content = Some("package app\n\nfunc Run() { undefinedSymbol() }\n".into());
    let before = checksums(&root);
    let result = apply(
        &plan,
        &root,
        &ApplyOptions {
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.status, ApplyStatus::RolledBack);
    let v = result.verify.unwrap();
    assert!(
        !v.build.ok() && v.build.output.contains("undefinedSymbol"),
        "{}",
        v.build.output
    );
    assert_eq!(v.build.command, "go build ./...");
    assert_eq!(v.vet.command, "go vet ./...");
    assert_eq!(v.test.command, "go test -count=1 ./...");
    assert_eq!(checksums(&root), before);
}

#[test]
fn successful_apply_embeds_all_three_command_outputs() {
    if !go_available() {
        return;
    }
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    let result = apply(
        &plan,
        &root,
        &ApplyOptions {
            verify: true,
            ..Default::default()
        },
    )
    .unwrap();
    let v = result.verify.unwrap();
    assert!(v.ok());
    assert!(
        v.test.output.contains("example.com/m/util"),
        "{}",
        v.test.output
    );
    assert!(
        v.test.exit_code == Some(0) && v.vet.exit_code == Some(0) && v.build.exit_code == Some(0)
    );
}

#[test]
fn post_apply_hook_failure_rolls_back() {
    if !go_available() {
        return;
    }
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    let before = checksums(&root);
    let result = apply(
        &plan,
        &root,
        &ApplyOptions {
            verify: true,
            post_apply: Some("echo regen-failed >&2; exit 7".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.status, ApplyStatus::RolledBack);
    assert!(
        result
            .verify
            .unwrap()
            .post_apply_hook
            .unwrap()
            .output
            .contains("regen-failed")
    );
    assert_eq!(checksums(&root), before);
}

#[test]
fn simulation_blocker_is_embedded_and_refuses_apply() {
    let root = input_copy("base");
    let sim = serde_json::json!({"feasible": false, "blockers": [{"message": "cycle via pkg x"}]});
    let plan = plan_move_files(
        &root,
        &["lib/util.go".into()],
        "example.com/m/util",
        Some(sim.clone()),
    )
    .unwrap();
    assert_eq!(plan.simulation_result, Some(sim));
    assert!(
        plan.blockers
            .iter()
            .any(|b| b.code == "simulation_blocker" && b.message.contains("cycle via pkg x"))
    );
    apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
}
