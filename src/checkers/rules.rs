use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::config::Severity;
use crate::file_size;

/// A function/method body spanning more lines than this is flagged by `long-function`.
const LONG_FUNCTION_LINES: usize = 40;
/// A function/method body whose control-flow nesting depth exceeds this is flagged by
/// `deep-nesting`. Depth 1 is the function body itself; each nested
/// if/for/switch/select/func_literal adds one.
const MAX_NESTING_DEPTH: usize = 4;
/// A function/method parameter list naming more identifiers than this is flagged by
/// `long-parameter-list`.
const LONG_PARAM_LIST_COUNT: usize = 5;
/// The repetition threshold for `replace-magic-literal`. Bumped from AC2's literal `2`
/// to `3` per ADR-001's pre-committed 40% false-positive bar: the corpus backtest
/// (`docs/backtest-triage/*/replace-magic-literal.jsonl`, 200 sampled findings across 7
/// real-world repos) found a 76% overall false-positive rate, 46% of it citing the same
/// table-driven-test/fixture overcorrection `duplicate-code` hit — both well past the
/// bar, so this is the binding half of that gate, not a preemptive guess.
const MAGIC_LITERAL_MIN_OCCURRENCES: usize = 3;
/// Near-universal literal values `replace-magic-literal` never flags regardless of
/// repeat count, compared against a value already normalized by
/// `normalize_literal_value` (delimiters/prefixes stripped) — so `""` here also matches
/// Kotlin's `""""""`, Rust's `r""`, Go's empty backtick string, and Python's `r""`/`b""`/
/// `f""` once normalized, not just a literal `""`/`''` in the raw source text.
/// `-1`'s per-grammar shape (Task 1.1.1b probe, `to_sexp()`-verified): every one of the 8
/// grammars here tokenizes a negative numeric literal as a `unary_expression`/
/// `-`-prefixed wrapper around the positive literal, *not* as a single literal token —
/// so `"-1"` is not itself ever an `occurrences` map key for these grammars; it's kept
/// in the allow-list anyway as defense-in-depth for any future grammar/version where it
/// might be.
const MAGIC_LITERAL_ALLOWLIST: &[&str] = &["0", "1", "-1", ""];

// `replace-magic-literal`'s per-language `literal_kinds`/`numeric_literal_kinds` node-kind
// tables, hoisted to named constants (rather than inline in each `*_lang_config()`) so
// adding them didn't push these constructors' bodies over `LONG_FUNCTION_LINES` — the
// same threshold this rule catalog enforces on every other codebase it checks.
const GO_LITERAL_KINDS: &[&str] = &[
    "int_literal",
    "float_literal",
    "imaginary_literal",
    "rune_literal",
    "interpreted_string_literal",
    "raw_string_literal",
];
const GO_NUMERIC_LITERAL_KINDS: &[&str] = &[
    "int_literal",
    "float_literal",
    "imaginary_literal",
    "rune_literal",
];
const PYTHON_LITERAL_KINDS: &[&str] = &["integer", "float", "string"];
const PYTHON_NUMERIC_LITERAL_KINDS: &[&str] = &["integer", "float"];
const JAVA_LITERAL_KINDS: &[&str] = &[
    "decimal_integer_literal",
    "hex_integer_literal",
    "octal_integer_literal",
    "binary_integer_literal",
    "decimal_floating_point_literal",
    "hex_floating_point_literal",
    "string_literal",
];
const JAVA_NUMERIC_LITERAL_KINDS: &[&str] = &[
    "decimal_integer_literal",
    "hex_integer_literal",
    "octal_integer_literal",
    "binary_integer_literal",
    "decimal_floating_point_literal",
    "hex_floating_point_literal",
];
// Verified against `tree-sitter-kotlin-ng` 1.1.0's `node-types.json`: no separate
// hex/binary/long/unsigned literal kinds exist — `number_literal` covers every integer
// form.
const KOTLIN_LITERAL_KINDS: &[&str] = &[
    "number_literal",
    "float_literal",
    "string_literal",
    "multiline_string_literal",
];
const KOTLIN_NUMERIC_LITERAL_KINDS: &[&str] = &["number_literal", "float_literal"];
const RUST_LITERAL_KINDS: &[&str] = &[
    "integer_literal",
    "float_literal",
    "string_literal",
    "raw_string_literal",
];
const RUST_NUMERIC_LITERAL_KINDS: &[&str] = &["integer_literal", "float_literal"];

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
        description: "Function/method body spans more than 40 lines — Fowler's Extract Function.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "deep-nesting",
        category: "complexity",
        description: "Function/method body nests if/for/switch/select/func_literal more than 4 levels deep — Fowler's Replace Nested Conditional with Guard Clauses.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "long-parameter-list",
        category: "style",
        description: "Function/method parameter list names more than 5 identifiers.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "flag-argument",
        category: "design",
        description: "Boolean parameter is branched on directly (if/ternary) in the function body — Fowler's Remove Flag Argument.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "unreachable-code",
        category: "dead-code",
        description: "A statement follows an unconditional return/break/continue/panic in the same block — Fowler's Remove Dead Code.",
        default_severity: Severity::Advisory,
    },
    RuleMeta {
        id: "replace-magic-literal",
        category: "duplication",
        description: "A non-trivial numeric or string literal is repeated 2+ times in one file with no bound named constant — Fowler's Replace Magic Literal.",
        default_severity: Severity::Advisory,
    },
];

/// A named-constant-style binding `binding_finder` found: a declaration whose direct
/// initializer is a literal, named so `replace-magic-literal`'s exclusion logic can
/// check whether that name is referenced elsewhere in the file. A named struct (not an
/// anonymous `(String, Node)` tuple) so a positional mix-up between `name` and
/// `initializer` is a compile error, not a silent bug.
pub(crate) struct ConstBinding<'tree> {
    pub(crate) name: String,
    pub(crate) initializer: Node<'tree>,
}

/// One occurrence of a literal value found by `walk_literals`, carrying the literal
/// node's own `node.kind()` alongside it so `emit_literal_findings` can label a finding
/// "numeric" vs. "string" from the grammar's own node-kind name (via
/// `LangRuleConfig::numeric_literal_kinds`) rather than re-deriving it from the raw
/// text's first character — a quote-character heuristic mislabels Go/Rust raw strings
/// (no leading `"`) and Python `r`/`b`/`f`-prefixed strings (leading `r`/`b`/`f`, not
/// `"`).
pub(crate) struct LiteralOccurrence<'tree> {
    pub(crate) node: Node<'tree>,
    pub(crate) kind: &'static str,
}

/// Accumulator for `walk_literals`'s single pass over the tree: every literal's raw-text
/// occurrences (grouped for repetition counting) and every qualifying named-constant
/// binding found (for `resolve_excluded_constants` to check against). Lives only for the
/// duration of one `check()` call.
#[derive(Default)]
pub(crate) struct LiteralCollector<'tree> {
    pub(crate) occurrences: HashMap<String, Vec<LiteralOccurrence<'tree>>>,
    pub(crate) bound: Vec<ConstBinding<'tree>>,
}

/// Per-language node-kind table the AST walk consults instead of hardcoded literals.
/// Verified against each grammar's real `to_sexp()` output, not guessed by analogy —
/// TS/JS/Go all use different node shapes for the "same" constructs (e.g. Go's chained
/// `else if` nests a bare `if_statement` under `alternative`, JS/TS wrap it in an
/// `else_clause` first).
pub(crate) struct LangRuleConfig {
    /// Checker name suffix distinguishing this language's registry entry — `lookup()`
    /// matches on exact name, so each language needs a distinct one (see
    /// `checker::lookup`'s first-match semantics).
    pub(crate) name: &'static str,
    pub(crate) file_globs: &'static [&'static str],
    /// Declaration-like node kinds checked for the rules below.
    pub(crate) function_kinds: &'static [&'static str],
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
    pub(crate) param_counter: fn(Node) -> usize,
    /// Locates a declaration's body node. Field-based (`child_by_field_name("body")`)
    /// for every grammar so far except Kotlin, whose `function_declaration`/
    /// `anonymous_function` expose no field names at all — only positional children.
    pub(crate) body_finder: fn(Node) -> Option<Node>,
    /// Locates a declaration's parameter-list node. Same field-vs-positional split as
    /// `body_finder`.
    pub(crate) params_finder: fn(Node) -> Option<Node>,
    /// Names of this language's boolean-typed parameters in a parameter-list node, for
    /// `flag-argument`. Returns only parameters whose type is unambiguously a plain
    /// boolean (skipping untyped/destructured/pattern parameters) to keep the check
    /// low-false-positive, per its issue's scope note.
    bool_param_finder: fn(Node, &[u8]) -> Vec<String>,
    /// The ternary-expression node kind that, like `if_kind`, exposes a `condition`
    /// field — `None` where the grammar has no ternary (Go, Kotlin, Rust) or where its
    /// ternary is positional rather than field-based (Python's `conditional_expression`,
    /// deliberately not special-cased here — the `if`-branching case is this check's
    /// primary target).
    ternary_kind: Option<&'static str>,
    /// Node kind for a `{ ... }`-style block whose (possibly indirect, see
    /// `statement_container`) children are an ordered statement sequence, for
    /// `unreachable-code`.
    block_kind: &'static str,
    /// Given a `block_kind` node, returns the node whose *named* children are that
    /// ordered statement sequence — identity for every grammar except Go, whose
    /// grammar nests the sequence one level deeper (`block > statement_list`).
    statement_container: fn(Node) -> Node,
    /// Given one raw child of `statement_container`'s result, returns the node to
    /// compare against `terminal_kinds` — identity for every grammar except Rust, whose
    /// grammar wraps a bare `return`/`break`/`continue` expression in an
    /// `expression_statement` (verified via `to_sexp()`; every other grammar here
    /// exposes `return_statement`/`break_statement`/`continue_statement` as a direct,
    /// unwrapped statement kind).
    unwrap_statement: fn(Node) -> Node,
    /// Node kinds that unconditionally end control flow when found (after
    /// `unwrap_statement`) as a statement in a `block_kind`. Empty of `break`/
    /// `continue` for Kotlin: tree-sitter-kotlin-ng 1.1.0 has no dedicated node kind for
    /// bare `break`/`continue` (verified — absent from its `node-types.json`; a bare
    /// `break` parses as a plain `identifier`, indistinguishable from a variable
    /// reference), so only `return_expression` is listed for it.
    terminal_kinds: &'static [&'static str],
    /// Returns true if the raw statement (before `unwrap_statement`) is a call/macro
    /// invocation this language treats as always-diverging — Go's `panic(...)`, Rust's
    /// `panic!`/`unreachable!`/`todo!`/`unimplemented!`. `no_panic_detector` for every
    /// other grammar, which has no such built-in.
    panic_detector: fn(Node, &[u8]) -> bool,
    /// Numeric/string literal node kinds `replace-magic-literal`'s `walk_literals` walks
    /// for this grammar.
    literal_kinds: &'static [&'static str],
    /// The subset of `literal_kinds` that are numeric (vs. string) — used to label a
    /// `replace-magic-literal` finding "numeric" or "string". Must be a subset of
    /// `literal_kinds` (checked by `node_kind_literals_are_valid_for_their_grammar`).
    numeric_literal_kinds: &'static [&'static str],
    /// Given any node, returns `Some(ConstBinding)` if that node is a named-constant-
    /// style binding (Go `const`, TS/JS `const` — not `let`, Java `final`, Kotlin `val`,
    /// Rust `const`/`static` — not `let`, Python SCREAMING_SNAKE_CASE) whose direct
    /// initializer is a literal; `None` otherwise. A literal bound this way, and
    /// referenced by name elsewhere in the file, is excluded from
    /// `replace-magic-literal` (see `resolve_excluded_constants`).
    binding_finder: for<'a> fn(Node<'a>, &'a [u8]) -> Option<ConstBinding<'a>>,
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

