use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CONFIG_FILENAME: &str = "inspect.json";
pub const CONFIG_DIR: &str = ".claude";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocking,
    Advisory,
}

/// A structured output shape kibitzer knows how to parse from a `command` check's
/// stdout, instead of only reading the process exit code. See `docs/output-formats.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// SARIF 2.1.0 (`https://sarifweb.azurewebsites.net/`) — the format most linters
    /// with a `--format sarif`/`--output-format sarif` flag already emit.
    Sarif,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Check {
    pub name: String,
    /// Shell command to run. `{file}` is substituted with the triggering file path.
    ///
    /// When kibitzer knows which lines an edit actually touched (the Claude Code hook,
    /// not batch mode), a check can opt into diff-aware scoping two ways:
    ///
    /// - `{changed_lines}`: substituted with a comma-separated list of 1-indexed,
    ///   inclusive `start-end` ranges (e.g. `12-15,40-40`), or empty if no ranges are
    ///   known — pass this to a linter/tool that supports scoping its own scan to a
    ///   line range.
    /// - Automatic output filtering: if a check's command emits output lines in the
    ///   `{file}:{line}: message` convention (most linters do), kibitzer filters those
    ///   lines down to ones inside the changed ranges and recomputes pass/fail from
    ///   what survives — no command changes needed. Output that doesn't follow this
    ///   convention is left untouched (conservatively kept, so unrecognized output
    ///   can't be silently swallowed).
    ///
    /// Mutually exclusive with `checker`/`architecture_checker` — exactly one of the
    /// three must be set.
    #[serde(default)]
    pub command: Option<String>,
    /// Name of a natively implemented checker (looked up in `checker::registry()`) to
    /// run in-process instead of shelling out via `command`. Mutually exclusive with
    /// `command`/`architecture_checker`.
    #[serde(default)]
    pub checker: Option<String>,
    /// Name of a natively implemented whole-repo architecture checker (looked up in
    /// `architecture_checks::registry()`) that runs once per batch against the whole
    /// repo's import graph, instead of once per triggering file. Mutually exclusive
    /// with `command`/`checker`. `triggers` must be empty or `["batch"]` — rebuilding
    /// the import graph on every `PostToolUse` edit is too expensive.
    #[serde(default)]
    pub architecture_checker: Option<String>,
    pub severity: Severity,
    /// Glob patterns (supporting `**`) a file path must match for this check to apply.
    #[serde(default)]
    pub scope: Vec<String>,
    /// Which hook events / run triggers this check fires on (e.g. "PostToolUse", "batch").
    #[serde(default)]
    pub triggers: Vec<String>,
    /// Message shown to the agent alongside command output when the check fails.
    #[serde(default)]
    pub message: Option<String>,
    /// A structured shape kibitzer should parse from `command`'s stdout instead of
    /// treating the check as pure exit-code pass/fail. Requires `command` — native
    /// checkers already report structured findings. See `docs/output-formats.md`.
    #[serde(default)]
    pub output_format: Option<OutputFormat>,
}

/// How a [`Check`] is dispatched: once per triggering file, or once per whole-repo
/// batch invocation; and via a shell `command` or an in-process native checker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    PerFileCommand,
    WholeRepoCommand,
    PerFileNative,
    WholeRepoNative,
}

impl Check {
    /// Which of the four dispatch shapes this check is.
    pub fn kind(&self) -> CheckKind {
        if self.architecture_checker.is_some() {
            return CheckKind::WholeRepoNative;
        }
        match (&self.command, &self.checker) {
            (Some(command), _) => {
                if command.contains("{file}") {
                    CheckKind::PerFileCommand
                } else {
                    CheckKind::WholeRepoCommand
                }
            }
            (None, Some(_)) => CheckKind::PerFileNative,
            (None, None) => CheckKind::PerFileNative,
        }
    }

    /// Whether this check is scoped to a single triggering file, as opposed to
    /// whole-repo (no `{file}` placeholder in `command`, a whole-repo `command`, or a
    /// whole-repo `architecture_checker`).
    pub fn is_per_file(&self) -> bool {
        matches!(
            self.kind(),
            CheckKind::PerFileCommand | CheckKind::PerFileNative
        )
    }
}

