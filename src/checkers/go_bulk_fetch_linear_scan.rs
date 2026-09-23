use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::go_call_resolution;
use crate::node_kind::GoKind;

/// Name prefixes conventionally used for a bulk-fetch-everything call (`ListAllX`,
/// `GetAllFoo`, `FindAllBar`) — flags the "fetch everything, then find one row" shape
/// from issue #30's profiled hotspot.
const BULK_FETCH_PREFIXES: &[&str] = &["List", "GetAll", "FindAll"];

/// Flags a Go function that bulk-fetches a collection (`List*`/`GetAll*`/`FindAll*`)
/// and then `range`s over it comparing each element's field against one of the
/// function's own parameters before returning — the "fetch everything to find one row"
/// shape an indexed/keyed lookup usually replaces. Purely structural: it doesn't
/// resolve whether an indexed alternative actually exists, only that the shape matches.
pub struct BulkFetchLinearScanChecker;

impl Checker for BulkFetchLinearScanChecker {
    fn name(&self) -> &str {
        "go-bulk-fetch-linear-scan"
    }

    fn description(&self) -> &str {
        "flags a bulk List*/GetAll*/FindAll* fetch that's immediately linear-scanned \
         for a single parameter match, instead of an indexed/keyed lookup"
    }

    fn language(&self) -> Option<Language> {
        Some(Language::Go)
    }

    fn file_globs(&self) -> &[&str] {
        &["**/*.go"]
    }

    fn check(&self, _file: &Path, ctx: &CheckContext) -> Result<Vec<Finding>> {
        let tree = ctx
            .tree
            .context("go-bulk-fetch-linear-scan checker requires a parsed tree")?;
        let mut findings = Vec::new();
        crate::tree_walk::walk_preorder(tree.root_node(), &mut |n| {
            if matches!(
                GoKind::of(n),
                GoKind::FunctionDeclaration | GoKind::MethodDeclaration
            ) {
                check_function(n, ctx.source.as_bytes(), &mut findings);
            }
            true
        });
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(BulkFetchLinearScanChecker)])
}

#[cfg(test)]
fn check_source(src: &str) -> Result<Vec<Finding>> {
    crate::test_support::check_go_source(&BulkFetchLinearScanChecker, src)
}

fn check_function<'a>(func: Node<'a>, src: &'a [u8], findings: &mut Vec<Finding>) {
    let Some(body) = func.child_by_field_name("body") else {
        return;
    };
    let Some(params) = func.child_by_field_name("parameters") else {
        return;
    };
    let param_names = collect_param_names(params, src);
    if param_names.is_empty() {
        return;
    }
    let bulk_vars = collect_bulk_fetch_vars(body, src);
    if bulk_vars.is_empty() {
        return;
    }
    if let Some(finding) = scan_for_lookup(body, src, &bulk_vars, &param_names, None) {
        // One finding per function, not per matching loop: a second offending loop in
        // the same function is real but redundant noise — the first is enough to send
        // someone to look at the function.
        findings.push(finding);
    }
}

/// The innermost enclosing bulk-fetch range loop a node is scanned under, if any: the
/// `for_statement` itself (for the finding's line) plus its ranged/item variable names.
type LoopContext<'a> = (Node<'a>, &'a str, &'a str);

