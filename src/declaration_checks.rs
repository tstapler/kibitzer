//! Story 2.1.3: the `DeclarationChecker` trait and its registry — the third checker
//! kind ADR-001 calls for, alongside `checker::Checker` (per-file AST checks) and
//! `architecture_checks::ArchitectureChecker` (whole-repo import-graph checks). Story
//! 2.2.1 adds `ContentChecker`. `NamingChecker` below is a minimal Story 3.1.1 (Task
//! 3.1.1a/3.1.1b) slice brought forward into this epic solely because Story 2.2.3's
//! git-HEAD-baseline-downgrade fix needs a *second* real Declaration-kind checker to
//! prove its dispatch fix is generic, not `content-rules`-specific — regex validation
//! at config-load time (Task 3.1.1c) and the zero-match-component/zero-match-pattern
//! advisory (Story 3.1.2) remain full Phase 3 scope, not implemented here.

use regex::Regex;

use crate::architecture_checks::{ArchFinding, zero_match_advisory};
use crate::config::ArchitectureConfig;
use crate::declarations::{DeclKind, DeclarationGraph};

/// Mirrors [`crate::architecture_checks::ArchitectureChecker`]'s shape, but running
/// against a whole-repo [`DeclarationGraph`] instead of an [`crate::import_graph::ImportGraph`]
/// — `content-rules`/`naming-rules` reason about individual declarations
/// (struct/class/interface/enum/function), not import edges, so they need their own
/// input type. Reuses `ArchFinding` unmodified as the output shape, same as
/// `ArchitectureChecker` (architecture.md §4) — no new finding shape for this checker
/// kind.
pub trait DeclarationChecker {
    fn name(&self) -> &str;
    fn check(&self, graph: &DeclarationGraph, config: &ArchitectureConfig) -> Vec<ArchFinding>;
}

/// Registers `ContentChecker` (Story 2.2.1) and the minimal `NamingChecker` above —
/// both resolve through `crate::check::lookup_any_architecture_checker`'s dual-registry
/// dispatch, which falls back to this registry once `architecture_checks::registry()`
/// misses.
pub fn registry() -> Vec<Box<dyn DeclarationChecker>> {
    vec![Box::new(ContentChecker), Box::new(NamingChecker)]
}

/// Maps a [`DeclKind`] to the string vocabulary `ContentRule.allowed_kinds`/
/// `NamingRule.kind` use in config — the one place that mapping lives, so both
/// checkers (and their tests) stay in sync.
fn kind_name(kind: DeclKind) -> &'static str {
    match kind {
        DeclKind::Struct => "struct",
        DeclKind::Class => "class",
        DeclKind::Interface => "interface",
        DeclKind::Enum => "enum",
        DeclKind::Function => "function",
    }
}

/// Story 2.2.1: "component X may only contain declaration kind Y" — the native
/// counterpart to `arch-go`'s `contentsRules`. A component with no `ContentRule` entry
/// allows every kind (no rule = no constraint, deliberately asymmetric with
/// `DependencyRule`'s deny-by-default: there's no sensible default kind list to deny
/// against for "no content rule at all").
pub struct ContentChecker;

impl DeclarationChecker for ContentChecker {
    fn name(&self) -> &str {
        "content-rules"
    }

    fn check(&self, graph: &DeclarationGraph, config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let mut findings: Vec<ArchFinding> = graph
            .declarations
            .iter()
            .filter_map(|decl| {
                let component = decl.component.as_deref()?;
                let rule = config
                    .content_rules
                    .iter()
                    .find(|r| r.component == component)?;
                let kind = kind_name(decl.kind);
                if rule.allowed_kinds.iter().any(|k| k == kind) {
                    return None;
                }
                Some(ArchFinding {
                    file: Some(decl.file.clone()),
                    line: Some(decl.line),
                    message: format!(
                        "[content] {}:{}: '{}' ({kind}) is not allowed in component '{component}' \
                         — allowed kinds: {}",
                        decl.file.display(),
                        decl.line,
                        decl.name,
                        rule.allowed_kinds.join(", ")
                    ),
                    severity_override: None,
                })
            })
            .collect();

        // Story 2.2.1's zero-match-component advisory, reusing the same helper
        // `ComponentDependencyChecker` uses (Story 1.1.3) — matched against
        // `graph.declarations`' resolved `component` field rather than
        // `ImportGraph.nodes`.
        let components = config.effective_components();
        findings.extend(components.iter().filter_map(|component| {
            zero_match_advisory(
                std::slice::from_ref(component),
                |c| {
                    graph
                        .declarations
                        .iter()
                        .any(|d| d.component.as_deref() == Some(c.name.as_str()))
                },
                |c| format!("component '{}' (glob '{}')", c.name, c.paths.join(", ")),
                "declarations",
            )
        }));

        findings
    }
}