/// A named, glob-mapped set of graph-node/file-path identifiers — the unit
/// `DependencyRule`/`ContentRule`/`NamingRule` reference.
#[derive(Debug, Clone, Deserialize)]
pub struct Component {
    pub name: String,
    // Read by `component_of`/`ComponentDependencyChecker` starting Phase 1 — not yet
    // consumed outside tests, hence `#[allow(dead_code)]`.
    #[allow(dead_code)]
    #[serde(default)]
    pub paths: Vec<String>,
}

/// Per-component allow-list (`may_depend_on`) and/or deny-list (`deny_depend_on`) of
/// other component names. Deny wins when both apply to the same target.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DependencyRule {
    pub component: String,
    #[serde(default)]
    pub may_depend_on: Option<Vec<String>>,
    #[serde(default)]
    pub deny_depend_on: Vec<String>,
}

/// Per-component allowed-declaration-kind list, e.g. "domain may only contain struct".
// Consumed by `ContentChecker` starting Phase 2 — not yet read outside tests.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct ContentRule {
    pub component: String,
    pub allowed_kinds: Vec<String>,
}

/// Per-component, per-`DeclKind` regex pattern a declaration's name must match.
// Consumed by `NamingChecker` starting Phase 3 — not yet read outside tests.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct NamingRule {
    pub component: String,
    pub kind: String,
    pub pattern: String,
}

/// Project-wide settings consumed by `architecture_checker`s that need more than the
/// import graph itself — currently just the declared layer order for `layering`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ArchitectureConfig {
    /// Declared layers, highest-level first (e.g. `["handlers", "domain", "infra"]`).
    /// A package/module belongs to the first layer whose name matches one of its path
    /// segments exactly; packages matching no layer are ignored by `layering`. An import
    /// edge from a package in an earlier layer to one in a later layer is expected
    /// (higher layers depend on lower ones); an edge running the other way — a later
    /// layer reaching back into an earlier one — is flagged.
    ///
    /// `layers` desugars into `components`/`dependency_rules` via
    /// [`ArchitectureConfig::effective_components`]/[`ArchitectureConfig::effective_dependency_rules`]
    /// — declaring a component with the same name as a layer is a config-load error.
    #[serde(default)]
    pub layers: Vec<String>,
    /// Named components that `dependency_rules`/`content_rules`/`naming_rules` reference
    /// by name. See [`ArchitectureConfig::effective_components`] for how these combine
    /// with `layers`-desugared components.
    #[serde(default)]
    pub components: Vec<Component>,
    /// Per-component dependency allow/deny rules. See
    /// [`ArchitectureConfig::effective_dependency_rules`] for how these combine with
    /// `layers`-desugared rules.
    #[serde(default)]
    pub dependency_rules: Vec<DependencyRule>,
    /// Per-component allowed-declaration-kind rules.
    #[serde(default)]
    pub content_rules: Vec<ContentRule>,
    /// Per-component, per-kind naming pattern rules.
    #[serde(default)]
    pub naming_rules: Vec<NamingRule>,
}

/// One `Component` per layer name, reproducing `layer_of()`'s "segment anywhere"
/// exact-match semantics as glob patterns: a bare segment match, a prefix match, a
/// suffix match, and an anywhere-nested match.
fn desugar_layers_to_components(layers: &[String]) -> Vec<Component> {
    layers
        .iter()
        .map(|layer| Component {
            name: layer.clone(),
            paths: vec![
                layer.clone(),
                format!("{layer}/**"),
                format!("**/{layer}"),
                format!("**/{layer}/**"),
            ],
        })
        .collect()
}

/// One `DependencyRule` per layer (not one per pair): a layer may depend on itself and
/// every layer declared after it, matching `layering`'s "higher layers depend on lower
/// ones" convention.
// Consumed by `effective_dependency_rules`, itself consumed by `ComponentDependencyChecker`
// starting Phase 1 — not yet called outside tests.
#[allow(dead_code)]
fn desugar_layers_to_rules(layers: &[String]) -> Vec<DependencyRule> {
    layers
        .iter()
        .enumerate()
        .map(|(i, layer)| DependencyRule {
            component: layer.clone(),
            may_depend_on: Some(layers[i..].to_vec()),
            deny_depend_on: vec![],
        })
        .collect()
}

