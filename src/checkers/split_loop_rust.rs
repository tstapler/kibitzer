//! `split-loop-rust`: Fowler's Split Loop. Flags a loop body advancing 2+ accumulators
//! (`x op= e`, `x = x <op> e`) that do not feed each other; precision over recall, so loops with
//! early exits, ambiguous accumulator uses or tally-only counters are skipped.
//! Rules and backtest: docs/split-loop-rust-false-positives.md.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use super::rust_common::{in_test_code, is_test_path, text};
use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::checkers::file_size::is_generated;
use crate::tree_walk::walk_preorder;
use crate::union_find;

/// Bounds the quadratic `accounted` scan; larger loops are skipped, not flagged.
const MAX_LOOP_BODY_NODES: usize = 5000;
const TUPLE_PREFIX: &str = "#tuple@";

pub struct SplitLoopRustChecker;

impl Checker for SplitLoopRustChecker {
    fn name(&self) -> &str {
        "split-loop-rust"
    }

    fn description(&self) -> &str {
        "flags a Rust loop that advances independent accumulators in one body — Fowler's Split Loop"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Rust)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.rs"]
    }

    fn check(&self, file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if is_generated(ctx.source) || is_test_path(file) {
            return Ok(Vec::new());
        }
        let tree = ctx.tree.context("split-loop-rust requires a parsed tree")?;
        // The scanners recurse; skip pathological nesting as the Go checker does.
        if crate::checkers::split_loop::exceeds_depth(
            tree.root_node(),
            crate::checkers::split_loop::MAX_TREE_DEPTH,
        ) {
            return Ok(Vec::new());
        }
        let mut findings = Vec::new();
        walk_preorder(tree.root_node(), &mut |node| {
            if matches!(
                node.kind(),
                "for_expression" | "while_expression" | "loop_expression"
            ) && !in_test_code(node, ctx.source)
                && let Some(names) = independent_accumulators(node, ctx.source)
            {
                findings.push(Finding {
                    line: node.start_position().row + 1,
                    message: format!(
                        "[split-loop-rust] loop advances independent accumulators ({}) — Fowler's Split Loop: consider one loop per accumulator group",
                        names.join(", ")
                    ),
                });
            }
            true
        });
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SplitLoopRustChecker)])
}

struct Update {
    name: String,
    deps: BTreeSet<String>,
    /// `(branching node id, arm index)` per enclosing `if`/`match` arm.
    branches: Vec<(usize, usize)>,
    /// `x += 1`: a tally riding along with another job.
    counter: bool,
}

#[derive(Default)]
struct Guard {
    idents: BTreeSet<String>,
    spans: Vec<(usize, usize)>,
    branch: Option<(usize, usize)>,
}

#[derive(Default)]
struct BodyScan {
    updates: Vec<Update>,
    locals: BTreeMap<String, BTreeSet<String>>,
    accounted: Vec<(usize, usize)>,
    plain_assigns: BTreeMap<String, BTreeSet<String>>,
    has_early_exit: bool,
}

fn exceeds_node_count(root: Node, limit: usize) -> bool {
    let mut count = 0usize;
    walk_preorder(root, &mut |_| {
        count += 1;
        count <= limit
    });
    count > limit
}

fn identifiers(node: Node, source: &str, out: &mut Vec<(String, usize)>) {
    walk_preorder(node, &mut |n| {
        if matches!(n.kind(), "identifier" | "shorthand_field_identifier") {
            out.push((text(n, source).to_string(), n.start_byte()));
        }
        true
    });
}

fn identifier_set(node: Node, source: &str) -> BTreeSet<String> {
    let mut v = Vec::new();
    identifiers(node, source, &mut v);
    v.into_iter().map(|(n, _)| n).collect()
}

fn header_variables(loop_node: Node, source: &str) -> BTreeSet<String> {
    let mut vars = BTreeSet::new();
    match loop_node.kind() {
        "for_expression" => {
            if let Some(p) = loop_node.child_by_field_name("pattern") {
                vars.extend(identifier_set(p, source));
            }
        }
        // `while cond { .. }` / `while let`: the condition steers every statement in the body.
        "while_expression" => {
            if let Some(c) = loop_node.child_by_field_name("condition") {
                vars.extend(identifier_set(c, source));
            }
        }
        _ => {}
    }
    vars
}