/// Rust's `parameters` children are `parameter` nodes (pattern + type fields) plus,
/// for a method, a leading `self_parameter` node (`&self`/`&mut self`/`self`) —
/// excluded from the count the same way Go's implicit receiver never appears in its
/// parameter list at all.
fn rust_param_count(params: Node) -> usize {
    let mut cursor = params.walk();
    params
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
        .count()
}

/// Go's `parameter_declaration` names its type once even when it groups several names
/// (`func f(a, b bool)` is one node with two `name` fields) — each grouped name is
/// still its own flag-argument candidate, so all of them are returned. Verified
/// against real `to_sexp()` output: a bool param's type is `type_identifier` with text
/// `"bool"` (not `qualified_type` or any wrapper).
fn go_bool_params(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for decl in params.children(&mut cursor) {
        if decl.kind() != "parameter_declaration" {
            continue;
        }
        let Some(ty) = decl.child_by_field_name("type") else {
            continue;
        };
        if ty.kind() != "type_identifier" || ty.utf8_text(src) != Ok("bool") {
            continue;
        }
        let mut names = decl.walk();
        for name in decl.children_by_field_name("name", &mut names) {
            if let Ok(text) = name.utf8_text(src) {
                out.push(text.to_string());
            }
        }
    }
    out
}

/// TS wraps each parameter in `required_parameter`/`optional_parameter` with `pattern`/
/// `type` fields (the `type` field is a `type_annotation` wrapping the real type node,
/// e.g. `predefined_type` for `boolean`); JS's bare pattern nodes carry neither field,
/// so `child_by_field_name("type")` naturally returns `None` there and the parameter is
/// skipped — correct, since JS has no static types to check. Destructured/rest patterns
/// (`pattern` isn't a plain `identifier`) are skipped too: not a nameable flag argument.
fn ts_js_bool_params(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        let name_node = param.child_by_field_name("pattern").unwrap_or(param);
        if name_node.kind() != "identifier" {
            continue;
        }
        let Some(annotation) = param.child_by_field_name("type") else {
            continue;
        };
        let mut acursor = annotation.walk();
        let is_bool = annotation
            .named_children(&mut acursor)
            .any(|t| t.kind() == "predefined_type" && t.utf8_text(src) == Ok("boolean"));
        if is_bool && let Ok(text) = name_node.utf8_text(src) {
            out.push(text.to_string());
        }
    }
    out
}

/// Python's `typed_parameter`/`typed_default_parameter` wrap the annotation in a `type`
/// field whose span is exactly the annotation text (`"bool"`, not `": bool"`) — verified
/// against real `to_sexp()` output. The parameter's name is its only other `identifier`
/// child (a default value, if any, is a different node kind, e.g. `true`/a literal).
fn py_bool_params(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        if param.kind() != "typed_parameter" && param.kind() != "typed_default_parameter" {
            continue;
        }
        let Some(ty) = param.child_by_field_name("type") else {
            continue;
        };
        if ty.utf8_text(src) != Ok("bool") {
            continue;
        }
        let ty_id = ty.id();
        let mut names = param.walk();
        if let Some(name) = param
            .named_children(&mut names)
            .find(|c| c.id() != ty_id && c.kind() == "identifier")
            && let Ok(text) = name.utf8_text(src)
        {
            out.push(text.to_string());
        }
    }
    out
}

/// Java's `formal_parameter` has `type`/`name` fields; a primitive bool is the distinct
/// `boolean_type` node kind, while the boxed `Boolean` is a plain `type_identifier` —
/// both count, since either can be branched on the same way. Verified against real
/// `to_sexp()` output for the primitive case.
fn java_bool_params(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for param in params.named_children(&mut cursor) {
        if param.kind() != "formal_parameter" {
            continue;
        }
        let Some(ty) = param.child_by_field_name("type") else {
            continue;
        };
        let is_bool = ty.kind() == "boolean_type"
            || (ty.kind() == "type_identifier" && ty.utf8_text(src) == Ok("Boolean"));
        if is_bool
            && let Some(name) = param.child_by_field_name("name")
            && let Ok(text) = name.utf8_text(src)
        {
            out.push(text.to_string());
        }
    }
    out
}

/// Kotlin's `parameter` node exposes no field names (same positional shape as
/// `kotlin_body`/`kotlin_params`): its identifier child is the name, and — since Kotlin
/// has no primitive-type keywords — a plain `Boolean` type is a `user_type` node
/// wrapping an `identifier` with that exact text. Verified against real `to_sexp()`
/// output.
fn kotlin_bool_params(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for param in params
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
    {
        let mut pcursor = param.walk();
        let children: Vec<Node> = param.named_children(&mut pcursor).collect();
        let Some(name_node) = children.iter().find(|c| c.kind() == "identifier") else {
            continue;
        };
        let is_bool = children.iter().any(|c| {
            c.kind() == "user_type"
                && c.named_child(0)
                    .is_some_and(|inner| inner.utf8_text(src) == Ok("Boolean"))
        });
        if is_bool && let Ok(text) = name_node.utf8_text(src) {
            out.push(text.to_string());
        }
    }
    out
}

/// Rust's `parameter` has `pattern`/`type` fields; a plain-by-value `bool` is the
/// distinct `primitive_type` node kind with text `"bool"` (a reference like `&bool` is
/// wrapped in `reference_type` instead and deliberately not unwrapped here — a
/// reference-to-bool parameter is rare enough, and different enough in call-site shape,
/// that it's left for a future pass rather than guessed at now). Destructured patterns
/// (tuple/struct) are skipped: not a nameable flag argument.
fn rust_bool_params(params: Node, src: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = params.walk();
    for param in params
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "parameter")
    {
        let Some(pattern) = param.child_by_field_name("pattern") else {
            continue;
        };
        if pattern.kind() != "identifier" {
            continue;
        }
        let Some(ty) = param.child_by_field_name("type") else {
            continue;
        };
        if ty.kind() != "primitive_type" || ty.utf8_text(src) != Ok("bool") {
            continue;
        }
        if let Ok(text) = pattern.utf8_text(src) {
            out.push(text.to_string());
        }
    }
    out
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

/// `statement_container`/`unwrap_statement` default for every grammar but Go/Rust.
fn identity_stmt(n: Node) -> Node {
    n
}

/// Go nests a block's actual statement sequence one level deeper than the `block`
/// node itself (`block > statement_list`) — an empty block has no `statement_list`
/// child at all, so this falls back to the (childless) block itself rather than
/// panicking.
fn go_statement_container(block: Node) -> Node {
    let mut cursor = block.walk();
    block
        .named_children(&mut cursor)
        .find(|c| c.kind() == "statement_list")
        .unwrap_or(block)
}

/// Go has no dedicated "diverging call" node kind — `panic(...)` is an ordinary
/// `call_expression`, so this matches on the called identifier's text.
fn go_panic_detector(stmt: Node, src: &[u8]) -> bool {
    if stmt.kind() != "expression_statement" {
        return false;
    }
    let Some(call) = stmt.named_child(0) else {
        return false;
    };
    if call.kind() != "call_expression" {
        return false;
    }
    let Some(func) = call.child_by_field_name("function") else {
        return false;
    };
    func.kind() == "identifier" && func.utf8_text(src) == Ok("panic")
}

/// No language besides Go/Rust has a built-in the `unreachable-code` rule treats as
/// always-diverging.
fn no_panic_detector(_stmt: Node, _src: &[u8]) -> bool {
    false
}

/// Rust's `return`/`break`/`continue` are expressions, so a bare one used as a
/// statement is wrapped in an `expression_statement` (verified via `to_sexp()`) —
/// unlike every other grammar here, which gives them their own direct statement kind.
fn rust_unwrap_statement(stmt: Node) -> Node {
    if stmt.kind() == "expression_statement" {
        stmt.named_child(0).unwrap_or(stmt)
    } else {
        stmt
    }
}

/// Matches Rust's diverging macros (`panic!`, `unreachable!`, `todo!`, `unimplemented!`)
/// invoked as a bare statement.
fn rust_panic_detector(stmt: Node, src: &[u8]) -> bool {
    if stmt.kind() != "expression_statement" {
        return false;
    }
    let Some(invocation) = stmt.named_child(0) else {
        return false;
    };
    if invocation.kind() != "macro_invocation" {
        return false;
    }
    let Some(name) = invocation.child_by_field_name("macro") else {
        return false;
    };
    matches!(
        name.utf8_text(src),
        Ok("panic") | Ok("unreachable") | Ok("todo") | Ok("unimplemented")
    )
}

