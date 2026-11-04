//! Go conditional-logic smells for `SyntaxRulesChecker`: `remove-control-flag` and
//! `consolidate-conditional` (`decompose-conditional` is cross-grammar and lives beside
//! `extract-variable` in `walk_dense_expressions`). See `docs/syntax-rules.md`.

use super::*;

/// Fewest consecutive same-value `if`-returns that count as a consolidatable run.
const CONSOLIDATE_CONDITIONAL_MIN_RUN: usize = 2;

pub(super) fn walk_go_conditional_smells(
    node: Node,
    cfg: &LangRuleConfig,
    src: &[u8],
    findings: &mut Vec<Finding>,
) {
    if cfg.function_kinds.contains(&node.kind()) {
        check_control_flags(node, src, findings);
    }
    if node.kind() == cfg.block_kind {
        check_block_for_duplicate_returns(node, cfg, src, findings);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_go_conditional_smells(child, cfg, src, findings);
    }
}

// ---------------------------------------------------------------- remove-control-flag

fn check_control_flags(decl: Node, src: &[u8], findings: &mut Vec<Finding>) {
    let Some(body) = decl.child_by_field_name("body") else {
        return;
    };
    let mut decls = Vec::new();
    collect_bool_decls(body, src, &mut decls);
    for (name, ident) in &decls {
        let declared_once = decls.iter().filter(|(n, _)| n == name).count() == 1;
        if declared_once && is_loop_exit_flag(body, name, *ident, src) {
            findings.push(Finding {
                line: ident.start_position().row + 1,
                message: format!(
                    "[remove-control-flag] boolean variable `{name}` is used only as a loop-exit flag — replace with a direct `break` or `return` (Fowler's Remove Control Flag: https://refactoring.com/catalog/removeControlFlag.html)"
                ),
            });
        }
    }
}

/// Every single-name bool local in `node`'s subtree: `x := true|false`, `var x bool`,
/// `var x = true|false`. Yields the declared name's identifier node.
fn collect_bool_decls<'t>(node: Node<'t>, src: &[u8], out: &mut Vec<(String, Node<'t>)>) {
    if let Some(ident) = bool_decl_ident(node, src)
        && let Ok(name) = ident.utf8_text(src)
    {
        out.push((name.to_string(), ident));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_bool_decls(child, src, out);
    }
}

fn bool_decl_ident<'t>(node: Node<'t>, src: &[u8]) -> Option<Node<'t>> {
    match node.kind() {
        "short_var_declaration" => {
            let ident = sole_named_child(node.child_by_field_name("left")?)?;
            let value = sole_named_child(node.child_by_field_name("right")?)?;
            (ident.kind() == "identifier" && is_bool_literal(value)).then_some(ident)
        }
        "var_spec" => {
            if node
                .children_by_field_name("name", &mut node.walk())
                .count()
                != 1
            {
                return None;
            }
            let ident = node.child_by_field_name("name")?;
            let typed_bool = node
                .child_by_field_name("type")
                .is_some_and(|t| t.utf8_text(src) == Ok("bool"));
            let literal_init = node
                .child_by_field_name("value")
                .and_then(sole_named_child)
                .is_some_and(is_bool_literal);
            let untyped_or_bool = node.child_by_field_name("type").is_none() || typed_bool;
            (untyped_or_bool && (typed_bool || literal_init)).then_some(ident)
        }
        _ => None,
    }
}

fn sole_named_child(list: Node) -> Option<Node> {
    (list.named_child_count() == 1)
        .then(|| list.named_child(0))
        .flatten()
}

fn is_bool_literal(node: Node) -> bool {
    matches!(node.kind(), "true" | "false")
}

/// True when every write to `name` is a `= true|false` inside one loop and every read is
/// in that same loop's condition — so a direct `break`/`return` could replace the flag.
/// Occurrences inside a closure the declaration is not itself in disqualify the flag:
/// a captured flag (goroutine, deferred func) is not a plain loop exit.
fn is_loop_exit_flag(body: Node, name: &str, decl_ident: Node, src: &[u8]) -> bool {
    let mut occurrences = Vec::new();
    collect_named_identifiers(body, name, src, &mut occurrences);
    occurrences.retain(|o| o.id() != decl_ident.id());

    let home_closure = enclosing_closure(decl_ident);
    if occurrences
        .iter()
        .any(|o| enclosing_closure(*o) != home_closure)
    {
        return false;
    }

    let (writes, reads): (Vec<Node>, Vec<Node>) = occurrences
        .into_iter()
        .partition(|o| is_bool_literal_write(*o));
    if writes.is_empty() || reads.is_empty() {
        return false;
    }

    let mut candidates = enclosing_loops_with_condition_containing(reads[0]);
    for read in &reads[1..] {
        let loops = enclosing_loops_with_condition_containing(*read);
        candidates.retain(|c| loops.iter().any(|l| l.id() == c.id()));
    }
    // One written value only: a flag reset and re-set each iteration (`again = false` ...
    // `again = true`) is a repeat-until-stable idiom, not an early exit.
    let single_value = writes
        .iter()
        .filter_map(|w| literal_written(*w, src))
        .collect::<std::collections::HashSet<_>>()
        .len()
        == 1;
    single_value
        && candidates.iter().any(|l| {
            decl_ident.end_byte() <= l.start_byte()
                && writes.iter().all(|w| {
                    contains(*l, *w)
                        && !inside_switch_or_select(*w, *l)
                        && exits_iteration_after(*w, *l)
                })
        })
}