/// Recursively scans `node` for the first `for`-loop that ranges over a known
/// bulk-fetch var and whose body contains a matching lookup `if`, returning on the
/// first hit in the same document order the old preorder-walk design used. A single
/// pass carrying `active` (the innermost enclosing bulk-fetch loop, if any) down through
/// the recursion — rather than, for each `for_statement` found, independently re-walking
/// that loop's whole body subtree — avoids the O(depth²) revisiting a chain of `k`
/// directly-nested bulk-fetch loops would otherwise cause (each of the `k` nested
/// bodies getting fully walked once for itself and again as part of every enclosing
/// loop's own body walk).
fn scan_for_lookup<'a>(
    node: Node<'a>,
    src: &'a [u8],
    bulk_vars: &[&str],
    param_names: &[&str],
    active: Option<LoopContext<'a>>,
) -> Option<Finding> {
    if GoKind::of(node) == GoKind::ForStatement
        && let Some((ranged_name, item_name)) = range_over_bulk_var(node, src, bulk_vars)
    {
        let loop_body = node.child_by_field_name("body")?;
        return scan_for_lookup(
            loop_body,
            src,
            bulk_vars,
            param_names,
            Some((node, ranged_name, item_name)),
        );
    }

    if GoKind::of(node) == GoKind::IfStatement
        && let Some((for_stmt, ranged_name, item_name)) = active
        && matches_lookup_pattern(node, src, item_name, param_names)
    {
        return Some(Finding {
            line: for_stmt.start_position().row + 1,
            message: format!(
                "ranges over `{ranged_name}` (bound from a bulk List/GetAll/FindAll fetch) \
                 and returns on the first element matching a parameter — an indexed/keyed \
                 lookup usually replaces fetching everything to find one row"
            ),
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(finding) = scan_for_lookup(child, src, bulk_vars, param_names, active) {
            return Some(finding);
        }
    }
    None
}

/// Non-blank parameter names declared on `params` (a function/method's own
/// `parameter_list`, not its receiver) — what a loop-body comparison must reference to
/// count as "checked against a parameter" rather than some unrelated local/field.
fn collect_param_names<'a>(params: Node<'a>, src: &'a [u8]) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut cursor = params.walk();
    for decl in params
        .children(&mut cursor)
        .filter(|n| GoKind::of(*n) == GoKind::ParameterDeclaration)
    {
        let mut inner = decl.walk();
        for name in decl.children_by_field_name("name", &mut inner) {
            if let Ok(text) = name.utf8_text(src)
                && text != "_"
            {
                names.push(text);
            }
        }
    }
    names
}

/// Variable names bound (via `:=`) to the direct result of a `List*`/`GetAll*`/
/// `FindAll*` call anywhere in `body` — e.g. the `all` in `all, err := s.ListAllX()`.
fn collect_bulk_fetch_vars<'a>(body: Node<'a>, src: &'a [u8]) -> Vec<&'a str> {
    let mut vars = Vec::new();
    crate::tree_walk::walk_preorder(body, &mut |n| {
        if GoKind::of(n) == GoKind::ShortVarDeclaration
            && let Some(name) = bulk_fetch_bound_var(n, src)
        {
            vars.push(name);
        }
        true
    });
    vars
}

/// When `decl` binds its first LHS name to a direct `List*`/`GetAll*`/`FindAll*` call
/// result, returns that bound name.
fn bulk_fetch_bound_var<'a>(decl: Node<'a>, src: &'a [u8]) -> Option<&'a str> {
    let call = go_call_resolution::single_rhs_call_expression(decl)?;
    let name = callee_name(call, src)?;
    if !is_bulk_fetch_name(name) {
        return None;
    }
    let left = decl.child_by_field_name("left")?;
    let mut cursor = left.walk();
    let bound = left
        .children(&mut cursor)
        .find(|c| GoKind::of(*c) == GoKind::Identifier)?;
    let text = bound.utf8_text(src).ok()?;
    (text != "_").then_some(text)
}

/// The callee's bare name: the identifier itself for a free-function call, or the
/// field name for a method/package-qualified call (`s.ListAllX()` → `ListAllX`).
fn callee_name<'a>(call: Node<'a>, src: &'a [u8]) -> Option<&'a str> {
    let function = call.child_by_field_name("function")?;
    let name_node = match GoKind::of(function) {
        GoKind::Identifier => function,
        GoKind::SelectorExpression => function.child_by_field_name("field")?,
        _ => return None,
    };
    name_node.utf8_text(src).ok()
}

fn is_bulk_fetch_name(name: &str) -> bool {
    BULK_FETCH_PREFIXES.iter().any(|&prefix| {
        name == prefix
            || (name.len() > prefix.len()
                && name.starts_with(prefix)
                && name.as_bytes()[prefix.len()].is_ascii_uppercase())
    })
}

/// When `for_stmt` is `for _, <item> := range <ranged>` and `<ranged>` is one of
/// `bulk_vars`, returns `(ranged, item)` — `None` for any other for-loop shape (a plain
/// C-style loop, or a range over something other than a known bulk-fetch var).
fn range_over_bulk_var<'a>(
    for_stmt: Node<'a>,
    src: &'a [u8],
    bulk_vars: &[&str],
) -> Option<(&'a str, &'a str)> {
    let mut cursor = for_stmt.walk();
    let range = for_stmt
        .children(&mut cursor)
        .find(|c| GoKind::of(*c) == GoKind::RangeClause)?;

    let right = range.child_by_field_name("right")?;
    if GoKind::of(right) != GoKind::Identifier {
        return None;
    }
    let ranged_name = right.utf8_text(src).ok()?;
    if !bulk_vars.contains(&ranged_name) {
        return None;
    }

    let left = range.child_by_field_name("left")?;
    let mut left_cursor = left.walk();
    let item_name = left
        .children(&mut left_cursor)
        .filter(|c| GoKind::of(*c) == GoKind::Identifier)
        .last()
        .and_then(|n| n.utf8_text(src).ok())
        .filter(|&name| name != "_")?;

    Some((ranged_name, item_name))
}