/// Go's `const_spec`'s `value` field is always an `expression_list` (even for a single
/// value, verified via `to_sexp()`) — a multi-name (`const a, b = 1, 2`) or multi-value
/// spec returns `None` rather than guessing which name pairs with which value
/// (Unresolved Question 3: an accepted, documented false-negative gap, not fixed here).
fn go_const_binding<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ConstBinding<'a>> {
    if node.kind() != "const_spec" {
        return None;
    }
    let mut name_cursor = node.walk();
    let names: Vec<Node> = node
        .children_by_field_name("name", &mut name_cursor)
        .filter(|n| n.kind() == "identifier")
        .collect();
    if names.len() != 1 {
        return None;
    }
    let value_list = node.child_by_field_name("value")?;
    let mut vcursor = value_list.walk();
    let values: Vec<Node> = value_list.named_children(&mut vcursor).collect();
    if values.len() != 1 {
        return None;
    }
    let initializer = values[0];
    if !matches!(
        initializer.kind(),
        "int_literal"
            | "float_literal"
            | "imaginary_literal"
            | "rune_literal"
            | "interpreted_string_literal"
            | "raw_string_literal"
    ) {
        return None;
    }
    let name = names[0].utf8_text(src).ok()?.to_string();
    Some(ConstBinding { name, initializer })
}

/// TS/JS `lexical_declaration` exposes its `const`/`let` keyword via the `kind` field
/// directly (verified via `node-types.json`) — no raw-child scan needed. Only a
/// single-declarator statement qualifies (`const a = 1, b = 2` returns `None`, same
/// accepted gap as Go's multi-value `const_spec`). `let` is deliberately excluded
/// (ADR-001 / Pattern Decision "JS/TS/Rust binding scope").
fn ts_js_const_binding<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ConstBinding<'a>> {
    if node.kind() != "lexical_declaration" {
        return None;
    }
    let kind = node.child_by_field_name("kind")?;
    if kind.kind() != "const" {
        return None;
    }
    let mut cursor = node.walk();
    let declarators: Vec<Node> = node
        .named_children(&mut cursor)
        .filter(|c| c.kind() == "variable_declarator")
        .collect();
    if declarators.len() != 1 {
        return None;
    }
    let declarator = declarators[0];
    let name_node = declarator.child_by_field_name("name")?;
    if name_node.kind() != "identifier" {
        return None;
    }
    let initializer = declarator.child_by_field_name("value")?;
    if !matches!(initializer.kind(), "number" | "string") {
        return None;
    }
    let name = name_node.utf8_text(src).ok()?.to_string();
    Some(ConstBinding { name, initializer })
}

/// Python has no `const` keyword — a SCREAMING_SNAKE_CASE module/class-level assignment
/// is the idiomatic convention (ADR-001, Pattern Decision "Python constant heuristic").
/// Weaker than the other 7 languages' keyword-backed guarantee, deliberately.
fn py_screaming_snake_binding<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ConstBinding<'a>> {
    if node.kind() != "assignment" {
        return None;
    }
    let left = node.child_by_field_name("left")?;
    if left.kind() != "identifier" {
        return None;
    }
    let name = left.utf8_text(src).ok()?;
    if name.is_empty()
        || !name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return None;
    }
    let initializer = node.child_by_field_name("right")?;
    if !matches!(initializer.kind(), "integer" | "float" | "string") {
        return None;
    }
    Some(ConstBinding {
        name: name.to_string(),
        initializer,
    })
}

/// Java's `final` modifier is an anonymous token inside the `modifiers` child — not a
/// named field, so it's found via a raw (not just named) child scan, per the anonymous-
/// token caveat `node_kind_literals_are_valid_for_their_grammar` already documents.
/// Only the first declarator is considered when a `final` statement declares several
/// (`final int a = 1, b = 2`) — same accepted narrow scope as Go/TS's multi-binding gap.
fn java_final_binding<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ConstBinding<'a>> {
    if node.kind() != "local_variable_declaration" && node.kind() != "field_declaration" {
        return None;
    }
    let mut cursor = node.walk();
    let modifiers = node
        .children(&mut cursor)
        .find(|c| c.kind() == "modifiers")?;
    let mut mcursor = modifiers.walk();
    let has_final = modifiers
        .children(&mut mcursor)
        .any(|c| c.kind() == "final");
    if !has_final {
        return None;
    }
    let declarator = node.child_by_field_name("declarator")?;
    let name_node = declarator.child_by_field_name("name")?;
    if name_node.kind() != "identifier" {
        return None;
    }
    let initializer = declarator.child_by_field_name("value")?;
    if !matches!(
        initializer.kind(),
        "decimal_integer_literal"
            | "hex_integer_literal"
            | "octal_integer_literal"
            | "binary_integer_literal"
            | "decimal_floating_point_literal"
            | "hex_floating_point_literal"
            | "string_literal"
    ) {
        return None;
    }
    let name = name_node.utf8_text(src).ok()?.to_string();
    Some(ConstBinding { name, initializer })
}

/// Kotlin's `property_declaration` exposes no field names for its `val`/`var` keyword
/// or its positional `variable_declaration`/initializer children (same positional shape
/// as `kotlin_body`/`kotlin_params`) — found via raw-child and kind-based scans.
fn kotlin_val_binding<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ConstBinding<'a>> {
    if node.kind() != "property_declaration" {
        return None;
    }
    let mut cursor = node.walk();
    let is_val = node.children(&mut cursor).any(|c| c.kind() == "val");
    if !is_val {
        return None;
    }
    let mut cursor2 = node.walk();
    let children: Vec<Node> = node.children(&mut cursor2).collect();
    let var_decl = children
        .iter()
        .find(|c| c.kind() == "variable_declaration")?;
    let mut vcursor = var_decl.walk();
    let name_node = var_decl
        .named_children(&mut vcursor)
        .find(|c| c.kind() == "identifier")?;
    let initializer = *children.iter().find(|c| {
        matches!(
            c.kind(),
            "number_literal" | "float_literal" | "string_literal" | "multiline_string_literal"
        )
    })?;
    let name = name_node.utf8_text(src).ok()?.to_string();
    Some(ConstBinding { name, initializer })
}

/// Rust's `const_item`/`static_item` have flat `name`/`value` fields (verified via
/// `node-types.json`) — `let_declaration` is deliberately excluded (ADR-001 / Pattern
/// Decision "JS/TS/Rust binding scope").
fn rust_const_binding<'a>(node: Node<'a>, src: &'a [u8]) -> Option<ConstBinding<'a>> {
    if node.kind() != "const_item" && node.kind() != "static_item" {
        return None;
    }
    let name_node = node.child_by_field_name("name")?;
    let initializer = node.child_by_field_name("value")?;
    if !matches!(
        initializer.kind(),
        "integer_literal" | "float_literal" | "string_literal" | "raw_string_literal"
    ) {
        return None;
    }
    let name = name_node.utf8_text(src).ok()?.to_string();
    Some(ConstBinding { name, initializer })
}

/// Shared with `comment_quality`'s over-commented check, which needs the same
/// per-language function-kind/body lookup this file already maintains (Kotlin's
/// positional-only body lookup in particular) rather than duplicating it.
pub(crate) fn lang_config(lang: Language) -> LangRuleConfig {
    match lang {
        Language::Go => go_lang_config(),
        Language::TypeScript => typescript_lang_config(),
        Language::Tsx => tsx_lang_config(),
        Language::JavaScript => javascript_lang_config(),
        Language::Python => python_lang_config(),
        Language::Java => java_lang_config(),
        Language::Kotlin => kotlin_lang_config(),
        Language::Rust => rust_lang_config(),
    }
}

fn go_lang_config() -> LangRuleConfig {
    LangRuleConfig {
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
        bool_param_finder: go_bool_params,
        ternary_kind: None,
        block_kind: "block",
        statement_container: go_statement_container,
        unwrap_statement: identity_stmt,
        terminal_kinds: &["return_statement", "break_statement", "continue_statement"],
        panic_detector: go_panic_detector,
        literal_kinds: GO_LITERAL_KINDS,
        numeric_literal_kinds: GO_NUMERIC_LITERAL_KINDS,
        binding_finder: go_const_binding,
    }
}

fn typescript_lang_config() -> LangRuleConfig {
    LangRuleConfig {
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
        bool_param_finder: ts_js_bool_params,
        ternary_kind: Some("ternary_expression"),
        block_kind: "statement_block",
        statement_container: identity_stmt,
        unwrap_statement: identity_stmt,
        terminal_kinds: &["return_statement", "break_statement", "continue_statement"],
        panic_detector: no_panic_detector,
        literal_kinds: &["number", "string"],
        numeric_literal_kinds: &["number"],
        binding_finder: ts_js_const_binding,
    }
}

fn tsx_lang_config() -> LangRuleConfig {
    LangRuleConfig {
        name: "syntax-rules-tsx",
        file_globs: &["**/*.tsx"],
        ..typescript_lang_config()
    }
}

fn javascript_lang_config() -> LangRuleConfig {
    LangRuleConfig {
        name: "syntax-rules-javascript",
        file_globs: &["**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"],
        ..typescript_lang_config()
    }
}

fn python_lang_config() -> LangRuleConfig {
    LangRuleConfig {
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
        bool_param_finder: py_bool_params,
        ternary_kind: None,
        block_kind: "block",
        statement_container: identity_stmt,
        unwrap_statement: identity_stmt,
        terminal_kinds: &["return_statement", "break_statement", "continue_statement"],
        panic_detector: no_panic_detector,
        // tree-sitter-python 0.23.6: an f-string parses as a distinct `string` node
        // whose contents include `interpolation` children — same top-level `string`
        // kind as a plain string literal (verified via `codegen/node-types/python.json`
        // and `to_sexp()`), so it's already visited by `literal_kinds` without a
        // separate entry.
        literal_kinds: PYTHON_LITERAL_KINDS,
        numeric_literal_kinds: PYTHON_NUMERIC_LITERAL_KINDS,
        binding_finder: py_screaming_snake_binding,
    }
}

