use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::checkers::file_size::is_generated;
use crate::node_kind::GoKind;
use crate::union_find;

/// Fowler's Split Loop (*Refactoring* 2nd ed., p. 227): a loop body advancing two or more
/// accumulators that don't feed each other is doing two jobs in one place.
///
/// Conservative v1, tuned for precision over recall (the fix is cheap once a human is
/// pointed at the loop): accumulators are `x op= e`, `x++`/`x--`, `x = append(x, ...)` and
/// `x[k] = e` on a plain local-or-outer identifier. Two accumulators are linked when one's
/// update (its right-hand side, index key, or enclosing `if`/`switch` guard, following
/// per-iteration locals declared in the body) mentions the other by plain identifier.
/// Any mention of an accumulator outside those update statements (a call argument, a
/// `break` guard, a nested loop or closure) makes the loop ambiguous, so it isn't flagged.
pub struct SplitLoopChecker;

impl Checker for SplitLoopChecker {
    fn name(&self) -> &str {
        "split-loop-go"
    }

    fn description(&self) -> &str {
        "flags a loop that advances independent accumulators in one body — Fowler's Split Loop"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        if is_generated(ctx.source) {
            return Ok(Vec::new());
        }
        let tree = ctx
            .tree
            .context("split-loop-go checker requires a parsed tree")?;
        let mut findings = Vec::new();
        visit(tree.root_node(), ctx.source.as_bytes(), &mut findings);
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SplitLoopChecker)])
}

fn visit(node: Node, src: &[u8], out: &mut Vec<Finding>) {
    if GoKind::of(node) == GoKind::ForStatement {
        if let Some(names) = independent_accumulators(node, src) {
            out.push(Finding {
                line: node.start_position().row + 1,
                message: format!(
                    "[split-loop-go] loop advances independent accumulators ({}) — \
                     Fowler's Split Loop: consider one loop per accumulator group",
                    names.join(", ")
                ),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, src, out);
    }
}

fn text<'a>(node: Node, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("")
}

/// One accumulator update: the accumulator name, the identifiers its update depends on,
/// and the source span the update statement (plus its guards) accounts for.
struct Update {
    name: String,
    deps: BTreeSet<String>,
    /// `(branching node id, branch index)` for each enclosing `if`/`switch` arm.
    branches: Vec<(usize, usize)>,
    /// `x++` / `x += 1`: a tally that rides along with some other loop job (sum+count).
    counter: bool,
}

/// An enclosing `if`/`switch` guard: the identifiers it mentions and where they sit.
#[derive(Default)]
struct Guard {
    idents: BTreeSet<String>,
    spans: Vec<(usize, usize)>,
    branch: Option<(usize, usize)>,
}

#[derive(Default)]
struct BodyScan {
    updates: Vec<Update>,
    /// Per-iteration locals (`:=`/`var` in the body) → identifiers of their definitions.
    locals: BTreeMap<String, BTreeSet<String>>,
    /// Byte spans of statements/guards whose identifier occurrences are accounted for.
    accounted: Vec<(usize, usize)>,
    /// Plain `=` to a name, remembered to treat it as a local-style derived value.
    plain_assigns: BTreeMap<String, BTreeSet<String>>,
    /// The body has a `break`/`continue`/`return`/`goto` (outside nested loops/closures):
    /// every accumulator shares that early exit, so a split would have to duplicate it.
    has_early_exit: bool,
}

/// Names of the independent accumulator groups' representatives when `for_node` should be
/// flagged, else `None`.
fn independent_accumulators(for_node: Node, src: &[u8]) -> Option<Vec<String>> {
    let body = for_node.child_by_field_name("body")?;
    let header_vars = header_variables(for_node, src);

    let mut scan = BodyScan::default();
    scan_statements(body, src, &mut Vec::new(), &mut scan);

    if scan.has_early_exit {
        return None;
    }
    let locals: BTreeSet<&String> = scan.locals.keys().collect();
    let mut by_name: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let tallies_only = |name: &str| {
        scan.updates
            .iter()
            .filter(|u| u.name == name)
            .all(|u| u.counter)
    };
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

    // Any use of an accumulator outside its own update statements is ambiguous.
    let mut idents = Vec::new();
    collect_identifiers(body, src, &mut idents);
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
            if dep == *name {
                continue;
            }
            if let Some(j) = names.iter().position(|n| *n == dep) {
                union_find::union(&mut parent, i, j);
            }
        }
    }
    link_partitioned_accumulators(&scan.updates, &names, &mut parent);
    let shared_inputs: BTreeSet<&String> = header_vars.iter().chain(scan.locals.keys()).collect();
    link_shared_inputs(&names, &by_name, &scan, &shared_inputs, &mut parent);
    let mut roots = BTreeSet::new();
    for i in 0..names.len() {
        roots.insert(union_find::root(&mut parent, i));
    }
    (roots.len() >= 2).then_some(names)
}