fn independent_accumulators(loop_node: Node, source: &str) -> Option<Vec<String>> {
    let body = loop_node.child_by_field_name("body")?;
    if exceeds_node_count(body, MAX_LOOP_BODY_NODES) {
        return None;
    }
    let header_vars = header_variables(loop_node, source);
    let mut scan = BodyScan::default();
    scan_statements(body, source, &mut Vec::new(), &mut scan);
    if scan.has_early_exit {
        return None;
    }
    let locals: BTreeSet<&String> = scan.locals.keys().collect();
    let tallies_only = |name: &str| {
        scan.updates
            .iter()
            .filter(|u| u.name == name)
            .all(|u| u.counter)
    };
    let mut by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for u in &scan.updates {
        if header_vars.contains(&u.name) || locals.contains(&u.name) || tallies_only(&u.name) {
            continue;
        }
        by_name
            .entry(u.name.clone())
            .or_default()
            .extend(u.deps.iter().cloned());
    }
    if by_name.len() < 2 {
        return None;
    }
    let names: Vec<String> = by_name.keys().cloned().collect();

    let mut idents = Vec::new();
    identifiers(body, source, &mut idents);
    let unaccounted = idents.iter().any(|(name, start)| {
        by_name.contains_key(name)
            && !scan
                .accounted
                .iter()
                .any(|(s, e)| *s <= *start && *start < *e)
    });
    if unaccounted {
        return None;
    }

    let mut parent: Vec<usize> = (0..names.len()).collect();
    for (i, name) in names.iter().enumerate() {
        for dep in expand_deps(&by_name[name], &scan) {
            if dep != *name
                && let Some(j) = names.iter().position(|n| *n == dep)
            {
                union_find::union(&mut parent, i, j);
            }
        }
    }
    link_partitioned(&scan.updates, &names, &mut parent);
    let shared: BTreeSet<&String> = header_vars.iter().chain(scan.locals.keys()).collect();
    link_shared_inputs(&names, &by_name, &scan, &shared, &mut parent);
    let mut roots = BTreeSet::new();
    for i in 0..names.len() {
        roots.insert(union_find::root(&mut parent, i));
    }
    (roots.len() >= 2).then_some(names)
}

fn link_partitioned(updates: &[Update], names: &[String], parent: &mut [usize]) {
    let index_of = |n: &str| names.iter().position(|x| x == n);
    for (i, a) in updates.iter().enumerate() {
        for b in &updates[i + 1..] {
            let same_guard = !a.branches.is_empty() && a.branches == b.branches;
            let exclusive = same_guard
                || a.branches.iter().any(|(id, arm)| {
                    b.branches
                        .iter()
                        .any(|(id2, arm2)| id == id2 && arm != arm2)
                });
            if let (true, Some(x), Some(y)) = (exclusive, index_of(&a.name), index_of(&b.name)) {
                union_find::union(parent, x, y);
            }
        }
    }
}

fn link_shared_inputs(
    names: &[String],
    by_name: &BTreeMap<String, BTreeSet<String>>,
    scan: &BodyScan,
    shared: &BTreeSet<&String>,
    parent: &mut [usize],
) {
    let inputs: Vec<BTreeSet<String>> = names
        .iter()
        .map(|n| {
            expand_deps(&by_name[n], scan)
                .into_iter()
                .filter(|d| d != "_" && (shared.contains(d) || d.starts_with(TUPLE_PREFIX)))
                .collect()
        })
        .collect();
    // Accumulators fed by *any* per-iteration value (`keys.push(k); values.push(v)` over
    // `(k, v)`) are parallel projections of one item, so they count as one group.
    let item_fed: Vec<usize> = (0..names.len())
        .filter(|&i| !inputs[i].is_empty())
        .collect();
    for pair in item_fed.windows(2) {
        union_find::union(parent, pair[0], pair[1]);
    }
}

fn expand_deps(direct: &BTreeSet<String>, scan: &BodyScan) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack: Vec<String> = direct.iter().cloned().collect();
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        for map in [&scan.locals, &scan.plain_assigns] {
            if let Some(defs) = map.get(&name) {
                stack.extend(defs.iter().cloned());
            }
        }
    }
    seen
}

