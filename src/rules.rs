use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::config::Severity;

/// A function/method body spanning more lines than this is flagged by `long-function`.
const LONG_FUNCTION_LINES: usize = 40;
/// A function/method body whose control-flow nesting depth exceeds this is flagged by
/// `deep-nesting`. Depth 1 is the function body itself; each nested
/// if/for/switch/select/func_literal adds one.
const MAX_NESTING_DEPTH: usize = 4;
/// A function/method parameter list naming more identifiers than this is flagged by
/// `long-parameter-list`.
const LONG_PARAM_LIST_COUNT: usize = 5;

/// Metadata for one rule in the catalog. Thresholds above are fixed for now —
/// per-rule configurability is a natural follow-up, not required for the initial
/// catalog.
#[allow(dead_code)]
pub struct RuleMeta {
    pub id: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub default_severity: Severity,
}

/// Self-documentation for future consumers (e.g. a `kibitzer rules list` command) —
/// not read by the checker logic itself, hence the blanket allow above.
#[allow(dead_code)]
pub const CATALOG: &[RuleMeta] = &[
    RuleMeta {
        id: "long-function",
        category: "complexity",
        description: "Function/method body spans more than 40 lines.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "deep-nesting",
        category: "complexity",
        description: "Function/method body nests if/for/switch/select/func_literal more than 4 levels deep.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "long-parameter-list",
        category: "style",
        description: "Function/method parameter list names more than 5 identifiers.",
        default_severity: Severity::Advisory,
    },
];

/// Per-language node-kind table the AST walk consults instead of hardcoded literals.
/// Verified against each grammar's real `to_sexp()` output, not guessed by analogy —
/// TS/JS/Go all use different node shapes for the "same" constructs (e.g. Go's chained
/// `else if` nests a bare `if_statement` under `alternative`, JS/TS wrap it in an
/// `else_clause` first).
struct LangRuleConfig {
    /// Checker name suffix distinguishing this language's registry entry — `lookup()`
    /// matches on exact name, so each language needs a distinct one (see
    /// `checker::lookup`'s first-match semantics).
    name: &'static str,
    file_globs: &'static [&'static str],
    /// Declaration-like node kinds checked for the three rules below.
    function_kinds: &'static [&'static str],
    /// Node kinds that add one level of nesting depth. `if_statement` is always
    /// implicitly included (handled specially to flatten `else if` chains) and should
    /// not be repeated here.
    nesting_kinds: &'static [&'static str],
    /// Node kinds an `if_statement`'s `alternative` field may be wrapped in before the
    /// chained `if_statement` itself — e.g. JS/TS's `else_clause`. Go has none.
    else_wrapper_kinds: &'static [&'static str],
    /// Counts the parameters in a `parameters`-field node for this language's grammar.
    param_counter: fn(Node) -> usize,
}

fn go_param_identifier_count(params: Node) -> usize {
    let mut count = 0;
    let mut cursor = params.walk();
    for decl in params.children(&mut cursor) {
        if decl.kind() != "parameter_declaration" && decl.kind() != "variadic_parameter_declaration"
        {
            continue;
        }
        let mut name_cursor = decl.walk();
        let named = decl
            .children_by_field_name("name", &mut name_cursor)
            .count();
        // A parameter_declaration with no name field still names one anonymous
        // parameter (e.g. `func f(string)` in an interface method set).
        count += named.max(1);
    }
    count
}

/// JS/TS `formal_parameters` children are one-parameter-per-named-child: bare
/// identifiers/patterns in JS (`identifier`, `assignment_pattern`, `rest_pattern`,
/// `object_pattern`, `array_pattern`), each wrapped in `required_parameter` /
/// `optional_parameter` / `rest_parameter` in TS. Either way, each named child of
/// `formal_parameters` is exactly one parameter — unlike Go, which can group several
/// names under one `parameter_declaration`.
fn js_ts_param_count(params: Node) -> usize {
    let mut cursor = params.walk();
    params.named_children(&mut cursor).count()
}

