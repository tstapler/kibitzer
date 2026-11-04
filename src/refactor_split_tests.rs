use super::*;
use crate::arch_model::{ModelCache, load_cached_model};

static N: AtomicU64 = AtomicU64::new(0);

fn run_git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(st.success(), "git {args:?}");
}

/// Go module where `a` (x.go, y.go; independent files) and `b` form an import cycle that
/// splitting `a` into its two files resolves.
fn fixture() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kibitzer-refactor-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(dir.join("a")).unwrap();
    std::fs::create_dir_all(dir.join("b")).unwrap();
    let w = |p: &str, c: &str| std::fs::write(dir.join(p), c).unwrap();
    w("go.mod", "module fixture\n\ngo 1.21\n");
    w(
        "a/x.go",
        "package a\n\nimport \"fixture/b\"\n\nfunc X() int { return b.B() }\n",
    );
    w("a/y.go", "package a\n\nfunc Y() int { return 1 }\n");
    w(
        "b/b.go",
        "package b\n\nimport \"fixture/a\"\n\nfunc B() int { return a.Y() }\n",
    );
    run_git(&dir, &["init", "-q"]);
    run_git(&dir, &["add", "-A"]);
    run_git(&dir, &["commit", "-q", "-m", "init"]);
    dir
}

fn model(dir: &Path) -> ArchModel {
    let cache = ModelCache::new();
    Arc::try_unwrap(load_cached_model(&cache, dir, true).unwrap()).unwrap_or_else(|a| (*a).clone())
}

use std::sync::Arc;

fn split() -> Vec<PartitionGroup> {
    vec![PartitionGroup {
        name: "ypart".into(),
        symbols: vec!["Y".into()],
    }]
}

fn plan(dir: &Path) -> RefactorPlan {
    plan_split(&model(dir), dir, "fixture/a", &split()).unwrap()
}

#[test]
fn simulate_resolves_cycle_and_rejects_bad_partition() {
    let dir = fixture();
    let m = model(&dir);
    let sim = simulate_split(&m, "fixture/a", &split());
    assert!(sim.feasible, "{sim:?}");
    assert_eq!(sim.violations_resolved, 1);
    let bad = simulate_split(
        &m,
        "fixture/a",
        &[PartitionGroup {
            name: "n".into(),
            symbols: vec!["Nope".into()],
        }],
    );
    assert!(!bad.feasible && bad.blocker.contains("not found"));
}

#[test]
fn suggest_matches_reviewed_list_with_abstention() {
    let dir = fixture();
    // A second, independent cycle of single-file packages: no clean partition, must abstain.
    for (pkg, other) in [("c", "d"), ("d", "c")] {
        std::fs::create_dir_all(dir.join(pkg)).unwrap();
        std::fs::write(
            dir.join(format!("{pkg}/{pkg}.go")),
            format!("package {pkg}\n\nimport \"fixture/{other}\"\n\nfunc F() int {{ return {other}.F() }}\n"),
        )
        .unwrap();
    }
    let (suggestions, abstained) = suggest(&model(&dir), &[]);
    let s: Vec<_> = suggestions
        .iter()
        .map(|s| (s.package.as_str(), s.evidence.cycles_resolved))
        .collect();
    let a: Vec<_> = abstained.iter().map(|a| a.package.as_str()).collect();
    assert_eq!(s, vec![("fixture/a", 1)], "{suggestions:?}");
    assert_eq!(
        a,
        vec!["fixture/b", "fixture/c", "fixture/d"],
        "{abstained:?}"
    );
    assert!(abstained[0].reason.contains("single connected component"));
}

#[test]
fn plan_never_writes_and_apply_then_revert_round_trip() {
    let dir = fixture();
    let p = plan(&dir);
    assert!(
        run_git_out(&dir, &["status", "--porcelain"]).is_empty(),
        "plan must not dirty the tree"
    );
    let out = apply(&dir, &p.plan_id).unwrap();
    assert!(out.applied, "{out:?}");
    assert!(dir.join("a/ypart/y.go").is_file() && !dir.join("a/y.go").exists());
    let b = std::fs::read_to_string(dir.join("b/b.go")).unwrap();
    assert!(
        b.contains("ypart.Y()")
            && b.contains("\"fixture/a/ypart\"")
            && !b.contains("\"fixture/a\""),
        "{b}"
    );
    assert!(
        std::fs::read_to_string(dir.join("a/ypart/y.go"))
            .unwrap()
            .starts_with("package ypart")
    );
    let files = revert(&dir, &p.plan_id).unwrap();
    assert!(!files.is_empty());
    assert!(dir.join("a/y.go").is_file() && !dir.join("a/ypart").exists());
    assert!(
        run_git_out(&dir, &["status", "--porcelain"]).is_empty(),
        "revert restores a clean tree"
    );
}

fn run_git_out(dir: &Path, args: &[&str]) -> String {
    git(dir, args).unwrap()
}

#[test]
fn apply_refuses_dirty_tree_without_changes() {
    let dir = fixture();
    let p = plan(&dir);
    std::fs::write(dir.join("README"), "dirty").unwrap();
    let e = apply(&dir, &p.plan_id).unwrap_err();
    assert_eq!(e.code, "dirty_tree");
    assert!(dir.join("a/y.go").is_file() && !dir.join("a/ypart").exists());
}

#[test]
fn apply_refuses_stale_plan_without_changes() {
    let dir = fixture();
    let p = plan(&dir);
    std::fs::write(dir.join("README"), "x").unwrap();
    run_git(&dir, &["add", "README"]);
    run_git(&dir, &["commit", "-q", "-m", "move head"]);
    let e = apply(&dir, &p.plan_id).unwrap_err();
    assert_eq!(e.code, "stale_plan");
    assert!(dir.join("a/y.go").is_file() && !dir.join("a/ypart").exists());
    assert_eq!(apply(&dir, "deadbeef").unwrap_err().code, "unknown_plan");
}

