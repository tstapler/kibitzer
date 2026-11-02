//! Fixture-backed acceptance tests. Fixtures live in `testdata/split-sim/<name>/`; set
//! `UPDATE_EXPECTED=1` to rewrite the committed snapshots in `testdata/split-sim/expected/`.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::*;
use crate::affected::{AffectedResult, compute_affected};
use crate::arch_model::PruneConfig;
use crate::config::AffectedConfig;

const PKG: &str = "ex.com/ss/lib";

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/split-sim")
        .join(name)
}

fn model(dir: &Path, include_private: bool) -> ArchModel {
    let files = crate::check::walk_and_collect_files(dir).unwrap();
    crate::arch_model::build_model_from_files(dir, &files, &PruneConfig { include_private })
        .unwrap()
}

fn spec(name: &str, files: &[&str]) -> PartitionSpec {
    PartitionSpec {
        name: name.into(),
        files: files.iter().map(|f| f.to_string()).collect(),
    }
}

fn simulate(fixture: &str, parts: &[PartitionSpec]) -> SplitSimResult {
    run(
        &model(&fixture_dir(fixture), true),
        PKG,
        parts,
        &SplitSimOpts::default(),
    )
}

fn ab() -> Vec<PartitionSpec> {
    vec![spec("P1", &["a.go"]), spec("P2", &["b.go"])]
}

fn kinds(r: &SplitSimResult, severity: BlockerSeverity) -> Vec<&str> {
    r.blockers
        .iter()
        .filter(|b| b.severity == severity)
        .map(|b| b.kind.as_str())
        .collect()
}

#[test]
fn legal_split_has_no_cycles_or_blockers_and_lists_exact_promotions() {
    let r = simulate("legal-split", &ab());
    assert!(r.abstained.is_none(), "{:?}", r.abstained);
    assert!(r.cycles.is_empty());
    assert!(r.blockers.is_empty(), "{:?}", r.blockers);
    assert!(r.feasible);
    let promote: Vec<&str> = r
        .exported_surface_delta
        .must_export
        .iter()
        .map(|e| e.symbol.as_str())
        .collect();
    assert_eq!(promote, ["ex.com/ss/lib::helper"]);
    let callers: Vec<&str> = r
        .callers_to_rewrite
        .iter()
        .map(|c| c.symbol.as_str())
        .collect();
    assert_eq!(
        callers,
        [
            "ex.com/ss/lib::Alpha",
            "ex.com/ss/lib::Beta",
            "ex.com/ss/lib::helper"
        ]
    );
}

#[test]
fn cycle_creating_split_names_the_edges_and_symbols() {
    let r = simulate("cycle-creating", &ab());
    assert!(!r.feasible);
    assert_eq!(r.cycles.len(), 1);
    let c = &r.cycles[0];
    assert_eq!(c.severity, CycleSeverity::Production);
    let edge = |from: &str, to: &str| {
        c.edges
            .iter()
            .find(|e| e.from.ends_with(from) && e.to.ends_with(to))
            .unwrap()
    };
    assert_eq!(edge("#P1", "#P2").symbols, ["ex.com/ss/lib::b1"]);
    assert_eq!(edge("#P2", "#P1").symbols, ["ex.com/ss/lib::a2"]);
    assert_eq!(edge("#P1", "#P2").sites, ["lib/a.go:3"]);
}

#[test]
fn private_shared_state_reports_the_unexported_field_and_the_indirect_sharing_caveat() {
    let r = simulate("private-shared-state", &ab());
    assert!(!r.feasible);
    let fields: Vec<&str> = r
        .blockers
        .iter()
        .filter(|b| b.kind == "shared_unexported_state")
        .map(|b| b.detail.as_str())
        .collect();
    assert!(fields.iter().any(|d| d.contains("Store.mu")), "{fields:?}");
    assert!(fields.iter().any(|d| d.contains("Store.n")), "{fields:?}");
    assert!(r.coverage.notes.iter().any(|n| n.contains("not detected")));
}

