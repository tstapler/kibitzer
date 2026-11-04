use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::checkers::duplicate_cross_file_checker::{
    discover_repo_root, index_path_in, save_json_atomic,
};
use crate::checkers::file_size::is_generated;
use crate::checkers::is_test_file;
use crate::node_kind::GoKind;

/// Types that are nil-checked everywhere by Go convention; a null-object for them is
/// never the right refactoring.
const EXCLUDED_TYPES: &[&str] = &[
    "error",
    "context.Context",
    "any",
    "string",
    "bool",
    "int",
    "int8",
    "int16",
    "int32",
    "int64",
    "uint",
    "uint8",
    "uint16",
    "uint32",
    "uint64",
    "uintptr",
    "float32",
    "float64",
    "byte",
    "rune",
];

const DEFAULT_MIN_OCCURRENCES: usize = 3;
const MIN_DISTINCT_FILES: usize = 2;

/// Candidate surfacer for Fowler's Introduce Special Case: counts, per static type, the
/// `if x == nil { ... } else { x.M() }` shape across the whole repo via a persistent
/// per-repo index (the #28 `duplicate-code-cross-file` index plumbing, reused). Type
/// resolution is syntactic (parameters, receivers, `var` declarations, `x := &T{}`), so
/// a value whose type can't be read from its own function is not counted.
pub struct IntroduceSpecialCaseChecker {
    min_occurrences: usize,
    exclude_tests: bool,
}