fn java_lang_config() -> LangRuleConfig {
    LangRuleConfig {
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
        bool_param_finder: java_bool_params,
        ternary_kind: Some("ternary_expression"),
        block_kind: "block",
        statement_container: identity_stmt,
        unwrap_statement: identity_stmt,
        terminal_kinds: &["return_statement", "break_statement", "continue_statement"],
        panic_detector: no_panic_detector,
        literal_kinds: JAVA_LITERAL_KINDS,
        numeric_literal_kinds: JAVA_NUMERIC_LITERAL_KINDS,
        binding_finder: java_final_binding,
    }
}

fn kotlin_lang_config() -> LangRuleConfig {
    LangRuleConfig {
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
        bool_param_finder: kotlin_bool_params,
        ternary_kind: None,
        block_kind: "block",
        statement_container: identity_stmt,
        unwrap_statement: identity_stmt,
        // `break`/`continue` omitted — see `terminal_kinds`'s doc comment.
        terminal_kinds: &["return_expression"],
        panic_detector: no_panic_detector,
        literal_kinds: KOTLIN_LITERAL_KINDS,
        numeric_literal_kinds: KOTLIN_NUMERIC_LITERAL_KINDS,
        binding_finder: kotlin_val_binding,
    }
}

fn rust_lang_config() -> LangRuleConfig {
    LangRuleConfig {
        name: "syntax-rules-rust",
        file_globs: &["**/*.rs"],
        // `function_item` covers free functions, inherent/trait `impl` methods, and
        // default trait-method bodies alike (one node kind for all three, verified
        // via `to_sexp()`). A trait method *declaration* with no body is the
        // distinct `function_signature_item` kind, deliberately excluded —
        // `body_finder` would return `None` for it anyway, same as Go interface
        // methods never appearing in `function_kinds`. `closure_expression` is
        // nesting-only (below), not function-like: its parameter list uses a
        // different node kind (`closure_parameters`, not `parameters`) and isn't
        // handled by `rust_param_count`.
        function_kinds: &["function_item"],
        // Rust's `if` is an expression with proper `condition`/`consequence`/
        // `alternative` fields (Go-like, no positional fallback needed). A chained
        // `else if` wraps in an intermediate `else_clause` before the nested
        // `if_expression` (JS/TS-like).
        if_kind: "if_expression",
        nesting_kinds: &[
            "for_expression",
            "while_expression",
            "loop_expression",
            "match_expression",
            "closure_expression",
        ],
        else_wrapper_kinds: &["else_clause"],
        chain_kinds: &[],
        param_counter: rust_param_count,
        body_finder: field_body,
        params_finder: field_params,
        bool_param_finder: rust_bool_params,
        ternary_kind: None,
        block_kind: "block",
        statement_container: identity_stmt,
        unwrap_statement: rust_unwrap_statement,
        terminal_kinds: &[
            "return_expression",
            "break_expression",
            "continue_expression",
        ],
        panic_detector: rust_panic_detector,
        literal_kinds: RUST_LITERAL_KINDS,
        numeric_literal_kinds: RUST_NUMERIC_LITERAL_KINDS,
        binding_finder: rust_const_binding,
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
        "native syntactic rule catalog: long-function, deep-nesting, long-parameter-list, flag-argument, unreachable-code, replace-magic-literal (see docs/syntax-rules.md)"
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
        let src = ctx.source.as_bytes();
        walk_declarations(tree.root_node(), &cfg, src, &mut findings);
        walk_blocks(tree.root_node(), &cfg, src, &mut findings);
        // Generated files are dense, mechanical, and by construction full of repeated
        // literals — more exposed to this rule than any of the other 4 (ADR-001,
        // pitfalls.md §2/§5). Scoped to just this pass; the other 4 rules are untouched.
        if !file_size::is_generated(ctx.source) {
            let mut collector = LiteralCollector::default();
            walk_literals(tree.root_node(), &cfg, src, &mut collector);
            let excluded = resolve_excluded_constants(tree.root_node(), src, &collector.bound);
            emit_literal_findings(&collector, &cfg, &excluded, &mut findings);
        }
        Ok(findings)
    }
}

fn walk_declarations(node: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>) {
    if cfg.function_kinds.contains(&node.kind()) {
        check_declaration(node, cfg, src, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_declarations(child, cfg, src, findings);
    }
}

/// Recurses over the whole tree (not just function bodies — a `unreachable-code` block
/// can be any `{ ... }`, including one nested inside another already-dead block) looking
/// for `block_kind` nodes to hand to `check_block_for_unreachable`.
fn walk_blocks(node: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>) {
    if node.kind() == cfg.block_kind {
        check_block_for_unreachable(node, cfg, src, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_blocks(child, cfg, src, findings);
    }
}

/// Collect-pass for `replace-magic-literal`: recurses the whole tree (not just function
/// bodies — a repeated literal anywhere in the file counts), grouping every
/// `literal_kinds` node by its raw source text and recording every qualifying
/// `binding_finder` match. Emits nothing itself — matches the two-pass collect-then-emit
/// shape architecture.md recommends over folding into `walk_blocks`'s single-pass
/// emit-immediately design.
fn walk_literals<'tree>(
    node: Node<'tree>,
    cfg: &LangRuleConfig,
    src: &'tree [u8],
    collector: &mut LiteralCollector<'tree>,
) {
    // `node.is_named()` matters here: TS/JS's grammar reuses the bare token names
    // "string"/"number" for the anonymous keyword inside a `predefined_type` type
    // annotation (`x: string`), which otherwise collides with the *named* literal
    // node kinds of the same name — `x: string` was firing as if `"string"` were a
    // repeated string literal. Confirmed via node-types.json: both a named (the
    // literal) and an unnamed (the type keyword) node share `kind() == "string"`.
    if node.is_named()
        && cfg.literal_kinds.contains(&node.kind())
        && let Ok(text) = node.utf8_text(src)
    {
        collector
            .occurrences
            .entry(text.to_string())
            .or_default()
            .push(LiteralOccurrence {
                node,
                kind: node.kind(),
            });
    }
    if let Some(binding) = (cfg.binding_finder)(node, src) {
        collector.bound.push(binding);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_literals(child, cfg, src, collector);
    }
}

/// Given every qualifying named-constant binding found by `walk_literals`, returns the
/// `Node::id()`s of the initializers that qualify for `replace-magic-literal`'s
/// exclusion: the binding's `name` must be referenced at least once beyond its own
/// declaration (count >= 2: the declaration itself plus >=1 real use). Same same-file,
/// non-scope-resolved heuristic as `collect_condition_identifiers` (ADR-001) — a binding
/// shadowed in a different function scope is an accepted, documented false negative.
fn resolve_excluded_constants(root: Node, src: &[u8], bound: &[ConstBinding]) -> HashSet<usize> {
    let mut ref_counts = HashMap::new();
    collect_identifiers(root, src, &mut ref_counts);
    bound
        .iter()
        .filter(|b| ref_counts.get(&b.name).copied().unwrap_or(0) >= 2)
        .map(|b| b.initializer.id())
        .collect()
}

/// Strips the language-specific delimiter/prefix from a raw literal's source text so an
/// empty-literal comparison against `MAGIC_LITERAL_ALLOWLIST` is structural, not a
/// fixed-string match — a raw-text allow-list of `"\"\""`/`"''"` never matches Kotlin's
/// empty `""""""`, Rust's `r""`, Go's empty backtick raw string, or Python's `r`/`b`/`f`-
/// prefixed empty string. Dispatches on `kind()` alone: `"raw_string_literal"` covers both
/// Go (backtick) and Rust (`r"..."`, tried second, after Go's strip no-ops on it) and
/// `"string"` covers both Python and TS/JS (harmless — a TS/JS string never starts with an
/// `r`/`b`/`f` prefix letter) — intentional, not accidental cross-language dispatch.
fn normalize_literal_value<'a>(raw: &'a str, kind: &str) -> &'a str {
    // Kotlin's multiline string: strip the triple-quote delimiter on each side.
    if kind == "multiline_string_literal"
        && let Some(inner) = raw
            .strip_prefix("\"\"\"")
            .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return inner;
    }
    // Go's raw string literal: backtick-delimited, no escape processing, no prefix.
    if kind == "raw_string_literal"
        && let Some(inner) = raw.strip_prefix('`').and_then(|s| s.strip_suffix('`'))
    {
        return inner;
    }
    // Rust's raw string literal: `r"..."`, `r#"..."#`, `r##"..."##`, etc.
    if kind == "raw_string_literal"
        && let Some(inner) = strip_rust_raw_string_delimiters(raw)
    {
        return inner;
    }
    if kind == "string"
        && let Some(inner) = strip_python_prefixed_quote_delimiter(raw)
    {
        return inner;
    }
    // Every other basic quoted form: `"..."`/`'...'` (single- or double-quoted).
    if let Some(inner) = strip_quote_delimiter(raw) {
        return inner;
    }
    raw
}

/// Strips Rust raw-string delimiters (`r"..."`, `r#"..."#`, `r##"..."##`, etc.) from
/// `raw`, if it matches that shape. Split out of `normalize_literal_value` to keep that
/// function's body under `LONG_FUNCTION_LINES`.
fn strip_rust_raw_string_delimiters(raw: &str) -> Option<&str> {
    let mut hashes = 0usize;
    let mut rest = raw.strip_prefix('r').unwrap_or(raw);
    while let Some(r) = rest.strip_prefix('#') {
        hashes += 1;
        rest = r;
    }
    let close = format!("\"{}", "#".repeat(hashes));
    rest.strip_prefix('"').and_then(|s| s.strip_suffix(&close))
}

/// Strips Python's string-prefix letters ahead of the quote character, then the quote
/// delimiter itself — `r`/`b`/`f`/`rb`/`br`/`rf`/`fr` (any case), plus the legacy
/// Python-2-compat standalone `u`/`U` (never combines with `r`/`b`/`f` in real Python,
/// but accepting the combination here is harmless: a real parse never produces it, so
/// being lenient about the character *set* can't misnormalize valid source). Split out
/// of `normalize_literal_value` to keep that function's body under `LONG_FUNCTION_LINES`.
fn strip_python_prefixed_quote_delimiter(raw: &str) -> Option<&str> {
    let prefix_len = raw
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .take(2)
        .count();
    let (prefix, rest) = raw.split_at(prefix_len);
    if !prefix
        .chars()
        .all(|c| matches!(c.to_ascii_lowercase(), 'r' | 'b' | 'f' | 'u'))
    {
        return None;
    }
    strip_quote_delimiter(rest)
}