#[test]
fn init_function_is_a_hard_blocker_without_ordering_analysis() {
    let r = simulate("init-registration", &ab());
    let init = r
        .blockers
        .iter()
        .find(|b| b.kind == "init_function")
        .expect("init blocker");
    assert_eq!(init.severity, BlockerSeverity::Hard);
    assert_eq!(init.file.as_deref(), Some("lib/b.go"));
    assert!(init.detail.contains("not analysed"));
    assert!(!r.feasible);
}

#[test]
fn build_tagged_file_is_a_blocker() {
    let r = simulate("build-tag-variant", &ab());
    let tag = r
        .blockers
        .iter()
        .find(|b| b.kind == "build_tag")
        .expect("build tag blocker");
    assert_eq!(
        (tag.severity, tag.file.as_deref(), tag.line),
        (BlockerSeverity::Hard, Some("lib/b.go"), Some(1))
    );
}

#[test]
fn test_only_cycle_is_a_labelled_warning_not_a_blocker() {
    let r = simulate(
        "test-only-cycle",
        &[spec("P1", &["a.go", "a_test.go"]), spec("P2", &["b.go"])],
    );
    assert_eq!(r.cycles.len(), 1);
    assert_eq!(r.cycles[0].severity, CycleSeverity::TestOnly);
    assert!(r.cycles[0].edges.iter().any(|e| e.test_only));
    assert!(kinds(&r, BlockerSeverity::Hard).is_empty());
    assert!(
        r.feasible,
        "a test-only cycle must not make the split infeasible"
    );
    assert!(render_text(&r).contains("WARNING: test-only import cycle"));
}

#[test]
fn generated_file_in_partition_warns_that_it_is_absent_from_the_model() {
    let r = simulate(
        "generated-file",
        &[
            spec("P1", &["a.go"]),
            spec("P2", &["b.go", "zz_generated.go"]),
        ],
    );
    assert!(r.abstained.is_none(), "{:?}", r.abstained);
    let w = r
        .blockers
        .iter()
        .find(|b| b.kind == "generated_file_in_partition")
        .expect("warning");
    assert_eq!(w.severity, BlockerSeverity::Warning);
    assert!(w.detail.contains("absent from the model"));
    assert!(r.feasible);
}

#[test]
fn pruned_private_symbols_are_disclosed_not_hidden() {
    let pruned = model(&fixture_dir("legal-split"), false);
    assert!(pruned.pruning.private_symbols_skipped > 0);
    let r = run(&pruned, PKG, &ab(), &SplitSimOpts::default());
    assert!(r.coverage.private_symbols_skipped > 0);
    assert!(
        r.coverage
            .notes
            .iter()
            .any(|n| n.contains("may be undercounted")),
        "{:?}",
        r.coverage.notes
    );
    let full = simulate("legal-split", &ab());
    assert!(
        !full
            .coverage
            .notes
            .iter()
            .any(|n| n.contains("may be undercounted"))
    );
}

#[test]
fn abstains_with_a_reason_on_bad_input() {
    let m = model(&fixture_dir("legal-split"), true);
    let opts = SplitSimOpts::default();
    let reason = |pkg: &str, parts: &[PartitionSpec]| run(&m, pkg, parts, &opts).abstained.unwrap();
    assert!(reason("no/such", &ab()).contains("not in the model"));
    assert!(reason(PKG, &[spec("P1", &["a.go", "b.go"])]).contains("at least two"));
    assert!(reason(PKG, &[spec("P1", &["a.go"]), spec("P2", &[])]).contains("no files"));
    assert!(reason(PKG, &[spec("P1", &["a.go"]), spec("P2", &["a.go"])]).contains("both"));
    assert!(reason(PKG, &[spec("P1", &["a.go", "b.go"]), spec("P2", &["b.go"])]).contains("both"));
    assert!(
        reason(
            PKG,
            &[spec("P1", &["a.go"]), spec("P2", &["b.go", "typo.go"])]
        )
        .contains("typo.go")
    );
    assert!(!run(&m, PKG, &[spec("P1", &["a.go"]), spec("P2", &[])], &opts).feasible);
}