/// Minimal Story 3.1.1 slice (Task 3.1.1a/3.1.1b only — see module doc): "declarations
/// of kind K in component X must match pattern Y." A declaration whose `(component,
/// kind)` matches no `NamingRule` is unaffected. An invalid regex is treated as "no
/// rule" for that declaration rather than panicking — Task 3.1.1c's config-load-time
/// validation (Phase 3) is what should actually prevent this case from reaching
/// `check()` in the first place.
pub struct NamingChecker;

impl DeclarationChecker for NamingChecker {
    fn name(&self) -> &str {
        "naming-rules"
    }

    fn check(&self, graph: &DeclarationGraph, config: &ArchitectureConfig) -> Vec<ArchFinding> {
        graph
            .declarations
            .iter()
            .filter_map(|decl| {
                let component = decl.component.as_deref()?;
                let kind = kind_name(decl.kind);
                let rule = config
                    .naming_rules
                    .iter()
                    .find(|r| r.component == component && r.kind == kind)?;
                let re = Regex::new(&rule.pattern).ok()?;
                if re.is_match(&decl.name) {
                    return None;
                }
                Some(ArchFinding {
                    file: Some(decl.file.clone()),
                    line: Some(decl.line),
                    message: format!(
                        "[naming] {}:{}: {kind} '{}' in component '{component}' does not match \
                         required pattern '{}'",
                        decl.file.display(),
                        decl.line,
                        decl.name,
                        rule.pattern
                    ),
                    severity_override: None,
                })
            })
            .collect()
    }
}

pub fn lookup(name: &str) -> Option<Box<dyn DeclarationChecker>> {
    registry().into_iter().find(|c| c.name() == name)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{Component, ContentRule, Severity};
    use crate::declarations::Declaration;

    #[test]
    fn lookup_finds_registered_checkers() {
        assert!(lookup("content-rules").is_some());
        assert!(lookup("naming-rules").is_some());
        assert!(lookup("anything").is_none());
    }

    // --- Story 2.2.1: ContentChecker ---

    #[test]
    fn content_checker_flags_disallowed_kind() {
        let graph = DeclarationGraph {
            declarations: vec![
                Declaration {
                    name: "Order".to_string(),
                    kind: DeclKind::Struct,
                    file: PathBuf::from("domain/domain.go"),
                    line: 3,
                    component: Some("domain".to_string()),
                },
                Declaration {
                    name: "Validate".to_string(),
                    kind: DeclKind::Function,
                    file: PathBuf::from("domain/domain.go"),
                    line: 8,
                    component: Some("domain".to_string()),
                },
            ],
        };
        let config = ArchitectureConfig {
            content_rules: vec![ContentRule {
                component: "domain".to_string(),
                allowed_kinds: vec!["struct".to_string()],
            }],
            ..Default::default()
        };

        let findings = ContentChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, Some(PathBuf::from("domain/domain.go")));
        assert_eq!(findings[0].line, Some(8));
        assert_eq!(
            findings[0].message,
            "[content] domain/domain.go:8: 'Validate' (function) is not allowed in component \
             'domain' — allowed kinds: struct"
        );
    }

    #[test]
    fn content_checker_skips_unmapped_declarations() {
        let graph = DeclarationGraph {
            declarations: vec![Declaration {
                name: "Loose".to_string(),
                kind: DeclKind::Function,
                file: PathBuf::from("misc/misc.go"),
                line: 1,
                component: None,
            }],
        };
        let config = ArchitectureConfig {
            content_rules: vec![ContentRule {
                component: "domain".to_string(),
                allowed_kinds: vec!["struct".to_string()],
            }],
            ..Default::default()
        };

        let findings = ContentChecker.check(&graph, &config);
        assert!(findings.is_empty());
    }

    #[test]
    fn content_checker_allows_everything_with_no_rule_for_component() {
        let graph = DeclarationGraph {
            declarations: vec![Declaration {
                name: "DoStuff".to_string(),
                kind: DeclKind::Function,
                file: PathBuf::from("infra/infra.go"),
                line: 1,
                component: Some("infra".to_string()),
            }],
        };
        let config = ArchitectureConfig {
            content_rules: vec![ContentRule {
                component: "domain".to_string(),
                allowed_kinds: vec!["struct".to_string()],
            }],
            ..Default::default()
        };

        let findings = ContentChecker.check(&graph, &config);
        assert!(findings.is_empty());
    }

    #[test]
    fn content_checker_flags_zero_match_component_as_advisory() {
        let graph = DeclarationGraph {
            declarations: vec![Declaration {
                name: "Thing".to_string(),
                kind: DeclKind::Struct,
                file: PathBuf::from("infra/infra.go"),
                line: 1,
                component: Some("infra".to_string()),
            }],
        };
        let config = ArchitectureConfig {
            components: vec![Component {
                name: "domain".to_string(),
                paths: vec!["**/domain".to_string()],
            }],
            ..Default::default()
        };

        let findings = ContentChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, None);
        assert_eq!(findings[0].line, None);
        assert_eq!(findings[0].severity_override, Some(Severity::Advisory));
        assert_eq!(
            findings[0].message,
            "[component] component 'domain' (glob '**/domain') matched 0 declarations — rules \
             referencing it will never fire"
        );
    }
}
