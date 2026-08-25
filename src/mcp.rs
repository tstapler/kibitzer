use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arch_model::{ModelCache, ModelLevel, SymbolKind, SymbolNode, load_cached_model};
use crate::check::{
    run_architecture_check, run_check, run_checks_for_trigger, walk_and_collect_files,
};
use crate::config::{Check, Severity, find_config};
use crate::glob::matches_scope;

#[derive(Debug, Clone)]
pub struct KibitzerServer {
    tool_router: ToolRouter<Self>,
    /// Single-slot, in-process cache shared by `list_architecture_symbols` and
    /// `get_architecture_node` (ADR-002) — `Arc`-wrapped so cloning `KibitzerServer` (the
    /// MCP framework's handler-clone convention) shares one cache instance, not a fresh
    /// empty one per clone.
    model_cache: Arc<ModelCache>,
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

#[derive(Serialize, Deserialize, JsonSchema)]
struct ListArchitectureSymbolsRequest {
    /// Any path inside the repo to query (the repo root or a subdirectory).
    path: String,
    /// Optional glob (relative to the repo root, `**` supported) restricting which
    /// packages are in scope. Defaults to the whole repo (no scope filter).
    #[serde(default)]
    scope: Option<String>,
    /// Restrict results to symbols belonging to exactly this package path (an
    /// `ArchModel::package` key, e.g. a Go module-qualified import path or a JS/TS
    /// directory path). Defaults to no package filter (all packages considered).
    #[serde(default)]
    package: Option<String>,
    /// Restrict results to one symbol kind: "type", "interface", "function", or
    /// "method". Defaults to no kind filter (all kinds returned).
    #[serde(default)]
    kind: Option<String>,
    /// "component" or "code". Defaults to "code" (individual symbols returned);
    /// "component" returns packages with `symbols` cleared, so it always yields zero
    /// symbol matches — pass "code" (or omit `level`) to see symbols.
    #[serde(default = "default_level")]
    level: String,
    /// Whether to include unexported/private symbols. Defaults to false (exported-only,
    /// matching every other pruning default in this tool).
    #[serde(default)]
    include_private: bool,
    /// Maximum number of symbols to return in one page. Defaults to 200; values above
    /// 1000 are clamped down to 1000.
    #[serde(default = "default_limit")]
    limit: usize,
    /// Opaque pagination cursor from a previous response's `next_cursor`. Defaults to
    /// `None`, which starts from the first match.
    #[serde(default)]
    cursor: Option<String>,
}

fn default_level() -> String {
    "code".to_string()
}

fn default_limit() -> usize {
    200
}

#[derive(Serialize)]
struct SymbolListEntry {
    package: String,
    symbol: SymbolNode,
}

#[derive(Serialize)]
struct ListArchitectureSymbolsResponse {
    total_matched: usize,
    returned: usize,
    next_cursor: Option<String>,
    /// True when `total_matched == 0`, `include_private` was false, and the pruning
    /// summary shows symbols were excluded by that default (scoped to `package`'s
    /// prefix when set) — distinguishes "nothing here" from "hidden by the exported-only
    /// default." See Story 3.1.1's pre-mortem P2 #2 finding.
    possibly_pruned: bool,
    symbols: Vec<SymbolListEntry>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct GetArchitectureNodeRequest {
    /// Any path inside the repo to query (the repo root or a subdirectory).
    path: String,
    /// The node to resolve: tried first as an exact `ArchModel::package` path, then as a
    /// `SymbolNode::id` (`"<package>::<Name>"`, or `"<package>::<Type>.<Method>"` for an
    /// owner-qualified method).
    node: String,
}

/// Serializes an ad hoc `{"error": "..."}` JSON object — kept as JSON (not a plain
/// string) so a caller of these two JSON-returning tools never has to branch on response
/// shape between the success and failure path, per ADR-001.
fn json_error(message: String) -> String {
    serde_json::to_string(&serde_json::json!({ "error": message }))
        .unwrap_or_else(|_| "{\"error\":\"failed to serialize error\"}".to_string())
}

fn symbol_kind_matches(kind: SymbolKind, want: &str) -> bool {
    match kind {
        SymbolKind::Type => want.eq_ignore_ascii_case("type"),
        SymbolKind::Interface => want.eq_ignore_ascii_case("interface"),
        SymbolKind::Function => want.eq_ignore_ascii_case("function"),
        SymbolKind::Method => want.eq_ignore_ascii_case("method"),
    }
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
            model_cache: Arc::new(ModelCache::new()),
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

    #[tool(
        description = "Query the repo's architecture model for a paginated, filtered slice of \
                        symbols (by package/kind/name) — returns JSON \
                        ({total_matched, returned, next_cursor, possibly_pruned, symbols}), \
                        not prose. Use this for a scoped lookup ('what does package X export?') \
                        instead of the whole-repo architecture_assessment report."
    )]
    async fn list_architecture_symbols(
        &self,
        req: Parameters<ListArchitectureSymbolsRequest>,
    ) -> String {
        let req = req.0;
        let path = PathBuf::from(&req.path);
        let repo_root = match find_config(&path) {
            Ok(Some((_, root))) => root,
            Ok(None) => {
                return json_error("no .claude/inspect.json found above this path".to_string());
            }
            Err(e) => return json_error(format!("error reading config: {e}")),
        };

        let level = if req.level.eq_ignore_ascii_case("component") {
            ModelLevel::Component
        } else {
            ModelLevel::Code
        };
        let limit = req.limit.clamp(1, 1000);

        // `None` legitimately means "start from page 1"; a non-numeric cursor is corrupt
        // input and must surface as an error rather than silently reset to page 1.
        let offset: usize = match req.cursor.as_deref() {
            None => 0,
            Some(c) => match c.parse::<usize>() {
                Ok(n) => n,
                Err(_) => return json_error(format!("invalid cursor: {c:?}")),
            },
        };

        // The model build (whole-repo walk + tree-sitter parse) is synchronous/blocking;
        // run it off the async call stack so it doesn't block this Tokio worker thread.
        let cache = Arc::clone(&self.model_cache);
        let build_root = repo_root.clone();
        let include_private = req.include_private;
        let model = match tokio::task::spawn_blocking(move || {
            load_cached_model(&cache, &build_root, include_private)
        })
        .await
        {
            Ok(Ok(m)) => m,
            Ok(Err(e)) => return json_error(format!("error building architecture model: {e}")),
            Err(e) => {
                return json_error(format!("architecture model build task failed: {e}"));
            }
        };

        let scope: Vec<String> = req.scope.iter().cloned().collect();
        let filtered = model.filtered(&scope, level);

        let mut matches: Vec<(String, SymbolNode)> = Vec::new();
        for (pkg_path, pkg) in &filtered.packages {
            if let Some(want_pkg) = &req.package
                && pkg_path != want_pkg
            {
                continue;
            }
            for sym in &pkg.symbols {
                if let Some(k) = &req.kind
                    && !symbol_kind_matches(sym.kind, k)
                {
                    continue;
                }
                matches.push((pkg_path.clone(), sym.clone()));
            }
        }

        let total_matched = matches.len();
        let page: Vec<(String, SymbolNode)> =
            matches.into_iter().skip(offset).take(limit).collect();
        let returned = page.len();
        let next_offset = offset + returned;
        let next_cursor = if next_offset < total_matched {
            Some(next_offset.to_string())
        } else {
            None
        };

        let possibly_pruned = total_matched == 0
            && !req.include_private
            && match &req.package {
                Some(pkg) => {
                    let prefix = format!("{pkg}::");
                    model
                        .pruning
                        .pruned_symbol_ids
                        .iter()
                        .any(|id| id.starts_with(&prefix))
                }
                None => !model.pruning.pruned_symbol_ids.is_empty(),
            };

        let symbols: Vec<SymbolListEntry> = page
            .into_iter()
            .map(|(package, symbol)| SymbolListEntry { package, symbol })
            .collect();

        let response = ListArchitectureSymbolsResponse {
            total_matched,
            returned,
            next_cursor,
            possibly_pruned,
            symbols,
        };
        serde_json::to_string(&response)
            .unwrap_or_else(|e| json_error(format!("error serializing response: {e}")))
    }