fn literal_written<'a>(ident: Node, src: &'a [u8]) -> Option<&'a str> {
    let assign = ident.parent()?.parent()?;
    assign
        .child_by_field_name("right")
        .and_then(sole_named_child)?
        .utf8_text(src)
        .ok()
}

/// True when nothing runs after the flag write in its iteration except a `break`/`continue`
/// — otherwise a direct `break` would skip code the flag version still executes.
fn exits_iteration_after(write_ident: Node, loop_node: Node) -> bool {
    let Some(mut cur) = write_ident
        .parent()
        .and_then(|p| p.parent())
        .filter(|a| a.kind() == "assignment_statement")
    else {
        return false;
    };
    let mut first = true;
    while cur.id() != loop_node.id() {
        let Some(parent) = cur.parent() else {
            return false;
        };
        if matches!(parent.kind(), "block" | "statement_list") {
            let mut later = Vec::new();
            let mut sib = cur.next_named_sibling();
            while let Some(n) = sib {
                if !is_comment(n) {
                    later.push(n);
                }
                sib = n.next_named_sibling();
            }
            let only_exit = later.len() == 1
                && matches!(later[0].kind(), "break_statement" | "continue_statement");
            if !(later.is_empty() || (first && only_exit)) {
                return false;
            }
            if first && only_exit {
                return true;
            }
        }
        first = false;
        cur = parent;
    }
    true
}

/// A write inside a `switch`/`select` nested in the loop: a bare `break` there only leaves
/// the switch/select, so "replace the flag with `break`" would be wrong advice.
fn inside_switch_or_select(node: Node, loop_node: Node) -> bool {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.id() == loop_node.id() {
            return false;
        }
        if matches!(
            n.kind(),
            "expression_switch_statement" | "type_switch_statement" | "select_statement"
        ) {
            return true;
        }
        cur = n.parent();
    }
    false
}

fn collect_named_identifiers<'t>(node: Node<'t>, name: &str, src: &[u8], out: &mut Vec<Node<'t>>) {
    if node.kind() == "identifier" && node.utf8_text(src) == Ok(name) {
        out.push(node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_named_identifiers(child, name, src, out);
    }
}

fn enclosing_closure(node: Node) -> Option<usize> {
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "func_literal" {
            return Some(n.id());
        }
        cur = n.parent();
    }
    None
}

/// `x = true` / `x = false` as a whole assignment statement (single target, single value).
fn is_bool_literal_write(ident: Node) -> bool {
    let Some(left) = ident.parent().filter(|p| p.kind() == "expression_list") else {
        return false;
    };
    let Some(assign) = left.parent().filter(|p| p.kind() == "assignment_statement") else {
        return false;
    };
    left.named_child_count() == 1
        && assign.child_by_field_name("left") == Some(left)
        && assign
            .child_by_field_name("operator")
            .is_some_and(|op| op.kind() == "=")
        && assign
            .child_by_field_name("right")
            .and_then(sole_named_child)
            .is_some_and(is_bool_literal)
}

fn enclosing_loops_with_condition_containing(node: Node) -> Vec<Node> {
    let mut loops = Vec::new();
    let mut cur = node.parent();
    while let Some(n) = cur {
        if n.kind() == "for_statement" && loop_condition(n).is_some_and(|cond| contains(cond, node))
        {
            loops.push(n);
        }
        cur = n.parent();
    }
    loops
}

/// A `for` loop's condition: `for cond {` carries it as a bare child, `for i; cond; post {`
/// under `for_clause`'s `condition` field. Range loops have none.
fn loop_condition(for_stmt: Node) -> Option<Node> {
    let body_id = for_stmt.child_by_field_name("body").map(|b| b.id());
    let mut cursor = for_stmt.walk();
    for child in for_stmt.named_children(&mut cursor) {
        match child.kind() {
            "for_clause" => return child.child_by_field_name("condition"),
            "range_clause" => return None,
            "comment" => {}
            _ if Some(child.id()) != body_id => return Some(child),
            _ => {}
        }
    }
    None
}