/// True for an `if item.Field == param { return ... }`-shaped (order-insensitive)
/// match, checked directly against `if_stmt` itself — `scan_for_lookup`'s own recursive
/// descent is what finds every `if_statement` in a bulk-fetch loop's body, so this only
/// needs to judge one at a time, not search a subtree itself.
fn matches_lookup_pattern(
    if_stmt: Node,
    src: &[u8],
    item_name: &str,
    param_names: &[&str],
) -> bool {
    let Some(condition) = if_stmt.child_by_field_name("condition") else {
        return false;
    };
    if !is_equality_of_item_field_and_param(condition, src, item_name, param_names) {
        return false;
    }
    let Some(consequence) = if_stmt.child_by_field_name("consequence") else {
        return false;
    };
    // `consequence` is a `block`, whose statements live one level down inside a
    // `statement_list` node (tree-sitter-go's grammar), not as its own direct named
    // children — walk the whole subtree rather than assume return sits at depth 1.
    let mut found = false;
    crate::tree_walk::walk_preorder(consequence, &mut |n| {
        if GoKind::of(n) == GoKind::ReturnStatement {
            found = true;
        }
        !found
    });
    found
}

fn is_equality_of_item_field_and_param(
    cond: Node,
    src: &[u8],
    item_name: &str,
    param_names: &[&str],
) -> bool {
    if GoKind::of(cond) != GoKind::BinaryExpression {
        return false;
    }
    let Some(operator) = cond.child_by_field_name("operator") else {
        return false;
    };
    if operator.utf8_text(src).unwrap_or("") != "==" {
        return false;
    }
    let Some(left) = cond.child_by_field_name("left") else {
        return false;
    };
    let Some(right) = cond.child_by_field_name("right") else {
        return false;
    };
    (is_item_field(left, src, item_name) && is_param_ref(right, src, param_names))
        || (is_item_field(right, src, item_name) && is_param_ref(left, src, param_names))
}

/// True for `<item_name>.Field` — a selector expression rooted at the range loop's
/// element variable.
fn is_item_field(n: Node, src: &[u8], item_name: &str) -> bool {
    GoKind::of(n) == GoKind::SelectorExpression
        && n.child_by_field_name("operand")
            .and_then(|op| op.utf8_text(src).ok())
            == Some(item_name)
}

fn is_param_ref(n: Node, src: &[u8], param_names: &[&str]) -> bool {
    GoKind::of(n) == GoKind::Identifier
        && n.utf8_text(src)
            .is_ok_and(|text| param_names.contains(&text))
}

#[cfg(test)]
#[path = "go_bulk_fetch_linear_scan_tests.rs"]
mod tests;
