use std::path::Path;

use tower_lsp::jsonrpc::Result as RpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::check::{CheckResult, run_checks_for_trigger};
use crate::config::{Severity, find_config};

/// Trigger name checks opt into via `.claude/inspect.json`'s `triggers` field to run under
/// `kibitzer lsp` specifically; a check with no `triggers` runs under every trigger,
/// including this one, same as "PostToolUse" and "batch".
const LSP_TRIGGER: &str = "lsp";

fn severity_to_lsp(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Blocking => DiagnosticSeverity::ERROR,
        Severity::Advisory => DiagnosticSeverity::WARNING,
    }
}

/// Translate one check's result into LSP diagnostics. When the output follows the
/// `{file}:{line}: message` convention (see `check::scope_output_to_changed_lines`),
/// each matched line becomes its own diagnostic at that line; otherwise (a whole-repo
/// check, or output that just doesn't follow the convention) the whole result collapses
/// into a single diagnostic on line 1 using `describe()`, so a failure is never dropped
/// just because it can't be attributed to a specific line.
fn diagnostics_from_result(result: &CheckResult, file_path: &Path) -> Vec<Diagnostic> {
    if result.passed {
        return Vec::new();
    }

    let severity = severity_to_lsp(result.severity);
    let prefix = format!("{}:", file_path.display());
    let mut diagnostics = Vec::new();

    for line in result.output.lines() {
        if let Some(rest) = line.strip_prefix(&prefix)
            && let Some((line_no_str, message)) = rest.split_once(':')
            && let Ok(line_no) = line_no_str.trim().parse::<u32>()
        {
            let zero_indexed = line_no.saturating_sub(1);
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position::new(zero_indexed, 0),
                    end: Position::new(zero_indexed, u32::MAX),
                },
                severity: Some(severity),
                source: Some(result.check_name.clone()),
                message: message.trim().to_string(),
                ..Default::default()
            });
        }
    }

    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, u32::MAX)),
            severity: Some(severity),
            source: Some(result.check_name.clone()),
            message: result.describe(),
            ..Default::default()
        });
    }

    diagnostics
}

/// Run every in-scope check against `path` (as it currently exists on disk — see the
/// module-level caveat about `did_change`) and translate the results into diagnostics.
fn diagnostics_for_file(path: &Path) -> anyhow::Result<Vec<Diagnostic>> {
    let Some((config, repo_root)) = find_config(path)? else {
        return Ok(Vec::new());
    };
    let results = run_checks_for_trigger(&config.checks, LSP_TRIGGER, &repo_root, path, None)?;
    Ok(results
        .iter()
        .flat_map(|r| diagnostics_from_result(r, path))
        .collect())
}

struct Backend {
    client: Client,
}

impl Backend {
    /// Re-run checks against `uri` and publish the result, replacing whatever
    /// diagnostics that file previously had.
    ///
    /// Every check reads `path` off disk (shell commands substitute `{file}`; native
    /// checkers `std::fs::read_to_string` it) rather than the editor's in-memory buffer,
    /// so diagnostics reflect the last-saved content, not unsaved keystrokes — the same
    /// disk-based model `run_checks_for_trigger` already uses for the Claude Code hook
    /// and batch mode. Wiring in the live buffer (via LSP's incremental sync) is real
    /// future work, called out in issue #11, not done here.
    async fn check_and_publish(&self, uri: Url) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        match diagnostics_for_file(&path) {
            Ok(diagnostics) => {
                self.client
                    .publish_diagnostics(uri, diagnostics, None)
                    .await
            }
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, format!("kibitzer: {err:#}"))
                    .await;
            }
        }
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _: InitializeParams) -> RpcResult<InitializeResult> {
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "kibitzer".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                ..Default::default()
            },
        })
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.check_and_publish(params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.check_and_publish(params.text_document.uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.check_and_publish(params.text_document.uri).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }
}

pub async fn run_lsp_server() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|client| Backend { client });
    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Severity;
    use std::path::PathBuf;

    fn result(severity: Severity, passed: bool, output: &str) -> CheckResult {
        CheckResult {
            check_name: "test-check".to_string(),
            severity,
            passed,
            output: output.to_string(),
            message: None,
            command: "true".to_string(),
        }
    }

    #[test]
    fn passing_result_has_no_diagnostics() {
        let path = PathBuf::from("src/main.go");
        let r = result(Severity::Blocking, true, "");
        assert!(diagnostics_from_result(&r, &path).is_empty());
    }

    #[test]
    fn line_attributed_output_becomes_one_diagnostic_per_line() {
        let path = PathBuf::from("src/main.go");
        let r = result(
            Severity::Advisory,
            false,
            "src/main.go:12: something's off\nsrc/main.go:40: something else",
        );
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].range.start.line, 11);
        assert_eq!(diags[0].message, "something's off");
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diags[1].range.start.line, 39);
        assert_eq!(diags[1].message, "something else");
    }

    #[test]
    fn blocking_severity_maps_to_error() {
        let path = PathBuf::from("f.go");
        let r = result(Severity::Blocking, false, "f.go:1: bad");
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn output_without_line_attribution_collapses_to_one_diagnostic() {
        let path = PathBuf::from("f.go");
        let r = result(Severity::Advisory, false, "some whole-repo failure text");
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "some whole-repo failure text");
        assert_eq!(diags[0].range.start.line, 0);
    }

    #[test]
    fn message_and_output_are_combined_via_describe() {
        let path = PathBuf::from("f.go");
        let mut r = result(Severity::Advisory, false, "why this is bad");
        r.message = Some("rule explanation".to_string());
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "rule explanation\nwhy this is bad");
    }
}