fn lang_config(lang: Language) -> LangRuleConfig {
    match lang {
        Language::Go => LangRuleConfig {
            name: "syntax-rules",
            file_globs: &["**/*.go"],
            function_kinds: &["function_declaration", "method_declaration"],
            nesting_kinds: &[
                "for_statement",
                "expression_switch_statement",
                "type_switch_statement",
                "select_statement",
                "func_literal",
            ],
            else_wrapper_kinds: &[],
            param_counter: go_param_identifier_count,
        },
        Language::TypeScript => LangRuleConfig {
            name: "syntax-rules-typescript",
            file_globs: &["**/*.ts"],
            function_kinds: &[
                "function_declaration",
                "function_expression",
                "generator_function_declaration",
                "method_definition",
                "arrow_function",
            ],
            nesting_kinds: &[
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
                "switch_statement",
                "arrow_function",
                "function_expression",
            ],
            else_wrapper_kinds: &["else_clause"],
            param_counter: js_ts_param_count,
        },
        Language::Tsx => LangRuleConfig {
            name: "syntax-rules-tsx",
            file_globs: &["**/*.tsx"],
            ..lang_config(Language::TypeScript)
        },
        Language::JavaScript => LangRuleConfig {
            name: "syntax-rules-javascript",
            file_globs: &["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"],
            ..lang_config(Language::TypeScript)
        },
    }
}

pub struct SyntaxRulesChecker {
    lang: Language,
}

impl SyntaxRulesChecker {
    pub fn new(lang: Language) -> Self {
        Self { lang }
    }
}

impl Checker for SyntaxRulesChecker {
    fn name(&self) -> &str {
        lang_config(self.lang).name
    }

    fn description(&self) -> &str {
        "native syntactic rule catalog: long-function, deep-nesting, long-parameter-list (see docs/syntax-rules.md)"
    }

    fn language(&self) -> Option<Language> {
        Some(self.lang)
    }

    fn file_globs(&self) -> &[&str] {
        lang_config(self.lang).file_globs
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("syntax-rules checker requires a parsed tree")?;
        let cfg = lang_config(self.lang);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }
}

fn walk_declarations(node: Node, cfg: &LangRuleConfig, findings: &mut Vec<Finding>) {
    if cfg.function_kinds.contains(&node.kind()) {
        check_declaration(node, cfg, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_declarations(child, cfg, findings);
    }
}

fn check_declaration(decl: Node, cfg: &LangRuleConfig, findings: &mut Vec<Finding>) {
    let line = decl.start_position().row + 1;

    if let Some(body) = decl.child_by_field_name("body") {
        let body_lines = body.end_position().row - body.start_position().row + 1;
        if body_lines > LONG_FUNCTION_LINES {
            findings.push(Finding {
                line,
                message: format!(
                    "[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider splitting it up"
                ),
            });
        }

        let depth = max_nesting_depth(body, 1, cfg);
        if depth > MAX_NESTING_DEPTH {
            findings.push(Finding {
                line,
                message: format!(
                    "[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider extracting a function or inverting a condition"
                ),
            });
        }
    }

    if let Some(params) = decl.child_by_field_name("parameters") {
        let count = (cfg.param_counter)(params);
        if count > LONG_PARAM_LIST_COUNT {
            findings.push(Finding {
                line,
                message: format!(
                    "[long-parameter-list] parameter list names {count} identifiers (over {LONG_PARAM_LIST_COUNT}) — consider a config struct"
                ),
            });
        }
    }
}

/// Depth of `node` itself (as passed in via `current_depth`), taking the max over all
/// descendants. Each nesting-construct body adds one — except a chained `else if`,
/// which stays at the current depth rather than adding one (see `walk_if_chain`).
fn max_nesting_depth(node: Node, current_depth: usize, cfg: &LangRuleConfig) -> usize {
    if node.kind() == "if_statement" {
        return walk_if_chain(node, current_depth, cfg);
    }

    let mut max_depth = current_depth;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_depth = if child.kind() == "if_statement" || cfg.nesting_kinds.contains(&child.kind()) {
            current_depth + 1
        } else {
            current_depth
        };
        max_depth = max_depth.max(max_nesting_depth(child, child_depth, cfg));
    }
    max_depth
}

