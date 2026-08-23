use std::collections::BTreeMap;
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
    { "name": "layering", "architecture_checker": "layering", "severity": "advisory" },
    { "name": "component-deps", "architecture_checker": "component-deps", "severity": "advisory" }
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

    /// The end-to-end test for the `severity_override` wiring gap: before this fix,
    /// `architecture_assessment` derived one `level` per `Check` from `result.severity`
    /// and stamped it on every line of `result.output` — so a `component-deps` check
    /// configured `"severity": "blocking"` rendered its own harmless zero-match-component
    /// advisory (`ArchFinding.severity_override: Some(Severity::Advisory)`, set
    /// unconditionally by `zero_match_advisory`) as `[blocking]`. Unlike the
    /// `architecture_checks.rs`/`declaration_checks.rs` unit tests that only assert on
    /// `ArchFinding` values in isolation, this exercises the real, user/agent-facing MCP
    /// tool output.
    ///
    /// Deliberately NOT a git repo: `run_architecture_check`'s "predates your edits"
    /// git-HEAD-baseline downgrade only fires for a `Blocking`-severity check whose
    /// findings are non-empty, and this fixture's `severity: "blocking"` config plus its
    /// always-present zero-match finding would satisfy that trigger — if this were a git
    /// repo with the same state committed, the baseline would *also* downgrade
    /// `CheckResult.severity` to `Advisory` for an unrelated reason, which would make the
    /// `[advisory]` assertion below pass even without this fix's `severity_override`
    /// wiring. Staying a non-repo keeps `check_native_against_git_head_repo` a clean
    /// `None` (not a git repo — no baseline to compare against), so `result.severity`
    /// stays `Blocking` end to end and the only thing that can make the finding line
    /// read `[advisory]` is `finding.severity_override`.
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
}