fn is_comment(node: Node) -> bool {
    node.kind() == "comment"
}

fn contains(outer: Node, inner: Node) -> bool {
    outer.start_byte() <= inner.start_byte() && inner.end_byte() <= outer.end_byte()
}

// ------------------------------------------------------------ consolidate-conditional

fn check_block_for_duplicate_returns(
    block: Node,
    cfg: &LangRuleConfig,
    src: &[u8],
    findings: &mut Vec<Finding>,
) {
    let container = (cfg.statement_container)(block);
    let mut run: Vec<(Node, String)> = Vec::new();
    let mut cursor = container.walk();
    let mut after_comment = false;
    for stmt in container.named_children(&mut cursor) {
        if is_comment(stmt) {
            after_comment = true;
            continue;
        }
        let candidate = single_return_if(stmt, cfg, src).filter(|_| !after_comment);
        after_comment = false;
        match candidate {
            Some(value) if run.last().is_some_and(|(_, v)| *v == value) => run.push((stmt, value)),
            Some(value) => {
                flush_run(&mut run, findings);
                run.push((stmt, value));
            }
            None => flush_run(&mut run, findings),
        }
    }
    flush_run(&mut run, findings);
}

fn flush_run(run: &mut Vec<(Node, String)>, findings: &mut Vec<Finding>) {
    if run.len() >= CONSOLIDATE_CONDITIONAL_MIN_RUN {
        let (first, value) = &run[0];
        let shown = if value.is_empty() {
            "(no value)"
        } else {
            value
        };
        findings.push(Finding {
            line: first.start_position().row + 1,
            message: format!(
                "[consolidate-conditional] {} consecutive if statements each return `{shown}` — combine into a single condition (Fowler's Consolidate Conditional Expression: https://refactoring.com/catalog/consolidateConditionalExpression.html)",
                run.len()
            ),
        });
    }
    run.clear();
}

