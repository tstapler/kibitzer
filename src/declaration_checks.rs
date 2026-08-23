//! Story 2.1.3: the `DeclarationChecker` trait and its registry — the third checker
//! kind ADR-001 calls for, alongside `checker::Checker` (per-file AST checks) and
//! `architecture_checks::ArchitectureChecker` (whole-repo import-graph checks). Story
//! 2.2.1 adds `ContentChecker`. Epic 2.2 also brought forward a minimal `NamingChecker`
//! slice (Task 3.1.1a/3.1.1b only) solely because Story 2.2.3's git-HEAD-baseline-
//! downgrade fix needed a *second* real Declaration-kind checker to prove its dispatch
//! fix is generic, not `content-rules`-specific. Epic 3.1 (this pass) completes
//! `NamingChecker` per plan.md's full Story 3.1.1/3.1.2 spec: config-load-time regex
//! validation (Task 3.1.1c, in `config.rs`) and the zero-match-pattern/zero-match-
//! component advisories (Story 3.1.2).

use regex::Regex;

use crate::architecture_checks::{ArchFinding, zero_match_advisory};
use crate::config::{ArchitectureConfig, Severity};
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

/// Registers `ContentChecker` (Story 2.2.1) and `NamingChecker` (Task 3.1.1b) — both
/// resolve through `crate::check::lookup_any_architecture_checker`'s dual-registry
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
                        "[content] '{}' ({kind}) is not allowed in component '{component}' \
                         — allowed kinds: {}",
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

/// Story 3.1.1/3.1.2: "declarations of kind K in component X must match pattern Y" —
/// deliberately *not* "types implementing interface I must match pattern Y" in v1 (see
/// plan.md's Pattern Decisions table, "Naming-rule scope (v1)" row: Go interface
/// satisfaction is structural/cross-file and out of scope here). A declaration whose
/// `(component, kind)` matches no `NamingRule` is unaffected. An invalid regex is
/// treated as "no rule" for that declaration rather than panicking as defense in depth,
/// but in practice never reaches `check()` with an invalid pattern — Task 3.1.1c's
/// `config::validate_naming_rule_patterns` rejects it at config-load time first.
pub struct NamingChecker;

impl DeclarationChecker for NamingChecker {
    fn name(&self) -> &str {
        "naming-rules"
    }

    fn check(&self, graph: &DeclarationGraph, config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let mut findings: Vec<ArchFinding> = graph
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
                        "[naming] {kind} '{}' in component '{component}' does not match \
                         required pattern '{}'",
                        decl.name, rule.pattern
                    ),
                    severity_override: None,
                })
            })
            .collect();

        // Story 3.1.2/Task 3.1.2a: a `NamingRule` whose `(component, kind)` matches zero
        // declarations in the current graph is a dead rule — surfaced as an advisory so
        // it never masquerades as blocking, distinct from the per-declaration mismatch
        // findings above.
        findings.extend(config.naming_rules.iter().filter_map(|rule| {
            let matched = graph.declarations.iter().any(|d| {
                d.component.as_deref() == Some(rule.component.as_str())
                    && kind_name(d.kind) == rule.kind
            });
            if matched {
                return None;
            }
            Some(ArchFinding {
                file: None,
                line: None,
                message: format!(
                    "[naming] naming rule for component '{}' kind '{}' matched 0 declarations \
                     — pattern '{}' will never fire",
                    rule.component, rule.kind, rule.pattern
                ),
                severity_override: Some(Severity::Advisory),
            })
        }));

        // Task 3.1.2c: the same shared zero-match-component advisory `ContentChecker`
        // emits (Task 2.2.1d), reused here against `graph.declarations`. Iterates every
        // declared component (not just `naming_rules`-referenced ones), same as
        // `ContentChecker` — so a component shared by both `content_rules` and
        // `naming_rules` intentionally produces the advisory from each checker
        // independently (documented duplication, see plan.md Pattern Decisions /
        // Unresolved Questions #4).
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