fn scan_statements(node: Node, source: &str, guards: &mut Vec<Guard>, scan: &mut BodyScan) {
    match node.kind() {
        "for_expression" | "while_expression" | "loop_expression" | "closure_expression"
        | "function_item" | "async_block" => return,
        "break_expression" | "continue_expression" | "return_expression" | "try_expression" => {
            scan.has_early_exit = true;
            return;
        }
        "compound_assignment_expr" | "assignment_expression" => {
            note_exits(node, scan);
            record_assignment(node, source, guards, scan);
            return;
        }
        "call_expression" => {
            note_exits(node, scan);
        }
        "let_declaration" => {
            note_exits(node, scan);
            record_local(node, source, scan);
            return;
        }
        "if_expression" => {
            scan_guarded(node, &["condition"], source, guards, scan);
            return;
        }
        "match_expression" => {
            scan_guarded(node, &["value"], source, guards, scan);
            return;
        }
        _ => {}
    }
    scan_children(node, source, guards, scan);
}

/// A `?`/`return`/`break`/`continue` buried inside a statement that is otherwise recorded whole.
fn note_exits(node: Node, scan: &mut BodyScan) {
    walk_preorder(node, &mut |n| {
        match n.kind() {
            "closure_expression" | "async_block" => return false,
            "try_expression" | "return_expression" | "break_expression" | "continue_expression" => {
                scan.has_early_exit = true;
            }
            _ => {}
        }
        true
    });
}

fn scan_children(node: Node, source: &str, guards: &mut Vec<Guard>, scan: &mut BodyScan) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_statements(child, source, guards, scan);
    }
}

fn scan_guarded(
    node: Node,
    guard_fields: &[&str],
    source: &str,
    guards: &mut Vec<Guard>,
    scan: &mut BodyScan,
) {
    let mut guard = Guard::default();
    for field in guard_fields {
        if let Some(g) = node.child_by_field_name(field) {
            guard.idents.extend(identifier_set(g, source));
            guard.spans.push((g.start_byte(), g.end_byte()));
        }
    }
    guards.push(guard);
    let mut cursor = node.walk();
    let children: Vec<Node> = node.children(&mut cursor).collect();
    let mut arm = 0usize;
    for child in children {
        let branch = match (node.kind(), child.kind()) {
            ("if_expression", _) => {
                let consequence = node.child_by_field_name("consequence");
                let alternative = node.child_by_field_name("alternative");
                if Some(child.id()) == consequence.map(|c| c.id()) {
                    Some((node.id(), 0))
                } else if Some(child.id()) == alternative.map(|c| c.id()) {
                    Some((node.id(), 1))
                } else {
                    None
                }
            }
            _ => None,
        };
        if node.kind() == "match_expression" && child.kind() == "match_block" {
            // Each arm is its own branch; the guard of an arm is its pattern.
            let mut c2 = child.walk();
            for arm_node in child.children(&mut c2).filter(|c| c.kind() == "match_arm") {
                let mut g = Guard {
                    branch: Some((node.id(), arm)),
                    ..Guard::default()
                };
                if let Some(p) = arm_node.child_by_field_name("pattern") {
                    g.idents.extend(identifier_set(p, source));
                    g.spans.push((p.start_byte(), p.end_byte()));
                }
                arm += 1;
                guards.push(g);
                scan_statements(arm_node, source, guards, scan);
                guards.pop();
            }
            continue;
        }
        guards.push(Guard {
            branch,
            ..Guard::default()
        });
        scan_statements(child, source, guards, scan);
        guards.pop();
    }
    guards.pop();
}

fn commit_update(
    scan: &mut BodyScan,
    guards: &[Guard],
    node: Node,
    name: String,
    mut deps: BTreeSet<String>,
    counter: bool,
) {
    for g in guards {
        deps.extend(g.idents.iter().cloned());
        scan.accounted.extend(g.spans.iter().copied());
    }
    scan.accounted.push((node.start_byte(), node.end_byte()));
    let branches = guards.iter().filter_map(|g| g.branch).collect();
    scan.updates.push(Update {
        name,
        deps,
        branches,
        counter,
    });
}

fn record_local(node: Node, source: &str, scan: &mut BodyScan) {
    let bound = node
        .child_by_field_name("pattern")
        .map(|p| identifier_set(p, source))
        .unwrap_or_default();
    let mut defs = node
        .child_by_field_name("value")
        .map(|v| identifier_set(v, source))
        .unwrap_or_default();
    if bound.len() > 1 {
        defs.insert(format!("{TUPLE_PREFIX}{}", node.start_byte()));
    }
    for name in bound {
        scan.locals
            .entry(name)
            .or_default()
            .extend(defs.iter().cloned());
    }
    scan.accounted.push((node.start_byte(), node.end_byte()));
}