/// The normalized return-value text when `stmt` is `if cond { return V }` with no `else`
/// and no init statement, else `None`. Also `None` (breaking any run) when the condition
/// is commented (see `has_comment`), calls a function (merging reorders side effects) or shares an identifier with the
/// returned value (the `if err != nil { return err }` guard-clause idiom).
fn single_return_if(stmt: Node, cfg: &LangRuleConfig, src: &[u8]) -> Option<String> {
    if stmt.kind() != cfg.if_kind
        || stmt.child_by_field_name("alternative").is_some()
        || stmt.child_by_field_name("initializer").is_some()
    {
        return None;
    }
    if has_comment(stmt) {
        return None;
    }
    let condition = stmt.child_by_field_name("condition")?;
    let consequence = stmt.child_by_field_name("consequence")?;
    let container = (cfg.statement_container)(consequence);
    let mut cursor = container.walk();
    let mut stmts = container
        .named_children(&mut cursor)
        .filter(|c| !is_comment(*c));
    let ret = stmts.next().filter(|s| s.kind() == cfg.return_kind)?;
    if stmts.next().is_some() || contains_call(condition, cfg) {
        return None;
    }
    let value = ret.named_child(0);
    let mut cond_ids = HashMap::new();
    collect_identifiers(condition, src, &mut cond_ids);
    let mut ret_ids = HashMap::new();
    if let Some(v) = value {
        collect_identifiers(v, src, &mut ret_ids);
    }
    if ret_ids.keys().any(|k| cond_ids.contains_key(k)) {
        return None;
    }
    let text = value.and_then(|v| v.utf8_text(src).ok()).unwrap_or("");
    Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A commented guard documents its own reason, so merging it would lose that explanation.
fn has_comment(node: Node) -> bool {
    let mut cursor = node.walk();
    is_comment(node) || node.children(&mut cursor).any(has_comment)
}

fn contains_call(node: Node, cfg: &LangRuleConfig) -> bool {
    if cfg.call_kinds.contains(&node.kind()) {
        return true;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .any(|child| contains_call(child, cfg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(body: &str, rule: &str) -> Vec<Finding> {
        let src = format!("package m\nfunc f() {{\n{body}\n}}\n");
        let tree = crate::test_support::parse_go(&src).expect("parse");
        let cfg = lang_config(Language::Go);
        let mut out = Vec::new();
        walk_go_conditional_smells(tree.root_node(), &cfg, src.as_bytes(), &mut out);
        out.retain(|f| f.message.contains(&format!("[{rule}]")));
        out
    }

    fn flag(body: &str) -> Vec<Finding> {
        findings(body, "remove-control-flag")
    }

    fn consolidate(body: &str) -> Vec<Finding> {
        findings(body, "consolidate-conditional")
    }

    #[test]
    fn remove_control_flag_should_fire_when_bool_used_only_as_loop_exit_flag() {
        let f = flag("done := false\nfor !done {\n\tif x() {\n\t\tdone = true\n\t}\n}");
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 3);
        assert!(f[0].message.contains("`done`"));
    }

    #[test]
    fn remove_control_flag_should_fire_for_var_declared_flag_and_for_clause_condition() {
        let f = flag("var found bool\nfor i := 0; i < n && !found; i++ {\n\tfound = true\n}");
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn remove_control_flag_should_fire_when_nested_loop_sets_flag_read_by_outer_condition() {
        let f = flag(
            "found := false\nfor !found {\n\tfor j := 0; j < 3; j++ {\n\t\tfound = true\n\t}\n}",
        );
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_flag_read_after_loop() {
        let f = flag("done := false\nfor !done {\n\tdone = true\n}\nif done {\n\tg()\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_flag_read_inside_loop_body() {
        let f = flag("done := false\nfor !done {\n\tif done {\n\t\tbreak\n\t}\n\tdone = true\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_flag_is_reset_and_set_each_iteration() {
        let f = flag(
            "again := true\nfor again {\n\tagain = false\n\tif x() {\n\t\tagain = true\n\t}\n}",
        );
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_code_runs_after_the_write() {
        let f = flag("done := false\nfor !done {\n\tif x() {\n\t\tdone = true\n\t}\n\tg()\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_fire_when_write_is_followed_only_by_continue() {
        let f = flag(
            "done := false\nfor !done {\n\tif x() {\n\t\tdone = true\n\t\tcontinue\n\t}\n\tg()\n}",
        );
        assert_eq!(f.len(), 1);
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_non_bool_assigned() {
        let f = flag("count := 0\nfor count < 3 {\n\tcount = 1\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_written_from_expression() {
        let f = flag("ok := false\nfor !ok {\n\tok = check()\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_flag_set_inside_select_or_switch() {
        let f = flag(
            "done := false\nfor !done {\n\tselect {\n\tcase <-c:\n\t\tg()\n\tdefault:\n\t\tdone = true\n\t}\n}",
        );
        assert!(f.is_empty());
        let f =
            flag("done := false\nfor !done {\n\tswitch x {\n\tcase 1:\n\t\tdone = true\n\t}\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn remove_control_flag_should_not_fire_when_captured_by_closure() {
        let f = flag("done := false\ngo func() {\n\tdone = true\n}()\nfor !done {\n\tg()\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn consolidate_conditional_should_fire_when_two_consecutive_if_return_same_value() {
        let f = consolidate(
            "if x > 10 {\n\treturn true\n}\nif x < -10 {\n\treturn true\n}\nreturn false",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].line, 3);
        assert!(f[0].message.contains("2 consecutive"));
    }

    #[test]
    fn consolidate_conditional_should_report_whole_run_of_three_once() {
        let f = consolidate(
            "if a {\n\treturn nil\n}\nif b {\n\treturn nil\n}\nif c {\n\treturn nil\n}\nreturn x",
        );
        assert_eq!(f.len(), 1);
        assert!(f[0].message.contains("3 consecutive"));
    }

    #[test]
    fn consolidate_conditional_should_not_fire_when_condition_contains_call_expression() {
        let f = consolidate("if ok(x) {\n\treturn true\n}\nif ok(y) {\n\treturn true\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn consolidate_conditional_should_not_fire_when_condition_references_return_variable() {
        let f = consolidate(
            "if err != nil {\n\treturn nil, err\n}\nif err != nil {\n\treturn nil, err\n}",
        );
        assert!(f.is_empty());
    }

    #[test]
    fn consolidate_conditional_should_not_fire_when_return_values_differ() {
        let f = consolidate("if a {\n\treturn 1\n}\nif b {\n\treturn 2\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn consolidate_conditional_should_not_fire_when_intervening_statement_breaks_run() {
        let f = consolidate("if a {\n\treturn true\n}\nx := 1\nif b {\n\treturn true\n}");
        assert!(f.is_empty());
    }

    #[test]
    fn consolidate_conditional_should_not_fire_when_guards_are_commented() {
        let inside =
            consolidate("if a {\n\t// a is special\n\treturn true\n}\nif b {\n\treturn true\n}");
        let before =
            consolidate("if a {\n\treturn true\n}\n// b is special\nif b {\n\treturn true\n}");
        assert!(inside.is_empty() && before.is_empty());
    }

    #[test]
    fn consolidate_conditional_should_not_fire_with_else_or_init_statement() {
        let f = consolidate(
            "if a {\n\treturn true\n} else {\n\tg()\n}\nif b {\n\treturn true\n}\nif v := h(); v > 1 {\n\treturn true\n}",
        );
        assert!(f.is_empty());
    }
}