impl ArchitectureConfig {
    /// Explicitly declared `components`, plus `layers` desugared into `Component`s —
    /// the backward-compat seam so every new checker gets `layers` support for free.
    pub fn effective_components(&self) -> Vec<Component> {
        self.components
            .iter()
            .cloned()
            .chain(desugar_layers_to_components(&self.layers))
            .collect()
    }

    /// Explicitly declared `dependency_rules`, plus `layers` desugared into
    /// `DependencyRule`s.
    // Consumed by `ComponentDependencyChecker` starting Phase 1 — not yet called
    // outside tests.
    #[allow(dead_code)]
    pub fn effective_dependency_rules(&self) -> Vec<DependencyRule> {
        self.dependency_rules
            .iter()
            .cloned()
            .chain(desugar_layers_to_rules(&self.layers))
            .collect()
    }
}

/// Resolves `node` to the name of the first component (in declaration order) whose
/// `paths` glob-matches it, mirroring `layer_of()`'s "first in declaration order"
/// resolution. Returns `None` if `node` matches no declared component — ignored,
/// matching `layer_of()`'s existing "no match = ignored" precedent.
// Consumed by `ComponentDependencyChecker`/`ContentChecker`/`NamingChecker` starting
// Phase 1 — not yet called outside tests.
#[allow(dead_code)]
pub fn component_of<'a>(node: &str, components: &'a [Component]) -> Option<&'a str> {
    components
        .iter()
        .find(|c| crate::glob::matches_scope(node, &c.paths))
        .map(|c| c.name.as_str())
}