/// Accumulators updated in different arms of the same `if`/`switch` partition the input
/// (`if ok { good++ } else { bad++ }`), and ones updated under the very same arms share a
/// gating predicate; either way splitting would duplicate the predicate, so they count as
/// one group.
fn link_partitioned_accumulators(updates: &[Update], names: &[String], parent: &mut [usize]) {
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

/// Accumulators fed from the same per-iteration value (the range element, the index, or a
/// local derived in the body) are parallel projections of one record, not separate jobs —
/// e.g. `env = append(env, ..port..); ports = append(ports, ..port..)` — so they count as
/// one group. Only loop-scoped names count; shared package names like `fmt` do not.
fn link_shared_inputs(
    names: &[String],
    by_name: &BTreeMap<String, BTreeSet<String>>,
    scan: &BodyScan,
    shared_inputs: &BTreeSet<&String>,
    parent: &mut [usize],
) {
    let inputs: Vec<BTreeSet<String>> = names
        .iter()
        .map(|n| {
            expand_deps(&by_name[n], scan)
                .into_iter()
                .filter(|d| d != "_" && (shared_inputs.contains(d) || d.starts_with(TUPLE_PREFIX)))
                .collect()
        })
        .collect();
    for i in 0..names.len() {
        for j in i + 1..names.len() {
            if !inputs[i].is_disjoint(&inputs[j]) {
                union_find::union(parent, i, j);
            }
        }
    }
}

/// Follows body-local definitions transitively so `t := sum * 2; count += t` links
/// `count` to `sum`.
fn expand_deps(direct: &BTreeSet<String>, scan: &BodyScan) -> BTreeSet<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
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

/// Variables introduced by the loop header (`i, v := range s`, `for i := 0; ...`).
fn header_variables(for_node: Node, src: &[u8]) -> BTreeSet<String> {
    let mut vars = BTreeSet::new();
    let mut cursor = for_node.walk();
    for child in for_node.children(&mut cursor) {
        match GoKind::of(child) {
            GoKind::RangeClause => {
                if let Some(left) = child.child_by_field_name("left") {
                    collect_names(left, src, &mut vars);
                }
            }
            GoKind::ForClause => {
                for field in ["initializer", "condition", "update"] {
                    if let Some(part) = child.child_by_field_name(field) {
                        collect_names(part, src, &mut vars);
                    }
                }
            }
            GoKind::Block => {}
            // `for cond { ... }`: the condition steers every statement in the body.
            _ if child.is_named() => collect_names(child, src, &mut vars),
            _ => {}
        }
    }
    vars
}

fn collect_names(node: Node, src: &[u8], out: &mut BTreeSet<String>) {
    let mut idents = Vec::new();
    collect_identifiers(node, src, &mut idents);
    out.extend(idents.into_iter().map(|(n, _)| n));
}

/// Every `identifier` node under `node` as `(name, start_byte)`. Selector fields
/// (`item.sum`) are `field_identifier`s and so are never matched.
fn collect_identifiers(node: Node, src: &[u8], out: &mut Vec<(String, usize)>) {
    if GoKind::of(node) == GoKind::Identifier {
        out.push((text(node, src).to_string(), node.start_byte()));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_identifiers(child, src, out);
    }
}

fn identifier_set(node: Node, src: &[u8]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    collect_names(node, src, &mut set);
    set
}

/// Walks the loop body's statements, recording accumulator updates. Does not descend
/// into nested loops or function literals — their accumulators belong to them.
fn scan_statements(node: Node, src: &[u8], guards: &mut Vec<Guard>, scan: &mut BodyScan) {
    match GoKind::of(node) {
        GoKind::ForStatement | GoKind::FuncLiteral => return,
        GoKind::BreakStatement
        | GoKind::ContinueStatement
        | GoKind::ReturnStatement
        | GoKind::GotoStatement => {
            scan.has_early_exit = true;
            return;
        }
        GoKind::AssignmentStatement => {
            record_assignment(node, src, guards, scan);
            return;
        }
        GoKind::IncStatement | GoKind::DecStatement => {
            if let Some((name, deps)) = node
                .named_child(0)
                .and_then(|target| accumulator_target(target, src))
            {
                commit_update(scan, guards, node, name, deps);
                mark_last_as_counter(scan);
            }
            return;
        }
        GoKind::ShortVarDeclaration | GoKind::VarSpec => {
            record_local(node, src, scan);
            return;
        }
        GoKind::IfStatement => {
            scan_guarded(node, &["initializer", "condition"], src, guards, scan);
            return;
        }
        GoKind::ExpressionSwitchStatement | GoKind::ExpressionCase => {
            scan_guarded(node, &["value"], src, guards, scan);
            return;
        }
        GoKind::TypeSwitchStatement => {
            scan_guarded(node, &["value"], src, guards, scan);
            return;
        }
        GoKind::TypeCase => {
            scan_guarded(node, &["type"], src, guards, scan);
            return;
        }
        GoKind::SelectStatement => {
            scan_guarded(node, &[], src, guards, scan);
            return;
        }
        GoKind::CommunicationCase => {
            scan_guarded(node, &["communication"], src, guards, scan);
            return;
        }
        _ => {}
    }
    scan_children(node, src, guards, scan);
}

fn scan_children(node: Node, src: &[u8], guards: &mut Vec<Guard>, scan: &mut BodyScan) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        scan_statements(child, src, guards, scan);
    }
}

