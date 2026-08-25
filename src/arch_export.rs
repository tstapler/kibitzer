//! `kibitzer architecture export` — walks a repo, builds the shared `ArchModel`
//! (`arch_model::build_model`), and writes it as pretty-printed JSON. Mirrors
//! `install.rs`'s `--dry-run`/pretty-print/write convention exactly (Story 2.1.1).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::arch_model::{ModelLevel, PruneConfig, build_model};
use crate::check::walk_and_collect_files;
use crate::config::find_config;
use crate::import_graph;

/// File extensions `build_model` (via `symbol_extract.rs`'s `LangSymbolConfig` table)
/// recognizes. Duplicated here (not imported) from `arch_model.rs`'s private
/// `language_for_path` — this phase is scoped to leave `arch_model.rs` untouched — so this
/// command can cheaply detect the "no supported languages" case up front, before doing any
/// parsing.
fn has_supported_extension(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("go")
            | Some("ts")
            | Some("tsx")
            | Some("js")
            | Some("jsx")
            | Some("mjs")
            | Some("cjs")
            | Some("py")
            | Some("java")
            | Some("kt")
            | Some("kts")
    )
}

/// Walks `repo_root`, resolves the `ImportGraph`, and reads every file's contents that
/// `read_to_string` succeeds on (binary/non-UTF8 files are silently skipped, not fatal —
/// the export shouldn't crash because a repo has an image or other binary asset in it).
fn collect_files(repo_root: &Path) -> Result<(import_graph::ImportGraph, Vec<(PathBuf, String)>)> {
    let all_files = walk_and_collect_files(repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;

    let graph = import_graph::build(repo_root, &all_files)
        .with_context(|| format!("building import graph for {}", repo_root.display()))?;

    let files: Vec<(PathBuf, String)> = all_files
        .into_iter()
        .filter_map(|f| std::fs::read_to_string(&f).ok().map(|s| (f, s)))
        .collect();

    Ok((graph, files))
}

/// Runs `kibitzer architecture export`. Always exits `ExitCode::SUCCESS` on a successful
/// write, regardless of model contents (no pass/fail concept for export, per UX research);
/// nonzero only on genuine I/O/config errors, propagated via `anyhow`.
pub fn run_export(
    path: PathBuf,
    scope: Option<String>,
    out: PathBuf,
    dry_run: bool,
    include_private: bool,
) -> Result<ExitCode> {
    let repo_root = match find_config(&path)? {
        Some((_, root)) => root,
        None => path.clone(),
    };

    let walked = walk_and_collect_files(&repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;
    if !walked.iter().any(|f| has_supported_extension(f)) {
        println!(
            "no supported languages found under {}; nothing to export",
            path.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let (graph, files) = collect_files(&repo_root)?;
    let prune = PruneConfig { include_private };
    let model = build_model(&repo_root, &files, &graph, &prune)?;

    let export_model = if let Some(scope_glob) = &scope {
        let filtered = model.filtered(std::slice::from_ref(scope_glob), ModelLevel::Code);
        if filtered.packages.is_empty() && !model.packages.is_empty() {
            println!(
                "no packages matched scope \"{scope_glob}\" under {}; nothing to export",
                path.display()
            );
            return Ok(ExitCode::SUCCESS);
        }
        filtered
    } else {
        model
    };

    let rendered = serde_json::to_string_pretty(&export_model)? + "\n";
    if dry_run {
        print!("{rendered}");
    } else {
        if let Some(parent) = out.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&out, &rendered).with_context(|| format!("writing {}", out.display()))?;
        println!("[kibitzer] wrote {}", out.display());
    }

    let pruning = &export_model.pruning;
    if pruning.total_files_scanned > 0 {
        let fraction =
            pruning.unsupported_language_files as f64 / pruning.total_files_scanned as f64;
        if fraction >= 0.5 {
            println!(
                "warning: {}/{} files scanned have no supported language extension — this \
                 export may not represent most of the repo",
                pruning.unsupported_language_files, pruning.total_files_scanned
            );
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-arch-export-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture(dir: &Path, rel: &str, contents: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// The compiled `kibitzer` binary's path. Cargo only sets `CARGO_BIN_EXE_<name>` for
    /// integration tests/benches (this crate has no `tests/` directory — everything is
    /// inline `#[cfg(test)]`, per `validation.md`'s Test Stack section), so it's not
    /// available here; `cargo test` still builds the plain `kibitzer` binary target as
    /// part of the same build, so it's resolved by convention instead.
    fn kibitzer_bin_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            })
            .join("kibitzer")
    }

    fn run_kibitzer(args: &[&str]) -> std::process::Output {
        std::process::Command::new(kibitzer_bin_path())
            .args(args)
            .output()
            .expect("kibitzer binary runs (run `cargo build` first if this fails)")
    }

    #[test]
    fn run_export_writes_pretty_printed_arch_model_json_with_trailing_newline() {
        let dir = tmp_dir("writes-pretty");
        write_fixture(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        write_fixture(&dir, "pkg/a.go", "package pkg\n\nfunc A() {}\n");
        let out = dir.join("arch.json");

        let result = run_export(dir.clone(), None, out.clone(), false, false).unwrap();
        assert_eq!(result, ExitCode::SUCCESS);

        let contents = std::fs::read_to_string(&out).unwrap();
        assert!(contents.ends_with("\n"));
        assert!(!contents.ends_with("\n\n"));

        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        let packages = parsed["packages"].as_object().unwrap();
        assert_eq!(packages.len(), 1);
        assert!(packages.contains_key("example.com/app/pkg"));

        // Pretty-printed (matching install.rs:35's convention) means multi-line, not a
        // single compact line.
        assert!(contents.lines().count() > 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_dry_run_prints_json_and_writes_no_file() {
        // No go.mod here (deliberately): a bare go.mod file would itself count as an
        // "unsupported language" scanned file and could trip the >=50% warning line onto
        // stdout after the JSON, which this test needs to parse as JSON alone.
        let dir = tmp_dir("dry-run");
        write_fixture(&dir, "pkg/a.go", "package pkg\n\nfunc A() {}\n");
        let out = dir.join("arch.json");

        let output = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--dry-run",
        ]);
        assert!(output.status.success());
        assert!(!out.exists(), "dry-run must not write the output file");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("dry-run stdout is the ArchModel JSON");
        assert_eq!(parsed["packages"].as_object().unwrap().len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_scope_filters_exported_packages() {
        // No go.mod: `import_graph::build_go` early-returns without one (it has nothing
        // to map local import paths back to), so these files never enter
        // `ImportGraph::file_packages` and `arch_model::package_key_for_file` falls back
        // to a plain repo-root-relative directory path — giving exactly "web/ui"/
        // "server/api" package keys, regardless of which languages import_graph.rs
        // extracts imports for elsewhere.
        let dir = tmp_dir("scope-filter");
        write_fixture(&dir, "web/ui/widget.go", "package ui\n\nfunc Widget() {}\n");
        write_fixture(
            &dir,
            "server/api/handler.go",
            "package api\n\nfunc Handler() {}\n",
        );
        let out = dir.join("arch.json");

        let result = run_export(
            dir.clone(),
            Some("web/**".to_string()),
            out.clone(),
            false,
            false,
        )
        .unwrap();
        assert_eq!(result, ExitCode::SUCCESS);

        let contents = std::fs::read_to_string(&out).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        let packages = parsed["packages"].as_object().unwrap();
        assert_eq!(packages.len(), 1);
        assert!(packages.keys().next().unwrap().starts_with("web/"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_reports_no_supported_languages_found_and_exits_zero() {
        let dir = tmp_dir("no-supported-languages");
        write_fixture(&dir, "README.md", "# hi\n");
        let out = dir.join("arch.json");

        let output = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(output.status.success());
        assert_eq!(output.status.code(), Some(0));
        assert!(!out.exists());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected = format!(
            "no supported languages found under {}; nothing to export",
            dir.display()
        );
        assert!(
            stdout.contains(&expected),
            "expected stdout to contain {expected:?}, got {stdout:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_reports_no_packages_matched_scope_and_exits_zero() {
        let dir = tmp_dir("no-packages-matched-scope");
        write_fixture(&dir, "web/ui/widget.py", "def widget():\n    pass\n");
        write_fixture(&dir, "server/api/handler.py", "def handler():\n    pass\n");
        let out = dir.join("arch.json");

        let output = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--scope",
            "nonexistent/**",
        ]);
        assert!(output.status.success());
        assert_eq!(output.status.code(), Some(0));
        assert!(!out.exists());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let expected = format!(
            "no packages matched scope \"nonexistent/**\" under {}; nothing to export",
            dir.display()
        );
        assert!(
            stdout.contains(&expected),
            "expected stdout to contain {expected:?}, got {stdout:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_warns_when_unsupported_language_fraction_is_at_least_half() {
        let dir = tmp_dir("unsupported-fraction-warning");
        write_fixture(&dir, "pkg/a.go", "package pkg\n\nfunc A() {}\n");
        write_fixture(&dir, "pkg/b.go", "package pkg\n\nfunc B() {}\n");
        for i in 0..8 {
            write_fixture(&dir, &format!("notes/note{i}.txt"), "not code\n");
        }
        let out = dir.join("arch.json");

        let output = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(output.status.success());
        assert!(out.exists());

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("warning: 8/10 files scanned have no supported language extension"),
            "expected stdout to contain the unsupported-fraction warning, got {stdout:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_completes_under_5s_on_benchmark_fixture() {
        let dir = tmp_dir("benchmark");
        write_fixture(&dir, "go.mod", "module example.com/app\n\ngo 1.21\n");
        for i in 0..40 {
            let go_src = format!(
                "package pkg{i}\n\ntype Widget{i} struct{{}}\n\nfunc (w Widget{i}) Do() {{}}\n\nfunc Handle{i}() {{}}\n"
            );
            write_fixture(&dir, &format!("svc{i}/widget.go"), &go_src);
            let ts_src = format!(
                "export interface Shape{i} {{ area(): number; }}\n\nexport function make{i}(): Shape{i} {{ return null as unknown as Shape{i}; }}\n"
            );
            write_fixture(&dir, &format!("web/mod{i}/shape.ts"), &ts_src);
        }
        let out = dir.join("arch.json");

        let start = std::time::Instant::now();
        let result = run_export(dir.clone(), None, out.clone(), false, false).unwrap();
        let elapsed = start.elapsed();

        assert_eq!(result, ExitCode::SUCCESS);
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "run_export took {elapsed:?} on an 80-file benchmark fixture, expected well under 5s"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_export_exit_code_is_zero_across_empty_and_nonempty_outcomes() {
        // Non-empty: symbols present.
        let dir_a = tmp_dir("exit-code-nonempty");
        write_fixture(&dir_a, "pkg/a.go", "package pkg\n\nfunc A() {}\n");
        let out_a = dir_a.join("arch.json");
        let output_a = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir_a.to_str().unwrap(),
            "--out",
            out_a.to_str().unwrap(),
        ]);
        assert_eq!(output_a.status.code(), Some(0));
        let _ = std::fs::remove_dir_all(&dir_a);

        // Empty: no supported languages at all.
        let dir_b = tmp_dir("exit-code-no-languages");
        write_fixture(&dir_b, "README.md", "# hi\n");
        let out_b = dir_b.join("arch.json");
        let output_b = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir_b.to_str().unwrap(),
            "--out",
            out_b.to_str().unwrap(),
        ]);
        assert_eq!(output_b.status.code(), Some(0));
        let _ = std::fs::remove_dir_all(&dir_b);

        // Empty: scope matches nothing.
        let dir_c = tmp_dir("exit-code-zero-scope-match");
        write_fixture(&dir_c, "pkg/a.go", "package pkg\n\nfunc A() {}\n");
        let out_c = dir_c.join("arch.json");
        let output_c = run_kibitzer(&[
            "architecture",
            "export",
            "--path",
            dir_c.to_str().unwrap(),
            "--out",
            out_c.to_str().unwrap(),
            "--scope",
            "nonexistent/**",
        ]);
        assert_eq!(output_c.status.code(), Some(0));
        let _ = std::fs::remove_dir_all(&dir_c);
    }
}