    #[tool(
        description = "Resolve one architecture node by exact reference — a package path or a \
                        symbol id — and return it as JSON ({\"kind\": \"package\"|\"symbol\"|\"not_found\", ...}). \
                        Use this for a single follow-up lookup after list_architecture_symbols; \
                        for a whole-repo report use architecture_assessment instead."
    )]
    async fn get_architecture_node(&self, req: Parameters<GetArchitectureNodeRequest>) -> String {
        let req = req.0;
        let path = PathBuf::from(&req.path);
        let repo_root = match find_config(&path) {
            Ok(Some((_, root))) => root,
            Ok(None) => {
                return json_error("no .claude/inspect.json found above this path".to_string());
            }
            Err(e) => return json_error(format!("error reading config: {e}")),
        };

        // Always resolved against the default (`include_private: false`) model: the
        // "exists but pruned" resolution step (Story 3.1.2 AC3) depends on
        // `pruning.pruned_symbol_ids`, which is only populated on that model. This also
        // shares the same `ModelCacheKey` `list_architecture_symbols` uses by default, so
        // the two tools share one cache slot within a session.
        //
        // The model build (whole-repo walk + tree-sitter parse) is synchronous/blocking;
        // run it off the async call stack so it doesn't block this Tokio worker thread.
        let cache = Arc::clone(&self.model_cache);
        let build_root = repo_root.clone();
        let model = match tokio::task::spawn_blocking(move || {
            load_cached_model(&cache, &build_root, false)
        })
        .await
        {
            Ok(Ok(m)) => m,
            Ok(Err(e)) => return json_error(format!("error building architecture model: {e}")),
            Err(e) => {
                return json_error(format!("architecture model build task failed: {e}"));
            }
        };

        if let Some(pkg) = model.package(&req.node) {
            let value = serde_json::json!({ "kind": "package", "package": pkg });
            return serde_json::to_string(&value)
                .unwrap_or_else(|e| json_error(format!("error serializing response: {e}")));
        }

        for pkg in model.packages.values() {
            if let Some(sym) = pkg.symbols.iter().find(|s| s.id == req.node) {
                let value = serde_json::json!({ "kind": "symbol", "symbol": sym });
                return serde_json::to_string(&value)
                    .unwrap_or_else(|e| json_error(format!("error serializing response: {e}")));
            }
        }

        let exists_but_pruned = model.pruning.pruned_symbol_ids.contains(&req.node);
        let value = if exists_but_pruned {
            serde_json::json!({
                "kind": "not_found",
                "node": req.node,
                "exists_but_pruned": true,
                "hint": "retry with include_private: true",
            })
        } else {
            serde_json::json!({
                "kind": "not_found",
                "node": req.node,
                "exists_but_pruned": false,
            })
        };
        serde_json::to_string(&value)
            .unwrap_or_else(|e| json_error(format!("error serializing response: {e}")))
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
                 architecture_assessment for a whole-repo structural + complexity review \
                 (all three return prose). For a scoped query into the repo's architecture \
                 model instead of a whole-repo report, use list_architecture_symbols (a \
                 paginated, filtered symbol slice) or get_architecture_node (one package or \
                 symbol by exact reference) — both return JSON, not prose."
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

    // --- Epic 3.1: list_architecture_symbols / get_architecture_node ---

    /// Builds a Go repo (module "fixture") with four packages exercising the ACs below:
    /// `widgets` (3 exported funcs), `many` (5 exported funcs, for pagination),
    /// `hidden` (2 unexported-only funcs, for the `possibly_pruned` case), and `shapes`
    /// (two types `A`/`B` each with a same-named `Close` method, for the
    /// owner-qualified-id collision case).
    fn write_arch_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join(".claude/inspect.json"), "{}").unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();

        std::fs::create_dir_all(dir.join("widgets")).unwrap();
        std::fs::write(
            dir.join("widgets/w.go"),
            "package widgets\n\nfunc A() {}\nfunc B() {}\nfunc C() {}\n",
        )
        .unwrap();

        std::fs::create_dir_all(dir.join("many")).unwrap();
        std::fs::write(
            dir.join("many/m.go"),
            "package many\n\nfunc F1() {}\nfunc F2() {}\nfunc F3() {}\nfunc F4() {}\nfunc F5() {}\n",
        )
        .unwrap();

        std::fs::create_dir_all(dir.join("hidden")).unwrap();
        std::fs::write(
            dir.join("hidden/h.go"),
            "package hidden\n\nfunc a() {}\nfunc b() {}\n",
        )
        .unwrap();

        std::fs::create_dir_all(dir.join("shapes")).unwrap();
        std::fs::write(
            dir.join("shapes/s.go"),
            "package shapes\n\ntype A struct{}\n\nfunc (a A) Close() {}\n\ntype B struct{}\n\nfunc (b B) Close() {}\n",
        )
        .unwrap();
    }

    fn list_req(
        dir: &std::path::Path,
        package: Option<&str>,
        limit: usize,
    ) -> ListArchitectureSymbolsRequest {
        list_req_with_cursor(dir, package, limit, None)
    }

    fn list_req_with_cursor(
        dir: &std::path::Path,
        package: Option<&str>,
        limit: usize,
        cursor: Option<String>,
    ) -> ListArchitectureSymbolsRequest {
        ListArchitectureSymbolsRequest {
            path: dir.display().to_string(),
            scope: None,
            package: package.map(|p| p.to_string()),
            kind: None,
            level: default_level(),
            include_private: false,
            limit,
            cursor,
        }
    }

    #[tokio::test]
    async fn list_architecture_symbols_returns_total_matched_and_json_symbols() {
        let dir = tmp_dir("list-total-matched");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_architecture_symbols(Parameters(list_req(
                &dir,
                Some("fixture/widgets"),
                default_limit(),
            )))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        assert_eq!(json["total_matched"], 3, "got: {json}");
        assert_eq!(json["returned"], 3, "got: {json}");
        assert!(json["next_cursor"].is_null(), "got: {json}");
        assert_eq!(json["symbols"].as_array().unwrap().len(), 3, "got: {json}");
    }

    #[tokio::test]
    async fn list_architecture_symbols_returns_empty_array_for_zero_matches_not_error() {
        let dir = tmp_dir("list-zero-matches");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_architecture_symbols(Parameters(list_req(
                &dir,
                Some("does/not/exist"),
                default_limit(),
            )))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert!(json.get("error").is_none(), "unexpected error: {json}");
        assert_eq!(json["total_matched"], 0, "got: {json}");
        assert_eq!(json["symbols"].as_array().unwrap().len(), 0, "got: {json}");
    }

    #[tokio::test]
    async fn list_architecture_symbols_paginates_full_set_via_next_cursor() {
        let dir = tmp_dir("list-pagination");
        write_arch_fixture(&dir);
        let server = KibitzerServer::new();

        let page1 = server
            .list_architecture_symbols(Parameters(list_req(&dir, Some("fixture/many"), 2)))
            .await;
        let json1: serde_json::Value = serde_json::from_str(&page1).expect("valid JSON");
        assert_eq!(json1["total_matched"], 5, "got: {json1}");
        assert_eq!(json1["returned"], 2, "got: {json1}");
        let cursor1 = json1["next_cursor"]
            .as_str()
            .expect("page 1 has next_cursor")
            .to_string();

        let page2 = server
            .list_architecture_symbols(Parameters(list_req_with_cursor(
                &dir,
                Some("fixture/many"),
                2,
                Some(cursor1),
            )))
            .await;
        let json2: serde_json::Value = serde_json::from_str(&page2).expect("valid JSON");
        assert_eq!(json2["returned"], 2, "got: {json2}");
        let cursor2 = json2["next_cursor"]
            .as_str()
            .expect("page 2 has next_cursor")
            .to_string();

        let page3 = server
            .list_architecture_symbols(Parameters(list_req_with_cursor(
                &dir,
                Some("fixture/many"),
                2,
                Some(cursor2),
            )))
            .await;
        let json3: serde_json::Value = serde_json::from_str(&page3).expect("valid JSON");
        assert_eq!(json3["returned"], 1, "got: {json3}");
        assert!(json3["next_cursor"].is_null(), "got: {json3}");

        std::fs::remove_dir_all(&dir).ok();

        let mut names: Vec<String> = Vec::new();
        for page in [&json1, &json2, &json3] {
            for entry in page["symbols"].as_array().unwrap() {
                names.push(entry["symbol"]["name"].as_str().unwrap().to_string());
            }
        }
        let mut sorted_names = names.clone();
        sorted_names.sort();
        assert_eq!(
            sorted_names,
            vec!["F1", "F2", "F3", "F4", "F5"],
            "union of all pages should equal the full 5-symbol set exactly once"
        );
        assert_eq!(names.len(), 5);
    }

    #[tokio::test]
    async fn list_architecture_symbols_possibly_pruned_true_for_all_private_package() {
        let dir = tmp_dir("list-possibly-pruned");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_architecture_symbols(Parameters(list_req(
                &dir,
                Some("fixture/hidden"),
                default_limit(),
            )))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(json["total_matched"], 0, "got: {json}");
        assert_eq!(json["possibly_pruned"], true, "got: {json}");
    }

    #[test]
    fn list_architecture_symbols_request_field_is_named_path() {
        let symbols_req = ListArchitectureSymbolsRequest {
            path: "some/path".to_string(),
            scope: None,
            package: None,
            kind: None,
            level: default_level(),
            include_private: false,
            limit: default_limit(),
            cursor: None,
        };
        assert_eq!(symbols_req.path, "some/path");

        let node_req = GetArchitectureNodeRequest {
            path: "some/path".to_string(),
            node: "pkg::Sym".to_string(),
        };
        assert_eq!(node_req.path, "some/path");
    }

    #[test]
    fn list_architecture_symbols_tool_description_mentions_json_and_contrasts_assessment() {
        let router = KibitzerServer::tool_router();
        let tools = router.list_all();
        let tool = tools
            .iter()
            .find(|t| t.name == "list_architecture_symbols")
            .expect("list_architecture_symbols is registered");
        let desc = tool.description.as_ref().expect("has a description");
        assert!(desc.contains("JSON"), "got: {desc}");
        assert!(desc.contains("architecture_assessment"), "got: {desc}");
    }

    #[tokio::test]
    async fn get_architecture_node_resolves_package_before_symbol_id() {
        let dir = tmp_dir("node-resolves-package");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .get_architecture_node(Parameters(GetArchitectureNodeRequest {
                path: dir.display().to_string(),
                node: "fixture/widgets".to_string(),
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(json["kind"], "package", "got: {json}");
        assert_eq!(json["package"]["path"], "fixture/widgets", "got: {json}");
    }

    #[tokio::test]
    async fn get_architecture_node_returns_not_found_echoing_queried_node() {
        let dir = tmp_dir("node-not-found");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .get_architecture_node(Parameters(GetArchitectureNodeRequest {
                path: dir.display().to_string(),
                node: "does/not/exist".to_string(),
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(json["kind"], "not_found", "got: {json}");
        assert_eq!(json["node"], "does/not/exist", "got: {json}");
        assert_eq!(json["exists_but_pruned"], false, "got: {json}");
        assert!(json.get("hint").is_none(), "got: {json}");
    }

    #[tokio::test]
    async fn get_architecture_node_reports_exists_but_pruned_for_a_pruned_private_symbol() {
        let dir = tmp_dir("node-exists-but-pruned");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .get_architecture_node(Parameters(GetArchitectureNodeRequest {
                path: dir.display().to_string(),
                node: "fixture/hidden::a".to_string(),
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(json["kind"], "not_found", "got: {json}");
        assert_eq!(json["exists_but_pruned"], true, "got: {json}");
        assert_eq!(
            json["hint"], "retry with include_private: true",
            "got: {json}"
        );
    }

    #[tokio::test]
    async fn get_architecture_node_resolves_owner_qualified_method_not_colliding_sibling_type() {
        let dir = tmp_dir("node-owner-qualified");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .get_architecture_node(Parameters(GetArchitectureNodeRequest {
                path: dir.display().to_string(),
                node: "fixture/shapes::A.Close".to_string(),
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(json["kind"], "symbol", "got: {json}");
        assert_eq!(json["symbol"]["name"], "Close", "got: {json}");
        assert_eq!(json["symbol"]["parent"], "A", "got: {json}");
    }

    #[test]
    fn get_info_instructions_name_both_new_tools_and_json() {
        let server = KibitzerServer::new();
        let info = server.get_info();
        let instructions = info.instructions.expect("has instructions");
        assert!(
            instructions.contains("list_architecture_symbols"),
            "got: {instructions}"
        );
        assert!(
            instructions.contains("get_architecture_node"),
            "got: {instructions}"
        );
        assert!(instructions.contains("JSON"), "got: {instructions}");
    }
}