/// Scans a branching node's children with its guard expressions pushed. Guard spans only
/// become "accounted" if an accumulator update inside the branch commits them, so a guard
/// that merely reads an accumulator (e.g. `if a > n { break }`) stays ambiguous.
fn scan_guarded(
    node: Node,
    guard_fields: &[&str],
    src: &[u8],
    guards: &mut Vec<Guard>,
    scan: &mut BodyScan,
) {
    let mut guard = Guard::default();
    for field in guard_fields {
        if let Some(g) = node.child_by_field_name(field) {
            guard.idents.extend(identifier_set(g, src));
            guard.spans.push((g.start_byte(), g.end_byte()));
        }
    }
    guards.push(guard);
    let mut cursor = node.walk();
    for (index, child) in node.children(&mut cursor).enumerate() {
        let branch = arm_index(node, child, index).map(|i| (node.id(), i));
        guards.push(Guard {
            branch,
            ..Guard::default()
        });
        scan_statements(child, src, guards, scan);
        guards.pop();
    }
    guards.pop();
}

/// Which arm of an `if`/`switch` `child` is, so updates in different arms can be told
/// apart: `if` consequence = 0, alternative = 1; each `switch` case/default is its own arm.
fn arm_index(parent: Node, child: Node, index: usize) -> Option<usize> {
    match GoKind::of(parent) {
        GoKind::IfStatement => {
            let consequence = parent.child_by_field_name("consequence")?;
            if child.id() == consequence.id() {
                return Some(0);
            }
            let alternative = parent.child_by_field_name("alternative")?;
            (child.id() == alternative.id()).then_some(1)
        }
        GoKind::ExpressionSwitchStatement => matches!(
            GoKind::of(child),
            GoKind::ExpressionCase | GoKind::DefaultCase
        )
        .then_some(index),
        GoKind::TypeSwitchStatement => {
            matches!(GoKind::of(child), GoKind::TypeCase | GoKind::DefaultCase).then_some(index)
        }
        GoKind::SelectStatement => matches!(
            GoKind::of(child),
            GoKind::CommunicationCase | GoKind::DefaultCase
        )
        .then_some(index),
        _ => None,
    }
}

/// Records an accumulator update, folding in the identifiers and spans of every
/// enclosing guard.
fn commit_update(
    scan: &mut BodyScan,
    guards: &[Guard],
    node: Node,
    name: String,
    mut deps: BTreeSet<String>,
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
        counter: false,
    });
}