#[test]
fn abstains_without_reading_files_outside_the_root() {
    let m = model(&fixture_dir("legal-split"), true);
    let reads = std::cell::RefCell::new(Vec::<String>::new());
    let read = |p: &str| {
        reads.borrow_mut().push(p.to_string());
        None
    };
    for escape in ["../secret.go", "/etc/passwd", "sub/../../x.go"] {
        let parts = [spec("P1", &["a.go", escape]), spec("P2", &["b.go"])];
        let r = run_with_sources(&m, PKG, &parts, &SplitSimOpts::default(), &read);
        assert!(r.abstained.is_some(), "{escape} should abstain");
    }
    assert!(
        reads.borrow().is_empty(),
        "read called with {:?}",
        reads.borrow()
    );
}

#[test]
fn abstains_on_oversized_partition_input() {
    let m = model(&fixture_dir("legal-split"), true);
    let many: Vec<PartitionSpec> = (0..=MAX_PARTS)
        .map(|i| spec(&format!("P{i}"), &["a.go"]))
        .collect();
    let r = run(&m, PKG, &many, &SplitSimOpts::default());
    assert!(r.abstained.unwrap().contains("exceeds"));
}

#[test]
fn output_is_deterministic_and_matches_committed_snapshots() {
    let cases: Vec<(&str, Vec<PartitionSpec>)> = vec![
        ("legal-split", ab()),
        ("cycle-creating", ab()),
        ("private-shared-state", ab()),
        ("init-registration", ab()),
        ("build-tag-variant", ab()),
        (
            "test-only-cycle",
            vec![spec("P1", &["a.go", "a_test.go"]), spec("P2", &["b.go"])],
        ),
        (
            "generated-file",
            vec![
                spec("P1", &["a.go"]),
                spec("P2", &["b.go", "zz_generated.go"]),
            ],
        ),
    ];
    for (name, parts) in cases {
        let first = serde_json::to_string_pretty(&simulate(name, &parts)).unwrap();
        let second = serde_json::to_string_pretty(&simulate(name, &parts)).unwrap();
        assert_eq!(first, second, "{name}: two runs differ");
        let snapshot = fixture_dir("expected").join(format!("{name}.json"));
        if std::env::var_os("UPDATE_EXPECTED").is_some() {
            std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
            std::fs::write(&snapshot, format!("{first}\n")).unwrap();
        }
        let expected = std::fs::read_to_string(&snapshot)
            .unwrap_or_else(|_| panic!("missing {}", snapshot.display()));
        assert_eq!(
            expected.trim_end(),
            first,
            "{name}: snapshot drifted (UPDATE_EXPECTED=1 to refresh)"
        );
    }
}

#[test]
fn partition_flags_parse_and_reject_reserved_delimiters() {
    let ok = parse_partition_flags(&["P1=a.go,b.go".into(), "P2=c.go".into()]).unwrap();
    assert_eq!(
        (ok[0].name.as_str(), ok[0].files.len(), ok[1].files.len()),
        ("P1", 2, 1)
    );
    assert!(parse_partition_flags(&["P1".into()]).is_err());
    assert!(parse_partition_flags(&["P1=".into()]).is_err());
    assert!(parse_partition_flags(&["P1=a=b.go".into()]).is_err());
}

#[test]
fn scan_finds_init_build_tags_cgo_values_and_platform_suffixes() {
    let src = "//go:build linux\n\npackage x_test\n\nimport \"C\"\n\nvar (\n\ta = 1\n\tmu sync.Mutex\n)\n\nconst K = 2\n\nfunc init() {}\n";
    let s = scan::scan(src);
    assert_eq!(
        (s.build_tag_line, s.cgo_line, s.init_line, s.external_test),
        (Some(1), Some(5), Some(14), true)
    );
    let names: Vec<(&str, bool)> = s
        .values
        .iter()
        .map(|v| (v.name.as_str(), v.is_lock))
        .collect();
    assert_eq!(names, [("a", false), ("mu", true), ("K", false)]);
    assert_eq!(
        scan::platform_suffix("x_linux_amd64.go").as_deref(),
        Some("linux_amd64")
    );
    assert_eq!(
        scan::platform_suffix("x_windows_test.go").as_deref(),
        Some("windows")
    );
    assert_eq!(scan::platform_suffix("plain.go"), None);
}