/// Walks a single `if_statement`, flattening any `else if` chain to the same depth
/// (an `else if` is a flat branch, not real nesting) while still recursing into the
/// condition/consequence/genuine-else-block at the same depth to find real nesting
/// inside them. `alternative` may point straight at the chained `if_statement` (Go)
/// or wrap it one or more levels deep in a language's `else_wrapper_kinds` (JS/TS's
/// `else_clause`) before reaching either another `if_statement` (chain continues) or
/// a plain block (chain ends, walked at the same depth since `else` itself isn't
/// nesting).
fn walk_if_chain(if_node: Node, depth: usize, cfg: &LangRuleConfig) -> usize {
    let mut max_depth = depth;

    if let Some(condition) = if_node.child_by_field_name("condition") {
        max_depth = max_depth.max(max_nesting_depth(condition, depth, cfg));
    }
    if let Some(consequence) = if_node.child_by_field_name("consequence") {
        max_depth = max_depth.max(max_nesting_depth(consequence, depth, cfg));
    }

    if let Some(alt) = if_node.child_by_field_name("alternative") {
        let mut cur = alt;
        loop {
            if cur.kind() == "if_statement" {
                max_depth = max_depth.max(walk_if_chain(cur, depth, cfg));
                break;
            } else if cfg.else_wrapper_kinds.contains(&cur.kind()) {
                match cur.named_child(0) {
                    Some(inner) => cur = inner,
                    None => break,
                }
            } else {
                max_depth = max_depth.max(max_nesting_depth(cur, depth, cfg));
                break;
            }
        }
    }

    max_depth
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .context("loading tree-sitter-go grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Go source with tree-sitter")?;

        let cfg = lang_config(Language::Go);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }

    #[test]
    fn allows_short_function() {
        let findings = check_source("package main\nfunc f() {\n\tprintln(\"ok\")\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_long_function() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn allows_shallow_nesting() {
        let findings = check_source(
            "package main\nfunc f(x int) {\n\tif x > 0 {\n\t\tprintln(\"pos\")\n\t}\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn flags_deep_nesting() {
        let src = "package main\n\
             func f(x int) {\n\
             \tif x > 0 {\n\
             \t\tfor i := 0; i < x; i++ {\n\
             \t\t\tswitch i {\n\
             \t\t\tcase 0:\n\
             \t\t\t\tif i == 0 {\n\
             \t\t\t\t\tprintln(\"deep\")\n\
             \t\t\t\t}\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             }\n";
        let findings = check_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn allows_short_parameter_list() {
        let findings = check_source("package main\nfunc f(a, b string) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn flags_long_parameter_list() {
        let findings = check_source("package main\nfunc f(a, b, c, d, e, f int) {}\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn rules_fire_independently_on_one_function() {
        let mut src = String::from("package main\nfunc f(a, b, c, d, e, g int) {\n");
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn flat_else_if_chain_does_not_count_as_nesting() {
        let src = "package main\n\
             func f(x int) {\n\
             \tif x == 0 {\n\
             \t\tprintln(\"a\")\n\
             \t} else if x == 1 {\n\
             \t\tprintln(\"b\")\n\
             \t} else if x == 2 {\n\
             \t\tprintln(\"c\")\n\
             \t} else if x == 3 {\n\
             \t\tprintln(\"d\")\n\
             \t} else if x == 4 {\n\
             \t\tprintln(\"e\")\n\
             \t} else {\n\
             \t\tprintln(\"f\")\n\
             \t}\n\
             }\n";
        let findings = check_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn empty_file_has_no_findings() {
        let findings = check_source("package main\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn checks_method_declarations_too() {
        let findings =
            check_source("package main\nfunc (r *T) f(a, b, c, d, e, g int) {}\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn all_three_rules_can_fire_on_one_function() {
        let mut src = String::from("package main\nfunc f(a, b, c, d, e, g int) {\n");
        src.push_str(
            "\tif a > 0 {\n\t\tfor i := 0; i < a; i++ {\n\t\t\tswitch i {\n\t\t\tcase 0:\n\t\t\t\tif i == 0 {\n\t\t\t\t\tprintln(\"deep\")\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n",
        );
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn function_at_exactly_forty_lines_is_not_long() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..38 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn function_at_forty_one_lines_is_long() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..39 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn five_params_is_not_long() {
        let findings = check_source("package main\nfunc f(a, b, c, d, e int) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn catalog_ids_are_unique_and_documented() {
        let ids: Vec<&str> = CATALOG.iter().map(|r| r.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len());
        assert!(CATALOG.iter().all(|r| !r.description.is_empty()));
    }

    fn check_ts_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .context("loading tree-sitter-typescript grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing TypeScript source with tree-sitter")?;

        let cfg = lang_config(Language::TypeScript);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }

    fn check_js_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .context("loading tree-sitter-javascript grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing JavaScript source with tree-sitter")?;

        let cfg = lang_config(Language::JavaScript);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }

    #[test]
    fn ts_allows_short_function() {
        let findings = check_ts_source("function f() {\n  console.log(\"ok\");\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ts_flags_long_function() {
        let mut src = String::from("function f() {\n");
        for _ in 0..45 {
            src.push_str("  console.log(\"line\");\n");
        }
        src.push_str("}\n");
        let findings = check_ts_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn ts_flags_deep_nesting() {
        let src = "function f(x: number) {\n\
             \tif (x > 0) {\n\
             \t\tfor (let i = 0; i < x; i++) {\n\
             \t\t\tswitch (i) {\n\
             \t\t\tcase 0:\n\
             \t\t\t\tif (i === 0) {\n\
             \t\t\t\t\tconsole.log(\"deep\");\n\
             \t\t\t\t}\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             }\n";
        let findings = check_ts_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn ts_flat_else_if_chain_does_not_count_as_nesting() {
        let src = "function f(x: number) {\n\
             \tif (x === 0) {\n\
             \t\tconsole.log(\"a\");\n\
             \t} else if (x === 1) {\n\
             \t\tconsole.log(\"b\");\n\
             \t} else if (x === 2) {\n\
             \t\tconsole.log(\"c\");\n\
             \t} else if (x === 3) {\n\
             \t\tconsole.log(\"d\");\n\
             \t} else if (x === 4) {\n\
             \t\tconsole.log(\"e\");\n\
             \t} else {\n\
             \t\tconsole.log(\"f\");\n\
             \t}\n\
             }\n";
        let findings = check_ts_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn ts_allows_short_parameter_list() {
        let findings = check_ts_source("function f(a: string, b: string) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn ts_flags_long_parameter_list_with_optional_and_defaults() {
        let findings = check_ts_source(
            "function f(a: string, b = 1, c?: string, d: string, e: number, g: boolean) {}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn ts_destructured_and_rest_params_each_count_as_one() {
        let findings =
            check_ts_source("function f(a: string, {b, c}: {b: string, c: string}, ...rest: number[]) {}\n")
                .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn ts_checks_arrow_functions_and_class_methods() {
        let findings = check_ts_source(
            "const f = (a: string, b: string, c: string, d: string, e: string, g: string) => a;\n\
             class C {\n\
             \tmethod(a: string, b: string, c: string, d: string, e: string, g: string) { return a; }\n\
             }\n",
        )
        .unwrap();
        assert_eq!(
            findings
                .iter()
                .filter(|f| f.message.contains("[long-parameter-list]"))
                .count(),
            2
        );
    }

    #[test]
    fn js_allows_short_function() {
        let findings = check_js_source("function f() {\n  console.log(\"ok\");\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn js_flags_long_parameter_list_with_destructuring_and_rest() {
        let findings =
            check_js_source("function f(a, b = 1, {c, d}, [e], g, ...rest) {}\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn js_short_parameter_list_with_destructuring_is_not_long() {
        let findings = check_js_source("function f(a, {b, c}, ...rest) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn js_flat_else_if_chain_does_not_count_as_nesting() {
        let src = "function f(x) {\n\
             \tif (x === 0) {\n\
             \t\tconsole.log(\"a\");\n\
             \t} else if (x === 1) {\n\
             \t\tconsole.log(\"b\");\n\
             \t} else {\n\
             \t\tconsole.log(\"c\");\n\
             \t}\n\
             }\n";
        let findings = check_js_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn checker_names_are_distinct_per_language() {
        let names: Vec<&str> = [
            Language::Go,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
        ]
        .iter()
        .map(|&lang| lang_config(lang).name)
        .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }
}