const TUPLE_PREFIX: &str = "#tuple@";

fn mark_last_as_counter(scan: &mut BodyScan) {
    if let Some(u) = scan.updates.last_mut() {
        u.counter = true;
    }
}

fn record_local(node: Node, src: &[u8], scan: &mut BodyScan) {
    let (names, defs) = match GoKind::of(node) {
        GoKind::ShortVarDeclaration => (
            node.child_by_field_name("left"),
            node.child_by_field_name("right"),
        ),
        _ => (
            node.child_by_field_name("name"),
            node.child_by_field_name("value"),
        ),
    };
    let mut def_idents = defs.map(|d| identifier_set(d, src)).unwrap_or_default();
    if let Some(n) = names {
        let bound = identifier_set(n, src);
        if bound.len() > 1 {
            // `a, b := f()`: one call feeds both, so they are projections of one value.
            def_idents.insert(format!("{TUPLE_PREFIX}{}", node.start_byte()));
        }
        for name in bound {
            scan.locals
                .entry(name)
                .or_default()
                .extend(def_idents.iter().cloned());
        }
    }
    scan.accounted.push((node.start_byte(), node.end_byte()));
}

fn record_assignment(node: Node, src: &[u8], guards: &[Guard], scan: &mut BodyScan) {
    let (Some(left), Some(right)) = (
        node.child_by_field_name("left"),
        node.child_by_field_name("right"),
    ) else {
        return;
    };
    let op = node
        .child_by_field_name("operator")
        .map(|o| text(o, src))
        .unwrap_or("=");
    if left.named_child_count() != 1 || right.named_child_count() != 1 {
        return;
    }
    let (Some(lhs), Some(rhs)) = (left.named_child(0), right.named_child(0)) else {
        return;
    };
    let is_plain = op == "=";
    let Some((name, mut deps)) = accumulator_target(lhs, src) else {
        return;
    };
    let rhs_idents = identifier_set(rhs, src);
    if is_plain && GoKind::of(lhs) == GoKind::Identifier {
        if is_self_append(rhs, &name, src) {
            deps.extend(rhs_idents);
            commit_update(scan, guards, node, name, deps);
        } else {
            // `t = sum * 2`: a derived value, not an accumulator shape. Track it so a later
            // `count += t` still links to `sum`.
            scan.plain_assigns
                .entry(name)
                .or_default()
                .extend(rhs_idents);
            scan.accounted.push((node.start_byte(), node.end_byte()));
        }
        return;
    }
    // `x op= e` or `x[k] = e` / `x[k] op= e`.
    deps.extend(rhs_idents);
    commit_update(scan, guards, node, name, deps);
    if op != "=" && GoKind::of(lhs) == GoKind::Identifier && GoKind::of(rhs) == GoKind::IntLiteral {
        mark_last_as_counter(scan);
    }
}

/// `x` or `x[k]` (with `x` a plain identifier) → `(x, identifiers in the index key)`.
fn accumulator_target(target: Node, src: &[u8]) -> Option<(String, BTreeSet<String>)> {
    match GoKind::of(target) {
        GoKind::Identifier => Some((text(target, src).to_string(), BTreeSet::new())),
        GoKind::IndexExpression => {
            let operand = target.child_by_field_name("operand")?;
            if GoKind::of(operand) != GoKind::Identifier {
                return None;
            }
            let index = target.child_by_field_name("index");
            let deps = index.map(|i| identifier_set(i, src)).unwrap_or_default();
            Some((text(operand, src).to_string(), deps))
        }
        _ => None,
    }
}

