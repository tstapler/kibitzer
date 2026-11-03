//! Turns a mutated [`Workspace`] plus operation findings into a reviewable [`MovePlan`].

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use super::ops::Findings;
use super::types::{
    BlockingDiagnostic, FileTransform, MovePlan, OperationKind, PlanStep, PlanStepKind,
};
use super::workspace::{Workspace, dir_of};
use crate::refactor_core::exec::{self, EnvPolicy};

pub use crate::refactor_core::sha256_hex;

/// One entry per input: the formatted source, `Ok(None)` when gofmt is unavailable, or gofmt's
/// complaint about that file.
type Formatted = Result<Option<String>, String>;

/// A caller-chosen id and the Go source to format.
type Source<'a> = (&'a str, &'a str);

const GOFMT_TIMEOUT: Duration = Duration::from_secs(300);

/// A private (0700) directory removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> std::io::Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            builder.mode(0o700);
        }
        loop {
            let dir = std::env::temp_dir().join(format!(
                "kibitzer-gofmt-{}-{nanos}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            match builder.create(&dir) {
                Ok(()) => return Ok(Self(dir)),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Formats every source with ONE `gofmt -w` run over a private temp directory, instead of one
/// process per file (a rename touching hundreds of importers would otherwise spawn hundreds).
/// Results are keyed by the caller's id.
fn gofmt_batch(srcs: &[Source]) -> HashMap<String, Formatted> {
    let mut out = HashMap::new();
    if srcs.is_empty() {
        return out;
    }
    let all = |out: &mut HashMap<String, Formatted>, r: &dyn Fn() -> Formatted| {
        for (id, _) in srcs {
            out.insert((*id).to_string(), r());
        }
    };
    let dir = match TempDir::new() {
        Ok(d) => d,
        Err(e) => {
            all(&mut out, &|| {
                Err(format!("cannot create a temp dir for gofmt: {e}"))
            });
            return out;
        }
    };
    for (i, (_, src)) in srcs.iter().enumerate() {
        if let Err(e) = std::fs::write(dir.0.join(format!("{i}.go")), src) {
            all(&mut out, &|| {
                Err(format!("cannot stage files for gofmt: {e}"))
            });
            return out;
        }
    }
    let run = exec::run(&exec::Spec {
        program: "gofmt",
        args: &["-w".to_string(), dir.0.display().to_string()],
        dir: Path::new("/"),
        timeout: GOFMT_TIMEOUT,
        env: EnvPolicy::Inherit,
    });
    if run.exit_code.is_none() && run.output.starts_with("failed to run") {
        all(&mut out, &|| Ok(None)); // gofmt is not installed
        return out;
    }
    let errors = gofmt_errors(&run.output, &dir.0);
    if run.exit_code.is_none() || (run.exit_code != Some(0) && errors.is_empty()) {
        all(&mut out, &|| Err(run.output.trim().to_string()));
        return out;
    }
    for (i, (id, _)) in srcs.iter().enumerate() {
        let r = match errors.get(&i) {
            Some(msg) => Err(msg.clone()),
            None => std::fs::read_to_string(dir.0.join(format!("{i}.go")))
                .map(Some)
                .map_err(|e| e.to_string()),
        };
        out.insert((*id).to_string(), r);
    }
    out
}

/// gofmt's stderr lines are `<dir>/<n>.go:line:col: message`; group them by `n`, rewriting the
/// temp path to `<file>` so the message is the same whichever directory was used.
fn gofmt_errors(stderr: &str, dir: &Path) -> HashMap<usize, String> {
    let prefix = format!("{}/", dir.display());
    let mut errors: HashMap<usize, String> = HashMap::new();
    let mut last = None;
    for line in stderr.lines() {
        if let Some(rest) = line.strip_prefix(&prefix)
            && let Some((num, tail)) = rest.split_once(".go")
            && let Ok(i) = num.parse::<usize>()
        {
            let entry = errors.entry(i).or_default();
            if !entry.is_empty() {
                entry.push('\n');
            }
            entry.push_str(&format!("<file>{tail}"));
            last = Some(i);
        } else if let Some(i) = last {
            // "too many errors" and similar continuation lines belong to the previous file.
            if let Some(entry) = errors.get_mut(&i) {
                entry.push('\n');
                entry.push_str(line);
            }
        }
    }
    errors
}

/// Simulation output (#153) reporting infeasibility becomes a blocker.
fn simulation_blockers(sim: &serde_json::Value) -> Vec<BlockingDiagnostic> {
    let mut out = vec![];
    let listed = sim
        .get("blockers")
        .and_then(|v| v.as_array())
        .is_some_and(|b| !b.is_empty());
    if sim.get("feasible").and_then(|v| v.as_bool()) == Some(false) && !listed {
        out.push(BlockingDiagnostic::new(
            "simulation_blocker",
            "feasibility simulation reports the operation is infeasible",
        ));
    }
    if let Some(bs) = sim.get("blockers").and_then(|v| v.as_array()) {
        for b in bs {
            let msg = b
                .get("message")
                .and_then(|m| m.as_str())
                .or_else(|| b.as_str())
                .unwrap_or("simulation blocker");
            out.push(BlockingDiagnostic::new("simulation_blocker", msg));
        }
    }
    out
}

pub fn build_plan(
    ws: &Workspace,
    op: OperationKind,
    repo_root: &str,
    mut f: Findings,
    simulation: Option<serde_json::Value>,
) -> MovePlan {
    if let Some(sim) = &simulation {
        for b in simulation_blockers(sim) {
            f.blockers.push(b);
        }
    }
    for u in &ws.unreadable {
        f.blockers.push(BlockingDiagnostic::new(
            "unreadable_go_file",
            format!("{u}; importers in it cannot be found or rewritten"),
        ));
    }
    let mut steps = vec![];
    let mut gofmt_missing = false;
    let moved_origins: BTreeSet<&String> = ws.moves().values().collect();
    let changed = ws.changed_paths();
    // Format everything up front: two gofmt runs for the whole plan (the originals, to learn
    // whether each was already gofmt-clean, and the rewritten bodies).
    let (basis_fmt, content_fmt) = {
        let mut bases: Vec<Source> = vec![];
        let mut contents: Vec<Source> = vec![];
        for path in &changed {
            let Some(new) = ws.get(path) else { continue };
            let basis = ws
                .moves()
                .get(path)
                .and_then(|o| ws.original(o))
                .or_else(|| ws.original(path));
            if let Some(b) = basis {
                bases.push((path.as_str(), b));
            }
            contents.push((path.as_str(), new));
        }
        (gofmt_batch(&bases), gofmt_batch(&contents))
    };
    for path in changed {
        let new = ws.get(&path).map(str::to_string);
        let orig = ws.original(&path).map(str::to_string);
        let Some(mut content) = new else {
            if orig.is_some() && !moved_origins.contains(&path) {
                steps.push(PlanStep {
                    kind: PlanStepKind::DeleteFile,
                    transform: FileTransform {
                        old_path: Some(path.clone()),
                        old_sha256: orig.as_deref().map(|o| sha256_hex(o.as_bytes())),
                        new_path: None,
                        new_content: None,
                    },
                    reason: "file removed".into(),
                });
            }
            continue;
        };
        let origin = ws.moves().get(&path).cloned();
        let basis = origin
            .as_ref()
            .and_then(|o| ws.original(o))
            .or(orig.as_deref());
        if crate::go_syntax::parse(&content).is_none_or(|t| t.root_node().has_error()) {
            f.blockers.push(
                BlockingDiagnostic::new("syntax_error", "file does not parse as Go (before or after the rewrite); refusing to plan around it")
                    .at(&path, None),
            );
        }
        let basis_result = basis_fmt.get(&path);
        // Without gofmt the basis check cannot run either, so the warning must be set here too.
        if matches!(basis_result, Some(Ok(None))) {
            gofmt_missing = true;
        }
        let basis_clean =
            basis_result.is_none_or(|r| matches!(r, Ok(Some(x)) if Some(x.as_str()) == basis));
        if basis_clean {
            match content_fmt.get(&path).cloned().unwrap_or(Ok(None)) {
                Ok(Some(formatted)) => content = formatted,
                Ok(None) => gofmt_missing = true,
                Err(e) => f.blockers.push(
                    BlockingDiagnostic::new(
                        "format_failed",
                        format!("rewritten file does not parse: {}", e.trim()),
                    )
                    .at(&path, None),
                ),
            }
        }
        let (kind, reason, old_path) = match (&origin, &orig) {
            (Some(o), _) => (
                PlanStepKind::MoveFile,
                format!("moved from {o}"),
                Some(o.clone()),
            ),
            (None, Some(_)) => {
                if basis == Some(content.as_str()) {
                    continue;
                }
                (
                    PlanStepKind::RewriteFile,
                    "import/qualifier/declaration rewrite".to_string(),
                    Some(path.clone()),
                )
            }
            (None, None) => (
                PlanStepKind::CreateFile,
                "new file holding extracted declarations".to_string(),
                None,
            ),
        };
        steps.push(PlanStep {
            kind,
            transform: FileTransform {
                old_sha256: basis.map(|b| sha256_hex(b.as_bytes())),
                old_path,
                new_path: Some(path),
                new_content: Some(content),
            },
            reason,
        });
    }
    if gofmt_missing {
        f.warnings
            .push("gofmt not found on PATH; rewritten files were not formatted".into());
    }
    steps.sort_by_key(key);

    let mut dirs: BTreeSet<String> = f.affected_dirs.clone();
    for s in &steps {
        for p in [&s.transform.old_path, &s.transform.new_path]
            .into_iter()
            .flatten()
        {
            dirs.insert(dir_of(p).to_string());
        }
    }
    let affected_packages = dirs
        .into_iter()
        .filter(|d| !ws.files_in_dir(d).is_empty())
        .map(|d| {
            if d.is_empty() {
                ".".to_string()
            } else {
                format!("./{d}")
            }
        })
        .collect();
    f.blockers.dedup();
    MovePlan {
        plan_id: String::new(),
        operation: op,
        repo_root: repo_root.to_string(),
        module: ws.module.clone(),
        from: f.from,
        to: f.to,
        steps,
        blockers: f.blockers,
        warnings: f.warnings,
        surface_changes: f.surface_changes,
        path_map: f.path_map,
        affected_packages,
        simulation_result: simulation,
    }
    .sealed()
}

fn key(s: &PlanStep) -> String {
    s.transform
        .new_path
        .clone()
        .or_else(|| s.transform.old_path.clone())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_gofmt() -> bool {
        let ok = exec::run(&exec::Spec {
            program: "gofmt",
            args: &["-h".into()],
            dir: Path::new("/"),
            timeout: Duration::from_secs(10),
            env: EnvPolicy::Inherit,
        })
        .output
        .contains("usage");
        assert!(
            ok || (std::env::var_os("CI").is_none()
                && std::env::var_os("KIBITZER_REQUIRE_GO").is_none()),
            "gofmt is required here (CI or KIBITZER_REQUIRE_GO is set)"
        );
        ok
    }

    #[test]
    fn an_empty_batch_spawns_nothing() {
        assert!(gofmt_batch(&[]).is_empty());
    }

    #[test]
    fn one_run_formats_every_source_and_pins_an_error_to_its_own_file() {
        if !have_gofmt() {
            return;
        }
        let srcs = [
            ("a", "package a\nfunc  f( ){}\n"),
            ("bad", "package b\nfunc ("),
            ("c", "package c\n"),
        ];
        let got = gofmt_batch(&srcs);
        assert_eq!(got.len(), 3);
        assert_eq!(got["a"], Ok(Some("package a\n\nfunc f() {}\n".to_string())));
        assert_eq!(got["c"], Ok(Some("package c\n".to_string())));
        let err = got["bad"].clone().unwrap_err();
        assert!(err.starts_with("<file>:"), "{err}");
        assert!(!err.contains("kibitzer-gofmt"), "temp path leaked: {err}");
    }

    #[test]
    fn gofmt_errors_are_grouped_by_file_with_continuation_lines() {
        let dir = Path::new("/tmp/d");
        let stderr = "/tmp/d/0.go:2:5: expected ')'\n/tmp/d/0.go:3:1: more\ntoo many errors\n/tmp/d/2.go:1:1: x\n";
        let e = gofmt_errors(stderr, dir);
        assert_eq!(e.len(), 2);
        assert!(e[&0].contains("<file>:2:5: expected ')'") && e[&0].contains("too many errors"));
        assert_eq!(e[&2], "<file>:1:1: x");
    }

    #[test]
    fn the_temp_dir_is_private_and_removed() {
        let path = {
            let d = TempDir::new().unwrap();
            assert!(d.0.is_dir());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                assert_eq!(
                    std::fs::metadata(&d.0).unwrap().permissions().mode() & 0o777,
                    0o700
                );
            }
            d.0.clone()
        };
        assert!(!path.exists());
    }
}