impl Default for IntroduceSpecialCaseChecker {
    fn default() -> Self {
        Self {
            min_occurrences: DEFAULT_MIN_OCCURRENCES,
            exclude_tests: true,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Options {
    min_occurrences: Option<usize>,
    exclude_tests: Option<bool>,
}

impl Checker for IntroduceSpecialCaseChecker {
    fn name(&self) -> &str {
        "introduce-special-case"
    }

    fn description(&self) -> &str {
        "flags a type whose nil-check-then-branch pattern repeats across the repo — an Introduce Special Case candidate"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn reads_cross_run_state(&self) -> bool {
        true
    }

    fn configure(&self, options: &serde_json::Value) -> Result<Option<Box<dyn Checker>>> {
        let opts: Options = serde_json::from_value(options.clone())
            .context("introduce-special-case options")?;
        let min = opts.min_occurrences.unwrap_or(self.min_occurrences);
        if min < MIN_DISTINCT_FILES {
            bail!("introduce-special-case: min_occurrences must be at least {MIN_DISTINCT_FILES}, got {min}");
        }
        Ok(Some(Box::new(Self {
            min_occurrences: min,
            exclude_tests: opts.exclude_tests.unwrap_or(self.exclude_tests),
        })))
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let repo_root = discover_repo_root(file);
        // Judged below the repo root, so a checkout living under e.g. `~/test/` isn't all "tests".
        let in_repo = file.strip_prefix(&repo_root).unwrap_or(file);
        if (self.exclude_tests && is_test_file(in_repo)) || is_generated(ctx.source) {
            return Ok(Vec::new());
        }
        let tree = ctx
            .tree
            .context("introduce-special-case checker requires a parsed tree")?;
        let checks = extract_nil_checks(tree.root_node(), ctx.source.as_bytes());

        let index_path = index_path_in(&special_case_index_dir(), &repo_root);
        let mut index = SpecialCaseIndex::load(&index_path);
        let was_indexed = index.files.contains_key(file.to_string_lossy().as_ref());
        let findings = index.reindex_and_find(file, &checks, self.min_occurrences);
        // Hook latency: skip rewriting the whole index for a file that never had entries.
        if was_indexed || !checks.is_empty() {
            index
                .save(&index_path)
                .with_context(|| format!("writing special-case index {}", index_path.display()))?;
        }
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(IntroduceSpecialCaseChecker::default())])
}

/// Redirects the index (see `backtest::IndexIsolation`).
const INDEX_DIR_OVERRIDE_ENV: &str = "KIBITZER_SPECIAL_CASE_INDEX_DIR";

fn special_case_index_dir() -> PathBuf {
    if let Ok(dir) = std::env::var(INDEX_DIR_OVERRIDE_ENV) {
        return PathBuf::from(dir);
    }
    crate::cache::default_cache_path()
        .parent()
        .map(|dir| dir.join("special-case-index"))
        .unwrap_or_else(|| std::env::temp_dir().join("kibitzer-special-case-index"))
}

#[derive(Debug, PartialEq, Eq)]
struct NilCheck {
    type_name: String,
    /// 1-indexed.
    line: usize,
}

/// Same shape and failure semantics as `duplicate_cross_file_checker::DuplicateIndex`:
/// last-writer-wins on concurrent saves, stale entries for deleted files are not pruned.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SpecialCaseIndex {
    occurrences: HashMap<String, Vec<(String, usize)>>,
    files: HashMap<String, Vec<String>>,
}

impl SpecialCaseIndex {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) -> Result<()> {
        save_json_atomic(self, path)
    }

    fn remove_file(&mut self, file_key: &str) {
        let Some(types) = self.files.remove(file_key) else {
            return;
        };
        for ty in types {
            if let Some(list) = self.occurrences.get_mut(&ty) {
                list.retain(|(f, _)| f != file_key);
                if list.is_empty() {
                    self.occurrences.remove(&ty);
                }
            }
        }
    }

    fn reindex_and_find(&mut self, file: &Path, checks: &[NilCheck], min: usize) -> Vec<Finding> {
        let file_key = file.to_string_lossy().into_owned();
        self.remove_file(&file_key);
        let mut types = Vec::new();
        for c in checks {
            self.occurrences
                .entry(c.type_name.clone())
                .or_default()
                .push((file_key.clone(), c.line));
            types.push(c.type_name.clone());
        }
        if !types.is_empty() {
            self.files.insert(file_key, types);
        }

        let mut findings = Vec::new();
        for c in checks {
            let all = &self.occurrences[&c.type_name];
            let distinct: HashSet<&str> = all.iter().map(|(f, _)| f.as_str()).collect();
            if all.len() < min || distinct.len() < MIN_DISTINCT_FILES {
                continue;
            }
            findings.push(Finding {
                line: c.line,
                message: format!(
                    "`{}` is nil-checked-then-branched {} times across {} files — repeated \
                     nil-checks for one type are an Introduce Special Case candidate (a null \
                     object callers needn't check)",
                    c.type_name,
                    all.len(),
                    distinct.len()
                ),
            });
        }
        findings
    }
}

type Env = HashMap<String, String>;

/// Every qualifying nil-check in the file, typed by the enclosing function's own
/// declarations. Qualifies when the `if` has an `else`, or is `x != nil` whose body calls
/// a method on `x` — a bare guard-clause `if x == nil { return }` is not this smell.
fn extract_nil_checks(root: Node, src: &[u8]) -> Vec<NilCheck> {
    let mut out = Vec::new();
    walk(root, src, &Env::new(), &mut out);
    out.sort_by_key(|c| c.line);
    out
}

/// Scoped like Go: a declaration is visible from its statement to the end of its block, a
/// nested function sees the enclosing scope's names, and a name the checker cannot type
/// (`a, err := f()`) clears any type it had, so a shadowed `err` is never mistaken for an
/// outer typed variable.
fn walk(node: Node, src: &[u8], env: &Env, out: &mut Vec<NilCheck>) {
    match GoKind::of(node) {
        GoKind::FunctionDeclaration | GoKind::MethodDeclaration | GoKind::FuncLiteral => {
            let mut scope = env.clone();
            // The receiver stays untyped: `if r != nil` inside a type's own method is a
            // nil-safe receiver, with no caller to simplify.
            if let Some(list) = node.child_by_field_name("receiver") {
                shadow_receiver(list, src, &mut scope);
            }
            if let Some(list) = node.child_by_field_name("parameters") {
                declare_params(list, src, &mut scope);
            }
            if let Some(body) = node.child_by_field_name("body") {
                walk(body, src, &scope, out);
            }
        }
        // The grammar wraps a block's (and a case clause's) statements in a statement list.
        GoKind::Block | GoKind::StatementList => {
            let mut scope = env.clone();
            let mut cursor = node.walk();
            for stmt in node.children(&mut cursor) {
                declare_stmt(stmt, src, &mut scope);
                walk(stmt, src, &scope, out);
            }
        }
        GoKind::IfStatement => {
            let mut scope = env.clone();
            if let Some(init) = node.child_by_field_name("initializer") {
                declare_stmt(init, src, &mut scope);
            }
            if let Some(check) = nil_check(node, src, &scope) {
                out.push(check);
            }
            walk_children(node, src, &scope, out);
        }
        _ => walk_children(node, src, env, out),
    }
}

fn walk_children(node: Node, src: &[u8], env: &Env, out: &mut Vec<NilCheck>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, src, env, out);
    }
}

/// Binds `name` to `ty`, or clears it when the new declaration's type is unknown.
fn bind(env: &mut Env, name: String, ty: Option<String>) {
    match ty {
        Some(ty) => {
            env.insert(name, ty);
        }
        None => {
            env.remove(&name);
        }
    }
}