#[test]
fn second_session_with_overlapping_paths_is_lock_conflicted() {
    let dir = fixture();
    let (first, second) = (plan(&dir), plan(&dir));
    assert!(apply(&dir, &first.plan_id).unwrap().applied);
    let e = apply(&dir, &second.plan_id).unwrap_err();
    assert_eq!(e.code, "lock_conflict", "{}", e.message);
}

#[test]
fn failed_verification_rolls_back() {
    let dir = fixture();
    let mut p = plan(&dir);
    p.verification_steps = vec!["false".into()];
    save_plan(&dir, &p).unwrap();
    let out = apply(&dir, &p.plan_id).unwrap();
    assert!(!out.applied && out.rolled_back);
    assert!(dir.join("a/y.go").is_file() && !dir.join("a/ypart").exists());
    assert!(run_git_out(&dir, &["status", "--porcelain"]).is_empty());
}

#[test]
fn edit_paths_must_stay_inside_repo() {
    assert!(safe_rel("../outside").is_err());
    assert!(safe_rel("/etc/passwd").is_err());
    assert!(safe_rel("a/b.go").is_ok());
}

/// Backtest driver (docs/refactor-tools.md): `KIBITZER_BACKTEST_REPO=<path> cargo test
/// backtest_suggest -- --ignored --nocapture`.
#[test]
#[ignore]
fn backtest_suggest_on_env_repo() {
    let Ok(repo) = std::env::var("KIBITZER_BACKTEST_REPO") else {
        return;
    };
    let start = std::time::Instant::now();
    let m = model(Path::new(&repo));
    let (s, a) = suggest(&m, &[]);
    eprintln!(
        "BACKTEST packages={} suggestions={} abstained={} elapsed={:?}",
        m.packages.len(),
        s.len(),
        a.len(),
        start.elapsed()
    );
    for x in s.iter().take(5) {
        eprintln!(
            "  SUGGEST {} cycles_resolved={} importers={} groups={}",
            x.package,
            x.evidence.cycles_resolved,
            x.evidence.importers_affected,
            x.partition.len()
        );
    }
    for x in a.iter().take(5) {
        eprintln!("  ABSTAIN {} {}", x.package, x.reason);
    }
}

#[test]
fn group_names_must_be_safe_go_identifiers() {
    for bad in ["../x", "a/b", "x\"", "Upper", "func", "", "1a"] {
        assert!(!valid_go_ident(bad), "{bad:?} should be rejected");
    }
    assert!(valid_go_ident("svc_store2"));
}

#[test]
fn truncation_respects_char_boundaries() {
    let mut s = "é".repeat(3000);
    truncate_at_boundary(&mut s, 4000);
    assert!(s.len() <= 4000 && s.chars().all(|c| c == 'é'));
}

/// A killed process leaves its transaction log and a half-applied tree behind.
fn crash_mid_transaction(dir: &Path, id: &str, changes: &[txn::Change]) {
    let pending = txn::session(dir)
        .unwrap()
        .begin(id, changes, &txn::real_rename)
        .unwrap();
    txn::simulate_kill(true);
    drop(pending);
    txn::simulate_kill(false);
}

#[test]
fn a_crashed_apply_is_recovered_before_the_next_apply_judges_the_tree() {
    let dir = fixture();
    let p = plan(&dir);
    crash_mid_transaction(&dir, &p.plan_id, &apply_changes(&dir, &p));
    assert!(
        !run_git_out(&dir, &["status", "--porcelain"]).is_empty(),
        "the crash left a half-applied (dirty) tree"
    );
    assert!(dir.join(".kibitzer/plans/.wal").exists());
    // Without recovery first this would be refused as `dirty_tree` and stay stuck forever.
    let out = apply(&dir, &p.plan_id).unwrap();
    assert!(out.applied, "{out:?}");
    assert!(dir.join("a/ypart/y.go").is_file() && !dir.join("a/y.go").exists());
    assert!(
        !dir.join(".kibitzer/plans/.wal").exists(),
        "log cleared on commit"
    );
}

#[test]
fn a_crashed_revert_is_rolled_back_and_the_revert_then_completes() {
    let dir = fixture();
    let p = plan(&dir);
    assert!(apply(&dir, &p.plan_id).unwrap().applied);
    crash_mid_transaction(
        &dir,
        &format!("revert-{}", p.plan_id),
        &revert_changes(&dir, &p),
    );
    assert!(
        dir.join("a/y.go").is_file(),
        "the crashed revert had already written"
    );
    // The next revert recovers first, so the plan's edits are back in their applied state and
    // the "modified since apply" check passes instead of misreading the half-reverted files.
    revert(&dir, &p.plan_id).unwrap();
    assert!(dir.join("a/y.go").is_file() && !dir.join("a/ypart").exists());
    assert!(run_git_out(&dir, &["status", "--porcelain"]).is_empty());
    assert!(!dir.join(".kibitzer/plans/.wal").exists());
}

#[test]
fn txn_error_keeps_a_failed_rollback_distinct_from_plain_io() {
    let failed = anyhow::Error::new(txn::RollbackFailed {
        cause: "boom".into(),
        rollback: "disk gone".into(),
    });
    assert_eq!(txn_error(failed).code, "rollback_failed");
    assert_eq!(txn_error(anyhow::anyhow!("plain")).code, "io");
}
