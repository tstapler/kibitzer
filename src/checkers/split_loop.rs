use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::checkers::file_size::is_generated;
use crate::node_kind::GoKind;
use crate::union_find;

/// Fowler's Split Loop: a loop body advancing two or more accumulators that don't feed each
/// other is doing two jobs. Precision over recall: accumulators are `x op= e`, `x++`/`x--`,
/// `x = append(x, ..)` and `x[k] = e`; they are linked by shared dependencies, guards and
/// per-iteration inputs, and any mention outside the update statements suppresses the finding.
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
        if exceeds_depth(tree.root_node(), MAX_TREE_DEPTH) {
            return Ok(Vec::new());
        }
        let mut findings = Vec::new();
        visit(tree.root_node(), ctx.source.as_bytes(), &mut findings);
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(SplitLoopChecker)])
}

/// The recursive walkers below would overflow the stack on pathologically nested input.
const MAX_TREE_DEPTH: usize = 256;
/// Bounds the quadratic `accounted` scan; larger loops are skipped, not flagged.
const MAX_LOOP_BODY_NODES: usize = 5000;

/// True when `root`'s subtree is deeper than `limit`. Iterative, so safe on any input.
fn exceeds_depth(root: Node, limit: usize) -> bool {
    let mut cursor = root.walk();
    let mut depth = 0usize;
    loop {
        if cursor.goto_first_child() {
            depth += 1;
            if depth > limit {
                return true;
            }
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return false;
            }
            depth -= 1;
        }
    }
}

/// True when `root`'s subtree has more than `limit` nodes. Iterative.
fn exceeds_node_count(root: Node, limit: usize) -> bool {
    let mut cursor = root.walk();
    let mut count = 1usize;
    loop {
        if cursor.goto_first_child() {
            count += 1;
        } else {
            while !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    return false;
                }
            }
            count += 1;
        }
        if count > limit {
            return true;
        }
    }
}

