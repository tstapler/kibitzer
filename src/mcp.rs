use std::path::PathBuf;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::check::{
    run_architecture_check, run_check, run_checks_for_trigger, walk_and_collect_files,
};
use crate::config::{Check, Severity, find_config};
use crate::glob::matches_scope;

#[derive(Debug, Clone)]
pub struct KibitzerServer {
    tool_router: ToolRouter<Self>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct RunChecksRequest {
    /// Absolute path to the file to check.
    file_path: String,
    /// Trigger name (e.g. "PostToolUse" or "batch"); checks with no triggers always run.
    #[serde(default = "default_trigger")]
    trigger: String,
}

fn default_trigger() -> String {
    "batch".to_string()
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ListChecksRequest {
    /// Any path inside the repo whose `.claude/inspect.json` should be listed.
    path: String,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct ArchitectureAssessmentRequest {
    /// Any path inside the repo to assess (the repo root or a subdirectory).
    path: String,
    /// Optional glob (relative to the repo root, `**` supported) restricting which files
    /// are in scope. Applies to both the import graph and the per-file complexity pass.
    /// Defaults to the whole repo.
    #[serde(default)]
    scope: Option<String>,
    /// Whether to append a Mermaid dependency-graph section (`graph TD`, import-cycle
    /// edges highlighted). Repos over 150 nodes fall back to a text note instead —
    /// pass a narrower `scope` to render a subgraph.
    #[serde(default = "default_true")]
    include_diagram: bool,
}

fn default_true() -> bool {
    true
}

/// Names of the natively registered per-file complexity checkers (one per language) that
/// an architecture assessment runs across every in-scope file, alongside whichever
/// `architecture_checker`s the repo's `.claude/inspect.json` configures. Kept as a fixed
/// list rather than deriving from `checker::registry()` so a future non-complexity native
/// checker (e.g. `primitive-obsession`) isn't silently swept into "architecture."
const SYNTAX_RULES_CHECKERS: &[&str] = &[
    "syntax-rules",
    "syntax-rules-typescript",
    "syntax-rules-tsx",
    "syntax-rules-javascript",
    "syntax-rules-python",
    "syntax-rules-java",
    "syntax-rules-kotlin",
];

/// Canned per-rule-id recommendation text for findings whose message doesn't already
/// embed one (coupling/long-function/deep-nesting/long-parameter-list all do, inline).
fn recommendation_for(check_name: &str) -> Option<&'static str> {
    match check_name {
        "import-cycles" => Some(
            "import-cycles: break the cycle by extracting the shared pieces both packages \
             depend on into a third package, or inverting one side to depend on an \
             interface instead of the concrete package.",
        ),
        "layering" => Some(
            "layering: move the offending import behind an interface owned by the higher \
             layer, or relocate the responsibility that requires it into a layer that's \
             already allowed to depend downward.",
        ),
        _ => None,
    }
}

#[tool_router(router = tool_router)]
impl KibitzerServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "List the checks configured in the nearest .claude/inspect.json above the given path."
    )]
    async fn list_checks(&self, req: Parameters<ListChecksRequest>) -> String {
        let path = PathBuf::from(&req.0.path);
        match find_config(&path) {
            Ok(Some((config, root))) => {
                let names: Vec<String> = config
                    .checks
                    .iter()
                    .map(|c| format!("{} ({:?}, scope={:?})", c.name, c.severity, c.scope))
                    .collect();
                format!(
                    "config root: {}\nchecks:\n{}",
                    root.display(),
                    names.join("\n")
                )
            }
            Ok(None) => "no .claude/inspect.json found above this path".to_string(),
            Err(e) => format!("error reading config: {e}"),
        }
    }

    #[tool(
        description = "Run a whole-repo architecture assessment: configured architecture_checker \
                        checks (import cycles, layering, coupling) plus per-file complexity rules \
                        across every in-scope file, with canned recommendations for structural findings."
    )]
    async fn architecture_assessment(
        &self,
        req: Parameters<ArchitectureAssessmentRequest>,
    ) -> String {
        let path = PathBuf::from(&req.0.path);
        let (config, repo_root) = match find_config(&path) {
            Ok(Some(c)) => c,
            Ok(None) => return "no .claude/inspect.json found above this path".to_string(),
            Err(e) => return format!("error reading config: {e}"),
        };

        let mut files = match walk_and_collect_files(&repo_root) {
            Ok(files) => files,
            Err(e) => return format!("error walking repo: {e}"),
        };
        if let Some(scope) = &req.0.scope {
            let scopes = [scope.clone()];
            files.retain(|f| {
                let rel = f
                    .strip_prefix(&repo_root)
                    .unwrap_or(f)
                    .to_string_lossy()
                    .to_string();
                matches_scope(&rel, &scopes)
            });
        }

        let mut lines: Vec<String> = Vec::new();
        let mut recommendations: Vec<&'static str> = Vec::new();
        let mut finding_count = 0usize;

        for check in &config.checks {
            if check.architecture_checker.is_none() {
                continue;
            }
            let result =
                match run_architecture_check(check, &repo_root, &files, &config.architecture) {
                    Ok(r) => r,
                    Err(e) => {
                        lines.push(format!("error running {}: {e}", check.name));
                        continue;
                    }
                };
            if result.passed {
                continue;
            }
            let level = match result.severity {
                Severity::Blocking => "blocking",
                Severity::Advisory => "advisory",
            };
            for finding_line in result.output.lines().filter(|l| !l.is_empty()) {
                lines.push(format!("[{level}] {finding_line}"));
                finding_count += 1;
            }
            if let Some(rec) = recommendation_for(&check.name) {
                recommendations.push(rec);
            }
        }

        for checker_name in SYNTAX_RULES_CHECKERS {
            let severity = config
                .checks
                .iter()
                .find(|c| c.checker.as_deref() == Some(*checker_name))
                .map(|c| c.severity)
                .unwrap_or(Severity::Advisory);
            let synthetic = Check {
                name: (*checker_name).to_string(),
                command: None,
                checker: Some((*checker_name).to_string()),
                architecture_checker: None,
                severity,
                scope: vec![],
                triggers: vec![],
                message: None,
                output_format: None,
            };
            for file in &files {
                let result = match run_check(&synthetic, &repo_root, file, None) {
                    Ok(r) => r,
                    Err(e) => {
                        lines.push(format!(
                            "error running {checker_name} on {}: {e}",
                            file.display()
                        ));
                        continue;
                    }
                };
                if result.passed || result.output.is_empty() {
                    continue;
                }
                let level = match result.severity {
                    Severity::Blocking => "blocking",
                    Severity::Advisory => "advisory",
                };
                for finding_line in result.output.lines().filter(|l| !l.is_empty()) {
                    lines.push(format!("[{level}] {finding_line}"));
                    finding_count += 1;
                }
            }
        }

        let mut output = format!(
            "architecture assessment: {} finding(s) across {} file(s)\n",
            finding_count,
            files.len()
        );
        if lines.is_empty() {
            output.push_str("no findings\n");
        } else {
            output.push_str(&lines.join("\n"));
            output.push('\n');
        }
        if !recommendations.is_empty() {
            recommendations.sort_unstable();
            recommendations.dedup();
            output.push_str("\n## Recommendations\n");
            for rec in recommendations {
                output.push_str(&format!("- {rec}\n"));
            }
        }
        output.push_str("\n## Dependency graph\n");
        if req.0.include_diagram {
            match crate::import_graph::build(&repo_root, &files) {
                Ok(graph) => {
                    let diagram = crate::mermaid::render_dependency_graph(&graph);
                    if diagram.starts_with("graph TD") {
                        output.push_str("```mermaid\n");
                        output.push_str(&diagram);
                        output.push_str("\n```\n");
                    } else {
                        output.push_str(&diagram);
                        output.push('\n');
                    }
                }
                Err(e) => output.push_str(&format!("error building import graph: {e}\n")),
            }
        } else {
            output.push_str("(omitted: include_diagram was false)\n");
        }
        output
    }

    #[tool(
        description = "Run all in-scope checks against a single file for the given trigger and report failures."
    )]
    async fn run_checks(&self, req: Parameters<RunChecksRequest>) -> String {
        let file_path = PathBuf::from(&req.0.file_path);
        let config = match find_config(&file_path) {
            Ok(Some(c)) => c,
            Ok(None) => return "no .claude/inspect.json found above this file".to_string(),
            Err(e) => return format!("error reading config: {e}"),
        };
        let (config, repo_root) = config;
        match run_checks_for_trigger(&config.checks, &req.0.trigger, &repo_root, &file_path, None) {
            Ok(results) => {
                let failures: Vec<String> = results
                    .iter()
                    .filter(|r| !r.passed)
                    .map(|r| format!("[{:?}] {}: {}", r.severity, r.check_name, r.describe()))
                    .collect();
                if failures.is_empty() {
                    "all checks passed".to_string()
                } else {
                    failures.join("\n")
                }
            }
            Err(e) => format!("error running checks: {e}"),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for KibitzerServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "kibitzer: cross-language code/doc inspection. Use list_checks to discover \
                 configured checks, run_checks to inspect a single file, and \
                 architecture_assessment for a whole-repo structural + complexity review."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}

pub async fn run_mcp_server() -> Result<()> {
    let server = KibitzerServer::new()
        .serve(rmcp::transport::stdio())
        .await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-mcp-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Builds a two-package Go fixture with a real import cycle and a declared-layers
    /// violation (`domain` importing back into `handlers`), git-initialized so the
    /// pre-existing-violation baseline machinery in `run_architecture_check` has a HEAD
    /// to diff against.
    fn write_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("handlers")).unwrap();
        std::fs::create_dir_all(dir.join("domain")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(
            dir.join("handlers/handlers.go"),
            "package handlers\n\nimport \"fixture/domain\"\n\nfunc Do() { domain.Do() }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("domain/domain.go"),
            "package domain\n\nimport \"fixture/handlers\"\n\nfunc Do() { handlers.Do() }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": { "layers": ["handlers", "domain"] },
  "checks": [
    { "name": "import-cycles", "architecture_checker": "import-cycles", "severity": "advisory" },
    { "name": "layering", "architecture_checker": "layering", "severity": "advisory" }
  ]
}"#,
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec!["commit", "-q", "-m", "init"],
        ] {
            let status = Command::new("git")
                .args(&args)
                .current_dir(dir)
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?} failed");
        }
    }

    #[tokio::test]
    async fn architecture_assessment_reports_cycle_and_layering_findings() {
        let dir = tmp_dir("cycle-and-layering");
        write_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .architecture_assessment(Parameters(ArchitectureAssessmentRequest {
                path: dir.display().to_string(),
                scope: None,
                include_diagram: true,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        assert!(
            output.contains("import cycle"),
            "expected an import-cycle finding, got:\n{output}"
        );
        assert!(
            output.contains("layering violation"),
            "expected a layering finding, got:\n{output}"
        );
        assert!(output.contains("## Recommendations"));
        assert!(output.contains("## Dependency graph"));
    }

    #[tokio::test]
    async fn architecture_assessment_reports_no_findings_for_clean_repo() {
        let dir = tmp_dir("clean-repo");
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{"checks": [{"name": "import-cycles", "architecture_checker": "import-cycles", "severity": "advisory"}]}"#,
        )
        .unwrap();

        let server = KibitzerServer::new();
        let output = server
            .architecture_assessment(Parameters(ArchitectureAssessmentRequest {
                path: dir.display().to_string(),
                scope: None,
                include_diagram: true,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        assert!(output.contains("no findings"), "got:\n{output}");
    }
}
