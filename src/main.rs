mod cache;
mod check;
mod checker;
mod config;
mod daemon;
mod glob;
mod hook;
mod markdown_link_integrity;
mod mcp;
mod primitive_obsession;
mod run;

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
}

#[derive(Subcommand)]
enum CheckCommand {
    /// Run a natively implemented checker (see `checker::registry()`) against a file.
    Native { name: String, file: PathBuf },
    /// List natively implemented checkers available to reference from
    /// `.claude/inspect.json`'s `checker` field.
    List,
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
<<<<<<< HEAD
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
        },
=======
        Command::Check { check } => {
            let (checker, file): (Box<dyn checker::Checker>, PathBuf) = match check {
                CheckCommand::PrimitiveObsession { file } => (
                    Box::new(primitive_obsession::PrimitiveObsessionChecker),
                    file,
                ),
                CheckCommand::MarkdownLinkIntegrity { file } => (
                    Box::new(markdown_link_integrity::MarkdownLinkIntegrityChecker),
                    file,
                ),
            };
            run_checker(checker.as_ref(), &file)
        }
    }
}

fn run_checker(checker: &dyn checker::Checker, file: &Path) -> Result<ExitCode> {
    let findings = checker.check_file(file)?;
    if findings.is_empty() {
        Ok(ExitCode::SUCCESS)
    } else {
        for finding in &findings {
            println!("{}:{}: {}", file.display(), finding.line, finding.message);
        }
        Ok(ExitCode::from(1))
>>>>>>> origin/master
    }
}