/// A parameter or `var` spec: `name... Type`.
fn declare_typed(node: Node, src: &[u8], env: &mut Env) {
    let ty = node
        .child_by_field_name("type")
        .and_then(|t| normalize_type(t, src));
    let mut cursor = node.walk();
    for name in node.children_by_field_name("name", &mut cursor) {
        bind(env, text(name, src), ty.clone());
    }
}

fn declare_params(list: Node, src: &[u8], env: &mut Env) {
    let mut cursor = list.walk();
    for param in list.children(&mut cursor) {
        if GoKind::of(param) == GoKind::ParameterDeclaration {
            declare_typed(param, src, env);
        }
    }
}

/// Shadows the receiver's name without typing it.
fn shadow_receiver(list: Node, src: &[u8], env: &mut Env) {
    let mut cursor = list.walk();
    for param in list.children(&mut cursor) {
        let mut names = param.walk();
        for name in param.children_by_field_name("name", &mut names) {
            bind(env, text(name, src), None);
        }
    }
}

/// `x := &T{}` types `x`; every other `:=` (including `a, err := f()`) leaves its names untyped.
fn declare_short_var(node: Node, src: &[u8], env: &mut Env) {
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    let (mut lc, mut rc) = (left.walk(), right.walk());
    let names: Vec<Node> = left.named_children(&mut lc).collect();
    let values: Vec<Node> = right.named_children(&mut rc).collect();
    for (i, name) in names.iter().enumerate() {
        if GoKind::of(*name) != GoKind::Identifier {
            continue;
        }
        let ty = if names.len() == values.len() {
            literal_type(values[i], src)
        } else {
            None
        };
        bind(env, text(*name, src), ty);
    }
}

