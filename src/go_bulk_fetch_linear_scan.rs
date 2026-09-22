use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::go_call_resolution;

/// Name prefixes conventionally used for a bulk-fetch-everything call (`ListAllX`,
/// `GetAllFoo`, `FindAllBar`) — see issue #30 for the profiled real-world hotspot this
/// checker was written against: a `FindInstanceDataByID`
/// helper called `ListInstanceData()` (a full ORM scan) once per session inside a
/// reconciliation loop, then linear-scanned the result for one ID match, instead of an
/// indexed `WHERE` query — ~25% of the process's live CPU in a pprof profile.
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
            if matches!(n.kind(), "function_declaration" | "method_declaration") {
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

    let mut reported = false;
    crate::tree_walk::walk_preorder(body, &mut |n| {
        if reported {
            return false;
        }
        if n.kind() == "for_statement"
            && let Some(finding) = check_for_statement(n, src, &bulk_vars, &param_names)
        {
            findings.push(finding);
            reported = true;
            return false;
        }
        true
    });
}

/// Non-blank parameter names declared on `params` (a function/method's own
/// `parameter_list`, not its receiver) — what a loop-body comparison must reference to
/// count as "checked against a parameter" rather than some unrelated local/field.
fn collect_param_names<'a>(params: Node<'a>, src: &'a [u8]) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut cursor = params.walk();
    for decl in params
        .children(&mut cursor)
        .filter(|n| n.kind() == "parameter_declaration")
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
        if n.kind() == "short_var_declaration"
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
        .find(|c| c.kind() == "identifier")?;
    let text = bound.utf8_text(src).ok()?;
    (text != "_").then_some(text)
}