/// Classic Levenshtein edit-distance DP, hand-rolled to avoid a new dependency for a
/// small, self-contained algorithm.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (a_len, b_len) = (a.len(), b.len());
    let mut dp = vec![vec![0usize; b_len + 1]; a_len + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a_len {
        for j in 1..=b_len {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a_len][b_len]
}

/// Collects every component name referenced across `dependency_rules` (`component`,
/// `may_depend_on` entries, `deny_depend_on` entries), `content_rules`, and
/// `naming_rules`; bails on the first one not present in `effective_components()`,
/// with a Levenshtein-distance-≤2 suggestion when one exists.
fn validate_component_references(config: &Config, config_path: &Path) -> Result<()> {
    let components = config.architecture.effective_components();
    let known: Vec<&str> = components.iter().map(|c| c.name.as_str()).collect();

    let mut referenced: Vec<&str> = Vec::new();
    for rule in &config.architecture.dependency_rules {
        referenced.push(rule.component.as_str());
        if let Some(may) = &rule.may_depend_on {
            referenced.extend(may.iter().map(String::as_str));
        }
        referenced.extend(rule.deny_depend_on.iter().map(String::as_str));
    }
    for rule in &config.architecture.content_rules {
        referenced.push(rule.component.as_str());
    }
    for rule in &config.architecture.naming_rules {
        referenced.push(rule.component.as_str());
    }

    for name in referenced {
        if known.contains(&name) {
            continue;
        }
        let declared = known.join(", ");
        let suggestion = known
            .iter()
            .map(|k| (*k, levenshtein(name, k)))
            .filter(|(_, d)| *d <= 2)
            .min_by_key(|(_, d)| *d)
            .map(|(k, _)| k);
        match suggestion {
            Some(s) => anyhow::bail!(
                "{}: architecture rule references undefined component '{}' — declared \
                 components are: {} (did you mean '{}'?)",
                config_path.display(),
                name,
                declared,
                s
            ),
            None => anyhow::bail!(
                "{}: architecture rule references undefined component '{}' — declared \
                 components are: {}",
                config_path.display(),
                name,
                declared
            ),
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub architecture: ArchitectureConfig,
}

fn validate(config: &Config, config_path: &Path) -> Result<()> {
    for layer in &config.architecture.layers {
        if config
            .architecture
            .components
            .iter()
            .any(|c| &c.name == layer)
        {
            anyhow::bail!(
                "{}: component '{}' is declared both explicitly and via 'layers'",
                config_path.display(),
                layer
            );
        }
    }
    validate_component_references(config, config_path)?;
    for check in &config.checks {
        let set_count = [
            check.command.is_some(),
            check.checker.is_some(),
            check.architecture_checker.is_some(),
        ]
        .into_iter()
        .filter(|set| *set)
        .count();
        if set_count > 1 {
            anyhow::bail!(
                "{}: check '{}' sets more than one of `command`/`checker`/`architecture_checker` \
                 — these are mutually exclusive",
                config_path.display(),
                check.name
            );
        }
        if set_count == 0 {
            anyhow::bail!(
                "{}: check '{}' sets none of `command`/`checker`/`architecture_checker` — \
                 exactly one is required",
                config_path.display(),
                check.name
            );
        }
        if let Some(checker_name) = &check.checker
            && crate::checker::lookup(checker_name).is_none()
        {
            anyhow::bail!(
                "{}: check '{}' references unknown checker '{}' — run `kibitzer check list` \
                 for available checkers",
                config_path.display(),
                check.name,
                checker_name
            );
        }
        if let Some(arch_name) = &check.architecture_checker {
            if crate::check::lookup_any_architecture_checker(arch_name).is_none() {
                anyhow::bail!(
                    "{}: check '{}' references unknown architecture checker '{}'",
                    config_path.display(),
                    check.name,
                    arch_name
                );
            }
            if check.triggers.iter().any(|t| t != "batch") {
                anyhow::bail!(
                    "{}: check '{}' sets `architecture_checker` with a trigger other than \
                     `batch` — whole-repo architecture checks may only run in batch mode, \
                     never on a per-edit trigger like `PostToolUse`",
                    config_path.display(),
                    check.name
                );
            }
        }
        if check.output_format.is_some() && check.command.is_none() {
            anyhow::bail!(
                "{}: check '{}' sets `output_format` without `command` — structured output \
                 parsing only applies to shell-command checks",
                config_path.display(),
                check.name
            );
        }
    }
    Ok(())
}

/// Walk upward from `start` looking for `.claude/inspect.json`, returning the parsed
/// config and the directory it was found in (the repo root, by convention).
pub fn find_config(start: &Path) -> Result<Option<(Config, PathBuf)>> {
    let mut dir = if start.is_dir() {
        start.to_path_buf()
    } else {
        start
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    };

    loop {
        let candidate = dir.join(CONFIG_DIR).join(CONFIG_FILENAME);
        if candidate.is_file() {
            let raw = std::fs::read_to_string(&candidate)
                .with_context(|| format!("reading {}", candidate.display()))?;
            let config: Config = serde_json::from_str(&raw)
                .with_context(|| format!("parsing {}", candidate.display()))?;
            validate(&config, &candidate)?;
            return Ok(Some((config, dir)));
        }
        match dir.parent() {
            Some(parent) => dir = parent.to_path_buf(),
            None => return Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Config> {
        let config: Config = serde_json::from_str(json)?;
        validate(&config, Path::new(".claude/inspect.json"))?;
        Ok(config)
    }

    #[test]
    fn accepts_checker_without_command() {
        let config = parse(
            r#"{"checks": [{"name": "n", "checker": "primitive-obsession", "severity": "advisory"}]}"#,
        )
        .unwrap();
        assert_eq!(
            config.checks[0].checker.as_deref(),
            Some("primitive-obsession")
        );
        assert!(config.checks[0].command.is_none());
    }

    #[test]
    fn accepts_command_without_checker() {
        let config = parse(
            r#"{"checks": [{"name": "n", "command": "true {file}", "severity": "advisory"}]}"#,
        )
        .unwrap();
        assert_eq!(config.checks[0].command.as_deref(), Some("true {file}"));
        assert!(config.checks[0].checker.is_none());
    }

    #[test]
    fn rejects_both_command_and_checker() {
        let err = parse(
            r#"{"checks": [{"name": "n", "command": "true", "checker": "x", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn rejects_neither_command_nor_checker() {
        let err = parse(r#"{"checks": [{"name": "n", "severity": "advisory"}]}"#).unwrap_err();
        assert!(err.to_string().contains("exactly one is required"));
    }

    #[test]
    fn rejects_unknown_checker_name() {
        let err = parse(
            r#"{"checks": [{"name": "n", "checker": "does-not-exist", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown checker"));
    }

    #[test]
    fn accepts_output_format_with_command() {
        let config = parse(
            r#"{"checks": [{"name": "n", "command": "lint --sarif {file}", "severity": "advisory", "output_format": "sarif"}]}"#,
        )
        .unwrap();
        assert_eq!(config.checks[0].output_format, Some(OutputFormat::Sarif));
    }

    #[test]
    fn rejects_output_format_without_command() {
        let err = parse(
            r#"{"checks": [{"name": "n", "checker": "primitive-obsession", "severity": "advisory", "output_format": "sarif"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("output_format"));
    }

    #[test]
    fn accepts_architecture_checker_with_batch_trigger() {
        let config = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "import-cycles", "severity": "advisory", "triggers": ["batch"]}]}"#,
        )
        .unwrap();
        assert_eq!(
            config.checks[0].architecture_checker.as_deref(),
            Some("import-cycles")
        );
        assert_eq!(config.checks[0].kind(), CheckKind::WholeRepoNative);
        assert!(!config.checks[0].is_per_file());
    }

    #[test]
    fn accepts_architecture_checker_with_no_triggers() {
        parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "import-cycles", "severity": "advisory"}]}"#,
        )
        .unwrap();
    }

    #[test]
    fn rejects_architecture_checker_alongside_checker() {
        let err = parse(
            r#"{"checks": [{"name": "n", "checker": "primitive-obsession", "architecture_checker": "import-cycles", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn rejects_unknown_architecture_checker_name() {
        let err = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "does-not-exist", "severity": "advisory"}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("unknown architecture checker"));
    }

    #[test]
    fn rejects_architecture_checker_on_non_batch_trigger() {
        let err = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "import-cycles", "severity": "advisory", "triggers": ["PostToolUse"]}]}"#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("batch"));
    }

    #[test]
    fn whole_repo_command_check_is_not_per_file() {
        let config =
            parse(r#"{"checks": [{"name": "n", "command": "true", "severity": "advisory"}]}"#)
                .unwrap();
        assert_eq!(config.checks[0].kind(), CheckKind::WholeRepoCommand);
        assert!(!config.checks[0].is_per_file());
    }

    // --- Story 0.1.1: Component/DependencyRule/ContentRule/NamingRule schema ---

    #[test]
    fn component_parses_name_and_paths() {
        let config = parse(
            r#"{"architecture": {"components": [{"name": "domain", "paths": ["**/domain", "**/domain/**"]}]}}"#,
        )
        .unwrap();
        assert_eq!(config.architecture.components[0].name, "domain");
        assert_eq!(
            config.architecture.components[0].paths,
            vec!["**/domain".to_string(), "**/domain/**".to_string()]
        );
    }

    #[test]
    fn existing_layers_only_config_still_parses_with_empty_new_fields() {
        let config = parse(r#"{"architecture": {"layers": ["domain", "infra"]}}"#).unwrap();
        assert!(config.architecture.components.is_empty());
        assert!(config.architecture.dependency_rules.is_empty());
        assert!(config.architecture.content_rules.is_empty());
        assert!(config.architecture.naming_rules.is_empty());
        assert_eq!(
            config.architecture.layers,
            vec!["domain".to_string(), "infra".to_string()]
        );
    }

    // --- Story 0.1.2: layers desugar accessors + component_of resolver ---

    fn three_layer_config() -> ArchitectureConfig {
        ArchitectureConfig {
            layers: vec!["handlers".into(), "domain".into(), "infra".into()],
            ..Default::default()
        }
    }

    #[test]
    fn layers_desugar_to_four_glob_patterns_per_layer() {
        let config = three_layer_config();
        let components = config.effective_components();
        let domain = components
            .iter()
            .find(|c| c.name == "domain")
            .expect("domain component present");
        assert_eq!(
            domain.paths,
            vec![
                "domain".to_string(),
                "domain/**".to_string(),
                "**/domain".to_string(),
                "**/domain/**".to_string(),
            ]
        );
    }

    #[test]
    fn layers_desugar_to_suffix_allow_lists_not_pairwise() {
        let config = three_layer_config();
        let rules = config.effective_dependency_rules();
        let domain_rule = rules
            .iter()
            .find(|r| r.component == "domain")
            .expect("domain rule present");
        assert_eq!(
            domain_rule.may_depend_on,
            Some(vec!["domain".to_string(), "infra".to_string()])
        );
    }

    #[test]
    fn component_of_returns_first_declaration_order_match() {
        let components = vec![
            Component {
                name: "handlers".into(),
                paths: vec!["**/handlers".into()],
            },
            Component {
                name: "domain".into(),
                paths: vec!["**/domain".into()],
            },
        ];
        assert_eq!(
            component_of("dogfood.example/app/domain", &components),
            Some("domain")
        );
    }

    #[test]
    fn component_of_returns_none_for_unmatched_node() {
        let components = vec![
            Component {
                name: "handlers".into(),
                paths: vec!["**/handlers".into()],
            },
            Component {
                name: "domain".into(),
                paths: vec!["**/domain".into()],
            },
        ];
        assert_eq!(
            component_of("dogfood.example/app/vendor", &components),
            None
        );
    }

    #[test]
    fn rejects_layer_and_component_name_collision() {
        let err = parse(
            r#"{"architecture": {"layers": ["domain"], "components": [{"name": "domain", "paths": ["x/**"]}]}}"#,
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("component 'domain' is declared both explicitly and via 'layers'")
        );
    }

    // --- Story 0.1.3: validate rule references ---

    #[test]
    fn rejects_unknown_component_in_dependency_rule() {
        let err = parse(
            r#"{"architecture": {"components": [{"name":"handlers","paths":["**/handlers"]}], "dependency_rules": [{"component": "hanlders", "may_depend_on": []}]}}"#,
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            ".claude/inspect.json: architecture rule references undefined component 'hanlders' \
             — declared components are: handlers (did you mean 'handlers'?)"
        );
    }

    #[test]
    fn rejects_unknown_component_in_content_rule() {
        let err = parse(
            r#"{"architecture": {"components": [{"name":"domain","paths":["**/domain"]}], "content_rules": [{"component": "doamin", "allowed_kinds": ["struct"]}]}}"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined component 'doamin'"));
        assert!(msg.contains("(did you mean 'domain'?)"));
    }

    #[test]
    fn rejects_unknown_component_in_naming_rule() {
        let err = parse(
            r#"{"architecture": {"components": [{"name":"domain","paths":["**/domain"]}], "naming_rules": [{"component": "doamin", "kind": "struct", "pattern": "^[A-Z]"}]}}"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined component 'doamin'"));
        assert!(msg.contains("(did you mean 'domain'?)"));
    }

    // --- Epic 1.2: dual-registry dispatch (`AnyArchitectureChecker`) ---

    // Story 1.2.1's original acceptance criterion expected this to fail (`content-rules`
    // wasn't in either registry yet — Phase 2 scope). Story 2.2.1 registers it in
    // `declaration_checks::registry()`, so `validate()` — going through
    // `lookup_any_architecture_checker` — now finds it and this input parses
    // successfully (Task 2.2.2a).
    #[test]
    fn accepts_content_rules_architecture_checker() {
        let config = parse(
            r#"{"checks": [{"name": "n", "architecture_checker": "content-rules", "severity": "advisory", "triggers": ["batch"]}]}"#,
        )
        .unwrap();
        assert_eq!(
            config.checks[0].architecture_checker.as_deref(),
            Some("content-rules")
        );
    }

    #[test]
    fn unknown_component_error_omits_suggestion_when_no_close_match() {
        let err = parse(
            r#"{"architecture": {"components": [{"name":"domain","paths":["**/domain"]}], "dependency_rules": [{"component": "zzzzzzzzzz", "may_depend_on": []}]}}"#,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("undefined component 'zzzzzzzzzz'"));
        assert!(!msg.contains("did you mean"));
    }
}