fn is_self_append(rhs: Node, name: &str, src: &[u8]) -> bool {
    if GoKind::of(rhs) != GoKind::CallExpression {
        return false;
    }
    let is_append = rhs
        .child_by_field_name("function")
        .is_some_and(|f| text(f, src) == "append");
    let first_arg_is_self = rhs
        .child_by_field_name("arguments")
        .and_then(|a| a.named_child(0))
        .is_some_and(|a| GoKind::of(a) == GoKind::Identifier && text(a, src) == name);
    is_append && first_arg_is_self
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(body: &str) -> Vec<Finding> {
        let src = format!("package p\n\nfunc F(items []int) {{\n{body}\n}}\n");
        crate::test_support::check_go_source(&SplitLoopChecker, &src).unwrap()
    }

    #[test]
    fn flags_two_independent_plus_equals_accumulators() {
        let f = findings("var a, b int\nfor _, v := range items {\n\ta += v\n\tb += len(items)\n}");
        assert_eq!(f.len(), 1, "got: {f:?}");
        assert!(f[0].message.contains("a, b"));
        assert!(f[0].message.contains("Split Loop"));
    }

    #[test]
    fn flags_two_independent_append_accumulators() {
        let f = findings(
            "var xs, ys []int\nfor _, v := range items {\n\txs = append(xs, v)\n\tys = append(ys, len(items))\n}",
        );
        assert_eq!(f.len(), 1, "got: {f:?}");
    }

    #[test]
    fn flags_independent_index_assignment_accumulators() {
        let f = findings(
            "m := map[int]int{}\nn := map[int]int{}\nfor _, v := range items {\n\tm[v] = 1\n\tn[0] += 2\n}",
        );
        assert_eq!(f.len(), 1, "got: {f:?}");
    }

    #[test]
    fn does_not_flag_a_single_accumulator() {
        assert!(findings("var a int\nfor _, v := range items {\n\ta += v\n}").is_empty());
    }

    #[test]
    fn does_not_flag_cross_referencing_accumulators() {
        let f = findings("var a, b int\nfor _, v := range items {\n\ta += v\n\tb += a\n}");
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_when_dependency_goes_through_a_local() {
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\ta += v\n\tt := a * 2\n\tb += t\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_when_one_update_is_guarded_by_the_other() {
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\ta += v\n\tif a > 10 {\n\t\tb++\n\t}\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_accumulators_partitioned_by_an_if_else() {
        let f = findings(
            "var good, bad []int\nfor _, v := range items {\n\tif v > 0 {\n\t\tgood = append(good, v)\n\t} else {\n\t\tbad = append(bad, v)\n\t}\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_accumulators_partitioned_by_a_switch() {
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\tswitch v {\n\tcase 1:\n\t\ta++\n\tdefault:\n\t\tb++\n\t}\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_when_the_body_continues_early() {
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\tif v < 0 {\n\t\tcontinue\n\t}\n\ta += v\n\tb += v * 2\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_accumulators_updated_under_the_same_guard() {
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\tif v > 0 {\n\t\ta += v\n\t\tb++\n\t}\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_parallel_projections_of_the_same_element() {
        let f = findings(
            "var xs, ys []int\nfor _, v := range items {\n\txs = append(xs, v)\n\tys = append(ys, v+1)\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_treat_range_variables_as_accumulators() {
        // `i` and `v` are range variables; only `a` is an accumulator.
        let f = findings("var a int\nfor i, v := range items {\n\ti += 1\n\tv += 1\n\ta += v\n}");
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_treat_the_c_style_counter_as_an_accumulator() {
        let f = findings("var a int\nfor i := 0; i < len(items); i++ {\n\ta += items[i]\n}");
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn excludes_nested_loop_accumulators_from_the_outer_loop() {
        // Outer loop has one accumulator (`a`); `b` is updated only in the inner loop.
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\ta += v\n\tfor _, w := range items {\n\t\tb += w\n\t}\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_when_an_accumulator_is_read_elsewhere_in_the_body() {
        let f = findings(
            "var a, b int\nfor _, v := range items {\n\ta += v\n\tb += v\n\tif a > 100 {\n\t\tbreak\n\t}\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_flag_per_iteration_locals() {
        let f = findings(
            "for _, v := range items {\n\tx := 0\n\ty := 0\n\tx += v\n\ty += v\n\t_ = x + y\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn ignores_field_selectors_named_like_an_accumulator() {
        // `item.a` is a field, not the accumulator `a` — so `b` doesn't depend on `a`.
        let src = "package p\n\ntype T struct{ a int }\n\nfunc F(items []T, other T) {\n\tvar a, b int\n\tfor _, it := range items {\n\t\ta += it.a\n\t\tb += other.a\n\t}\n}\n";
        let f = crate::test_support::check_go_source(&SplitLoopChecker, src).unwrap();
        assert_eq!(f.len(), 1, "got: {f:?}");
    }
}
