mod arch_diagram;
mod arch_export;
mod arch_model;
mod architecture_checks;
mod backtest;
mod cache;
mod change_coupling;
mod check;
mod checker;
mod comment_quality;
mod complexity;
mod config;
mod daemon;
mod declaration_checks;
mod declarations;
mod dedup;
mod duplicate_code;
mod duplicate_cross_file_checker;
mod file_size;
mod glob;
mod go_blank_imports;
mod go_call_resolution;
mod go_error_context;
mod go_ignored_error;
mod hook;
mod hook_log;
mod import_graph;
mod install;
mod lsp;
mod markdown_link_integrity;
mod mcp;
mod mermaid;
mod plugin;
mod primitive_obsession;
mod rules;
mod run;
mod status;
mod symbol_extract;
mod task_stop;
#[cfg(test)]
mod test_support;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "kibitzer", about = "Cross-language code/doc inspection")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Batch mode: run checks for `trigger` against every file under `dir`.
    Run {
        #[arg(default_value = ".")]
        dir: PathBuf,
        #[arg(long, default_value = "batch")]
        trigger: String,
    },
    /// Claude Code PostToolUse hook mode: read the event off stdin.
    Hook,
    /// Run kibitzer as an MCP server over stdio.
    Mcp,
    /// Run kibitzer as an LSP server over stdio, publishing check results as diagnostics.
    Lsp,
    /// Manage the background daemon that caches check results across invocations.
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
    /// Run a specific built-in analysis directly against a file (for wiring into
    /// .claude/inspect.json's shell-command checks).
    Check {
        #[command(subcommand)]
        check: CheckCommand,
    },
    /// Summarize the PostToolUse hook log: firing counts, per-check pass/fail/blocked
    /// stats, and per-repo activity.
    Status,
    /// Install kibitzer's PostToolUse hook into a Claude Code settings.json, merging
    /// with whatever hooks are already configured there.
    Install {
        /// Install into ~/.claude/settings.json (all projects) instead of
        /// <cwd>/.claude/settings.json (this project only).
        #[arg(long)]
        global: bool,
        /// Print what would be written instead of writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Query the shared architecture model (packages, symbols, import edges) — export it
    /// as JSON, or render it as a diagram.
    Architecture {
        #[command(subcommand)]
        action: ArchitectureAction,
    },
    /// Install, list, remove, or check the status of optional external checker plugins
    /// (see `src/plugin.rs`).
    Plugin {
        #[command(subcommand)]
        action: PluginAction,
    },
}

#[derive(Subcommand)]
enum PluginAction {
    /// Fetch, verify, and register a plugin from an HTTPS source or a local manifest path.
    Install {
        #[arg(value_parser = plugin::PluginName::parse)]
        name: plugin::PluginName,
        #[arg(long)]
        source: String,
        /// Re-run the fetch/verify/place/register pipeline even if this exact version is
        /// already installed.
        #[arg(long)]
        force: bool,
    },
    /// List every installed plugin.
    List,
    /// Delete a plugin's binary and registry entry.
    Remove {
        #[arg(value_parser = plugin::PluginName::parse)]
        name: plugin::PluginName,
        /// Proceed even if a local `.claude/inspect.json` check still references this
        /// plugin by name.
        #[arg(long)]
        force: bool,
    },
    /// Report whether an installed plugin's binary is present and its checksum/version
    /// are still valid. With no `<name>`, reports on every installed plugin.
    Status {
        #[arg(value_parser = plugin::PluginName::parse)]
        name: Option<plugin::PluginName>,
    },
}

