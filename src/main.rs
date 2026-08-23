mod architecture_checks;
mod backtest;
mod cache;
mod check;
mod checker;
mod config;
mod daemon;
mod dedup;
mod duplicate_code;
mod glob;
mod go_blank_imports;
mod go_error_context;
mod go_ignored_error;
mod hook;
mod import_graph;
mod install;
mod lsp;
mod markdown_link_integrity;
mod mcp;
mod mermaid;
mod primitive_obsession;
mod rules;
mod run;

use std::path::PathBuf;
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
}

#[derive(Subcommand)]
enum CheckCommand {
    /// Run a natively implemented checker (see `checker::registry()`) against a file.
    Native { name: String, file: PathBuf },
    /// List natively implemented checkers available to reference from
    /// `.claude/inspect.json`'s `checker` field.
    List,
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
        Command::Install { global, dry_run } => install::run_install(global, dry_run),
    }
}
