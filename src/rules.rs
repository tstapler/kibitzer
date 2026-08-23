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
    /// The if-like node kind for this grammar — `"if_statement"` everywhere except
    /// Kotlin's `"if_expression"`.
    if_kind: &'static str,
    /// Node kinds that add one level of nesting depth. `if_kind` is always implicitly
    /// included (handled specially to flatten `else if` chains) and should not be
    /// repeated here.
    nesting_kinds: &'static [&'static str],
    /// Node kinds an if-node's `alternative` field may be wrapped in before the
    /// chained if-node itself — e.g. JS/TS's `else_clause`. Go has none.
    else_wrapper_kinds: &'static [&'static str],
    /// Node kinds that behave like the if-node itself (own `condition`/`consequence`/
    /// `alternative` fields, or the positional equivalent) when found as a chained
    /// `alternative` — e.g. Python's `elif_clause`, which (unlike JS/TS) is a distinct
    /// node kind rather than an if-node wrapped in an `else_clause`. Empty everywhere
    /// else, since Go/JS/TS chain via a literal if-node and Kotlin's `if_expression`
    /// chains via a nested `if_expression` directly (already covered by `if_kind`).
    chain_kinds: &'static [&'static str],
    /// Counts the parameters in the node `params_finder` returns.
    param_counter: fn(Node) -> usize,
    /// Locates a declaration's body node. Field-based (`child_by_field_name("body")`)
    /// for every grammar so far except Kotlin, whose `function_declaration`/
    /// `anonymous_function` expose no field names at all — only positional children.
    body_finder: fn(Node) -> Option<Node>,
    /// Locates a declaration's parameter-list node. Same field-vs-positional split as
    /// `body_finder`.
    params_finder: fn(Node) -> Option<Node>,
}

fn field_body(decl: Node) -> Option<Node> {
    decl.child_by_field_name("body")
}

fn field_params(decl: Node) -> Option<Node> {
    decl.child_by_field_name("parameters")
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

/// Python's `parameters` children are one-parameter-per-named-child, like JS —
/// `identifier`, `default_parameter`, `typed_parameter`, `typed_default_parameter`,
/// `list_splat_pattern` (`*args`), `dictionary_splat_pattern` (`**kwargs`) — except it
/// also includes bare `positional_separator` (`/`) and `keyword_separator` (`*`) marker
/// nodes, which name no parameter and must be excluded from the count.
fn py_param_count(params: Node) -> usize {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|c| c.kind() != "positional_separator" && c.kind() != "keyword_separator")
        .count()
}

/// Kotlin's `function_value_parameters` children are `parameter` nodes plus a sibling
/// `parameter_modifiers` node (holding `vararg`/etc.) — the modifier node names no
/// parameter itself and must be excluded from the count.
fn kotlin_param_count(params: Node) -> usize {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
        .count()
}

/// Kotlin's `function_declaration`/`anonymous_function` expose no field names at all
/// (verified via an explicit `field_name_for_child` dump, not just `to_sexp()`, since
/// the latter's omission of field names was initially ambiguous) — the body is instead
/// the first positional `function_body` child.
fn kotlin_body(decl: Node) -> Option<Node> {
    let mut cursor = decl.walk();
    decl.named_children(&mut cursor)
        .find(|c| c.kind() == "function_body")
}

/// Same positional situation as `kotlin_body`: the parameter list is the first
/// `function_value_parameters` child, found by kind rather than field name.
fn kotlin_params(decl: Node) -> Option<Node> {
    let mut cursor = decl.walk();
    decl.named_children(&mut cursor)
        .find(|c| c.kind() == "function_value_parameters")
}