// --- Ground truth: simulated reverse-dep counts vs. `kibitzer architecture affected` on the
// --- tree after performing the real split (testdata/split-sim/<name>-after).

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn copy_tree(from: &Path, to: &Path) {
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&dest).unwrap();
            copy_tree(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), dest).unwrap();
        }
    }
}

/// Importers `affected` reports when a file in `package_dir` changes (the package itself excluded).
fn affected_importers(repo: &Path, package_dir: &str, package_key: &str) -> Vec<String> {
    let file = std::fs::read_dir(repo.join(package_dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "go"))
        .unwrap();
    let original = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, format!("{original}\n// touched\n")).unwrap();
    let result = compute_affected(repo, "HEAD", &AffectedConfig::default()).unwrap();
    std::fs::write(&file, original).unwrap();
    let AffectedResult::Packages(mut pkgs) = result else {
        panic!("affected bailed out")
    };
    pkgs.retain(|p| p != package_key);
    pkgs.sort();
    pkgs
}

fn committed_copy(fixture: &str) -> PathBuf {
    let dir = crate::test_support::unique_temp_dir(&format!("split-sim-gt-{fixture}"));
    copy_tree(&fixture_dir(fixture), &dir);
    git(&dir, &["init", "-q"]);
    git(&dir, &["config", "user.email", "test@example.com"]);
    git(&dir, &["config", "user.name", "Test"]);
    git(&dir, &["add", "."]);
    git(&dir, &["commit", "-q", "-m", "base"]);
    dir
}

/// `parts`: (part name, directory it becomes in the `-after` tree).
fn assert_reverse_deps_match_ground_truth(
    fixture: &str,
    parts: &[PartitionSpec],
    dirs: &[(&str, &str)],
) {
    let sim = simulate(fixture, parts);
    assert!(sim.abstained.is_none(), "{:?}", sim.abstained);

    let before = committed_copy(fixture);
    let truth_before = affected_importers(&before, "lib", PKG);
    assert_eq!(
        sim.reverse_dep_impact.before.transitive_packages, truth_before,
        "{fixture}: before"
    );
    assert_eq!(sim.reverse_dep_impact.before.transitive, truth_before.len());

    let after = committed_copy(&format!("{fixture}-after"));
    for (part, dir) in dirs {
        let key = format!("ex.com/ss/{dir}");
        let truth = affected_importers(&after, dir, &key);
        let sim_part = sim
            .reverse_dep_impact
            .after
            .iter()
            .find(|d| d.node.ends_with(&format!("#{part}")))
            .unwrap();
        let mapped: Vec<String> = {
            let mut v: Vec<String> = sim_part
                .transitive_packages
                .iter()
                .map(|p| {
                    dirs.iter()
                        .find(|(n, _)| p.ends_with(&format!("#{n}")))
                        .map_or(p.clone(), |(_, d)| format!("ex.com/ss/{d}"))
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(mapped, truth, "{fixture}: after {part}");
        assert_eq!(sim_part.transitive, truth.len());
    }
    let _ = std::fs::remove_dir_all(before);
    let _ = std::fs::remove_dir_all(after);
}

#[test]
fn reverse_dep_counts_match_affected_after_the_real_split_legal_split() {
    assert_reverse_deps_match_ground_truth("legal-split", &ab(), &[("P1", "p1"), ("P2", "p2")]);
}

#[test]
fn reverse_dep_counts_match_affected_after_the_real_split_fanout_split() {
    let parts = [spec("P1", &["a.go", "b.go"]), spec("P2", &["c.go"])];
    assert_reverse_deps_match_ground_truth("fanout-split", &parts, &[("P1", "p1"), ("P2", "p2")]);
    let sim = simulate("fanout-split", &parts);
    // The split narrows fan-out: P2 is reached by fewer packages than the original package was.
    assert!(sim.reverse_dep_impact.after[1].transitive < sim.reverse_dep_impact.before.transitive);
}