#[derive(Subcommand)]
enum ArchitectureAction {
    /// Build the repo's architecture model (packages, symbols, import edges) and write it
    /// as pretty-printed JSON.
    Export {
        /// Any path inside the repo to export (the repo root or a subdirectory).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Glob (relative to the repo root, `**` supported) restricting which packages are
        /// exported. Defaults to the whole repo.
        #[arg(long)]
        scope: Option<String>,
        /// File to write the ArchModel JSON to.
        #[arg(long)]
        out: PathBuf,
        /// Print the JSON that would be written instead of writing it.
        #[arg(long)]
        dry_run: bool,
        /// Include unexported (private) symbols. Default: excluded.
        #[arg(long)]
        include_private: bool,
    },
    /// Render a Component/Code-level diagram in Mermaid notation *inspired by* C4 — not a
    /// standards-conformant C4 Context/Container diagram.
    Diagram {
        /// Any path inside the repo to diagram (the repo root or a subdirectory).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// Glob (relative to the repo root, `**` supported) restricting which packages are
        /// diagrammed. Defaults to the whole repo.
        #[arg(long)]
        scope: Option<String>,
        /// Diagram granularity: package-to-package boxes, or symbols nested inside their
        /// package's box.
        #[arg(long, value_enum, default_value_t = arch_diagram::DiagramLevel::Component)]
        level: arch_diagram::DiagramLevel,
        /// File to write the combined text-tree + Mermaid output to. Defaults to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Batch-only temporal-coupling report: file pairs that change together across git
    /// history, independent of any import/call relationship (see `change_coupling.rs`).
    /// Never wired into `default_checks()`/hook mode — this is a "look here"
    /// prioritization report, not a per-edit pass/fail check.
    ChangeCoupling {
        /// Any path inside the repo to analyze (the repo root or a subdirectory).
        #[arg(long, default_value = ".")]
        path: PathBuf,
        /// How many of the most recent non-merge commits to scan.
        #[arg(long, default_value_t = 1000)]
        limit: usize,
        /// How many top-coupled pairs to report.
        #[arg(long, default_value_t = 20)]
        top: usize,
    },
}

#[derive(Subcommand)]
enum CheckCommand {
    /// Run a natively implemented checker (see `checker::registry()`) against a file.
    Native { name: String, file: PathBuf },
    /// Run a whole-repo architecture/declaration checker (see
    /// `architecture_checks::registry()`/`declaration_checks::registry()`, resolved via
    /// `check::lookup_any_architecture_checker`) directly against a directory, without
    /// needing a `.claude/inspect.json` check entry for it.
    Architecture { name: String, dir: PathBuf },
    /// List natively implemented checkers available to reference from
    /// `.claude/inspect.json`'s `checker` field.
    List,
    /// Cross-file (repo-wide) duplicate-code detection — see #28. `duplicate-code`
    /// (the `Checker` registered under that name) only compares a file against
    /// itself; this pools the same line-window hashing across every file under `dir`
    /// matching `DuplicateCodeChecker::file_globs()` instead, catching the same logic
    /// copy-pasted across call sites in any of those languages.
    Duplicates { dir: PathBuf },
    /// Backtests a checker (or "all") against file edits reconstructed from Claude
    /// Code session transcripts, to validate it against real historical edits before
    /// shipping it. See docs/backtesting.md.
    Backtest {
        /// Checker name from `checker::registry()`, or "all" to run every checker.
        name: String,
        /// Directory of `<session>/*.jsonl` transcripts (default: `~/.claude/projects`).
        #[arg(long)]
        transcripts_dir: Option<PathBuf>,
        /// Only report findings introduced by the edit itself, dropping ones that
        /// also fired against the pre-edit content.
        #[arg(long)]
        only_new: bool,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Run the daemon in the foreground (background it yourself: `&`, systemd, launchd).
    Start,
    /// Ask a running daemon to shut down.
    Stop,
    /// Report whether a daemon is currently reachable.
    Status,
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Run { dir, trigger } => run::run_batch(dir, &trigger),
        Command::Hook => hook::run_hook(),
        Command::Mcp => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(mcp::run_mcp_server())?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Lsp => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(lsp::run_lsp_server())?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Daemon { action } => match action {
            DaemonAction::Start => {
                daemon::run_daemon(&daemon::default_socket_path())?;
                Ok(ExitCode::SUCCESS)
            }
            DaemonAction::Stop => {
                if daemon::shutdown() {
                    println!("[kibitzer] daemon stopped");
                } else {
                    println!("[kibitzer] no daemon was running");
                }
                Ok(ExitCode::SUCCESS)
            }
            DaemonAction::Status => {
                if daemon::is_alive() {
                    println!("[kibitzer] daemon is running");
                } else {
                    println!("[kibitzer] no daemon running");
                }
                Ok(ExitCode::SUCCESS)
            }
        },
        Command::Check { check } => match check {
            CheckCommand::List => {
                for checker in checker::registry() {
                    let language = checker
                        .language()
                        .map(|l| format!("{l:?}"))
                        .unwrap_or_else(|| "any".to_string());
                    println!(
                        "{}: {} (language: {language}, globs: {})",
                        checker.name(),
                        checker.description(),
                        checker.file_globs().join(", ")
                    );
                }
                Ok(ExitCode::SUCCESS)
            }
            CheckCommand::Native { name, file } => {
                let checker = checker::lookup(&name)
                    .with_context(|| format!("no checker named '{name}' registered"))?;
                let source = std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?;
                let cache = checker::GrammarCache::new();
                let tree = match checker.language() {
                    Some(language) => Some(cache.parse(language, &source)?),
                    None => None,
                };
                let ctx = checker::CheckContext {
                    source: &source,
                    tree: tree.as_ref(),
                };
                let findings = checker.check(&file, &ctx)?;
                if findings.is_empty() {
                    Ok(ExitCode::SUCCESS)
                } else {
                    for finding in &findings {
                        println!("{}:{}: {}", file.display(), finding.line, finding.message);
                    }
                    Ok(ExitCode::from(1))
                }
            }
            CheckCommand::Architecture { name, dir } => run_architecture_cli(&name, &dir),
            CheckCommand::Duplicates { dir } => run_duplicates_cli(&dir),
            CheckCommand::Backtest {
                name,
                transcripts_dir,
                only_new,
            } => {
                let dir = transcripts_dir
                    .or_else(backtest::default_projects_dir)
                    .context("no transcripts directory given and $HOME is unset")?;
                let transcripts = backtest::discover_transcripts(&dir)
                    .with_context(|| format!("discovering transcripts under {}", dir.display()))?;
                let checker_names: Vec<String> = if name == "all" {
                    Vec::new()
                } else {
                    vec![name]
                };
                let cache_path = backtest::default_cache_path();
                let mut cache = backtest::BacktestCache::load(&cache_path);
                let report =
                    backtest::run_backtest(&transcripts, &checker_names, only_new, &mut cache)?;
                cache.save(&cache_path)?;
                println!(
                    "[kibitzer] scanned {} transcript(s), checked {} snapshot(s), {} edit(s) unreconstructable",
                    report.stats.transcripts_scanned,
                    report.stats.snapshots_checked,
                    report.stats.edits_unreconstructable
                );
                for finding in &report.findings {
                    println!(
                        "{}#{} {}:{}: [{}]{} {}",
                        finding.transcript.display(),
                        finding.seq,
                        finding.file_path.display(),
                        finding.line,
                        finding.checker,
                        if finding.pre_existing {
                            " (pre-existing)"
                        } else {
                            ""
                        },
                        finding.message
                    );
                }
                if report.findings.iter().any(|f| !f.pre_existing) {
                    Ok(ExitCode::from(1))
                } else {
                    Ok(ExitCode::SUCCESS)
                }
            }
        },
        Command::Status => status::run_status(),
        Command::Install { global, dry_run } => install::run_install(global, dry_run),
        Command::Architecture { action } => match action {
            ArchitectureAction::Export {
                path,
                scope,
                out,
                dry_run,
                include_private,
            } => arch_export::run_export(path, scope, out, dry_run, include_private),
            ArchitectureAction::Diagram {
                path,
                scope,
                level,
                out,
            } => arch_diagram::run_diagram(path, scope, level, out),
            ArchitectureAction::ChangeCoupling { path, limit, top } => {
                run_change_coupling(&path, limit, top)
            }
        },
        Command::Plugin { action } => match action {
            PluginAction::Install {
                name,
                source,
                force,
            } => plugin::install_plugin(&name, &source, force),
            PluginAction::List => {
                let plugins = plugin::list_plugins()?;
                print_plugin_list(&plugins);
                Ok(ExitCode::SUCCESS)
            }
            PluginAction::Remove { name, force } => plugin::remove_plugin(&name, force),
            PluginAction::Status { name } => match name {
                Some(name) => {
                    let report = plugin::plugin_status(&name)?;
                    println!("{}", format_plugin_status_line(&report));
                    Ok(ExitCode::SUCCESS)
                }
                None => {
                    let plugins = plugin::list_plugins()?;
                    if plugins.is_empty() {
                        println!("[kibitzer] no plugins installed");
                    } else {
                        for installed in &plugins {
                            let report = plugin::plugin_status(&installed.name)?;
                            println!("{}", format_plugin_status_line(&report));
                        }
                    }
                    Ok(ExitCode::SUCCESS)
                }
            },
        },
    }
}

/// `kibitzer plugin list`'s rendering (Task 3.3.1a): one line per plugin, or the exact
/// literal `[kibitzer] no plugins installed` when the registry is empty (Story 3.1.1's AC).
fn print_plugin_list(plugins: &[plugin::InstalledPlugin]) {
    if plugins.is_empty() {
        println!("[kibitzer] no plugins installed");
        return;
    }
    for installed in plugins {
        println!(
            "{} v{} — {}",
            installed.name,
            installed.version,
            installed.binary_path.display()
        );
    }
}

/// `kibitzer plugin status`'s per-plugin rendering (Task 3.3.1b): `ok` when the binary is
/// present, its hash matches, and it's version-compatible; otherwise the specific failing
/// dimension(s), always containing the literal substring `binary missing` when the file is
/// gone.
fn format_plugin_status_line(report: &plugin::PluginStatusReport) -> String {
    let name = &report.installed.name;
    let version = &report.installed.version;
    if !report.binary_present {
        return format!(
            "{name} v{version}: binary missing (expected at {})",
            report.installed.binary_path.display()
        );
    }
    if report.hash_matches && report.version_compatible {
        return format!("{name} v{version}: ok");
    }
    let mut issues = Vec::new();
    if !report.hash_matches {
        issues.push("hash mismatch");
    }
    if !report.version_compatible {
        issues.push("incompatible with the currently running kibitzer version");
    }
    format!("{name} v{version}: {}", issues.join(", "))
}

/// `kibitzer check architecture <name> <dir>` (Story 1.2.2): runs one whole-repo
/// architecture/declaration checker directly against `dir`, bypassing `.claude/inspect.json`'s
/// `checks` list entirely — `name` only needs to be registered in
/// `architecture_checks::registry()`/`declaration_checks::registry()`, not referenced by
/// any configured check. `dir`'s own `.claude/inspect.json` (if any) still supplies the
/// `ArchitectureConfig` (`components`/`dependency_rules`/etc.) the checker runs against,
/// same as batch mode; an absent config just means an empty one (most checkers report no
/// findings against zero declared components/rules).
///
/// Extracted as a plain function (rather than inlined in the `match` arm) so it's directly
/// callable from `#[cfg(test)]` — `main.rs` has no subprocess-test precedent to match
/// (Task 1.2.2c).
fn run_architecture_cli(name: &str, dir: &Path) -> Result<ExitCode> {
    let Some(any_checker) = check::lookup_any_architecture_checker(name) else {
        eprintln!("no architecture checker named '{name}' registered");
        return Ok(ExitCode::from(1));
    };

    let files =
        check::walk_and_collect_files(dir).with_context(|| format!("walking {}", dir.display()))?;
    let arch_config = config::find_config(dir)?
        .map(|(config, _)| config.architecture)
        .unwrap_or_default();

    let findings = match any_checker {
        check::AnyArchitectureChecker::Import(checker) => {
            let graph = import_graph::build(dir, &files)
                .with_context(|| format!("building import graph for {}", dir.display()))?;
            checker.check(&graph, &arch_config)
        }
        check::AnyArchitectureChecker::Model(checker) => {
            let model = check::build_arch_model_for_check(dir, &files)
                .with_context(|| format!("building architecture model for {}", dir.display()))?;
            checker.check(&model, &arch_config)
        }
        check::AnyArchitectureChecker::Declaration(checker) => {
            let components = arch_config.effective_components();
            let graph = declarations::build(dir, &files, &components)
                .with_context(|| format!("building declaration graph for {}", dir.display()))?;
            checker.check(&graph, &arch_config)
        }
    };

    if findings.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }

    for finding in &findings {
        let location = match (&finding.file, finding.line) {
            (Some(file), Some(line)) => format!("{}:{}: ", file.display(), line),
            (Some(file), None) => format!("{}: ", file.display()),
            (None, _) => String::new(),
        };
        println!("{location}{}", finding.message);
    }
    Ok(ExitCode::from(1))
}

/// `kibitzer architecture change-coupling`: reports temporal coupling (see
/// `change_coupling.rs`) over `path`'s git history. Always exits `ExitCode::SUCCESS` when
/// the analysis itself succeeds, regardless of what it finds — same "report, don't
/// gate" convention as `run_export` (no pass/fail concept for a prioritization report).
fn run_change_coupling(path: &Path, limit: usize, top: usize) -> Result<ExitCode> {
    let pairs = change_coupling::analyze(path, limit, top)
        .with_context(|| format!("analyzing change coupling for {}", path.display()))?;

    if pairs.is_empty() {
        println!("[kibitzer] no coupled file pairs found above threshold");
        return Ok(ExitCode::SUCCESS);
    }

    for pair in &pairs {
        println!(
            "{:.0}% coupled ({}/{} shared revisions): {} <-> {}",
            pair.coupling * 100.0,
            pair.shared_commits,
            pair.revisions_a + pair.revisions_b - pair.shared_commits,
            pair.file_a,
            pair.file_b
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Covers every language `duplicate-code` (single-file) covers — see
/// `duplicate_code::DuplicateCodeChecker::file_globs()` — so a block copy-pasted
/// across files is caught regardless of language.
fn run_duplicates_cli(dir: &Path) -> Result<ExitCode> {
    let files = collect_files_for_duplicate_scan(dir)?;
    let duplicates = duplicate_code::find_cross_file_duplicates(&files);
    if duplicates.is_empty() {
        return Ok(ExitCode::SUCCESS);
    }
    print_cross_file_duplicates(&duplicates);
    Ok(ExitCode::from(1))
}

/// Walks `dir` and reads every file whose extension matches
/// `DuplicateCodeChecker::file_globs()`, skipping anything else (build output,
/// non-source config, etc).
fn collect_files_for_duplicate_scan(dir: &Path) -> Result<Vec<(PathBuf, String)>> {
    use checker::Checker as _;

    let extensions: std::collections::HashSet<&str> = duplicate_code::DuplicateCodeChecker
        .file_globs()
        .iter()
        .filter_map(|glob| glob.rsplit('.').next())
        .collect();

    let all_files =
        check::walk_and_collect_files(dir).with_context(|| format!("walking {}", dir.display()))?;
    let mut files = Vec::new();
    for path in all_files {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if !extensions.contains(ext) {
            continue;
        }
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        files.push((path, source));
    }
    Ok(files)
}

fn print_cross_file_duplicates(duplicates: &[duplicate_code::CrossFileDuplicate]) {
    for dup in duplicates {
        let locations: Vec<String> = dup
            .occurrences
            .iter()
            .map(|o| format!("{}:{}", o.file.display(), o.line))
            .collect();
        let file_count = dup
            .occurrences
            .iter()
            .map(|o| &o.file)
            .collect::<std::collections::HashSet<_>>()
            .len();
        let first = &dup.occurrences[0];
        println!(
            "{}:{}: block repeated {} times across {file_count} files (locations: {}) — consider extracting a shared function",
            first.file.display(),
            first.line,
            dup.occurrences.len(),
            locations.join(", ")
        );
    }
}

#[cfg(test)]
mod architecture_cli_tests {
    use super::*;

    struct TempRepo {
        dir: PathBuf,
    }

    impl TempRepo {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "kibitzer-main-cli-test-{}-{name}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self { dir }
        }

        fn write(&self, rel_path: &str, content: &str) {
            let path = self.dir.join(rel_path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, content).unwrap();
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    // Proves criterion 8 / Task 1.2.2c generically against a checker guaranteed to be
    // registered regardless of Epic 1.1's concurrent landing status: a Go import cycle
    // between two packages, checked via the already-registered `import-cycles`
    // architecture checker rather than the not-yet-necessarily-landed `component-deps`.
    // The `testdata/dogfood-architecture` fixture named in validation.md's own acceptance
    // criterion is Phase 7 scope and doesn't exist yet.
    #[test]
    fn cli_architecture_check_flags_violation_and_exits_nonzero() {
        let repo = TempRepo::new("import-cycle");
        repo.write("go.mod", "module kibitzer.example/cycletest\n\ngo 1.21\n");
        repo.write(
            "a/a.go",
            "package a\n\nimport _ \"kibitzer.example/cycletest/b\"\n",
        );
        repo.write(
            "b/b.go",
            "package b\n\nimport _ \"kibitzer.example/cycletest/a\"\n",
        );

        let exit = run_architecture_cli("import-cycles", &repo.dir).unwrap();
        assert_eq!(exit, ExitCode::from(1));
    }

    #[test]
    fn cli_architecture_check_unknown_checker_exits_nonzero_with_message() {
        let repo = TempRepo::new("unknown-checker");
        let exit = run_architecture_cli("does-not-exist", &repo.dir).unwrap();
        assert_eq!(exit, ExitCode::from(1));
    }

    #[test]
    fn cli_architecture_check_passes_on_clean_directory() {
        let repo = TempRepo::new("clean");
        repo.write("go.mod", "module kibitzer.example/cleantest\n\ngo 1.21\n");
        repo.write("a/a.go", "package a\n");

        let exit = run_architecture_cli("import-cycles", &repo.dir).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    // Regression test for the Declaration arm of `run_architecture_cli`, which used to
    // print a Phase-2-era stub message ("not yet implemented (Phase 2)") for every
    // Declaration-kind checker instead of building a DeclarationGraph and dispatching,
    // even after ContentChecker/NamingChecker landed. `check.rs::run_architecture_check`
    // (the batch/MCP dispatch path) already did this correctly; this proves the CLI verb
    // now matches it for `naming-rules`.
    #[test]
    fn cli_architecture_check_dispatches_naming_rules_declaration_checker() {
        let repo = TempRepo::new("naming-rules");
        repo.write("go.mod", "module kibitzer.example/namingtest\n\ngo 1.21\n");
        repo.write(
            ".claude/inspect.json",
            r#"{"architecture": {"components": [{"name": "infra", "paths": ["**/infra", "**/infra/**"]}], "naming_rules": [{"component": "infra", "kind": "struct", "pattern": "^.*(Repository|Client)$"}]}}"#,
        );
        repo.write(
            "infra/infra.go",
            "package infra\n\ntype OrderStore struct{}\n",
        );

        let exit = run_architecture_cli("naming-rules", &repo.dir).unwrap();
        assert_eq!(exit, ExitCode::from(1));
    }

    /// Same regression concern as the naming-rules test above, for the newer `Model` arm
    /// (`AnyArchitectureChecker::Model`) added alongside `InstabilityChecker` —
    /// `check.rs::run_architecture_check` has its own dispatch test; this proves the CLI
    /// verb (`run_architecture_cli`'s separate match arm) reaches the checker too.
    #[test]
    fn cli_architecture_check_dispatches_instability_model_checker() {
        let repo = TempRepo::new("instability");
        repo.write(
            "go.mod",
            "module kibitzer.example/instabilitytest\n\ngo 1.21\n",
        );
        repo.write(
            "stable/stable.go",
            "package stable\n\ntype Widget struct{}\n",
        );
        repo.write(
            "consumer/consumer.go",
            "package consumer\n\nimport \"kibitzer.example/instabilitytest/stable\"\n\n\
             var _ = stable.Widget{}\n",
        );

        let exit = run_architecture_cli("instability", &repo.dir).unwrap();
        assert_eq!(exit, ExitCode::from(1));
    }

    /// `change_coupling.rs`'s own tests cover the coupling math and the real-`git log`
    /// parsing path end-to-end; this proves `run_change_coupling` (the CLI-verb wrapper —
    /// argument plumbing and output formatting) actually reaches it without erroring,
    /// which none of those lower-level tests exercise.
    #[test]
    fn run_change_coupling_cli_verb_succeeds_against_a_real_git_repo() {
        let repo = TempRepo::new("change-coupling");
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&repo.dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "test"]);
        for i in 0..10 {
            repo.write("a.txt", &format!("{i}"));
            repo.write("b.txt", &format!("{i}"));
            git(&["add", "a.txt", "b.txt"]);
            git(&["commit", "-q", "-m", &format!("commit {i}")]);
        }

        let exit = run_change_coupling(&repo.dir, 1000, 20).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn cli_duplicates_flags_block_repeated_across_go_files() {
        let repo = TempRepo::new("cross-file-dup");
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        repo.write("pkg1/a.go", &format!("package pkg1\n\n{block}"));
        repo.write("pkg2/b.go", &format!("package pkg2\n\n{block}"));
        repo.write("pkg3/c.go", &format!("package pkg3\n\n{block}"));

        let exit = run_duplicates_cli(&repo.dir).unwrap();
        assert_eq!(exit, ExitCode::from(1));
    }

    #[test]
    fn cli_duplicates_flags_block_repeated_across_ts_files() {
        let repo = TempRepo::new("cross-file-dup-ts");
        let block = "function doWork(id) {\n  const conn = openConnection(id);\n  const result = conn.fetch(id);\n  console.log(result);\n  conn.close();\n  return conn.validate(result);\n}\n";
        repo.write("a.ts", block);
        repo.write("b.ts", block);
        repo.write("c.ts", block);

        let exit = run_duplicates_cli(&repo.dir).unwrap();
        assert_eq!(exit, ExitCode::from(1));
    }

    #[test]
    fn cli_duplicates_ignores_files_outside_duplicate_code_globs() {
        let repo = TempRepo::new("cross-file-dup-unsupported-ext");
        let block = "some duplicated prose line one\nsome duplicated prose line two\nsome duplicated prose line three\nsome duplicated prose line four\nsome duplicated prose line five\nsome duplicated prose line six\n";
        repo.write("a.md", block);
        repo.write("b.md", block);
        repo.write("c.md", block);

        let exit = run_duplicates_cli(&repo.dir).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }

    #[test]
    fn cli_duplicates_passes_on_clean_directory() {
        let repo = TempRepo::new("cross-file-dup-clean");
        repo.write(
            "a.go",
            "package main\n\nfunc a() {\n\tfmt.Println(\"a\")\n}\n",
        );
        repo.write(
            "b.go",
            "package main\n\nfunc b() {\n\tfmt.Println(\"b\")\n}\n",
        );

        let exit = run_duplicates_cli(&repo.dir).unwrap();
        assert_eq!(exit, ExitCode::SUCCESS);
    }
}
