use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::arch_model::{
    ArchModel, CallEdge, ModelCache, ModelLevel, SymbolKind, SymbolNode, load_cached_model,
};
use crate::check::{
    CheckResult, run_architecture_check, run_check, run_checks_for_trigger, walk_and_collect_files,
};
use crate::config::{Check, Severity, find_config, find_effective_config, find_repo_root};
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

#[derive(Serialize, Deserialize, JsonSchema)]
struct CallTraversalRequest {
    /// Any path inside the repo to query (the repo root or a subdirectory).
    path: String,
    /// The `SymbolNode::id` to traverse from — as returned by `list_architecture_symbols`
    /// or `get_architecture_node`.
    node: String,
    /// How many hops to follow. Defaults to 1 (immediate callers/callees only); clamped
    /// to [1, `MAX_CALL_DEPTH`]. The walk keeps a visited-node set so a recursive or
    /// mutually-recursive call chain can't loop forever.
    #[serde(default = "default_depth")]
    depth: usize,
}

fn default_depth() -> usize {
    1
}

/// Upper bound `call_traversal` clamps `CallTraversalRequest::depth` to — a single named
/// constant so the clamp call site and this doc comment can't drift apart.
const MAX_CALL_DEPTH: usize = 10;

#[derive(Serialize)]
struct CallTraversalResponse {
    node: String,
    depth: usize,
    /// True when the walk still had unexpanded nodes left when `depth` ran out —
    /// increasing `depth` may surface more edges. `false` means every reachable edge
    /// within the graph was already collected.
    truncated: bool,
    edges: Vec<CallEdge>,
}

/// Serializes an ad hoc `{"error": "..."}` JSON object — kept as JSON (not a plain
/// string) so a caller of these two JSON-returning tools never has to branch on response
/// shape between the success and failure path, per ADR-001.
fn json_error(message: String) -> String {
    serde_json::to_string(&serde_json::json!({ "error": message }))
        .unwrap_or_else(|_| "{\"error\":\"failed to serialize error\"}".to_string())
}

/// Which direction `traverse_call_edges` walks — an enum instead of a boolean flag so
/// call sites read as `CallDirection::Callees`/`Callers` rather than a bare `true`/`false`.
#[derive(Debug, Clone, Copy)]
enum CallDirection {
    /// Follows `from -> to`: "what does this node call?"
    Callees,
    /// Follows `to -> from`: "what calls this node?"
    Callers,
}

impl CallDirection {
    /// `(near, far)`: `near` is the endpoint a frontier node is matched against, `far` is
    /// the endpoint that continues the walk.
    fn endpoints(self, edge: &CallEdge) -> (&str, &str) {
        match self {
            CallDirection::Callees => (&edge.from, &edge.to),
            CallDirection::Callers => (&edge.to, &edge.from),
        }
    }
}

/// `direction`-relevant near-endpoint -> edges index, built once per `traverse_call_edges`
/// call instead of rescanning `edges` linearly on every hop — a repo-scale call graph
/// walked at `depth: 10` would otherwise redo an O(edges) scan per frontier node per hop.
type CallAdjacency<'a> = std::collections::HashMap<&'a str, Vec<&'a CallEdge>>;

fn build_call_adjacency(edges: &[CallEdge], direction: CallDirection) -> CallAdjacency<'_> {
    let mut by_near: CallAdjacency = std::collections::HashMap::new();
    for edge in edges {
        by_near
            .entry(direction.endpoints(edge).0)
            .or_default()
            .push(edge);
    }
    by_near
}

/// One BFS layer: every edge in `by_near` keyed by a node in `frontier` is collected, and
/// its far endpoint queued for the next layer unless already `visited` — the guard that
/// stops a recursive/mutually-recursive call chain from looping forever.
fn expand_call_frontier(
    by_near: &CallAdjacency,
    direction: CallDirection,
    frontier: &[String],
    visited: &mut std::collections::HashSet<String>,
    collected: &mut Vec<CallEdge>,
) -> Vec<String> {
    let mut next = Vec::new();
    for node in frontier {
        let Some(edges) = by_near.get(node.as_str()) else {
            continue;
        };
        for edge in edges {
            collected.push((*edge).clone());
            let far = direction.endpoints(edge).1;
            if visited.insert(far.to_string()) {
                next.push(far.to_string());
            }
        }
    }
    next
}

/// Bounded BFS over `edges` from `start` (a `SymbolNode::id`), in `direction`. Returns
/// the collected edges and whether a further hop would actually surface more edges — a
/// node in the final frontier still being a `by_near` key means expanding once more would
/// find something, vs. `false` when `depth` happened to land exactly on a leaf (see
/// `CallTraversalResponse::truncated`).
fn traverse_call_edges(
    edges: &[CallEdge],
    start: &str,
    depth: usize,
    direction: CallDirection,
) -> (Vec<CallEdge>, bool) {
    let by_near = build_call_adjacency(edges, direction);
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
    visited.insert(start.to_string());
    let mut frontier: Vec<String> = vec![start.to_string()];
    let mut collected: Vec<CallEdge> = Vec::new();

    for hop in 0..depth {
        let next_frontier =
            expand_call_frontier(&by_near, direction, &frontier, &mut visited, &mut collected);
        if next_frontier.is_empty() {
            return (collected, false);
        }
        if hop + 1 == depth {
            let truncated = next_frontier
                .iter()
                .any(|n| by_near.contains_key(n.as_str()));
            return (collected, truncated);
        }
        frontier = next_frontier;
    }
    (collected, false)
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

/// The 7 finding categories every `ArchFinding` message now self-tags with a mandatory
/// `[category]` bracket prefix (Story 6.1.1) — in the declaration order the breakdown
/// line below renders them in. `component` is the checker-independent zero-match-
/// advisory tag `zero_match_advisory<T>` emits (Story 1.1.3), distinct from
/// `component-deps`: `ContentChecker`/`NamingChecker` emit it too, not just
/// `ComponentDependencyChecker`.
const FINDING_CATEGORIES: [&str; 7] = [
    "import-cycle",
    "layering",
    "coupling",
    "component-deps",
    "content",
    "naming",
    "component",
];

/// Parses each output line's mandatory `[category]` bracket prefix (Story 6.1.1) and
/// counts occurrences per category. A line with no recognized category tag — a per-file
/// complexity finding from `SYNTAX_RULES_CHECKERS`, or an `error running ...` line — is
/// not counted. A line is credited to at most one category (the first match in
/// `FINDING_CATEGORIES` order), since every finding carries exactly one category tag.
fn category_breakdown(lines: &[String]) -> BTreeMap<String, usize> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in lines {
        for category in FINDING_CATEGORIES {
            if line.contains(&format!("[{category}]")) {
                *counts.entry(category.to_string()).or_insert(0) += 1;
                break;
            }
        }
    }
    counts
}