fn lang_config(lang: Language) -> LangRuleConfig {
    match lang {
        Language::Go => LangRuleConfig {
            name: "syntax-rules",
            file_globs: &["**/*.go"],
            function_kinds: &["function_declaration", "method_declaration"],
            if_kind: "if_statement",
            nesting_kinds: &[
                "for_statement",
                "expression_switch_statement",
                "type_switch_statement",
                "select_statement",
                "func_literal",
            ],
            else_wrapper_kinds: &[],
            chain_kinds: &[],
            param_counter: go_param_identifier_count,
            body_finder: field_body,
            params_finder: field_params,
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
            if_kind: "if_statement",
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
            chain_kinds: &[],
            param_counter: js_ts_param_count,
            body_finder: field_body,
            params_finder: field_params,
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
        Language::Python => LangRuleConfig {
            name: "syntax-rules-python",
            file_globs: &["**/*.py"],
            // Decorators wrap a `function_definition` in a `decorated_definition` node
            // (with the function as its `definition` field) — no separate entry needed
            // here since `walk_declarations` recurses into every child regardless of
            // kind, so the wrapped `function_definition` is still found. `async def`
            // produces a plain `function_definition` too (verified via to_sexp — no
            // distinct "async" node kind wraps it).
            function_kinds: &["function_definition"],
            if_kind: "if_statement",
            nesting_kinds: &[
                "for_statement",
                "while_statement",
                "match_statement",
                "lambda",
            ],
            else_wrapper_kinds: &[],
            // Python's `elif` is a distinct `elif_clause` node (not an `if_statement`
            // wrapped in an `else_clause` like JS/TS) but carries the same
            // condition/consequence/alternative fields, so it chains like `if_statement`
            // itself once recognized here.
            chain_kinds: &["elif_clause"],
            param_counter: py_param_count,
            body_finder: field_body,
            params_finder: field_params,
        },
        Language::Java => LangRuleConfig {
            name: "syntax-rules-java",
            file_globs: &["**/*.java"],
            // Constructors (`constructor_declaration`) are deliberately excluded for
            // now — unverified against the real grammar; can be added later without
            // disturbing this entry.
            function_kinds: &["method_declaration", "lambda_expression"],
            if_kind: "if_statement",
            nesting_kinds: &[
                "for_statement",
                "enhanced_for_statement",
                "while_statement",
                "do_statement",
                "switch_expression",
                "lambda_expression",
            ],
            else_wrapper_kinds: &[],
            chain_kinds: &[],
            param_counter: js_ts_param_count,
            body_finder: field_body,
            params_finder: field_params,
        },
        Language::Kotlin => LangRuleConfig {
            name: "syntax-rules-kotlin",
            file_globs: &["**/*.kt", "**/*.kts"],
            // `function_declaration` covers both top-level functions and class methods
            // (like Python's `function_definition`). `anonymous_function` is Kotlin's
            // `fun(x: Int) { ... }` expression form — checked the same way TS/JS check
            // `arrow_function`/`function_expression` (both a function-kind and a
            // nesting-kind). Lambda literals (`{ x -> ... }`) are nesting-only, like
            // Go's `func_literal` — their `lambda_parameters` node shape differs from
            // `function_value_parameters` and isn't handled by `kotlin_params`.
            function_kinds: &["function_declaration", "anonymous_function"],
            // Kotlin's `if_expression` exposes only `condition` as a named field — the
            // then-branch and else/elif continuation are positional named children.
            // `walk_if_chain`'s field lookups fall back to positional order when the
            // field lookup returns `None`, so no separate flag is needed here; the
            // chained `elif` is itself a nested `if_expression` (covered by `if_kind`
            // already, not a distinct wrapper kind).
            if_kind: "if_expression",
            nesting_kinds: &[
                "for_statement",
                "while_statement",
                "do_while_statement",
                "when_expression",
                "lambda_literal",
                "anonymous_function",
            ],
            else_wrapper_kinds: &[],
            chain_kinds: &[],
            param_counter: kotlin_param_count,
            body_finder: kotlin_body,
            params_finder: kotlin_params,
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

    if let Some(body) = (cfg.body_finder)(decl) {
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

    if let Some(params) = (cfg.params_finder)(decl) {
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
    if node.kind() == cfg.if_kind {
        return walk_if_chain(node, current_depth, cfg);
    }

    let mut max_depth = current_depth;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_depth =
            if child.kind() == cfg.if_kind || cfg.nesting_kinds.contains(&child.kind()) {
                current_depth + 1
            } else {
                current_depth
            };
        max_depth = max_depth.max(max_nesting_depth(child, child_depth, cfg));
    }
    max_depth
}

/// Returns an if-node's then-branch and else/elif continuation. Prefers the
/// `consequence`/`alternative` fields (every grammar handled so far except Kotlin);
/// falls back to positional order — the first and second named children after
/// `condition` — for Kotlin's `if_expression`, which names only `condition`.
fn if_branches(if_node: Node<'_>) -> (Option<Node<'_>>, Option<Node<'_>>) {
    if let Some(consequence) = if_node.child_by_field_name("consequence") {
        return (
            Some(consequence),
            if_node.child_by_field_name("alternative"),
        );
    }

    let condition_id = if_node.child_by_field_name("condition").map(|n| n.id());
    let mut cursor = if_node.walk();
    let mut rest = if_node
        .named_children(&mut cursor)
        .filter(|c| Some(c.id()) != condition_id);
    (rest.next(), rest.next())
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
    let (consequence, alternative) = if_branches(if_node);
    if let Some(consequence) = consequence {
        max_depth = max_depth.max(max_nesting_depth(consequence, depth, cfg));
    }

    if let Some(alt) = alternative {
        let mut cur = alt;
        loop {
            if cur.kind() == cfg.if_kind || cfg.chain_kinds.contains(&cur.kind()) {
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
        let findings = check_ts_source(
            "function f(a: string, {b, c}: {b: string, c: string}, ...rest: number[]) {}\n",
        )
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
            Language::Python,
            Language::Java,
            Language::Kotlin,
        ]
        .iter()
        .map(|&lang| lang_config(lang).name)
        .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names.len(), sorted.len());
    }

    fn check_py_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .context("loading tree-sitter-python grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Python source with tree-sitter")?;

        let cfg = lang_config(Language::Python);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }

    #[test]
    fn py_allows_short_function() {
        let findings = check_py_source("def f():\n    print(\"ok\")\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn py_flags_long_function() {
        let mut src = String::from("def f():\n");
        for _ in 0..45 {
            src.push_str("    print(\"line\")\n");
        }
        let findings = check_py_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn py_flags_deep_nesting() {
        let src = "def f(x):\n\
             \tif x > 0:\n\
             \t\tfor i in range(x):\n\
             \t\t\twhile i > 0:\n\
             \t\t\t\tif i == 0:\n\
             \t\t\t\t\tprint(\"deep\")\n";
        let findings = check_py_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn py_flat_elif_chain_does_not_count_as_nesting() {
        let src = "def f(x):\n\
             \tif x == 0:\n\
             \t\tprint(0)\n\
             \telif x == 1:\n\
             \t\tprint(1)\n\
             \telif x == 2:\n\
             \t\tprint(2)\n\
             \telse:\n\
             \t\tprint(3)\n";
        let findings = check_py_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn py_allows_short_parameter_list() {
        let findings = check_py_source("def f(a, b, c):\n    return a + b + c\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn py_flags_long_parameter_list() {
        let findings = check_py_source("def f(a, b, c, d, e, f, g):\n    return a\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn py_positional_and_keyword_separators_do_not_count_as_parameters() {
        // `/` and `*` are marker nodes, not parameters — five real parameters here,
        // which must stay at the >5 threshold despite the two separators present.
        let findings = check_py_source("def f(a, b, /, c, *, d, e):\n    return a\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn py_decorated_and_async_functions_are_checked() {
        let mut src = String::from("@decorator\nasync def f():\n");
        for _ in 0..45 {
            src.push_str("    print(\"line\")\n");
        }
        let findings = check_py_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn py_checks_class_methods() {
        let mut src = String::from("class C:\n    def m(self):\n");
        for _ in 0..45 {
            src.push_str("        print(\"line\")\n");
        }
        let findings = check_py_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    fn check_java_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_java::LANGUAGE.into())
            .context("loading tree-sitter-java grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Java source with tree-sitter")?;

        let cfg = lang_config(Language::Java);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }

    #[test]
    fn java_allows_short_method() {
        let findings =
            check_java_source("class C {\n    void f() {\n        g();\n    }\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn java_flags_long_method() {
        let mut src = String::from("class C {\n    void f() {\n");
        for _ in 0..45 {
            src.push_str("        g();\n");
        }
        src.push_str("    }\n}\n");
        let findings = check_java_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn java_flags_deep_nesting() {
        let src = "class C {\n\
             \tvoid f(int x) {\n\
             \t\tif (x > 0) {\n\
             \t\t\tfor (int i = 0; i < x; i++) {\n\
             \t\t\t\twhile (i > 0) {\n\
             \t\t\t\t\tif (i == 0) {\n\
             \t\t\t\t\t\tg();\n\
             \t\t\t\t\t}\n\
             \t\t\t\t}\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             }\n";
        let findings = check_java_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn java_flat_else_if_chain_does_not_count_as_nesting() {
        let src = "class C {\n\
             \tvoid f(int x) {\n\
             \t\tif (x == 0) {\n\
             \t\t\tg();\n\
             \t\t} else if (x == 1) {\n\
             \t\t\tg();\n\
             \t\t} else {\n\
             \t\t\tg();\n\
             \t\t}\n\
             \t}\n\
             }\n";
        let findings = check_java_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn java_flags_long_parameter_list() {
        let findings = check_java_source(
            "class C {\n    void f(int a, int b, int c, int d, int e, int g) {}\n}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn java_allows_short_parameter_list() {
        let findings = check_java_source("class C {\n    void f(int a, int b) {}\n}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    fn check_kotlin_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_kotlin_ng::LANGUAGE.into())
            .context("loading tree-sitter-kotlin-ng grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Kotlin source with tree-sitter")?;

        let cfg = lang_config(Language::Kotlin);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, &mut findings);
        Ok(findings)
    }

    #[test]
    fn kotlin_allows_short_function() {
        let findings = check_kotlin_source("fun f() {\n    g()\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn kotlin_flags_long_function() {
        let mut src = String::from("fun f() {\n");
        for _ in 0..45 {
            src.push_str("    g()\n");
        }
        src.push_str("}\n");
        let findings = check_kotlin_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn kotlin_flags_deep_nesting() {
        let src = "fun f(x: Int) {\n\
             \tif (x > 0) {\n\
             \t\tfor (i in 0..x) {\n\
             \t\t\twhile (i > 0) {\n\
             \t\t\t\tif (i == 0) {\n\
             \t\t\t\t\tg()\n\
             \t\t\t\t}\n\
             \t\t\t}\n\
             \t\t}\n\
             \t}\n\
             }\n";
        let findings = check_kotlin_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn kotlin_flat_else_if_chain_does_not_count_as_nesting() {
        // Exercises `if_branches`'s positional fallback: Kotlin's `if_expression` has
        // no `consequence`/`alternative` fields, only `condition`.
        let src = "fun f(x: Int) {\n\
             \tif (x == 0) {\n\
             \t\tg()\n\
             \t} else if (x == 1) {\n\
             \t\tg()\n\
             \t} else {\n\
             \t\tg()\n\
             \t}\n\
             }\n";
        let findings = check_kotlin_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn kotlin_flags_long_parameter_list() {
        let findings =
            check_kotlin_source("fun f(a: Int, b: Int, c: Int, d: Int, e: Int, g: Int) {}\n")
                .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn kotlin_allows_short_parameter_list() {
        let findings = check_kotlin_source("fun f(a: Int, b: Int) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn kotlin_vararg_modifier_does_not_inflate_parameter_count() {
        // `vararg` produces a sibling `parameter_modifiers` node, not one nested inside
        // `parameter` — `kotlin_param_count` must filter to `kind() == "parameter"` only.
        let findings = check_kotlin_source("fun f(a: Int, vararg b: Int) {}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }
}