fn visit(node: Node, src: &[u8], out: &mut Vec<Finding>) {
    if GoKind::of(node) == GoKind::ForStatement
        && let Some(names) = independent_accumulators(node, src)
    {
        out.push(Finding {
            line: node.start_position().row + 1,
            message: format!(
                "[split-loop-go] loop advances independent accumulators ({}) — \
                 Fowler's Split Loop: consider one loop per accumulator group",
                names.join(", ")
            ),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit(child, src, out);
    }
}

/// One accumulator update: the accumulator name and the identifiers its update depends on.
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

/// Names of every accumulator in the loop when `for_node` has 2+ independent groups
/// (and so should be flagged), else `None`.
fn independent_accumulators(for_node: Node, src: &[u8]) -> Option<Vec<String>> {
    let body = for_node.child_by_field_name("body")?;
    if exceeds_node_count(body, MAX_LOOP_BODY_NODES) {
        return None;
    }
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
        out.push((
            node.utf8_text(src).unwrap_or("").to_string(),
            node.start_byte(),
        ));
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
                commit_update(scan, guards, node, name, deps, true);
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
        GoKind::ExpressionSwitchStatement
        | GoKind::ExpressionCase
        | GoKind::TypeSwitchStatement => {
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

const TUPLE_PREFIX: &str = "#tuple@";

fn record_local(node: Node, src: &[u8], scan: &mut BodyScan) {
    let (bound, defs) = match GoKind::of(node) {
        GoKind::ShortVarDeclaration => (
            node.child_by_field_name("left")
                .map(|n| identifier_set(n, src))
                .unwrap_or_default(),
            node.child_by_field_name("right"),
        ),
        _ => {
            // `var a, b int` repeats the `name` field once per declared identifier.
            let mut cursor = node.walk();
            let bound = node
                .children_by_field_name("name", &mut cursor)
                .flat_map(|n| identifier_set(n, src))
                .collect();
            (bound, node.child_by_field_name("value"))
        }
    };
    let mut def_idents = defs.map(|d| identifier_set(d, src)).unwrap_or_default();
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
        .map(|o| o.utf8_text(src).unwrap_or(""))
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
            commit_update(scan, guards, node, name, deps, false);
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
    let counter =
        op != "=" && GoKind::of(lhs) == GoKind::Identifier && GoKind::of(rhs) == GoKind::IntLiteral;
    commit_update(scan, guards, node, name, deps, counter);
}

/// `x` or `x[k]` (with `x` a plain identifier) → `(x, identifiers in the index key)`.
fn accumulator_target(target: Node, src: &[u8]) -> Option<(String, BTreeSet<String>)> {
    match GoKind::of(target) {
        GoKind::Identifier => Some((
            target.utf8_text(src).unwrap_or("").to_string(),
            BTreeSet::new(),
        )),
        GoKind::IndexExpression => {
            let operand = target.child_by_field_name("operand")?;
            if GoKind::of(operand) != GoKind::Identifier {
                return None;
            }
            let index = target.child_by_field_name("index");
            let deps = index.map(|i| identifier_set(i, src)).unwrap_or_default();
            Some((operand.utf8_text(src).unwrap_or("").to_string(), deps))
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
        .is_some_and(|f| f.utf8_text(src).unwrap_or("") == "append");
    let first_arg_is_self = rhs
        .child_by_field_name("arguments")
        .and_then(|a| a.named_child(0))
        .is_some_and(|a| {
            GoKind::of(a) == GoKind::Identifier && a.utf8_text(src).unwrap_or("") == name
        });
    is_append && first_arg_is_self
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(body: &str) -> Vec<Finding> {
        let src = format!("package p\n\nfunc F(items []int) {{\n{body}\n}}\n");
        check(&src)
    }

    fn check(src: &str) -> Vec<Finding> {
        crate::test_support::check_go_source(&SplitLoopChecker, src).unwrap()
    }

    /// `loop_line` is the 1-based line of `for` within a `findings` body (file line = +3).
    fn assert_flagged(f: &[Finding], loop_line: usize, names: &str) {
        assert_flagged_at(f, loop_line + 3, names);
    }

    fn assert_flagged_at(f: &[Finding], line: usize, names: &str) {
        assert_eq!(f.len(), 1, "got: {f:?}");
        assert_eq!(f[0].line, line, "got: {f:?}");
        assert!(f[0].message.contains(&format!("({names})")), "got: {f:?}");
    }

    #[test]
    fn flags_two_independent_plus_equals_accumulators() {
        let f = findings("var a, b int\nfor _, v := range items {\n\ta += v\n\tb += len(items)\n}");
        assert_flagged(&f, 2, "a, b");
        assert!(f[0].message.contains("Split Loop"));
    }

    #[test]
    fn flags_two_independent_append_accumulators() {
        let f = findings(
            "var xs, ys []int\nfor _, v := range items {\n\txs = append(xs, v)\n\tys = append(ys, len(items))\n}",
        );
        assert_flagged(&f, 2, "xs, ys");
    }

    #[test]
    fn flags_independent_index_assignment_accumulators() {
        let f = findings(
            "m := map[int]int{}\nn := map[int]int{}\nfor _, v := range items {\n\tm[v] = 1\n\tn[0] += 2\n}",
        );
        assert_flagged(&f, 3, "m, n");
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
    fn follows_a_plain_assigned_local_to_its_source() {
        let linked =
            findings("var a, b, t int\nfor range items {\n\ta += x()\n\tt = a * 2\n\tb += t\n}");
        assert!(linked.is_empty(), "got: {linked:?}");
        let independent =
            findings("var a, b, t int\nfor range items {\n\ta += x()\n\tt = y() * 2\n\tb += t\n}");
        assert_flagged(&independent, 2, "a, b");
    }

    #[test]
    fn does_not_flag_when_one_update_is_guarded_by_the_other() {
        let guarded = findings(
            "var a, b int\nfor range items {\n\ta += x()\n\tif a > 10 {\n\t\tb += y()\n\t}\n}",
        );
        assert!(guarded.is_empty(), "got: {guarded:?}");
        let unrelated_guard = findings(
            "var a, b int\nfor range items {\n\ta += x()\n\tif c() {\n\t\tb += y()\n\t}\n}",
        );
        assert_flagged(&unrelated_guard, 2, "a, b");
    }

    #[test]
    fn does_not_flag_accumulators_partitioned_by_an_if_else() {
        let partitioned = findings(
            "var a, b int\nfor range items {\n\tif c() {\n\t\ta += x()\n\t} else {\n\t\tb += y()\n\t}\n}",
        );
        assert!(partitioned.is_empty(), "got: {partitioned:?}");
        let flat = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n}");
        assert_flagged(&flat, 2, "a, b");
    }

    #[test]
    fn does_not_flag_accumulators_partitioned_by_a_switch() {
        let partitioned = findings(
            "var a, b int\nfor range items {\n\tswitch k() {\n\tcase 1:\n\t\ta += x()\n\tdefault:\n\t\tb += y()\n\t}\n}",
        );
        assert!(partitioned.is_empty(), "got: {partitioned:?}");
        let flat = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n}");
        assert_flagged(&flat, 2, "a, b");
    }

    #[test]
    fn does_not_flag_accumulators_partitioned_by_a_select() {
        let partitioned = findings(
            "var a, b int\nfor range items {\n\tselect {\n\tcase <-ch():\n\t\ta += x()\n\tdefault:\n\t\tb += y()\n\t}\n}",
        );
        assert!(partitioned.is_empty(), "got: {partitioned:?}");
        let one_arm = findings(
            "var a, b int\nfor range items {\n\ta += x()\n\tselect {\n\tcase <-ch():\n\t\tb += y()\n\tdefault:\n\t}\n}",
        );
        assert_flagged(&one_arm, 2, "a, b");
    }

    #[test]
    fn does_not_flag_accumulators_partitioned_by_a_type_switch() {
        let partitioned = findings(
            "var a, b int\nfor range items {\n\tswitch k().(type) {\n\tcase int:\n\t\ta += x()\n\tdefault:\n\t\tb += y()\n\t}\n}",
        );
        assert!(partitioned.is_empty(), "got: {partitioned:?}");
    }

    #[test]
    fn does_not_flag_when_the_body_continues_early() {
        let early = findings(
            "var a, b int\nfor range items {\n\tif c() {\n\t\tcontinue\n\t}\n\ta += x()\n\tb += y()\n}",
        );
        assert!(early.is_empty(), "got: {early:?}");
        let no_exit = findings(
            "var a, b int\nfor range items {\n\tif c() {\n\t\tlog()\n\t}\n\ta += x()\n\tb += y()\n}",
        );
        assert_flagged(&no_exit, 2, "a, b");
    }

    #[test]
    fn does_not_flag_when_the_body_exits_with_goto() {
        let early = findings(
            "var a, b int\nfor range items {\n\tif c() {\n\t\tgoto done\n\t}\n\ta += x()\n\tb += y()\n}\ndone:\n\tlog()",
        );
        assert!(early.is_empty(), "got: {early:?}");
        let no_exit = findings(
            "var a, b int\nfor range items {\n\tif c() {\n\t\tlog()\n\t}\n\ta += x()\n\tb += y()\n}\ndone:\n\tlog()",
        );
        assert_flagged(&no_exit, 2, "a, b");
    }

    #[test]
    fn does_not_flag_accumulators_updated_under_the_same_guard() {
        let guarded = findings(
            "var a, b int\nfor range items {\n\tif c() {\n\t\ta += x()\n\t\tb += y()\n\t}\n}",
        );
        assert!(guarded.is_empty(), "got: {guarded:?}");
        let unguarded = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n}");
        assert_flagged(&unguarded, 2, "a, b");
    }

    #[test]
    fn does_not_flag_parallel_projections_of_the_same_element() {
        let f = findings(
            "var xs, ys []int\nfor _, v := range items {\n\txs = append(xs, v)\n\tys = append(ys, v+1)\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn multi_name_var_declarations_are_all_body_locals() {
        let f = findings(
            "var a int\nfor _, v := range items {\n\tvar x, y int\n\tx += v\n\ty += v\n\ta += v\n}",
        );
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn does_not_treat_range_variables_as_accumulators() {
        let range_var = findings("var a int\nfor _, v := range items {\n\tv += y()\n\ta += x()\n}");
        assert!(range_var.is_empty(), "got: {range_var:?}");
        let real_second = findings("var a, b int\nfor range items {\n\tb += y()\n\ta += x()\n}");
        assert_flagged(&real_second, 2, "a, b");
    }

    #[test]
    fn does_not_treat_the_c_style_counter_as_an_accumulator() {
        let counter = findings("var a int\nfor i := 0; i < 3; i++ {\n\ti += y()\n\ta += x()\n}");
        assert!(counter.is_empty(), "got: {counter:?}");
        let real_second =
            findings("var a, b int\nfor i := 0; i < 3; i++ {\n\tb += y()\n\ta += x()\n}");
        assert_flagged(&real_second, 2, "a, b");
    }

    #[test]
    fn does_not_treat_a_condition_only_loop_variable_as_an_accumulator() {
        let cond = findings("var a, k int\nfor k < 10 {\n\tk += y()\n\ta += x()\n}");
        assert!(cond.is_empty(), "got: {cond:?}");
        let real_second = findings("var a, b, k int\nfor k < 10 {\n\tb += y()\n\ta += x()\n}");
        assert_flagged(&real_second, 2, "a, b");
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
        let read = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n\tuse(a)\n}");
        assert!(read.is_empty(), "got: {read:?}");
        let unrelated_read =
            findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n\tuse(z)\n}");
        assert_flagged(&unrelated_read, 2, "a, b");
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
        assert_flagged_at(&check(src), 7, "a, b");
    }

    #[test]
    fn skips_generated_files() {
        let body = "func F(items []int) {\n\tvar a, b int\n\tfor range items {\n\t\ta += x()\n\t\tb += y()\n\t}\n}\n";
        let generated = format!("// Code generated by x. DO NOT EDIT.\n\npackage p\n\n{body}");
        assert!(check(&generated).is_empty());
        let handwritten = format!("package p\n\n{body}");
        assert_flagged_at(&check(&handwritten), 5, "a, b");
    }

    #[test]
    fn does_not_scan_closure_bodies() {
        let in_closure = findings(
            "var a, b int\nfor range items {\n\tfunc() {\n\t\ta += x()\n\t\tb += y()\n\t}()\n}",
        );
        assert!(in_closure.is_empty(), "got: {in_closure:?}");
    }

    #[test]
    fn links_accumulators_fed_by_one_tuple_call() {
        let tuple =
            findings("var a, b int\nfor range items {\n\tp, q := f()\n\ta += p\n\tb += q\n}");
        assert!(tuple.is_empty(), "got: {tuple:?}");
        let separate = findings(
            "var a, b int\nfor range items {\n\tp := f()\n\tq := g()\n\ta += p\n\tb += q\n}",
        );
        assert_flagged(&separate, 2, "a, b");
    }

    #[test]
    fn links_an_accumulator_used_as_an_index_key() {
        let keyed =
            findings("var a int\nxs := []int{}\nfor range items {\n\ta += x()\n\txs[a] = y()\n}");
        assert!(keyed.is_empty(), "got: {keyed:?}");
        let constant_key =
            findings("var a int\nxs := []int{}\nfor range items {\n\ta += x()\n\txs[0] = y()\n}");
        assert_flagged(&constant_key, 3, "a, xs");
    }

    #[test]
    fn an_indexed_increment_is_not_a_counter() {
        let indexed = findings(
            "var a int\nm := map[int]int{}\nfor range items {\n\ta += x()\n\tm[0] += 1\n}",
        );
        assert_flagged(&indexed, 3, "a, m");
        let plain = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += 1\n}");
        assert!(plain.is_empty(), "got: {plain:?}");
    }

    #[test]
    fn decrement_statements_are_recorded_as_updates() {
        // `b--` is a tally, but it must still account for `b` or `b += y()` looks read-elsewhere.
        let f = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n\tb--\n}");
        assert_flagged(&f, 2, "a, b");
        let only_tallies = findings("var a, b int\nfor range items {\n\ta += x()\n\tb--\n}");
        assert!(only_tallies.is_empty(), "got: {only_tallies:?}");
    }

    #[test]
    fn deeply_nested_input_is_skipped_without_overflowing_the_stack() {
        let depth = 5000;
        let nested = format!("{}x(){}", "(".repeat(depth), ")".repeat(depth));
        let f = findings(&format!(
            "var a, b int\nfor range items {{\n\ta += {nested}\n\tb += y()\n}}"
        ));
        assert!(f.is_empty(), "got: {f:?}");
    }

    #[test]
    fn oversized_loop_bodies_are_skipped() {
        let stmts = "\ta += x()\n\tb += y()\n".repeat(1000);
        let big = findings(&format!("var a, b int\nfor range items {{\n{stmts}}}"));
        assert!(big.is_empty(), "got: {big:?}");
        let small = findings("var a, b int\nfor range items {\n\ta += x()\n\tb += y()\n}");
        assert_flagged(&small, 2, "a, b");
    }
}