/// `x` or `x[k]` (with `x` a plain identifier) → `(x, identifiers in the index key)`.
fn accumulator_target(target: Node, source: &str) -> Option<(String, BTreeSet<String>)> {
    match target.kind() {
        "identifier" => Some((text(target, source).to_string(), BTreeSet::new())),
        "index_expression" => {
            let base = target.named_child(0)?;
            if base.kind() != "identifier" {
                return None;
            }
            let deps = target
                .named_child(1)
                .map(|i| identifier_set(i, source))
                .unwrap_or_default();
            Some((text(base, source).to_string(), deps))
        }
        _ => None,
    }
}

fn record_assignment(node: Node, source: &str, guards: &[Guard], scan: &mut BodyScan) {
    let (Some(lhs), Some(rhs)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    let is_plain = node.kind() == "assignment_expression";
    let Some((name, mut deps)) = accumulator_target(lhs, source) else {
        return;
    };
    let rhs_idents = value_identifiers(rhs, source);
    if is_plain {
        // Only `x = x <op> e` accumulates; indexed stores (`x[k] = e`) and other plain
        // assignments build collections or move state, which backtests showed are not jobs.
        if lhs.kind() == "identifier"
            && rhs_idents.contains(&name)
            && rhs.kind() == "binary_expression"
        {
            deps.extend(rhs_idents);
            commit_update(scan, guards, node, name, deps, false);
        } else if lhs.kind() == "identifier" {
            scan.plain_assigns
                .entry(name)
                .or_default()
                .extend(rhs_idents);
            scan.accounted.push((node.start_byte(), node.end_byte()));
        }
        return;
    }
    // `ok &= cond` / `any |= cond` fold a predicate: a flag, not an aggregate job.
    if is_flag_fold(node, rhs, source) {
        scan.accounted.push((node.start_byte(), node.end_byte()));
        return;
    }
    // `x += 1` and `x += CONST_STEP` advance a position or tally; they ride along with the
    // real job instead of being one.
    let counter = rhs.kind() == "integer_literal" || rhs_idents.iter().all(|i| is_constant_like(i));
    deps.extend(rhs_idents);
    commit_update(scan, guards, node, name, deps, counter);
}

/// Identifiers read by `node`, without module paths (`libc::CPU_MAX` is a constant, not input).
fn value_identifiers(node: Node, source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_preorder(node, &mut |n| {
        if n.kind() == "scoped_identifier" {
            return false;
        }
        if matches!(n.kind(), "identifier" | "shorthand_field_identifier") {
            out.insert(text(n, source).to_string());
        }
        true
    });
    out
}

fn is_flag_fold(assign: Node, rhs: Node, source: &str) -> bool {
    let op = assign
        .child_by_field_name("operator")
        .map_or("", |o| text(o, source));
    matches!(op, "&=" | "|=")
        && rhs.kind() == "binary_expression"
        && rhs.child_by_field_name("operator").is_some_and(|o| {
            matches!(
                o.kind(),
                "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||"
            )
        })
}

/// `CONST_NAME`, a primitive type from an `as` cast, or `_`: not a per-iteration input.
fn is_constant_like(ident: &str) -> bool {
    const PRIMITIVES: &[&str] = &[
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "f32", "f64", "bool", "char",
    ];
    ident == "_"
        || PRIMITIVES.contains(&ident)
        || ident
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(body: &str) -> Vec<Finding> {
        let src = format!("fn f(items: &[i32]) {{\n{body}\n}}\n");
        crate::test_support::check_rust_source(&SplitLoopRustChecker, &src).unwrap()
    }

    fn flagged(body: &str) -> bool {
        !findings(body).is_empty()
    }

    #[test]
    fn flags_two_independent_plus_equals_accumulators() {
        let f = findings(
            "let mut a = 0; let mut b = 0;\nfor v in items { a += v; b += items.len() as i32; }",
        );
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].message.contains("(a, b)"));
    }

    #[test]
    fn collection_builders_are_not_accumulators() {
        assert!(!flagged(
            "let mut xs = vec![]; let mut ys = vec![];\nfor v in items { xs.push(*v); ys.push(items.len()); }"
        ));
        assert!(!flagged(
            "let mut xs = [0; 4]; let mut ys = [0; 4];\nfor v in items { xs[0] = *v; ys[1] = items.len(); }"
        ));
    }

    #[test]
    fn constant_step_updates_are_not_accumulators() {
        assert!(!flagged(
            "let mut a = 0; let mut off = 0;\nfor v in items { a += v; off += STEP as i32; }"
        ));
    }

    #[test]
    fn loop_form_and_nested_loops() {
        assert!(!flagged(
            "let (mut a, mut b) = (0, 0);\nloop { a += g(); b += h(); if i() { return; } }"
        ));
        assert!(flagged(
            "let (mut a, mut b) = (0, 0); let mut n = 0;\nwhile n < 3 { a += g(); b += h(); n += 1; }"
        ));
        assert!(!flagged(
            "let (mut a, mut b) = (0, 0);\nfor v in items { a += v; for w in items { b += h(*w); } }"
        ));
    }

    #[test]
    fn predicate_flag_folds_are_not_accumulators() {
        assert!(!flagged(
            "let (mut a, mut b) = (true, true);\nfor _ in 0..n { a &= f() == 0; b &= g() == 0; }"
        ));
    }

    #[test]
    fn independent_numeric_products_are_flagged() {
        assert!(flagged(
            "let (mut a, mut b) = (1.0, 1.0);\nfor _ in 0..n { a *= f(); b *= g(); }"
        ));
    }

    #[test]
    fn flags_while_and_loop_forms_too() {
        assert!(flagged(
            "let mut a = 0; let mut b = 0; let mut i = 0;\nwhile i < 3 { a += g(); b += h(); i += 1; }"
        ));
    }

    #[test]
    fn single_accumulator_is_not_flagged() {
        assert!(!flagged("let mut a = 0;\nfor v in items { a += v; }"));
    }

    #[test]
    fn cross_referencing_accumulators_are_not_flagged() {
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor v in items { a += v; b += a; }"
        ));
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor v in items { a += v; let t = a * 2; b += t; }"
        ));
    }

    #[test]
    fn same_per_iteration_input_is_one_group() {
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor v in items { a += v; b += v * 2; }"
        ));
    }

    #[test]
    fn parallel_projections_of_one_item_are_one_group() {
        assert!(!flagged(
            "let mut ks = vec![]; let mut vs = vec![];\nfor (k, v) in items { ks.push(k); vs.push(v); }"
        ));
    }

    #[test]
    fn initialisation_state_evolution_are_not_accumulation() {
        assert!(!flagged(
            "let mut xs = vec![]; let mut ys = vec![];\nfor _ in items { xs.push(Vec::new()); ys.push(0); }"
        ));
        assert!(!flagged(
            "let (mut w, mut h) = (1, 1);\nfor _ in items { w = std::cmp::max(w / 2, 1); h = std::cmp::max(h / 2, 1); }"
        ));
    }

    #[test]
    fn if_else_partition_is_one_group() {
        assert!(!flagged(
            "let (mut a, mut b) = (0, 0);\nfor v in items { if c() { a += g(); } else { b += h(); } }"
        ));
    }

    #[test]
    fn guard_reading_the_other_accumulator_links_them() {
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor _ in items { a += g(); if a > 10 { b += h(); } }"
        ));
        assert!(flagged(
            "let mut a = 0; let mut b = 0;\nfor _ in items { a += g(); if c() { b += h(); } }"
        ));
    }

    #[test]
    fn early_exits_and_question_marks_are_not_flagged() {
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor _ in items { a += g(); b += h(); if x() { break; } }"
        ));
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor _ in items { a += g()?; b += h(); }"
        ));
    }

    #[test]
    fn use_of_an_accumulator_elsewhere_is_ambiguous() {
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor _ in items { a += g(); b += h(); println!(\"{}\", a); }"
        ));
    }

    #[test]
    fn tally_counters_do_not_count() {
        assert!(!flagged(
            "let mut a = 0; let mut n = 0;\nfor _ in items { a += g(); n += 1; }"
        ));
    }

    #[test]
    fn nested_loops_and_closures_are_their_own_scope() {
        assert!(!flagged(
            "let mut a = 0; let mut b = 0;\nfor v in items { a += v; let c = || { b += g(); }; }"
        ));
    }
}