pub fn lookup(name: &str) -> Option<Box<dyn DeclarationChecker>> {
    registry().into_iter().find(|c| c.name() == name)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{Component, ContentRule, NamingRule, Severity};
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
            "[content] 'Validate' (function) is not allowed in component 'domain' — allowed \
             kinds: struct"
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

    // --- Story 3.1.1: NamingChecker pattern evaluation ---

    #[test]
    fn naming_checker_flags_non_matching_name() {
        let graph = DeclarationGraph {
            declarations: vec![Declaration {
                name: "OrderStore".to_string(),
                kind: DeclKind::Struct,
                file: PathBuf::from("infra/infra.go"),
                line: 4,
                component: Some("infra".to_string()),
            }],
        };
        let config = ArchitectureConfig {
            naming_rules: vec![NamingRule {
                component: "infra".to_string(),
                kind: "struct".to_string(),
                pattern: ".*Repository$|.*Client$".to_string(),
            }],
            ..Default::default()
        };

        let findings = NamingChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, Some(PathBuf::from("infra/infra.go")));
        assert_eq!(findings[0].line, Some(4));
        assert_eq!(
            findings[0].message,
            "[naming] struct 'OrderStore' in component 'infra' does not match required \
             pattern '.*Repository$|.*Client$'"
        );
    }

    #[test]
    fn naming_checker_allows_matching_name() {
        let graph = DeclarationGraph {
            declarations: vec![Declaration {
                name: "OrderRepository".to_string(),
                kind: DeclKind::Struct,
                file: PathBuf::from("infra/infra.go"),
                line: 4,
                component: Some("infra".to_string()),
            }],
        };
        let config = ArchitectureConfig {
            naming_rules: vec![NamingRule {
                component: "infra".to_string(),
                kind: "struct".to_string(),
                pattern: ".*Repository$|.*Client$".to_string(),
            }],
            ..Default::default()
        };

        let findings = NamingChecker.check(&graph, &config);
        assert!(findings.is_empty());
    }

    #[test]
    fn naming_checker_scoped_to_declared_kind_only() {
        // A second, pattern-matching `struct` declaration keeps the `(infra, struct)`
        // rule from tripping Story 3.1.2's zero-match-rule advisory below, isolating
        // this test's actual subject: a `struct`-scoped rule must not affect a
        // `Function` declaration of the same name in the same component.
        let graph = DeclarationGraph {
            declarations: vec![
                Declaration {
                    name: "OrderRepository".to_string(),
                    kind: DeclKind::Struct,
                    file: PathBuf::from("infra/infra.go"),
                    line: 2,
                    component: Some("infra".to_string()),
                },
                Declaration {
                    name: "OrderStore".to_string(),
                    kind: DeclKind::Function,
                    file: PathBuf::from("infra/infra.go"),
                    line: 4,
                    component: Some("infra".to_string()),
                },
            ],
        };
        let config = ArchitectureConfig {
            naming_rules: vec![NamingRule {
                component: "infra".to_string(),
                kind: "struct".to_string(),
                pattern: ".*Repository$|.*Client$".to_string(),
            }],
            ..Default::default()
        };

        let findings = NamingChecker.check(&graph, &config);
        assert!(findings.is_empty());
    }

    // --- Story 3.1.2: zero-match naming pattern / zero-match component advisories ---

    #[test]
    fn naming_checker_flags_zero_match_rule_as_advisory() {
        let graph = DeclarationGraph::default();
        let config = ArchitectureConfig {
            naming_rules: vec![NamingRule {
                component: "handlers".to_string(),
                kind: "interface".to_string(),
                pattern: ".*Handler$".to_string(),
            }],
            ..Default::default()
        };

        let findings = NamingChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, None);
        assert_eq!(findings[0].line, None);
        assert_eq!(findings[0].severity_override, Some(Severity::Advisory));
        assert_eq!(
            findings[0].message,
            "[naming] naming rule for component 'handlers' kind 'interface' matched 0 \
             declarations — pattern '.*Handler$' will never fire"
        );
    }

    #[test]
    fn naming_checker_flags_zero_match_component_as_advisory() {
        let graph = DeclarationGraph::default();
        let config = ArchitectureConfig {
            components: vec![Component {
                name: "handlers".to_string(),
                paths: vec!["**/handlers".to_string()],
            }],
            ..Default::default()
        };

        let findings = NamingChecker.check(&graph, &config);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].file, None);
        assert_eq!(findings[0].line, None);
        assert_eq!(findings[0].severity_override, Some(Severity::Advisory));
        assert_eq!(
            findings[0].message,
            "[component] component 'handlers' (glob '**/handlers') matched 0 declarations — \
             rules referencing it will never fire"
        );
    }
}