/// Strips a matching pair of `"`/`'` (single-char) or `"""` (triple, for a plain,
/// non-prefixed Python triple-quoted string) delimiters from `s`, if present.
fn strip_quote_delimiter(s: &str) -> Option<&str> {
    if let Some(inner) = s
        .strip_prefix("\"\"\"")
        .and_then(|s| s.strip_suffix("\"\"\""))
    {
        return Some(inner);
    }
    if let Some(inner) = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        return Some(inner);
    }
    if let Some(inner) = s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        return Some(inner);
    }
    None
}

fn is_allowlisted_literal(raw: &str, kind: &str) -> bool {
    MAGIC_LITERAL_ALLOWLIST.contains(&normalize_literal_value(raw, kind))
}

/// Emit-pass for `replace-magic-literal`: applies the allow-list and the excluded-node-
/// id set, filters to literal values with >= `MAGIC_LITERAL_MIN_OCCURRENCES` remaining
/// occurrences, and pushes one `Finding` per qualifying value, anchored at the first
/// remaining occurrence's line.
fn emit_literal_findings(
    collector: &LiteralCollector,
    cfg: &LangRuleConfig,
    excluded: &HashSet<usize>,
    findings: &mut Vec<Finding>,
) {
    // `collector.occurrences` is a `HashMap`, whose iteration order is randomized per
    // process — collected into a local `Vec` and sorted by line so this rule's findings
    // are deterministic across runs, matching `go_table_driven_test.rs`/
    // `markdown_link_integrity.rs`'s existing `sort_by_key(|f| f.line)` convention.
    let mut new_findings: Vec<Finding> = collector
        .occurrences
        .iter()
        .filter_map(|(text, occurrences)| build_literal_finding(text, occurrences, cfg, excluded))
        .collect();
    new_findings.sort_by_key(|f| f.line);
    findings.extend(new_findings);
}

/// Applies the allow-list and the excluded-node-id set to one literal value's
/// occurrences, returning a `Finding` if >= `MAGIC_LITERAL_MIN_OCCURRENCES` remain —
/// anchored at the first remaining occurrence's line. Split out of
/// `emit_literal_findings` so the loop stays a filter/collect, not a 40+-line body.
fn build_literal_finding(
    text: &str,
    occurrences: &[LiteralOccurrence],
    cfg: &LangRuleConfig,
    excluded: &HashSet<usize>,
) -> Option<Finding> {
    let first = occurrences.first()?;
    if is_allowlisted_literal(text, first.kind) {
        return None;
    }
    let mut remaining: Vec<&LiteralOccurrence> = occurrences
        .iter()
        .filter(|occ| !excluded.contains(&occ.node.id()))
        .collect();
    if remaining.len() < MAGIC_LITERAL_MIN_OCCURRENCES {
        return None;
    }
    remaining.sort_by_key(|occ| occ.node.start_position().row);
    let label = if cfg.numeric_literal_kinds.contains(&remaining[0].kind) {
        "numeric"
    } else {
        "string"
    };
    let line = remaining[0].node.start_position().row + 1;
    let other_lines: Vec<String> = remaining[1..]
        .iter()
        .map(|occ| (occ.node.start_position().row + 1).to_string())
        .collect();
    // `MAGIC_LITERAL_MIN_OCCURRENCES` is >= 2, so `other_lines` always has >= 1 entry —
    // pluralize "line" instead of hardcoding the singular.
    let line_word = if other_lines.len() == 1 {
        "line"
    } else {
        "lines"
    };
    Some(Finding {
        line,
        message: format!(
            "[replace-magic-literal] {label} literal {text} appears {} times (also {line_word} {}) — consider extracting a named constant",
            remaining.len(),
            other_lines.join(", ")
        ),
    })
}

/// Flags at most one statement per block: the first one found after an unconditional
/// `return`/`break`/`continue`/panic-call. Everything past that point is dead by
/// construction, so a second finding for the same block would be noise, not signal.
/// Doesn't descend into `switch`/`match`/`when` case bodies — those aren't `block_kind`
/// nodes in any of the seven grammars here, so this scope is a property of the walk, not
/// a separate carve-out.
fn check_block_for_unreachable(
    block: Node,
    cfg: &LangRuleConfig,
    src: &[u8],
    findings: &mut Vec<Finding>,
) {
    let container = (cfg.statement_container)(block);
    let mut terminal: Option<(&'static str, usize)> = None;
    let mut cursor = container.walk();
    for stmt in container.named_children(&mut cursor) {
        if stmt.kind().contains("comment") {
            continue;
        }
        if let Some((label, term_line)) = terminal {
            findings.push(Finding {
                line: stmt.start_position().row + 1,
                message: format!(
                    "[unreachable-code] statement is unreachable — an unconditional `{label}` on line {term_line} ends this block first"
                ),
            });
            break;
        }
        let effective = (cfg.unwrap_statement)(stmt);
        if cfg.terminal_kinds.contains(&effective.kind()) {
            terminal = Some((
                terminal_label(effective.kind()),
                stmt.start_position().row + 1,
            ));
        } else if (cfg.panic_detector)(stmt, src) {
            terminal = Some(("panic", stmt.start_position().row + 1));
        }
    }
}

fn terminal_label(kind: &str) -> &'static str {
    if kind.starts_with("return") {
        "return"
    } else if kind.starts_with("break") {
        "break"
    } else {
        "continue"
    }
}