/// Applies the declaration (if any) that `stmt` makes to the scope that follows it.
fn declare_stmt(stmt: Node, src: &[u8], env: &mut Env) {
    match GoKind::of(stmt) {
        GoKind::ShortVarDeclaration => declare_short_var(stmt, src, env),
        GoKind::VarDeclaration | GoKind::VarSpecList => {
            let mut cursor = stmt.walk();
            for child in stmt.children(&mut cursor) {
                match GoKind::of(child) {
                    GoKind::VarSpec => declare_typed(child, src, env),
                    GoKind::VarSpecList => declare_stmt(child, src, env),
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// `&T{...}` / `T{...}` -> `T`.
fn literal_type(value: Node, src: &[u8]) -> Option<String> {
    let lit = if GoKind::of(value) == GoKind::UnaryExpression {
        value.child_by_field_name("operand")?
    } else {
        value
    };
    if GoKind::of(lit) != GoKind::CompositeLiteral {
        return None;
    }
    normalize_type(lit.child_by_field_name("type")?, src)
}

fn normalize_type(node: Node, src: &[u8]) -> Option<String> {
    let raw = text(node, src);
    let ty = raw.trim_start_matches('*');
    let simple = ty
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        && !ty.is_empty();
    (simple && !EXCLUDED_TYPES.contains(&ty)).then(|| ty.to_string())
}

fn nil_check(if_stmt: Node, src: &[u8], env: &Env) -> Option<NilCheck> {
    let cond = if_stmt.child_by_field_name("condition")?;
    if GoKind::of(cond) != GoKind::BinaryExpression {
        return None;
    }
    let op = text(cond.child_by_field_name("operator")?, src);
    if op != "==" && op != "!=" {
        return None;
    }
    let (l, r) = (
        cond.child_by_field_name("left")?,
        cond.child_by_field_name("right")?,
    );
    let ident = match (GoKind::of(l), GoKind::of(r)) {
        (GoKind::Identifier, GoKind::Nil) => l,
        (GoKind::Nil, GoKind::Identifier) => r,
        _ => return None,
    };
    let name = text(ident, src);
    let qualifies = if_stmt.child_by_field_name("alternative").is_some()
        || (op == "!="
            && if_stmt
                .child_by_field_name("consequence")
                .is_some_and(|body| calls_method_on(body, &name, src)));
    if !qualifies {
        return None;
    }
    Some(NilCheck {
        type_name: env.get(&name)?.clone(),
        line: if_stmt.start_position().row + 1,
    })
}

/// True when `name.Method(...)` is called anywhere under `node`; reading a field
/// (`name.Field`) does not count.
fn calls_method_on(node: Node, name: &str, src: &[u8]) -> bool {
    if GoKind::of(node) == GoKind::CallExpression
        && let Some(function) = node.child_by_field_name("function")
        && GoKind::of(function) == GoKind::SelectorExpression
        && function
            .child_by_field_name("operand")
            .is_some_and(|o| text(o, src) == name)
    {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|c| calls_method_on(c, name, src))
}

fn text(node: Node, src: &[u8]) -> String {
    String::from_utf8_lossy(&src[node.start_byte()..node.end_byte()]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks(src: &str) -> Vec<NilCheck> {
        let tree = crate::test_support::parse_go(src).unwrap();
        extract_nil_checks(tree.root_node(), src.as_bytes())
    }

    fn types(src: &str) -> Vec<String> {
        checks(src).into_iter().map(|c| c.type_name).collect()
    }

    #[test]
    fn eq_nil_with_else_resolves_param_type() {
        let src = "package p\nfunc f(svc *Svc) string {\n\tif svc == nil {\n\t\treturn \"\"\n\t} else {\n\t\treturn svc.Name()\n\t}\n}\n";
        assert_eq!(types(src), ["Svc"]);
    }

    #[test]
    fn ne_nil_with_method_call_and_no_else_counts() {
        let src = "package p\nfunc f(svc *Svc) {\n\tif svc != nil {\n\t\tsvc.Run()\n\t}\n}\n";
        assert_eq!(types(src), ["Svc"]);
    }

    #[test]
    fn bare_guard_clause_is_not_counted() {
        let src = "package p\nfunc f(svc *Svc) {\n\tif svc == nil {\n\t\treturn\n\t}\n\tsvc.Run()\n}\n";
        assert!(types(src).is_empty());
    }

    #[test]
    fn error_and_context_are_excluded() {
        let src = "package p\nfunc f(err error, ctx context.Context) {\n\tif err != nil {\n\t\terr.Error()\n\t} else {\n\t}\n\tif ctx == nil {\n\t} else {\n\t}\n}\n";
        assert!(types(src).is_empty());
    }

    #[test]
    fn unresolved_identifier_is_skipped() {
        let src = "package p\nfunc f() {\n\tif mystery == nil {\n\t} else {\n\t}\n}\n";
        assert!(types(src).is_empty());
    }

    #[test]
    fn var_and_literal_declarations_resolve_but_receiver_does_not() {
        let src = "package p\nfunc (r *Repo) f() {\n\tvar a *Alpha\n\tb := &Beta{}\n\tif r != nil {\n\t\tr.x()\n\t}\n\tif a == nil {\n\t} else {\n\t}\n\tif b == nil {\n\t} else {\n\t}\n}\n";
        assert_eq!(types(src), ["Alpha", "Beta"]);
    }

    #[test]
    fn field_read_is_not_a_use_but_method_call_is() {
        let field = "package p\nfunc f(p *Pod) {\n\tif p != nil {\n\t\tx = p.Name\n\t}\n}\n";
        assert!(types(field).is_empty());
        let call = "package p\nfunc f(p *Pod) {\n\tif p != nil {\n\t\tx = p.Name()\n\t}\n}\n";
        assert_eq!(types(call), ["Pod"]);
    }

    #[test]
    fn untyped_redeclaration_shadows_an_outer_typed_name() {
        // `err` is an untyped `:=` result in the outer block; the typed `var err` sits in a
        // later, nested block and must not leak backwards.
        let src = "package p\nfunc f() {\n\ta, err := g()\n\tif err != nil {\n\t\th()\n\t} else {\n\t}\n\t{\n\t\tvar err *StatusError\n\t\tif err != nil {\n\t\t\terr.Error()\n\t\t}\n\t}\n}\n";
        assert_eq!(types(src), ["StatusError"]);
    }

    #[test]
    fn typed_parameter_is_shadowed_by_an_untyped_short_declaration() {
        let src = "package p\nfunc f(svc *Svc) {\n\tsvc, ok := lookup()\n\tif svc == nil {\n\t} else {\n\t}\n}\n";
        assert!(types(src).is_empty());
    }

    #[test]
    fn threshold_boundary_and_distinct_files() {
        let c = |line| NilCheck { type_name: "T".into(), line };
        let mut idx = SpecialCaseIndex::default();
        assert!(idx.reindex_and_find(Path::new("a.go"), &[c(1)], 3).is_empty());
        assert!(idx.reindex_and_find(Path::new("b.go"), &[c(1)], 3).is_empty());
        let f = idx.reindex_and_find(Path::new("c.go"), &[c(7)], 3);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 7);
        assert!(f[0].message.contains("`T`"));
        assert!(f[0].message.contains("3 times"));
        assert!(f[0].message.contains("Introduce Special Case"));
    }

    #[test]
    fn three_occurrences_in_one_file_do_not_fire() {
        let c = |line| NilCheck { type_name: "T".into(), line };
        let mut idx = SpecialCaseIndex::default();
        assert!(idx.reindex_and_find(Path::new("a.go"), &[c(1), c(2), c(3)], 3).is_empty());
    }

    #[test]
    fn re_indexing_a_file_replaces_its_entries() {
        let c = |line| NilCheck { type_name: "T".into(), line };
        let mut idx = SpecialCaseIndex::default();
        idx.reindex_and_find(Path::new("a.go"), &[c(1)], 3);
        idx.reindex_and_find(Path::new("a.go"), &[c(2)], 3);
        assert_eq!(idx.occurrences["T"], vec![("a.go".to_string(), 2)]);
    }

    #[test]
    fn configure_sets_threshold_and_rejects_malformed() {
        let base = IntroduceSpecialCaseChecker::default();
        assert!(base.configure(&serde_json::json!({"min_occurrences": 5})).unwrap().is_some());
        assert!(base.configure(&serde_json::json!({"min_occurrences": "x"})).is_err());
        assert!(base.configure(&serde_json::json!({"min_occurrences": 1})).is_err());
        assert!(base.configure(&serde_json::json!({"bogus": 1})).is_err());
    }

    #[test]
    fn registered_and_reads_cross_run_state() {
        assert!(crate::checker::reads_cross_run_state("introduce-special-case"));
        assert!(IntroduceSpecialCaseChecker::default().reads_cross_run_state());
    }

    #[test]
    fn default_scope_excludes_top_level_and_nested_vendor_dirs() {
        let check = crate::config::default_checks()
            .into_iter()
            .find(|c| c.name == "introduce-special-case")
            .unwrap();
        let in_scope = |p: &str| crate::glob::matches_scope(p, &check.scope);
        assert!(in_scope("pkg/a.go"));
        assert!(!in_scope("vendor/x/a.go"));
        assert!(!in_scope("third/vendor/x/a.go"));
    }

    #[test]
    #[allow(non_snake_case)]
    fn check_should_StillFire_When_RepoRootSitsUnderATestNamedDir() {
        let _isolation = crate::backtest::IndexIsolation::new();
        let base = std::env::temp_dir().join(format!("kibitzer-isc-{}", std::process::id()));
        let repo = base.join("test").join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/introduce-special-case/fires");
        let checker = IntroduceSpecialCaseChecker::default();
        let mut last = Vec::new();
        for name in ["a.go", "b.go", "c.go"] {
            let source = std::fs::read_to_string(fixtures.join(name)).unwrap();
            let tree = crate::test_support::parse_go(&source).unwrap();
            let ctx = CheckContext { source: &source, tree: Some(&tree) };
            last = checker.check(&repo.join(name), &ctx).unwrap();
        }
        let _ = std::fs::remove_dir_all(&base);
        assert!(!last.is_empty(), "a repo under a `test/` parent must not be treated as tests");
    }

    fn run_fixture(dir: &str, checker: &IntroduceSpecialCaseChecker) -> Vec<Vec<Finding>> {
        let _isolation = crate::backtest::IndexIsolation::new();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/introduce-special-case").join(dir);
        let mut files: Vec<_> = std::fs::read_dir(root).unwrap().map(|e| e.unwrap().path()).collect();
        files.sort();
        files
            .iter()
            .map(|p| {
                let source = std::fs::read_to_string(p).unwrap();
                let tree = crate::test_support::parse_go(&source).unwrap();
                let ctx = CheckContext { source: &source, tree: Some(&tree) };
                checker.check(p, &ctx).unwrap()
            })
            .collect()
    }

    #[test]
    fn fixture_fires_on_third_file() {
        let r = run_fixture("fires", &IntroduceSpecialCaseChecker::default());
        assert!(r[0].is_empty() && r[1].is_empty());
        assert_eq!(r[2].len(), 1, "{r:?}");
        assert!(r[2][0].message.contains("OrderService"));
    }

    #[test]
    fn fixture_below_threshold_is_silent() {
        let r = run_fixture("below-threshold", &IntroduceSpecialCaseChecker::default());
        assert!(r.iter().all(Vec::is_empty), "{r:?}");
    }

    #[test]
    fn fixture_distinct_types_is_silent() {
        let r = run_fixture("distinct-types", &IntroduceSpecialCaseChecker::default());
        assert!(r.iter().all(Vec::is_empty), "{r:?}");
    }

    #[test]
    fn raising_min_occurrences_silences_the_fires_fixture() {
        let strict = IntroduceSpecialCaseChecker { min_occurrences: 5, exclude_tests: true };
        let r = run_fixture("fires", &strict);
        assert!(r.iter().all(Vec::is_empty), "{r:?}");
    }
}