/// The callee's bare name: the identifier itself for a free-function call, or the
/// field name for a method/package-qualified call (`s.ListAllX()` → `ListAllX`).
fn callee_name<'a>(call: Node<'a>, src: &'a [u8]) -> Option<&'a str> {
    let function = call.child_by_field_name("function")?;
    let name_node = match function.kind() {
        "identifier" => function,
        "selector_expression" => function.child_by_field_name("field")?,
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

/// If `for_stmt` ranges over one of `bulk_vars` and its body contains an
/// `item.Field == param`-shaped (order-insensitive) equality check inside an `if` that
/// returns, builds the finding for it.
fn check_for_statement(
    for_stmt: Node,
    src: &[u8],
    bulk_vars: &[&str],
    param_names: &[&str],
) -> Option<Finding> {
    let (ranged_name, item_name) = range_over_bulk_var(for_stmt, src, bulk_vars)?;
    let loop_body = for_stmt.child_by_field_name("body")?;
    if !loop_body_has_lookup_match(loop_body, src, item_name, param_names) {
        return None;
    }
    Some(Finding {
        line: for_stmt.start_position().row + 1,
        message: format!(
            "ranges over `{ranged_name}` (bound from a bulk List/GetAll/FindAll fetch) and \
             returns on the first element matching a parameter — an indexed/keyed lookup \
             usually replaces fetching everything to find one row"
        ),
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
        .find(|c| c.kind() == "range_clause")?;

    let right = range.child_by_field_name("right")?;
    if right.kind() != "identifier" {
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
        .filter(|c| c.kind() == "identifier")
        .last()
        .and_then(|n| n.utf8_text(src).ok())
        .filter(|&name| name != "_")?;

    Some((ranged_name, item_name))
}

/// True when `loop_body` contains an `if item.Field == param { return ... }`-shaped
/// (order-insensitive) match anywhere inside it.
fn loop_body_has_lookup_match(
    loop_body: Node,
    src: &[u8],
    item_name: &str,
    param_names: &[&str],
) -> bool {
    let mut found = false;
    crate::tree_walk::walk_preorder(loop_body, &mut |n| {
        if found {
            return false;
        }
        if n.kind() == "if_statement" && matches_lookup_pattern(n, src, item_name, param_names) {
            found = true;
            return false;
        }
        true
    });
    found
}

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
        if n.kind() == "return_statement" {
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
    if cond.kind() != "binary_expression" {
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
    n.kind() == "selector_expression"
        && n.child_by_field_name("operand")
            .and_then(|op| op.utf8_text(src).ok())
            == Some(item_name)
}

fn is_param_ref(n: Node, src: &[u8], param_names: &[&str]) -> bool {
    n.kind() == "identifier"
        && n.utf8_text(src)
            .is_ok_and(|text| param_names.contains(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_the_profiled_stapler_squad_shape() {
        // The exact shape from issue #30: bulk List* fetch, range, compare an element
        // field against a parameter, return on match.
        let findings = check_source(
            "package main\n\
             func (s *Store) FindInstanceDataByID(id string) (*Data, error) {\n\
             \tall, err := s.ListInstanceData()\n\
             \tif err != nil {\n\
             \t\treturn nil, err\n\
             \t}\n\
             \tfor _, item := range all {\n\
             \t\tif item.ID == id {\n\
             \t\t\treturn &item, nil\n\
             \t\t}\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("`all`"));
    }

    #[test]
    fn flags_comparison_order_swapped() {
        let findings = check_source(
            "package main\n\
             func FindX(id string) (*T, error) {\n\
             \tall, err := ListAllX()\n\
             \tif err != nil { return nil, err }\n\
             \tfor _, item := range all {\n\
             \t\tif id == item.ID {\n\
             \t\t\treturn &item, nil\n\
             \t\t}\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn flags_get_all_and_find_all_prefixes() {
        for prefix in ["GetAllFoo", "FindAllBar"] {
            let findings = check_source(&format!(
                "package main\n\
                 func FindX(id string) (*T, error) {{\n\
                 \tall, err := {prefix}()\n\
                 \tif err != nil {{ return nil, err }}\n\
                 \tfor _, item := range all {{\n\
                 \t\tif item.ID == id {{\n\
                 \t\t\treturn &item, nil\n\
                 \t\t}}\n\
                 \t}}\n\
                 \treturn nil, ErrNotFound\n\
                 }}\n"
            ))
            .unwrap();
            assert_eq!(findings.len(), 1, "prefix {prefix} should flag");
        }
    }

    #[test]
    fn ignores_non_bulk_fetch_name() {
        // "Listener" starts with "List" but the next char isn't uppercase-boundary —
        // and more importantly this isn't a List/GetAll/FindAll-shaped name at all.
        let findings = check_source(
            "package main\n\
             func FindX(id string) (*T, error) {\n\
             \tall, err := loadCandidates()\n\
             \tif err != nil { return nil, err }\n\
             \tfor _, item := range all {\n\
             \t\tif item.ID == id {\n\
             \t\t\treturn &item, nil\n\
             \t\t}\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_comparison_against_non_parameter() {
        // Compares against a local, not one of the function's own parameters — not
        // the single-key lookup shape this check targets.
        let findings = check_source(
            "package main\n\
             func FindX(id string) (*T, error) {\n\
             \tall, err := ListAllX()\n\
             \tif err != nil { return nil, err }\n\
             \twant := \"fixed\"\n\
             \tfor _, item := range all {\n\
             \t\tif item.ID == want {\n\
             \t\t\treturn &item, nil\n\
             \t\t}\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_loop_with_no_early_return() {
        // A range that accumulates/aggregates over every element (no per-match early
        // return) isn't the "find one row" shape this check targets.
        let findings = check_source(
            "package main\n\
             func SumX(id string) int {\n\
             \tall, _ := ListAllX()\n\
             \ttotal := 0\n\
             \tfor _, item := range all {\n\
             \t\tif item.ID == id {\n\
             \t\t\ttotal += item.Amount\n\
             \t\t}\n\
             \t}\n\
             \treturn total\n\
             }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_inequality_comparison() {
        let findings = check_source(
            "package main\n\
             func FindX(id string) (*T, error) {\n\
             \tall, err := ListAllX()\n\
             \tif err != nil { return nil, err }\n\
             \tfor _, item := range all {\n\
             \t\tif item.ID != id {\n\
             \t\t\tcontinue\n\
             \t\t}\n\
             \t\treturn &item, nil\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_range_over_unrelated_slice() {
        let findings = check_source(
            "package main\n\
             func FindX(id string, others []T) (*T, error) {\n\
             \t_, err := ListAllX()\n\
             \tif err != nil { return nil, err }\n\
             \tfor _, item := range others {\n\
             \t\tif item.ID == id {\n\
             \t\t\treturn &item, nil\n\
             \t\t}\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }

    #[test]
    fn ignores_function_with_no_parameters() {
        let findings = check_source(
            "package main\n\
             func FindDefault() (*T, error) {\n\
             \tall, err := ListAllX()\n\
             \tif err != nil { return nil, err }\n\
             \tfor _, item := range all {\n\
             \t\tif item.ID == \"default\" {\n\
             \t\t\treturn &item, nil\n\
             \t\t}\n\
             \t}\n\
             \treturn nil, ErrNotFound\n\
             }\n",
        )
        .unwrap();
        assert!(findings.is_empty());
    }
}