fn check_declaration(decl: Node, cfg: &LangRuleConfig, src: &[u8], findings: &mut Vec<Finding>) {
    let line = decl.start_position().row + 1;

    if let Some(body) = (cfg.body_finder)(decl) {
        let body_lines = body.end_position().row - body.start_position().row + 1;
        if body_lines > LONG_FUNCTION_LINES {
            findings.push(Finding {
                line,
                message: format!(
                    "[long-function] body spans {body_lines} lines (over {LONG_FUNCTION_LINES}) — consider Fowler's Extract Function (https://refactoring.com/catalog/extractFunction.html)"
                ),
            });
        }

        let depth = max_nesting_depth(body, 1, cfg);
        if depth > MAX_NESTING_DEPTH {
            findings.push(Finding {
                line,
                message: format!(
                    "[deep-nesting] body nests {depth} levels deep (over {MAX_NESTING_DEPTH}) — consider Fowler's Replace Nested Conditional with Guard Clauses (https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html)"
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

        let bool_params = (cfg.bool_param_finder)(params, src);
        if let Some(body) = (cfg.body_finder)(decl)
            && !bool_params.is_empty()
        {
            let mut branched_on = HashMap::new();
            collect_condition_identifiers(body, cfg, src, &mut branched_on);
            for name in bool_params {
                if branched_on.contains_key(&name) {
                    findings.push(Finding {
                        line,
                        message: format!(
                            "[flag-argument] boolean parameter `{name}` is branched on directly in the body — Fowler's Remove Flag Argument: split into two named functions or replace with a small enum"
                        ),
                    });
                }
            }
        }
    }
}

/// Collects every identifier referenced in the `condition` of an `if`/ternary anywhere
/// under `node` — not just a direct match, since the condition may be a compound
/// expression (`!flag`, `flag && x`) and the issue's scope explicitly covers "the
/// condition (or an operand of the condition)". Doesn't restrict to the passed-in
/// parameter's own function scope (a nested closure/lambda could shadow the name) —
/// deliberately: this check is a mechanical, low-false-positive heuristic, not a
/// scope-resolving analysis, and a shadowed name would just be one more legitimate hit.
fn collect_condition_identifiers(
    node: Node,
    cfg: &LangRuleConfig,
    src: &[u8],
    out: &mut HashMap<String, usize>,
) {
    if (node.kind() == cfg.if_kind || cfg.ternary_kind == Some(node.kind()))
        && let Some(condition) = node.child_by_field_name("condition")
    {
        collect_identifiers(condition, src, out);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_condition_identifiers(child, cfg, src, out);
    }
}

/// Counts every identifier reference under `node` by name — used both by
/// `collect_condition_identifiers` (is this name referenced in a branch condition at
/// all?) and `resolve_excluded_constants` (is this name referenced *beyond* its own
/// declaration — i.e. is its count >= 2?).
fn collect_identifiers(node: Node, src: &[u8], out: &mut HashMap<String, usize>) {
    if node.kind() == "identifier"
        && let Ok(text) = node.utf8_text(src)
    {
        *out.entry(text.to_string()).or_insert(0) += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers(child, src, out);
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

inventory::submit! {
    crate::checker::CheckerFactory(|| {
        Language::ALL
            .iter()
            .map(|&lang| Box::new(SyntaxRulesChecker::new(lang)) as Box<dyn Checker>)
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_source(src: &str) -> Result<Vec<Finding>> {
        let tree = crate::test_support::parse_go(src)?;

        let cfg = lang_config(Language::Go);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
        Ok(findings)
    }

    /// `check_unreachable` companions below need `walk_blocks`, not `walk_declarations` —
    /// takes the grammar as a parameter since `unreachable-code` is exercised across all
    /// seven languages, unlike `check_source` above (Go-only).
    fn check_unreachable(
        lang: Language,
        ts_lang: tree_sitter::Language,
        src: &str,
    ) -> Vec<Finding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang).expect("loading grammar");
        let tree = parser.parse(src, None).expect("parsing source");
        let cfg = lang_config(lang);
        let mut findings = Vec::new();
        walk_blocks(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
        findings
    }

    #[test]
    fn go_unreachable_after_return() {
        let findings = check_unreachable(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package m\nfunc f() {\n\tif true {\n\t\treturn\n\t\tfmt.Println(1)\n\t}\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("[unreachable-code]"));
        assert!(findings[0].message.contains("return"));
    }

    #[test]
    fn go_unreachable_after_panic() {
        let findings = check_unreachable(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package m\nfunc f() {\n\tpanic(\"x\")\n\tfmt.Println(1)\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("panic"));
    }

    #[test]
    fn go_allows_reachable_code() {
        let findings = check_unreachable(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package m\nfunc f() {\n\tif true {\n\t\tfmt.Println(1)\n\t}\n\tfmt.Println(2)\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn go_only_flags_first_statement_in_dead_region() {
        let findings = check_unreachable(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package m\nfunc f() {\n\treturn\n\tfmt.Println(1)\n\tfmt.Println(2)\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn typescript_unreachable_after_break() {
        let findings = check_unreachable(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "function f() {\n  while (true) {\n    break;\n    let x = 1;\n  }\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("break"));
    }

    #[test]
    fn python_unreachable_after_continue() {
        let findings = check_unreachable(
            Language::Python,
            tree_sitter_python::LANGUAGE.into(),
            "def f():\n    while True:\n        continue\n        x = 1\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("continue"));
    }

    #[test]
    fn java_unreachable_after_return() {
        let findings = check_unreachable(
            Language::Java,
            tree_sitter_java::LANGUAGE.into(),
            "class C { void f() {\n  if (true) {\n    return;\n    System.out.println(1);\n  }\n} }\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn kotlin_unreachable_after_return() {
        let findings = check_unreachable(
            Language::Kotlin,
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            "fun f() {\n  if (true) {\n    return\n    println(1)\n  }\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn kotlin_bare_break_is_not_flagged() {
        // tree-sitter-kotlin-ng 1.1.0 has no dedicated node kind for a bare `break` (it
        // parses as a plain `identifier`) — documenting the resulting false-negative
        // rather than silently relying on it, per `terminal_kinds`'s doc comment.
        let findings = check_unreachable(
            Language::Kotlin,
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            "fun f() {\n  while (true) {\n    break\n    println(1)\n  }\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn rust_unreachable_after_return_expression() {
        let findings = check_unreachable(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n  if true {\n    return;\n    println!(\"1\");\n  }\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("return"));
    }

    #[test]
    fn rust_unreachable_after_panic_macro() {
        let findings = check_unreachable(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n  panic!(\"x\");\n  println!(\"1\");\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("panic"));
    }

    #[test]
    fn rust_allows_reachable_code() {
        let findings = check_unreachable(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n  if true {\n    println!(\"1\");\n  }\n  println!(\"2\");\n}\n",
        );
        assert!(findings.is_empty());
    }

    /// The next-best thing to compile-time verification for the plain `&'static str`
    /// node-kind literals scattered through `lang_config`: tree-sitter core has no
    /// macro/codegen path that would let the compiler itself reject a typo'd kind name
    /// (that would require generating constants from each grammar crate's
    /// `node-types.json`, which none of them currently ship), but `Language::
    /// id_for_node_kind` returns 0 for any string that isn't a real *named* node kind for
    /// that grammar — so this test fails immediately, at the same "before it ships"
    /// point a compile error would, if any literal here or added later drifts from the
    /// real grammar.
    #[test]
    fn node_kind_literals_are_valid_for_their_grammar() {
        fn assert_valid(ts_lang: tree_sitter::Language, cfg: &LangRuleConfig) {
            let mut kinds: Vec<&str> = vec![cfg.if_kind, cfg.block_kind];
            kinds.extend(cfg.function_kinds);
            kinds.extend(cfg.nesting_kinds);
            kinds.extend(cfg.else_wrapper_kinds);
            kinds.extend(cfg.chain_kinds);
            kinds.extend(cfg.terminal_kinds);
            if let Some(t) = cfg.ternary_kind {
                kinds.push(t);
            }
            for kind in kinds {
                assert_ne!(
                    ts_lang.id_for_node_kind(kind, true),
                    0,
                    "`{kind}` is not a valid named node kind for the `{}` grammar",
                    cfg.name
                );
            }
        }

        assert_valid(tree_sitter_go::LANGUAGE.into(), &lang_config(Language::Go));
        assert_valid(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            &lang_config(Language::TypeScript),
        );
        assert_valid(
            tree_sitter_typescript::LANGUAGE_TSX.into(),
            &lang_config(Language::Tsx),
        );
        assert_valid(
            tree_sitter_javascript::LANGUAGE.into(),
            &lang_config(Language::JavaScript),
        );
        assert_valid(
            tree_sitter_python::LANGUAGE.into(),
            &lang_config(Language::Python),
        );
        assert_valid(
            tree_sitter_java::LANGUAGE.into(),
            &lang_config(Language::Java),
        );
        assert_valid(
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            &lang_config(Language::Kotlin),
        );
        assert_valid(
            tree_sitter_rust::LANGUAGE.into(),
            &lang_config(Language::Rust),
        );
    }

    /// Epic 1.0's equivalence probe (validation.md): confirms the dispatcher wiring
    /// survived splitting the old single `lang_config()` match arm into 8 named
    /// constructor functions.
    #[test]
    fn lang_config_dispatches_to_the_matching_constructor() {
        assert_eq!(lang_config(Language::Go).name, go_lang_config().name);
        assert_eq!(
            lang_config(Language::TypeScript).name,
            typescript_lang_config().name
        );
        assert_eq!(lang_config(Language::Tsx).name, tsx_lang_config().name);
        assert_eq!(
            lang_config(Language::JavaScript).name,
            javascript_lang_config().name
        );
        assert_eq!(
            lang_config(Language::Python).name,
            python_lang_config().name
        );
        assert_eq!(lang_config(Language::Java).name, java_lang_config().name);
        assert_eq!(
            lang_config(Language::Kotlin).name,
            kotlin_lang_config().name
        );
        assert_eq!(lang_config(Language::Rust).name, rust_lang_config().name);
    }

    /// Epic 1.0's equivalence probe (validation.md): catches a mistranslation of the
    /// `..lang_config(Language::TypeScript)` struct-update syntax into
    /// `..typescript_lang_config()` during Task 1.0.1a's extraction — e.g. a field
    /// accidentally hardcoded instead of inherited. Compares every data field (not the
    /// function-pointer fields: the compiler's own `unpredictable_function_pointer_
    /// comparisons` lint warns those addresses aren't reliably comparable across
    /// codegen units, so a derived/manual `PartialEq` on them would be a flaky test).
    #[test]
    fn tsx_and_javascript_configs_inherit_typescript_fields() {
        fn assert_data_fields_match(inherited: &LangRuleConfig, base: &LangRuleConfig) {
            assert_eq!(inherited.function_kinds, base.function_kinds);
            assert_eq!(inherited.if_kind, base.if_kind);
            assert_eq!(inherited.nesting_kinds, base.nesting_kinds);
            assert_eq!(inherited.else_wrapper_kinds, base.else_wrapper_kinds);
            assert_eq!(inherited.chain_kinds, base.chain_kinds);
            assert_eq!(inherited.ternary_kind, base.ternary_kind);
            assert_eq!(inherited.block_kind, base.block_kind);
            assert_eq!(inherited.terminal_kinds, base.terminal_kinds);
            assert_eq!(inherited.literal_kinds, base.literal_kinds);
            assert_eq!(inherited.numeric_literal_kinds, base.numeric_literal_kinds);
        }
        let ts = typescript_lang_config();
        assert_data_fields_match(&tsx_lang_config(), &ts);
        assert_data_fields_match(&javascript_lang_config(), &ts);
    }

    /// Task 1.3.1c: dedicated companion to `node_kind_literals_are_valid_for_their_grammar`
    /// covering `replace-magic-literal`'s two new per-language fields — kept as its own
    /// test function (rather than folded into that one) so neither grows past
    /// `long-function`'s own 40-line threshold, the exact violation Epic 1.0 exists to
    /// avoid reintroducing.
    #[test]
    fn replace_magic_literal_node_kinds_are_valid_for_their_grammar() {
        fn assert_literal_kinds_valid(ts_lang: tree_sitter::Language, cfg: &LangRuleConfig) {
            for kind in cfg.literal_kinds {
                assert_ne!(
                    ts_lang.id_for_node_kind(kind, true),
                    0,
                    "`{kind}` is not a valid named node kind for the `{}` grammar",
                    cfg.name
                );
            }
            for kind in cfg.numeric_literal_kinds {
                assert!(
                    cfg.literal_kinds.contains(kind),
                    "`{kind}` is in `{}`'s numeric_literal_kinds but missing from literal_kinds",
                    cfg.name
                );
            }
        }

        let pairs: [(tree_sitter::Language, Language); 8] = [
            (tree_sitter_go::LANGUAGE.into(), Language::Go),
            (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                Language::TypeScript,
            ),
            (tree_sitter_typescript::LANGUAGE_TSX.into(), Language::Tsx),
            (
                tree_sitter_javascript::LANGUAGE.into(),
                Language::JavaScript,
            ),
            (tree_sitter_python::LANGUAGE.into(), Language::Python),
            (tree_sitter_java::LANGUAGE.into(), Language::Java),
            (tree_sitter_kotlin_ng::LANGUAGE.into(), Language::Kotlin),
            (tree_sitter_rust::LANGUAGE.into(), Language::Rust),
        ];
        for (ts_lang, lang) in pairs {
            assert_literal_kinds_valid(ts_lang, &lang_config(lang));
        }
    }

    /// Companion to the test above: anonymous binding-keyword tokens each
    /// `binding_finder` checks directly via a raw-child scan (not through
    /// `literal_kinds`, which only covers named nodes) — split into its own test
    /// function for the same `long-function` reason. Go's `const_spec`, Rust's
    /// `const_item`/`static_item`, and Python's casing heuristic all check named node
    /// kinds already covered above, so only TS/JS's `const`, Java's `final`, and
    /// Kotlin's `val` need this separate lookup.
    #[test]
    fn replace_magic_literal_binding_tokens_are_valid_for_their_grammar() {
        fn assert_anonymous_token_valid(ts_lang: tree_sitter::Language, name: &str, token: &str) {
            assert_ne!(
                ts_lang.id_for_node_kind(token, false),
                0,
                "`{token}` is not a valid anonymous node kind for the `{name}` grammar"
            );
        }
        assert_anonymous_token_valid(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            "const",
        );
        assert_anonymous_token_valid(tree_sitter_java::LANGUAGE.into(), "java", "final");
        assert_anonymous_token_valid(tree_sitter_kotlin_ng::LANGUAGE.into(), "kotlin", "val");
    }

    /// Full-pipeline helper for `replace-magic-literal`: unlike `check_source`/
    /// `check_unreachable` above (which call one walk directly), this runs the whole
    /// `SyntaxRulesChecker::check()` pipeline — declarations + blocks + the literal
    /// pass — since this rule's only externally observable behavior is through
    /// `check()` itself (validation.md's Test Stack note).
    fn check_full(lang: Language, ts_lang: tree_sitter::Language, src: &str) -> Vec<Finding> {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&ts_lang).expect("loading grammar");
        let tree = parser.parse(src, None).expect("parsing source");
        let ctx = CheckContext {
            source: src,
            tree: Some(&tree),
        };
        SyntaxRulesChecker::new(lang)
            .check(Path::new("<source>"), &ctx)
            .expect("check() should not error")
    }

    fn magic_literal_findings(
        lang: Language,
        ts_lang: tree_sitter::Language,
        src: &str,
    ) -> Vec<Finding> {
        check_full(lang, ts_lang, src)
            .into_iter()
            .filter(|f| f.message.contains("[replace-magic-literal]"))
            .collect()
    }

    #[test]
    fn flags_replace_magic_literal() {
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f() {\n\ta := 86400\n\tb := 86400\n\tc := 86400\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("86400"));
        assert!(findings[0].message.contains("3 times"));
    }

    #[test]
    fn allows_replace_magic_literal_single_occurrence() {
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f() {\n\ta := 86400\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_replace_magic_literal_below_bumped_threshold() {
        // Locks in ADR-001's Story 3.2.2 outcome: the corpus backtest's 76%
        // false-positive rate (46% citing table-driven-test/fixture noise) crossed
        // the pre-committed 40% bar, so MAGIC_LITERAL_MIN_OCCURRENCES bumped from the
        // literal `2` in AC2's text to `3` — 2 occurrences alone must no longer fire.
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f() {\n\ta := 86400\n\tb := 86400\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn allows_replace_magic_literal_allowlisted_values() {
        // Each allowlisted value appears 3x (>= MAGIC_LITERAL_MIN_OCCURRENCES) so this
        // test actually discriminates: without the allow-list, each would fire.
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f() {\n\ta := 0\n\tb := 0\n\tc := 0\n\td := 1\n\te := 1\n\tg := 1\n\th := -1\n\ti := -1\n\tj := -1\n\tk := \"\"\n\tl := \"\"\n\tm := \"\"\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_replace_magic_literal_non_allowlisted_small_value() {
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f() {\n\ta := 2\n\tb := 2\n\tc := 2\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_replace_magic_literal_excludes_named_constant() {
        // Discriminating, not tautological: without the exclusion, "30" appears 3 times
        // (the const initializer plus 2 raw uses) which meets the bumped threshold and
        // would fire; the const initializer's exclusion drops the remaining count to 2,
        // below threshold, so this genuinely proves the exclusion logic runs.
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nconst Timeout = 30\nfunc f() { wait(Timeout) }\nfunc g() { sleep(30) }\nfunc h() { sleep(30) }\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_replace_magic_literal_does_not_exclude_destructured_binding() {
        // Documented gap (Unresolved Question 3): `go_const_binding` only handles a
        // single-name/single-value `const_spec` — this multi-name/multi-value spec
        // returns `None`, so this already-correctly-factored pair of constants is
        // still (incorrectly) flagged as a repeated magic literal.
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nconst a, b = 42, 42\nfunc f() { use(a); use(b); use2(42) }\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn flags_replace_magic_literal_skips_generated_file() {
        let body = "func f() {\n\ta := 1234\n\tb := 1234\n\tc := 1234\n}\n";
        let generated =
            format!("// Code generated by protoc-gen-go. DO NOT EDIT.\npackage main\n{body}");
        let hand_written = format!("package main\n{body}");

        assert!(
            magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), &generated)
                .is_empty()
        );
        assert_eq!(
            magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), &hand_written)
                .len(),
            1,
            "sanity check: the same body without the generated-file marker should fire"
        );
    }

    #[test]
    fn flags_replace_magic_literal_allowlist_empty_backtick_string() {
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f() {\n\ta := ``\n\tb := ``\n\tc := ``\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_replace_magic_literal_empty_file_produces_no_findings_and_does_not_panic() {
        let findings = magic_literal_findings(Language::Go, tree_sitter_go::LANGUAGE.into(), "");
        assert!(findings.is_empty());
    }

    #[test]
    fn flags_replace_magic_literal_no_literals_produces_no_findings() {
        let findings = magic_literal_findings(
            Language::Go,
            tree_sitter_go::LANGUAGE.into(),
            "package main\nfunc f(a, b int) int { return a + b }\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn ts_flags_replace_magic_literal() {
        let findings = magic_literal_findings(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "function f() {\n  console.log(42);\n  console.log(42);\n  console.log(42);\n}\n",
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("42"));
    }

    #[test]
    fn ts_flags_replace_magic_literal_excludes_const_not_let() {
        // Locks in ADR-001's deliberate narrowing: only `const` (not `let`) qualifies
        // for the named-constant exclusion, even though `x` is referenced once beyond
        // its own declaration. All 3 occurrences are `let`, so none should be excluded.
        let findings = magic_literal_findings(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "function f() {\n  let x = 42;\n  use(x);\n  let y = 42;\n  let z = 42;\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn ts_flags_replace_magic_literal_excludes_const_referenced_elsewhere() {
        // Companion to the `let`-negative-case test above: proves `const` actually IS
        // excluded, discriminating (not tautological) — without the exclusion, "7"
        // appears 3 times (meets the bumped threshold); with it, the const initializer
        // drops out, leaving 2, below threshold.
        let findings = magic_literal_findings(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "function f() {\n  const x = 7;\n  use(x);\n}\nfunction g() {\n  use2(7);\n}\nfunction h() {\n  use3(7);\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn ts_replace_magic_literal_ignores_predefined_type_keywords() {
        // Regression for a corpus-backtest finding (docs/backtest-triage/microsoft-vscode,
        // docs/backtest-triage/denoland-deno): TS/JS's grammar reuses the bare token
        // "string"/"number" for the anonymous keyword inside a `predefined_type` type
        // annotation, which collides with the *named* literal node kinds of the same
        // name — `walk_literals` was flagging `x: string` as if `"string"` were a
        // repeated string literal, with zero actual literals in the file.
        let findings = magic_literal_findings(
            Language::TypeScript,
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "function f(a: string, b: string): string {\n  return a + b;\n}\nfunction g(x: number, y: number): number {\n  return x + y;\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn py_flags_replace_magic_literal() {
        let findings = magic_literal_findings(
            Language::Python,
            tree_sitter_python::LANGUAGE.into(),
            "def f():\n    print(42)\n    print(42)\n    print(42)\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn py_flags_replace_magic_literal_excludes_screaming_snake_case() {
        // Discriminating: without the exclusion, "30" appears 3 times (meets the
        // bumped threshold); with it, the assignment's value drops out, leaving 2.
        let findings = magic_literal_findings(
            Language::Python,
            tree_sitter_python::LANGUAGE.into(),
            "TIMEOUT = 30\ndef f():\n    wait(TIMEOUT)\ndef g():\n    sleep(30)\ndef h():\n    sleep(30)\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn py_flags_replace_magic_literal_allowlist_prefixed_empty_string() {
        let findings = magic_literal_findings(
            Language::Python,
            tree_sitter_python::LANGUAGE.into(),
            "def f():\n    a = r\"\"\n    b = r\"\"\n    c = r\"\"\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn py_flags_replace_magic_literal_allowlist_u_prefixed_empty_string() {
        // Regression: Python's legacy (PEP 414) `u`/`U` prefix wasn't in
        // strip_python_prefixed_quote_delimiter's accepted character set, so `u""`
        // normalized to the 4-char string `u""` instead of an empty string, missing
        // the allow-list match — `u""` repeated would have been incorrectly flagged.
        let findings = magic_literal_findings(
            Language::Python,
            tree_sitter_python::LANGUAGE.into(),
            "def f():\n    a = u\"\"\n    b = u\"\"\n    c = u\"\"\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn java_flags_replace_magic_literal() {
        let findings = magic_literal_findings(
            Language::Java,
            tree_sitter_java::LANGUAGE.into(),
            "class C {\n  void f() {\n    System.out.println(42);\n    System.out.println(42);\n    System.out.println(42);\n  }\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn java_flags_replace_magic_literal_excludes_final() {
        // Discriminating: without the exclusion, "30" appears 3 times (meets the
        // bumped threshold); with it, the final field's initializer drops out.
        let findings = magic_literal_findings(
            Language::Java,
            tree_sitter_java::LANGUAGE.into(),
            "class C {\n  void f() {\n    final int Timeout = 30;\n    use(Timeout);\n  }\n  void g() {\n    sleep(30);\n  }\n  void h() {\n    sleep(30);\n  }\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn kotlin_flags_replace_magic_literal() {
        let findings = magic_literal_findings(
            Language::Kotlin,
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            "fun f() {\n  println(42)\n  println(42)\n  println(42)\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn kotlin_flags_replace_magic_literal_excludes_val() {
        // Discriminating: without the exclusion, "30" appears 3 times (meets the
        // bumped threshold); with it, the `val`'s initializer drops out.
        let findings = magic_literal_findings(
            Language::Kotlin,
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            "fun f() {\n  val Timeout = 30\n  use(Timeout)\n}\nfun g() {\n  sleep(30)\n}\nfun h() {\n  sleep(30)\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn kotlin_flags_replace_magic_literal_allowlist_empty_multiline_string() {
        let findings = magic_literal_findings(
            Language::Kotlin,
            tree_sitter_kotlin_ng::LANGUAGE.into(),
            "fun f() {\n  val a = \"\"\"\"\"\"\n  val b = \"\"\"\"\"\"\n  val c = \"\"\"\"\"\"\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn rust_flags_replace_magic_literal() {
        let findings = magic_literal_findings(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n    use(42);\n    use(42);\n    use(42);\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn rust_flags_replace_magic_literal_excludes_const_not_let() {
        // All 3 occurrences are `let`, so none should be excluded (locks in ADR-001's
        // narrowing: only `const`/`static`, not `let`, qualifies).
        let findings = magic_literal_findings(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n    let x = 42;\n    use(x);\n    let y = 42;\n    let z = 42;\n}\n",
        );
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn rust_flags_replace_magic_literal_excludes_const_referenced_elsewhere() {
        // Companion to the `let`-negative-case test above: proves `const` actually IS
        // excluded, discriminating (not tautological) — without the exclusion, "7"
        // appears 3 times (meets the bumped threshold); with it, the const's
        // initializer drops out, leaving 2, below threshold.
        let findings = magic_literal_findings(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n    const X: i32 = 7;\n    use(X);\n}\nfn g() {\n    use2(7);\n}\nfn h() {\n    use3(7);\n}\n",
        );
        assert!(findings.is_empty());
    }

    #[test]
    fn rust_flags_replace_magic_literal_allowlist_empty_raw_string() {
        let findings = magic_literal_findings(
            Language::Rust,
            tree_sitter_rust::LANGUAGE.into(),
            "fn f() {\n    let a = r\"\";\n    let b = r\"\";\n    let c = r\"\";\n}\n",
        );
        assert!(findings.is_empty());
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
    fn long_function_message_cites_extract_function() {
        let mut src = String::from("package main\nfunc f() {\n");
        for _ in 0..45 {
            src.push_str("\tprintln(\"line\")\n");
        }
        src.push_str("}\n");
        let findings = check_source(&src).unwrap();
        assert!(findings.iter().any(|f| {
            f.message.contains("Extract Function")
                && f.message
                    .contains("https://refactoring.com/catalog/extractFunction.html")
        }));
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
    fn deep_nesting_message_cites_guard_clauses() {
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
        assert!(findings.iter().any(|f| f.message.contains("Guard Clauses")
            && f.message.contains(
                "https://refactoring.com/catalog/replaceNestedConditionalWithGuardClauses.html"
            )));
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

    #[test]
    fn catalog_cites_fowler_for_long_function_and_deep_nesting() {
        let long_function = CATALOG.iter().find(|r| r.id == "long-function").unwrap();
        assert!(long_function.description.contains("Extract Function"));

        let deep_nesting = CATALOG.iter().find(|r| r.id == "deep-nesting").unwrap();
        assert!(deep_nesting.description.contains("Guard Clauses"));
    }

    #[test]
    fn flags_bool_param_branched_on_directly() {
        let findings = check_source(
            "package main\nfunc f(verbose bool) {\n\tif verbose {\n\t\tprintln(\"v\")\n\t}\n}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`verbose`"))
        );
    }

    #[test]
    fn grouped_bool_param_names_are_each_checked() {
        let findings =
            check_source("package main\nfunc f(a, ok bool) {\n\tif ok {\n\t\tprintln(a)\n\t}\n}\n")
                .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`ok`"))
        );
    }

    #[test]
    fn does_not_flag_bool_param_never_branched_on() {
        let findings =
            check_source("package main\nfunc f(verbose bool) {\n\tprintln(verbose)\n}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }

    #[test]
    fn does_not_flag_non_bool_param_branched_on() {
        let findings = check_source(
            "package main\nfunc f(count int) {\n\tif count > 0 {\n\t\tprintln(count)\n\t}\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
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
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
        Ok(findings)
    }

    #[test]
    fn ts_flags_bool_param_branched_on_directly() {
        let findings = check_ts_source(
            "function f(verbose: boolean) {\n  if (verbose) {\n    console.log(\"v\");\n  }\n}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`verbose`"))
        );
    }

    #[test]
    fn ts_flags_bool_param_branched_on_via_ternary() {
        let findings =
            check_ts_source("function f(verbose: boolean) {\n  return verbose ? 1 : 2;\n}\n")
                .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }

    #[test]
    fn ts_does_not_flag_bool_param_only_forwarded() {
        let findings =
            check_ts_source("function f(verbose: boolean) {\n  g(verbose);\n}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
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
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
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
    fn js_untyped_param_is_never_flagged_as_flag_argument() {
        // JS has no static types to check, so `verbose` can't be identified as boolean —
        // this documents that gap rather than guessing from the name.
        let findings = check_js_source(
            "function f(verbose) {\n  if (verbose) {\n    console.log(\"v\");\n  }\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }

    #[test]
    fn checker_names_are_distinct_per_language() {
        // Derives from `Language::ALL` rather than its own hand-rolled list: this test
        // used to omit `Rust`, silently leaving `syntax-rules-rust`'s name unchecked —
        // the same class of "forgot to update a hand-rolled language list" gap
        // `Language::extensions`'s doc comment documents finding elsewhere in this
        // codebase (a real missed-Rust bug in `arch_model.rs`).
        let names: Vec<&str> = Language::ALL
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
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
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

    #[test]
    fn py_flags_bool_param_branched_on_directly() {
        let findings =
            check_py_source("def f(verbose: bool):\n    if verbose:\n        print(\"v\")\n")
                .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`verbose`"))
        );
    }

    #[test]
    fn py_does_not_flag_untyped_bool_looking_param() {
        // No type annotation means the param's runtime type can't be verified — must
        // not be guessed from usage alone.
        let findings =
            check_py_source("def f(verbose):\n    if verbose:\n        print(\"v\")\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }

    fn check_java_source(src: &str) -> Result<Vec<Finding>> {
        let tree = crate::test_support::parse_java(src)?;
        let cfg = lang_config(Language::Java);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
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

    #[test]
    fn java_flags_bool_param_branched_on_directly() {
        let findings = check_java_source(
            "class C {\n    void f(boolean verbose) {\n        if (verbose) {\n            g();\n        }\n    }\n}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`verbose`"))
        );
    }

    #[test]
    fn java_flags_bool_param_branched_on_via_ternary() {
        let findings = check_java_source(
            "class C {\n    int f(boolean verbose) {\n        return verbose ? 1 : 2;\n    }\n}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }

    #[test]
    fn java_does_not_flag_non_bool_param() {
        let findings = check_java_source(
            "class C {\n    void f(String name) {\n        if (name != null) {\n            g();\n        }\n    }\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
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
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
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

    #[test]
    fn kotlin_flags_bool_param_branched_on_directly() {
        let findings = check_kotlin_source(
            "fun f(verbose: Boolean) {\n    if (verbose) {\n        g()\n    }\n}\n",
        )
        .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`verbose`"))
        );
    }

    #[test]
    fn kotlin_does_not_flag_non_bool_param() {
        let findings = check_kotlin_source(
            "fun f(name: String) {\n    if (name != \"\") {\n        g()\n    }\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }

    fn check_rust_source(src: &str) -> Result<Vec<Finding>> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .context("loading tree-sitter-rust grammar")?;
        let tree = parser
            .parse(src, None)
            .context("parsing Rust source with tree-sitter")?;

        let cfg = lang_config(Language::Rust);
        let mut findings = Vec::new();
        walk_declarations(tree.root_node(), &cfg, src.as_bytes(), &mut findings);
        Ok(findings)
    }

    #[test]
    fn rust_allows_short_function() {
        let findings = check_rust_source("fn f() {\n    println!(\"ok\");\n}\n").unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn rust_flags_long_function() {
        let mut src = String::from("fn f() {\n");
        for _ in 0..45 {
            src.push_str("    do_thing();\n");
        }
        src.push_str("}\n");
        let findings = check_rust_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn rust_flags_deep_nesting() {
        let src = "fn f(x: i32) {\n    if x > 0 {\n        if x > 1 {\n            if x > 2 {\n                if x > 3 {\n                    do_thing();\n                }\n            }\n        }\n    }\n}\n";
        let findings = check_rust_source(src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]"))
        );
    }

    #[test]
    fn rust_flat_else_if_chain_does_not_count_as_nesting() {
        let src = "fn f(x: i32) {\n    if x == 1 {\n        a();\n    } else if x == 2 {\n        b();\n    } else if x == 3 {\n        c();\n    } else if x == 4 {\n        d();\n    } else {\n        e();\n    }\n}\n";
        let findings = check_rust_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[deep-nesting]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_flags_long_parameter_list() {
        let findings =
            check_rust_source("fn f(a: i32, b: i32, c: i32, d: i32, e: i32, g: i32) {}\n").unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]"))
        );
    }

    #[test]
    fn rust_self_parameter_does_not_count_toward_parameter_list() {
        // `&self` is a `self_parameter` node, not a `parameter` — must not be counted,
        // the same way Go's receiver is never part of its parameter list at all.
        let findings = check_rust_source(
            "struct S;\nimpl S {\n    fn m(&self, a: i32, b: i32, c: i32, d: i32) {}\n}\n",
        )
        .unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_checks_impl_methods_too() {
        let mut src = String::from("struct S;\nimpl S {\n    fn m(&self) {\n");
        for _ in 0..45 {
            src.push_str("        do_thing();\n");
        }
        src.push_str("    }\n}\n");
        let findings = check_rust_source(&src).unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[long-function]"))
        );
    }

    #[test]
    fn rust_closure_is_nesting_but_not_a_separate_function_kind() {
        // A closure body deep enough to trip deep-nesting on its own counts toward the
        // enclosing function's nesting depth (closure_expression is nesting-only, like
        // Go's func_literal) but never produces its own long-function/long-parameter-list
        // finding, since it isn't in `function_kinds`.
        let src =
            "fn f() {\n    let g = |a: i32, b: i32, c: i32, d: i32, e: i32, h: i32| a + b;\n}\n";
        let findings = check_rust_source(src).unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[long-parameter-list]")),
            "findings: {findings:?}"
        );
    }

    #[test]
    fn rust_flags_bool_param_branched_on_directly() {
        let findings =
            check_rust_source("fn f(verbose: bool) {\n    if verbose {\n        g();\n    }\n}\n")
                .unwrap();
        assert!(
            findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]") && f.message.contains("`verbose`"))
        );
    }

    #[test]
    fn rust_does_not_flag_bool_param_only_forwarded() {
        let findings = check_rust_source("fn f(verbose: bool) {\n    g(verbose);\n}\n").unwrap();
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("[flag-argument]"))
        );
    }
}