/// Renders `category_breakdown`'s counts as the one-line summary shown immediately
/// under the aggregate count line (Story 6.1.2), in `FINDING_CATEGORIES`' declared
/// order, omitting categories with zero findings. `None` when nothing was categorized
/// (e.g. a clean repo with no findings at all).
fn format_category_breakdown(counts: &BTreeMap<String, usize>) -> Option<String> {
    let parts: Vec<String> = FINDING_CATEGORIES
        .iter()
        .filter_map(|cat| counts.get(*cat).map(|n| format!("{cat}: {n}")))
        .collect();
    if parts.is_empty() {
        None
    } else {
        Some(format!("  {}", parts.join(", ")))
    }
}

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
        "component-deps" => Some(
            "component-deps: either relocate the offending code into a component already \
             allowed to hold that dependency, or add it to the target component's \
             may_depend_on list if the dependency is actually intended.",
        ),
        "content-rules" => Some(
            "content-rules: move the offending declaration into a component whose \
             allowed_kinds already permits its kind, or add that kind to the target \
             component's allowed_kinds if it's actually intended to live there.",
        ),
        "naming-rules" => Some(
            "naming-rules: rename the offending declaration to match its component's \
             required pattern, or adjust the naming_rules pattern if the existing name is \
             actually intended.",
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
        description = "List the checks that actually run above the given path: the built-in default \
                        catalog, overlaid with the nearest .claude/inspect.json if one exists."
    )]
    async fn list_checks(&self, req: Parameters<ListChecksRequest>) -> String {
        let path = PathBuf::from(&req.0.path);
        match find_effective_config(&path) {
            Ok((config, root)) => {
                // Loaded once for the whole listing rather than once per check.
                let registry =
                    crate::plugin::Registry::load(&crate::plugin::default_registry_path());
                let names: Vec<String> = config
                    .checks
                    .iter()
                    .map(|c| {
                        // Task 4.3.1d: flag a plugin-backed check whose binary is missing
                        // right in the listing, instead of only discovering it via a
                        // failed `run_checks` call.
                        let plugin_tag = if registry.missing_binary_for(&c.name).is_some() {
                            ", plugin=not-installed"
                        } else {
                            ""
                        };
                        format!(
                            "{} ({:?}, scope={:?}{plugin_tag})",
                            c.name, c.severity, c.scope
                        )
                    })
                    .collect();
                format!(
                    "config root: {}\nchecks:\n{}",
                    root.display(),
                    names.join("\n")
                )
            }
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
        let (config, repo_root) = match find_effective_config(&path) {
            Ok(c) => c,
            Err(e) => return format!("error reading config: {e}"),
        };
        // Loaded once for the whole assessment (every checker, every file below)
        // instead of once per (file, checker) pair — same rationale as `registry`.
        let accepted = match crate::accepted_findings::find_accepted_findings(&repo_root) {
            Ok(a) => a,
            Err(e) => return format!("error reading accepted findings: {e}"),
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
            // Render each finding's EFFECTIVE severity — `severity_override.unwrap_or(check.severity)`
            // — rather than `result.severity` once for the whole `Check`. Without this, a
            // zero-match-component advisory (`severity_override: Some(Advisory)`, set
            // unconditionally by `zero_match_advisory`/every checker) would inherit
            // whatever severity the user configured for the enclosing `Check` — so a
            // `component-deps` check set to `"severity": "blocking"` renders its own
            // harmless zero-match advisories as `[blocking]`, defeating the guarantee
            // `ArchFinding.severity_override` exists to provide. `result.findings` (populated
            // by `run_architecture_check`) carries the per-finding data; fall back to the
            // old flattened-`output` rendering only if `findings` is unexpectedly empty
            // (e.g. an old cached `CheckResult` predating this field, or a future
            // architecture-checker source that doesn't populate it).
            if result.findings.is_empty() {
                let level = match result.severity {
                    Severity::Blocking => "blocking",
                    Severity::Advisory => "advisory",
                };
                for finding_line in result.output.lines().filter(|l| !l.is_empty()) {
                    lines.push(format!("[{level}] {finding_line}"));
                    finding_count += 1;
                }
            } else {
                for finding in &result.findings {
                    let level = match finding.severity_override.unwrap_or(result.severity) {
                        Severity::Blocking => "blocking",
                        Severity::Advisory => "advisory",
                    };
                    let location = match (&finding.file, finding.line) {
                        (Some(file), Some(line)) => format!("{}:{}: ", file.display(), line),
                        (Some(file), None) => format!("{}: ", file.display()),
                        (None, _) => String::new(),
                    };
                    lines.push(format!("[{level}] {location}{}", finding.message));
                    finding_count += 1;
                }
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
            // `synthetic.checker` is always `Some` here, so `run_check` takes the native-
            // checker path and never consults the registry — an empty one avoids an
            // unused `registry.json` load on every (file, checker) pair in this loop.
            let no_plugins = crate::plugin::Registry::default();
            for file in &files {
                let result =
                    match run_check(&synthetic, &repo_root, file, None, &no_plugins, &accepted) {
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
        if let Some(breakdown) = format_category_breakdown(&category_breakdown(&lines)) {
            output.push_str(&breakdown);
            output.push('\n');
        }
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
                    let components = config.architecture.effective_components();
                    let diagram = crate::mermaid::render_dependency_graph(&graph, &components);
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
        let trigger = req.0.trigger;
        // Command-based checks can each block for up to `COMMAND_TIMEOUT` — run the
        // whole synchronous dispatch on a blocking-pool thread (the same pattern
        // `load_model_off_stack` uses for the whole-repo model build below) instead of
        // inline on this `async fn`'s stack, where it would tie up a tokio worker thread
        // with no `.await` point for the entire duration.
        let outcome = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<CheckResult>> {
            let (config, repo_root) = find_effective_config(&file_path)?;
            let registry = crate::plugin::Registry::load(&crate::plugin::default_registry_path());
            let accepted = crate::accepted_findings::find_accepted_findings(&repo_root)?;
            run_checks_for_trigger(
                &config.checks,
                &trigger,
                &repo_root,
                &file_path,
                None,
                &registry,
                &accepted,
            )
        })
        .await;

        match outcome {
            Ok(Ok(results)) => {
                let failures: Vec<String> = results
                    .iter()
                    .filter(|r| !r.passed)
                    .map(|r| {
                        // Task 4.3.1c: a plugin-backed check whose binary is missing
                        // renders `[skipped]`, not `[Advisory]`/`[Blocking]` — an agent
                        // shouldn't read "not installed" as "ran and found a defect."
                        if r.plugin_missing {
                            format!("[skipped] {}: {}", r.check_name, r.describe())
                        } else {
                            format!("[{:?}] {}: {}", r.severity, r.check_name, r.describe())
                        }
                    })
                    .collect();
                if failures.is_empty() {
                    "all checks passed".to_string()
                } else {
                    failures.join("\n")
                }
            }
            Ok(Err(e)) => format!("error running checks: {e}"),
            Err(e) => format!("run_checks task failed: {e}"),
        }
    }

    /// Resolves `path`'s nearest `.claude/inspect.json` repo root if one exists, else falls
    /// back to the nearest `.git` root (or `path` itself) via `find_repo_root` — these
    /// architecture-query tools only need a directory to walk for source files, not a real
    /// config, so a missing `.claude/inspect.json` is never fatal here. Still returns `Err`
    /// for a config file that exists but fails to parse. Shared by `list_architecture_symbols`/
    /// `get_architecture_node`/the call-graph traversal tools, which previously each inlined
    /// this same `find_config` dispatch.
    fn resolve_repo_root(path: &Path) -> Result<PathBuf, String> {
        match find_config(path) {
            Ok(Some((_, root))) => Ok(root),
            Ok(None) => Ok(find_repo_root(path)),
            Err(e) => Err(format!("error reading config: {e}")),
        }
    }

    /// Loads the cached `ArchModel` for `repo_root` via `spawn_blocking` (the build is
    /// synchronous/blocking — whole-repo walk, disk reads, tree-sitter parsing — so it must
    /// not run inline on the async call stack). Shared by `list_architecture_symbols`/
    /// `get_architecture_node`, which previously each inlined this same dispatch.
    async fn load_model_off_stack(
        &self,
        repo_root: PathBuf,
        include_private: bool,
    ) -> Result<Arc<ArchModel>, String> {
        let cache = Arc::clone(&self.model_cache);
        tokio::task::spawn_blocking(move || load_cached_model(&cache, &repo_root, include_private))
            .await
            .map_err(|e| format!("architecture model build task failed: {e}"))?
            .map_err(|e| format!("error building architecture model: {e}"))
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
        let repo_root = match Self::resolve_repo_root(&path) {
            Ok(root) => root,
            Err(e) => return json_error(e),
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

        let model = match self
            .load_model_off_stack(repo_root, req.include_private)
            .await
        {
            Ok(m) => m,
            Err(e) => return json_error(e),
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
        let repo_root = match Self::resolve_repo_root(&path) {
            Ok(root) => root,
            Err(e) => return json_error(e),
        };

        // Always resolved against the default (`include_private: false`) model: the
        // "exists but pruned" resolution step (Story 3.1.2 AC3) depends on
        // `pruning.pruned_symbol_ids`, which is only populated on that model. This also
        // shares the same `ModelCacheKey` `list_architecture_symbols` uses by default, so
        // the two tools share one cache slot within a session.
        let model = match self.load_model_off_stack(repo_root, false).await {
            Ok(m) => m,
            Err(e) => return json_error(e),
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

    #[tool(
        description = "Backward call-graph traversal: who calls `node` (a SymbolNode id), up to \
                        `depth` hops — returns JSON ({node, depth, truncated, edges}), not prose. \
                        An edge with resolved: false carries the call site's raw, unresolved \
                        callee text instead of a symbol id (best-effort static resolution — dynamic \
                        dispatch/DI/reflection can leave gaps). Use for impact analysis ('what \
                        breaks if I change this function') or dead-code checks (an exported symbol \
                        with zero callers)."
    )]
    async fn list_callers(&self, req: Parameters<CallTraversalRequest>) -> String {
        self.call_traversal(req.0, CallDirection::Callers).await
    }

    #[tool(
        description = "Forward call-graph traversal: what `node` (a SymbolNode id) calls, up to \
                        `depth` hops — returns JSON ({node, depth, truncated, edges}), not prose. \
                        An edge with resolved: false carries the call site's raw, unresolved \
                        callee text instead of a symbol id (best-effort static resolution — dynamic \
                        dispatch/DI/reflection can leave gaps). Use for impact analysis ('what does \
                        this function actually reach') or security path verification."
    )]
    async fn list_callees(&self, req: Parameters<CallTraversalRequest>) -> String {
        self.call_traversal(req.0, CallDirection::Callees).await
    }
}

impl KibitzerServer {
    /// Shared body for `list_callers`/`list_callees` — the two tools differ only in
    /// which direction they walk `ArchModel::call_edges`.
    async fn call_traversal(&self, req: CallTraversalRequest, direction: CallDirection) -> String {
        let path = PathBuf::from(&req.path);
        let repo_root = match Self::resolve_repo_root(&path) {
            Ok(root) => root,
            Err(e) => return json_error(e),
        };
        let depth = req.depth.clamp(1, MAX_CALL_DEPTH);

        // Same default-model choice as `get_architecture_node`: unscoped, exported-only,
        // sharing that tool's cache slot.
        let model = match self.load_model_off_stack(repo_root, false).await {
            Ok(m) => m,
            Err(e) => return json_error(e),
        };

        let (edges, truncated) =
            traverse_call_edges(&model.call_edges, &req.node, depth, direction);

        let response = CallTraversalResponse {
            node: req.node,
            depth,
            truncated,
            edges,
        };
        serde_json::to_string(&response)
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
                 paginated, filtered symbol slice), get_architecture_node (one package or \
                 symbol by exact reference), or list_callers/list_callees (function-level \
                 call-graph traversal, Go/TS/JS only) — all four return JSON, not prose."
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

    #[test]
    fn recommendation_for_covers_all_five_native_architecture_checkers() {
        assert!(recommendation_for("import-cycles").is_some());
        assert!(recommendation_for("layering").is_some());
        assert!(recommendation_for("component-deps").is_some());
        assert!(recommendation_for("content-rules").is_some());
        assert!(recommendation_for("naming-rules").is_some());
        // `coupling` deliberately has no canned recommendation — see this file's own
        // comment at `recommendation_for`'s definition and plan.md Story 6.2.1.
        assert!(recommendation_for("coupling").is_none());
    }

    // --- traverse_call_edges: direct BFS unit tests against hand-built CallEdge literals,
    // decoupled from file I/O/parsing/resolution so a BFS bug fails independently of a
    // resolution bug (the fixture-based list_callers/list_callees tests below still cover
    // the full extraction-through-traversal path end to end).

    fn edge(from: &str, to: &str, resolved: bool) -> CallEdge {
        CallEdge {
            from: from.to_string(),
            to: to.to_string(),
            resolved,
            file: PathBuf::new(),
            line: 1,
        }
    }

    #[test]
    fn traverse_call_edges_callees_stops_at_depth_and_reports_truncated() {
        let edges = vec![edge("a", "b", true), edge("b", "c", true)];
        let (collected, truncated) = traverse_call_edges(&edges, "a", 1, CallDirection::Callees);
        assert_eq!(collected, vec![edge("a", "b", true)]);
        assert!(truncated, "b still calls c beyond depth 1");
    }

    #[test]
    fn traverse_call_edges_callees_at_exact_leaf_depth_reports_not_truncated() {
        let edges = vec![edge("a", "b", true), edge("b", "c", true)];
        let (collected, truncated) = traverse_call_edges(&edges, "a", 2, CallDirection::Callees);
        assert_eq!(collected, vec![edge("a", "b", true), edge("b", "c", true)]);
        assert!(!truncated, "c has no further callees");
    }

    #[test]
    fn traverse_call_edges_callers_walks_the_reverse_direction() {
        let edges = vec![edge("a", "b", true), edge("b", "c", true)];
        let (collected, _) = traverse_call_edges(&edges, "c", 2, CallDirection::Callers);
        assert_eq!(collected, vec![edge("b", "c", true), edge("a", "b", true)]);
    }

    #[test]
    fn traverse_call_edges_self_loop_terminates_via_visited_set_not_depth() {
        let edges = vec![edge("a", "a", true)];
        let (collected, _) = traverse_call_edges(&edges, "a", 5, CallDirection::Callees);
        assert_eq!(
            collected,
            vec![edge("a", "a", true)],
            "the self-edge is collected once, not once per remaining hop"
        );
    }

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
    { "name": "layering", "architecture_checker": "layering", "severity": "advisory" },
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "advisory" }
  ]
}"#,
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.name=test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-q",
                "-m",
                "init",
            ],
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
        // Story 6.1.2: per-category count breakdown line, immediately under the
        // aggregate count line. `component-deps` is included in this fixture
        // deliberately — design/ux.md flagged plan.md's own illustrative breakdown
        // example as omitting it; `write_fixture`'s `layers` config desugars into an
        // equivalent `component-deps` violation on the same `domain` -> `handlers`
        // edge `layering` also flags (documented overlap, plan.md Pattern Decisions
        // "Post-merge overlap between layering and component-deps").
        assert!(
            output.contains(
                "architecture assessment: 3 finding(s) across 4 file(s)\n  import-cycle: 1, layering: 1, component-deps: 1\n"
            ),
            "expected count line immediately followed by the category breakdown, got:\n{output}"
        );
    }

    /// Story 6.1.1/6.1.2 (UX Criterion 3, `validation.md`): a fixture producing at
    /// least one finding in every one of the 7 categories — `import-cycle`, `layering`,
    /// `coupling`, `component-deps`, `content`, `naming`, and the checker-independent
    /// `component` zero-match-advisory tag — so the breakdown line and the
    /// mandatory-bracket-prefix guarantee are proven across the full category set, not
    /// just the two or three any single earlier fixture happened to exercise.
    fn write_every_category_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("cyclea")).unwrap();
        std::fs::create_dir_all(dir.join("cycleb")).unwrap();
        std::fs::create_dir_all(dir.join("l1")).unwrap();
        std::fs::create_dir_all(dir.join("l2")).unwrap();
        std::fs::create_dir_all(dir.join("hub")).unwrap();
        std::fs::create_dir_all(dir.join("svcs")).unwrap();
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        std::fs::create_dir_all(dir.join("contentcomp")).unwrap();
        std::fs::create_dir_all(dir.join("namingcomp")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();

        // `import-cycle`: cyclea <-> cycleb.
        std::fs::write(
            dir.join("cyclea/cyclea.go"),
            "package cyclea\n\nimport \"fixture/cycleb\"\n\nfunc A() { cycleb.B() }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("cycleb/cycleb.go"),
            "package cycleb\n\nimport \"fixture/cyclea\"\n\nfunc B() { cyclea.A() }\n",
        )
        .unwrap();

        // `layering`: layers = ["l1", "l2"], l2 (lower) importing l1 (higher) reverses
        // the declared order. Also produces a duplicate `component-deps` finding via
        // `layers`' desugar into `effective_dependency_rules()` (documented overlap,
        // same as `write_fixture` above) — harmless here since this test only asserts
        // per-category presence, not exact counts.
        std::fs::write(dir.join("l1/l1.go"), "package l1\n\nfunc F() {}\n").unwrap();
        std::fs::write(
            dir.join("l2/l2.go"),
            "package l2\n\nimport \"fixture/l1\"\n\nfunc G() { l1.F() }\n",
        )
        .unwrap();

        // `coupling`: hub fans out to 11 distinct packages, over
        // `architecture_checks::MAX_FAN_OUT` (10, private to that module).
        const FAN_OUT: usize = 11;
        let mut hub_imports = String::new();
        let mut hub_calls = String::new();
        for n in 0..FAN_OUT {
            let dep_dir = dir.join(format!("dep{n}"));
            std::fs::create_dir_all(&dep_dir).unwrap();
            std::fs::write(
                dep_dir.join(format!("dep{n}.go")),
                format!("package dep{n}\n\nfunc F() {{}}\n"),
            )
            .unwrap();
            hub_imports.push_str(&format!("\t\"fixture/dep{n}\"\n"));
            hub_calls.push_str(&format!("\tdep{n}.F()\n"));
        }
        std::fs::write(
            dir.join("hub/hub.go"),
            format!("package hub\n\nimport (\n{hub_imports})\n\nfunc Use() {{\n{hub_calls}}}\n"),
        )
        .unwrap();

        // `component-deps`: svcs -> ext, only "svcs" is on svcs's allow-list.
        std::fs::write(
            dir.join("svcs/svcs.go"),
            "package svcs\n\nimport \"fixture/ext\"\n\nfunc Use() { ext.Do() }\n",
        )
        .unwrap();
        std::fs::write(dir.join("ext/ext.go"), "package ext\n\nfunc Do() {}\n").unwrap();

        // `content`: contentcomp only allows "struct", but declares a function.
        std::fs::write(
            dir.join("contentcomp/contentcomp.go"),
            "package contentcomp\n\nfunc NotAllowed() {}\n",
        )
        .unwrap();

        // `naming`: namingcomp requires struct names matching "^Good.*$", declares "Bad".
        std::fs::write(
            dir.join("namingcomp/namingcomp.go"),
            "package namingcomp\n\ntype Bad struct{}\n",
        )
        .unwrap();

        // `component`: "ghost" is declared but its glob matches no import-graph node and
        // no declaration, so `component-deps`/`content-rules`/`naming-rules` each emit
        // the shared zero-match-component advisory (Story 1.1.3) for it independently.
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": {
    "layers": ["l1", "l2"],
    "components": [
      {"name": "svcs", "paths": ["**/svcs", "**/svcs/**"]},
      {"name": "ext", "paths": ["**/ext", "**/ext/**"]},
      {"name": "contentcomp", "paths": ["**/contentcomp", "**/contentcomp/**"]},
      {"name": "namingcomp", "paths": ["**/namingcomp", "**/namingcomp/**"]},
      {"name": "ghost", "paths": ["**/ghost", "**/ghost/**"]}
    ],
    "dependency_rules": [
      {"component": "svcs", "may_depend_on": ["svcs"]}
    ],
    "content_rules": [
      {"component": "contentcomp", "allowed_kinds": ["struct"]}
    ],
    "naming_rules": [
      {"component": "namingcomp", "kind": "struct", "pattern": "^Good.*$"}
    ]
  },
  "checks": [
    { "name": "import-cycles", "architecture_checker": "import-cycles", "severity": "advisory" },
    { "name": "layering", "architecture_checker": "layering", "severity": "advisory" },
    { "name": "coupling", "architecture_checker": "coupling", "severity": "advisory" },
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "advisory" },
    { "name": "content-rules", "architecture_checker": "content-rules", "severity": "advisory" },
    { "name": "naming-rules", "architecture_checker": "naming-rules", "severity": "advisory" }
  ]
}"#,
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=test",
                "commit",
                "-q",
                "-m",
                "init",
            ],
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
    async fn architecture_assessment_every_category_finding_has_bracket_prefix() {
        let dir = tmp_dir("every-category");
        write_every_category_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .architecture_assessment(Parameters(ArchitectureAssessmentRequest {
                path: dir.display().to_string(),
                scope: None,
                include_diagram: false,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        // Every one of the 7 categories fired at least once...
        for category in FINDING_CATEGORIES {
            assert!(
                output.contains(&format!("[{category}]")),
                "expected a [{category}] finding, got:\n{output}"
            );
        }
        // ...and the breakdown line lists all 7 in declaration order, immediately under
        // the aggregate count line.
        let breakdown_line = output
            .lines()
            .nth(1)
            .expect("output has a second line (the breakdown)");
        let expected_order: Vec<&str> = FINDING_CATEGORIES
            .iter()
            .filter(|cat| breakdown_line.contains(&format!("{cat}: ")))
            .copied()
            .collect();
        assert_eq!(
            expected_order,
            FINDING_CATEGORIES.to_vec(),
            "expected every category present in declaration order, got:\n{breakdown_line}"
        );

        // Every finding line (starts with `[blocking]`/`[advisory]`) carries a second,
        // category bracket somewhere in the line — the mandatory prefix Story 6.1.1
        // guarantees uniformly across all 7 categories.
        let finding_lines: Vec<&str> = output
            .lines()
            .skip(2) // count line + breakdown line
            .take_while(|l| l.starts_with('['))
            .collect();
        assert!(
            !finding_lines.is_empty(),
            "expected finding lines, got:\n{output}"
        );
        for line in finding_lines {
            assert!(
                FINDING_CATEGORIES
                    .iter()
                    .any(|cat| line.contains(&format!("[{cat}]"))),
                "finding line missing a recognized [category] tag: {line}"
            );
        }
    }

    /// Task 2.2.2b: `content-rules`-configured fixture — a `domain` component whose
    /// `ContentRule` only allows `struct`, and a `Validate` function declared inside it,
    /// so `architecture_assessment` surfaces a real `[content]` finding end to end
    /// (config parse -> `run_architecture_check`'s Declaration dispatch ->
    /// `ContentChecker` -> MCP tool output).
    fn write_content_rules_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("domain")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(
            dir.join("domain/domain.go"),
            "package domain\n\ntype Order struct {\n\tID string\n}\n\n\
             func Validate(o Order) error { return nil }\n",
        )
        .unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": {
    "components": [{"name": "domain", "paths": ["**/domain", "**/domain/**"]}],
    "content_rules": [{"component": "domain", "allowed_kinds": ["struct"]}]
  },
  "checks": [
    { "name": "content-rules", "architecture_checker": "content-rules", "severity": "advisory" }
  ]
}"#,
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "-A"],
            vec![
                "-c",
                "user.email=test@example.com",
                "-c",
                "user.name=test",
                "commit",
                "-q",
                "-m",
                "init",
            ],
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
    async fn architecture_assessment_reports_content_rules_findings() {
        let dir = tmp_dir("content-rules");
        write_content_rules_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .architecture_assessment(Parameters(ArchitectureAssessmentRequest {
                path: dir.display().to_string(),
                scope: None,
                include_diagram: false,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        assert!(
            output.contains("[content]"),
            "expected a content-rules finding, got:\n{output}"
        );
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

    fn write_zero_match_only_component_deps_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join("pkg")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        // No imports at all: `ComponentDependencyChecker` has no edges to flag as a real
        // violation. The only finding is the zero-match advisory for "ghost", whose glob
        // matches no import-graph node.
        std::fs::write(dir.join("pkg/pkg.go"), "package pkg\n\nfunc F() {}\n").unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": {
    "components": [{"name": "ghost", "paths": ["**/ghost", "**/ghost/**"]}]
  },
  "checks": [
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "blocking" }
  ]
}"#,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn architecture_assessment_renders_zero_match_advisory_as_advisory_under_a_blocking_check()
     {
        let dir = tmp_dir("zero-match-under-blocking");
        write_zero_match_only_component_deps_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .architecture_assessment(Parameters(ArchitectureAssessmentRequest {
                path: dir.display().to_string(),
                scope: None,
                include_diagram: false,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        // The real rendered output line: `[advisory]`, never `[blocking]`, despite the
        // enclosing `component-deps` check being configured `"severity": "blocking"`.
        let finding_line = output
            .lines()
            .find(|l| l.contains("[component]"))
            .unwrap_or_else(|| panic!("expected a [component] zero-match finding, got:\n{output}"));
        assert!(
            finding_line.starts_with("[advisory]"),
            "zero-match advisory rendered with the wrong level, expected `[advisory] ...`, got: {finding_line}"
        );
        assert!(
            !finding_line.contains("[blocking]"),
            "zero-match advisory must never render [blocking]: {finding_line}"
        );
        // No hard failure anywhere in the assessment output: with only a zero-match
        // advisory and no real component-deps violation, nothing in the rendered output
        // is tagged [blocking] at all.
        assert!(
            !output.contains("[blocking]"),
            "expected no [blocking] finding anywhere in output, got:\n{output}"
        );
        assert!(
            output.contains("architecture assessment: 1 finding(s)"),
            "got:\n{output}"
        );
    }

    /// Regression guard alongside the test above: a check configured `"severity":
    /// "blocking"` against a fixture with a REAL violation (not a zero-match advisory)
    /// must still render `[blocking]` — `severity_override` only ever narrows a finding
    /// toward `Advisory`, it never suppresses a genuine violation's severity.
    #[tokio::test]
    async fn architecture_assessment_still_renders_a_real_blocking_violation_as_blocking() {
        let dir = tmp_dir("real-violation-stays-blocking");
        std::fs::create_dir_all(dir.join("svcs")).unwrap();
        std::fs::create_dir_all(dir.join("ext")).unwrap();
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();
        std::fs::write(
            dir.join("svcs/svcs.go"),
            "package svcs\n\nimport \"fixture/ext\"\n\nfunc Use() { ext.Do() }\n",
        )
        .unwrap();
        std::fs::write(dir.join("ext/ext.go"), "package ext\n\nfunc Do() {}\n").unwrap();
        std::fs::write(
            dir.join(".claude/inspect.json"),
            r#"{
  "architecture": {
    "components": [
      {"name": "svcs", "paths": ["**/svcs", "**/svcs/**"]},
      {"name": "ext", "paths": ["**/ext", "**/ext/**"]}
    ],
    "dependency_rules": [{"component": "svcs", "may_depend_on": []}]
  },
  "checks": [
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "blocking" }
  ]
}"#,
        )
        .unwrap();
        // Deliberately NOT a git repo, same reasoning as the fixture above — keeps
        // `check_native_against_git_head_repo` a clean `None` so `result.severity` isn't
        // touched by the unrelated "predates your edits" baseline logic, isolating what
        // this test actually asserts.

        let server = KibitzerServer::new();
        let output = server
            .architecture_assessment(Parameters(ArchitectureAssessmentRequest {
                path: dir.display().to_string(),
                scope: None,
                include_diagram: false,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let finding_line = output
            .lines()
            .find(|l| l.contains("[component-deps]"))
            .unwrap_or_else(|| {
                panic!("expected a [component-deps] violation finding, got:\n{output}")
            });
        assert!(
            finding_line.starts_with("[blocking]"),
            "real violation must still render [blocking], got: {finding_line}"
        );
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

    /// `Entry` calls `Middle`, which calls `Leaf` — a 2-hop chain for depth-clamping
    /// tests — plus `Recursive` calling itself (to exercise the visited-set) and an
    /// `Unresolvable` call to an ambiguous same-named function in two other packages.
    fn write_call_graph_fixture(dir: &std::path::Path) {
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        std::fs::write(dir.join(".claude/inspect.json"), "{}").unwrap();
        std::fs::write(dir.join("go.mod"), "module fixture\ngo 1.21\n").unwrap();

        std::fs::create_dir_all(dir.join("chain")).unwrap();
        std::fs::write(
            dir.join("chain/c.go"),
            "package chain\n\nfunc Leaf() {}\n\nfunc Middle() {\n\tLeaf()\n}\n\nfunc Entry() {\n\tMiddle()\n}\n\nfunc Recursive() {\n\tRecursive()\n}\n\nfunc Unresolvable() {\n\tAmbiguous()\n}\n",
        )
        .unwrap();

        std::fs::create_dir_all(dir.join("one")).unwrap();
        std::fs::write(dir.join("one/o.go"), "package one\n\nfunc Ambiguous() {}\n").unwrap();

        std::fs::create_dir_all(dir.join("two")).unwrap();
        std::fs::write(dir.join("two/t.go"), "package two\n\nfunc Ambiguous() {}\n").unwrap();
    }

    fn call_req(dir: &std::path::Path, node: &str, depth: usize) -> CallTraversalRequest {
        CallTraversalRequest {
            path: dir.display().to_string(),
            node: node.to_string(),
            depth,
        }
    }

    #[tokio::test]
    async fn list_callees_follows_one_hop_by_default() {
        let dir = tmp_dir("callees-one-hop");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callees(Parameters(call_req(&dir, "fixture/chain::Entry", 1)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        let edges = json["edges"].as_array().expect("edges array");
        assert_eq!(edges.len(), 1, "got: {json}");
        assert_eq!(edges[0]["from"], "fixture/chain::Entry", "got: {json}");
        assert_eq!(edges[0]["to"], "fixture/chain::Middle", "got: {json}");
        assert_eq!(edges[0]["resolved"], true, "got: {json}");
        assert_eq!(
            json["truncated"], true,
            "Middle still calls Leaf beyond depth 1: {json}"
        );
    }

    #[tokio::test]
    async fn list_callees_depth_two_reaches_the_leaf_and_reports_not_truncated() {
        let dir = tmp_dir("callees-two-hops");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callees(Parameters(call_req(&dir, "fixture/chain::Entry", 2)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        let targets: Vec<&str> = json["edges"]
            .as_array()
            .expect("edges array")
            .iter()
            .map(|e| e["to"].as_str().unwrap())
            .collect();
        assert_eq!(
            targets,
            vec!["fixture/chain::Middle", "fixture/chain::Leaf"],
            "got: {json}"
        );
        assert_eq!(
            json["truncated"], false,
            "Leaf has no further callees: {json}"
        );
    }

    #[tokio::test]
    async fn list_callers_walks_backward_from_the_leaf() {
        let dir = tmp_dir("callers-backward");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callers(Parameters(call_req(&dir, "fixture/chain::Leaf", 2)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        let callers: Vec<&str> = json["edges"]
            .as_array()
            .expect("edges array")
            .iter()
            .map(|e| e["from"].as_str().unwrap())
            .collect();
        assert_eq!(
            callers,
            vec!["fixture/chain::Middle", "fixture/chain::Entry"],
            "got: {json}"
        );
    }

    #[tokio::test]
    async fn list_callees_on_a_recursive_function_terminates_via_the_visited_set() {
        let dir = tmp_dir("callees-recursive");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callees(Parameters(call_req(&dir, "fixture/chain::Recursive", 5)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        // The self-edge is collected exactly once, not once per would-be hop — proves the
        // visited set stopped the walk instead of looping until `depth` ran out.
        let edges = json["edges"].as_array().expect("edges array");
        assert_eq!(edges.len(), 1, "got: {json}");
        assert_eq!(edges[0]["from"], "fixture/chain::Recursive", "got: {json}");
        assert_eq!(edges[0]["to"], "fixture/chain::Recursive", "got: {json}");
    }

    #[tokio::test]
    async fn list_callers_on_a_recursive_function_terminates_via_the_visited_set() {
        // Mirrors the callees case above for the Callers direction specifically — a
        // near/far mixup in `CallDirection::endpoints` for `Callers` wouldn't be caught by
        // a Callees-only test.
        let dir = tmp_dir("callers-recursive");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callers(Parameters(call_req(&dir, "fixture/chain::Recursive", 5)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        let edges = json["edges"].as_array().expect("edges array");
        assert_eq!(edges.len(), 1, "got: {json}");
        assert_eq!(edges[0]["from"], "fixture/chain::Recursive", "got: {json}");
        assert_eq!(edges[0]["to"], "fixture/chain::Recursive", "got: {json}");
    }

    #[tokio::test]
    async fn list_callees_reports_an_ambiguous_target_as_unresolved_with_raw_text() {
        let dir = tmp_dir("callees-unresolved");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callees(Parameters(call_req(&dir, "fixture/chain::Unresolvable", 1)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        let edges = json["edges"].as_array().expect("edges array");
        assert_eq!(edges.len(), 1, "got: {json}");
        assert_eq!(edges[0]["resolved"], false, "got: {json}");
        assert_eq!(edges[0]["to"], "Ambiguous", "got: {json}");
    }

    #[tokio::test]
    async fn list_callees_returns_empty_array_not_an_error_for_an_unknown_node() {
        let dir = tmp_dir("callees-unknown-node");
        write_call_graph_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_callees(Parameters(call_req(&dir, "fixture/chain::DoesNotExist", 1)))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output)
            .unwrap_or_else(|e| panic!("expected JSON: {e}\n{output}"));
        assert_eq!(json["edges"].as_array().unwrap().len(), 0, "got: {json}");
        assert_eq!(json["truncated"], false, "got: {json}");
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
    async fn list_architecture_symbols_works_without_inspect_json_via_git_root() {
        let dir = tmp_dir("list-no-config-git-root");
        write_arch_fixture(&dir);
        std::fs::remove_file(dir.join(".claude/inspect.json")).unwrap();
        std::fs::remove_dir(dir.join(".claude")).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();

        let server = KibitzerServer::new();
        let output = server
            .list_architecture_symbols(Parameters(list_req(
                &dir,
                Some("fixture/widgets"),
                default_limit(),
            )))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).unwrap_or_else(|e| {
            panic!("expected JSON, not the old config-required error: {e}\n{output}")
        });
        assert_eq!(json["total_matched"], 3, "got: {json}");
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

    #[tokio::test]
    async fn list_architecture_symbols_kind_filter_returns_only_matching_kind() {
        let dir = tmp_dir("list-kind-filter");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_architecture_symbols(Parameters(ListArchitectureSymbolsRequest {
                path: dir.display().to_string(),
                scope: None,
                package: Some("fixture/shapes".to_string()),
                kind: Some("method".to_string()),
                level: default_level(),
                include_private: false,
                limit: default_limit(),
                cursor: None,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        let symbols = json["symbols"].as_array().unwrap();
        assert!(!symbols.is_empty(), "got: {json}");
        for entry in symbols {
            assert_eq!(entry["symbol"]["kind"], "method", "got: {json}");
        }
        // `shapes` has two `Close` methods and no other symbol kinds, so filtering to
        // "method" must match exactly those two, not the types they're attached to.
        assert_eq!(json["total_matched"], 2, "got: {json}");
    }

    #[tokio::test]
    async fn list_architecture_symbols_level_component_returns_no_symbols() {
        let dir = tmp_dir("list-level-component");
        write_arch_fixture(&dir);

        let server = KibitzerServer::new();
        let output = server
            .list_architecture_symbols(Parameters(ListArchitectureSymbolsRequest {
                path: dir.display().to_string(),
                scope: None,
                package: Some("fixture/widgets".to_string()),
                kind: None,
                level: "component".to_string(),
                include_private: false,
                limit: default_limit(),
                cursor: None,
            }))
            .await;

        std::fs::remove_dir_all(&dir).ok();

        let json: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        assert_eq!(json["total_matched"], 0, "got: {json}");
        assert_eq!(json["symbols"].as_array().unwrap().len(), 0, "got: {json}");
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
        assert!(instructions.contains("list_callers"), "got: {instructions}");
        assert!(instructions.contains("list_callees"), "got: {instructions}");
        assert!(instructions.contains("JSON"), "got: {instructions}");
    }

    #[test]
    fn call_traversal_tools_are_registered_with_json_descriptions() {
        let router = KibitzerServer::tool_router();
        let tools = router.list_all();
        for name in ["list_callers", "list_callees"] {
            let tool = tools
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{name} is registered"));
            let desc = tool.description.as_ref().expect("has a description");
            assert!(desc.contains("JSON"), "{name} got: {desc}");
        }
    }

    /// Task 4.3.1c/d: `list_checks`/`run_checks` render a distinct, actionable signal for a
    /// plugin-backed check whose binary is missing, instead of raw shell noise a real
    /// command failure would look like.
    mod plugin_missing_rendering_tests {
        use super::*;
        use crate::config::OutputFormat;
        use crate::plugin::{InstalledPlugin, PluginName, Registry};

        const PLUGIN_NAME: &str = "kibitzer-stub-plugin";

        /// Points `default_registry_path()` at a private temp dir, with `PLUGIN_NAME`
        /// registered but its binary absent, for as long as this guard is alive —
        /// serialized (via the held lock) against every other module
        /// (`plugin.rs`/`config.rs`/`check.rs`) that mutates `XDG_DATA_HOME`, and restored
        /// on drop even if the test panics.
        struct RegisteredMissingPlugin {
            _lock: std::sync::MutexGuard<'static, ()>,
            xdg_dir: PathBuf,
            previous_xdg_data_home: Option<String>,
        }

        impl RegisteredMissingPlugin {
            fn new() -> Self {
                // `.unwrap_or_else(|e| e.into_inner())`, not `.unwrap()`: an unrelated
                // test panicking while holding this lock (in this module or another)
                // must not poison it for every other test in the process.
                let lock = crate::plugin::XDG_DATA_HOME_LOCK
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let xdg_dir = tmp_dir("plugin-missing-xdg");
                let previous_xdg_data_home = std::env::var("XDG_DATA_HOME").ok();
                // SAFETY: the held `lock` serializes every test across modules that
                // touches this env var.
                unsafe {
                    std::env::set_var("XDG_DATA_HOME", &xdg_dir);
                }

                let plugin = InstalledPlugin {
                    name: PluginName::parse(PLUGIN_NAME).unwrap(),
                    version: "0.1.0".to_string(),
                    min_kibitzer_version: "0.1.0".to_string(),
                    sha256: "deadbeef".to_string(),
                    binary_path: xdg_dir.join("this-binary-does-not-exist"),
                    severity: Severity::Blocking,
                    scope: vec!["**/*".to_string()],
                    triggers: vec!["batch".to_string()],
                    output_format: OutputFormat::Sarif,
                };
                Registry::save(
                    &crate::plugin::default_registry_path(),
                    &Registry {
                        plugins: vec![plugin],
                    },
                )
                .unwrap();

                Self {
                    _lock: lock,
                    xdg_dir,
                    previous_xdg_data_home,
                }
            }
        }

        impl Drop for RegisteredMissingPlugin {
            fn drop(&mut self) {
                // SAFETY: still holding `_lock`.
                unsafe {
                    match &self.previous_xdg_data_home {
                        Some(value) => std::env::set_var("XDG_DATA_HOME", value),
                        None => std::env::remove_var("XDG_DATA_HOME"),
                    }
                }
                let _ = std::fs::remove_dir_all(&self.xdg_dir);
            }
        }

        #[tokio::test]
        async fn list_checks_appends_plugin_not_installed_tag_when_binary_missing() {
            let _plugin = RegisteredMissingPlugin::new();
            let dir = tmp_dir("list-checks-plugin-missing");

            let output = KibitzerServer::new()
                .list_checks(Parameters(ListChecksRequest {
                    path: dir.display().to_string(),
                }))
                .await;

            std::fs::remove_dir_all(&dir).ok();

            let plugin_line = output
                .lines()
                .find(|line| line.contains(PLUGIN_NAME))
                .unwrap_or_else(|| panic!("no line for {PLUGIN_NAME} in output:\n{output}"));
            assert!(
                plugin_line.contains("plugin=not-installed"),
                "got: {plugin_line}"
            );
        }

        #[tokio::test]
        async fn run_checks_renders_skipped_prefix_for_plugin_missing_result() {
            let _plugin = RegisteredMissingPlugin::new();
            let dir = tmp_dir("run-checks-plugin-missing");
            let file_path = dir.join("some-file.stub-only-extension");
            std::fs::write(&file_path, "irrelevant content").unwrap();

            let output = KibitzerServer::new()
                .run_checks(Parameters(RunChecksRequest {
                    file_path: file_path.display().to_string(),
                    trigger: "batch".to_string(),
                }))
                .await;

            std::fs::remove_dir_all(&dir).ok();

            assert!(
                output.contains(&format!("[skipped] {PLUGIN_NAME}")),
                "got: {output}"
            );
            assert!(!output.contains("[Blocking]"), "got: {output}");
            assert!(!output.contains("[Advisory]"), "got: {output}");
        }
    }
}
