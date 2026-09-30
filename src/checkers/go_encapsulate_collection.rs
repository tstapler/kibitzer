use std::path::Path;

use anyhow::{Context, Result};
use tree_sitter::Node;

use crate::checker::{CheckContext, Checker, Finding, Language};
use crate::checkers::file_size::is_generated;
use crate::node_kind::GoKind;

/// Flags a getter method whose entire body is `return <receiver>.<field>`, where
/// `field`'s declared type (on the receiver's struct, resolved within the same file) is
/// a slice or map — Fowler's **Encapsulate Collection**. Handing out a live reference to
/// a mutable slice/map field lets any caller corrupt the receiver's internal state
/// without ever touching the receiver directly.
///
/// Deliberately narrow (issue #46's v1 scope): only a single-statement body of exactly
/// `return recv.field` counts — a body with any other statement is assumed to already be
/// doing its own defensive copy (or something else this check shouldn't second-guess).
/// Struct-field resolution is same-file only: a receiver type declared in another file
/// is silently skipped rather than guessed at.
pub struct EncapsulateCollectionChecker;

impl Checker for EncapsulateCollectionChecker {
    fn name(&self) -> &str {
        "go-encapsulate-collection"
    }

    fn description(&self) -> &str {
        "flags a getter that returns a slice/map field directly, handing callers a live reference to mutable internal state"
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
            .context("go-encapsulate-collection checker requires a parsed tree")?;
        let src = ctx.source.as_bytes();
        let root = tree.root_node();

        let mut findings = Vec::new();
        collect_uncopied_collection_getters(root, root, src, &mut findings);
        Ok(findings)
    }
}

inventory::submit! {
    crate::checker::CheckerFactory(|| vec![Box::new(EncapsulateCollectionChecker)])
}

fn collect_uncopied_collection_getters(node: Node, root: Node, src: &[u8], out: &mut Vec<Finding>) {
    if GoKind::of(node) == GoKind::MethodDeclaration
        && let Some(finding) = check_method(node, root, src)
    {
        out.push(finding);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_uncopied_collection_getters(child, root, src, out);
    }
}

fn check_method(method: Node, root: Node, src: &[u8]) -> Option<Finding> {
    let receiver_name = receiver_type_name(method, src)?;
    let (recv_var, field_name) = sole_return_of_receiver_field(method, src)?;
    if recv_var != receiver_var_name(method, src)? {
        return None;
    }

    let field_type = struct_field_type(root, src, receiver_name, field_name)?;
    if !matches!(GoKind::of(field_type), GoKind::SliceType | GoKind::MapType) {
        return None;
    }

    let method_name = method
        .child_by_field_name("name")
        .and_then(|n| n.utf8_text(src).ok())
        .unwrap_or("<method>");

    let (kind, copy_example) = if GoKind::of(field_type) == GoKind::SliceType {
        ("slice", format!("append([]T(nil), s.{field_name}...)"))
    } else {
        ("map", format!("maps.Clone(s.{field_name})"))
    };

    Some(Finding {
        line: method.start_position().row + 1,
        message: format!(
            "[go-encapsulate-collection] `{method_name}` returns field `{field_name}` \
             (a {kind}) directly — callers get a live reference to `{receiver_name}`'s \
             internal state and can mutate it without going through `{receiver_name}` at \
             all; return a copy (`{copy_example}`) or document that sharing is intentional"
        ),
    })
}

/// The receiver's declared variable name (`s` in `func (s *S) Items() ...`), stripped of
/// nothing — this is just the identifier itself, unlike [`receiver_type_name`] which
/// unwraps a pointer.
fn receiver_var_name<'a>(method: Node, src: &'a [u8]) -> Option<&'a str> {
    let receiver_param = sole_receiver_param(method)?;
    let name = receiver_param.child_by_field_name("name")?;
    name.utf8_text(src).ok()
}

/// The receiver's struct type name, unwrapping a pointer receiver (`*S` -> `S`).
fn receiver_type_name<'a>(method: Node, src: &'a [u8]) -> Option<&'a str> {
    let receiver_param = sole_receiver_param(method)?;
    let ty = receiver_param.child_by_field_name("type")?;
    let ty = if GoKind::of(ty) == GoKind::PointerType {
        let mut cursor = ty.walk();
        ty.children(&mut cursor)
            .find(|c| GoKind::of(*c) == GoKind::TypeIdentifier)?
    } else {
        ty
    };
    if GoKind::of(ty) != GoKind::TypeIdentifier {
        return None;
    }
    ty.utf8_text(src).ok()
}

fn sole_receiver_param(method: Node) -> Option<Node> {
    let receiver_list = method.child_by_field_name("receiver")?;
    let mut cursor = receiver_list.walk();
    let mut params = receiver_list
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::ParameterDeclaration);
    let only = params.next()?;
    if params.next().is_some() {
        return None;
    }
    Some(only)
}

/// If `method`'s body is exactly one statement, `return <ident>.<field>`, returns
/// `(ident_text, field_text)`. Anything else — an empty body, more than one statement, a
/// return of anything other than a single bare selector expression — is `None`.
fn sole_return_of_receiver_field<'a>(method: Node, src: &'a [u8]) -> Option<(&'a str, &'a str)> {
    let body = method.child_by_field_name("body")?;
    // A non-empty `block`'s statements live under an intervening `statement_list`
    // child, not directly under `block` — an empty body has no `statement_list` at
    // all, which correctly falls through to `None` below via `body`.
    let mut block_cursor = body.walk();
    let statement_list = body
        .children(&mut block_cursor)
        .find(|c| GoKind::of(*c) == GoKind::StatementList)?;
    let mut cursor = statement_list.walk();
    let mut statements = statement_list
        .children(&mut cursor)
        .filter(|c| c.is_named() && GoKind::of(*c) != GoKind::Comment);
    let only = statements.next()?;
    if statements.next().is_some() {
        return None;
    }
    if GoKind::of(only) != GoKind::ReturnStatement {
        return None;
    }
    sole_returned_selector(only, src)
}

/// If `return_stmt` returns exactly one value, and that value is a bare
/// `ident.field` selector expression, returns `(ident_text, field_text)`.
fn sole_returned_selector<'a>(return_stmt: Node, src: &'a [u8]) -> Option<(&'a str, &'a str)> {
    let mut cursor = return_stmt.walk();
    let expr_list = return_stmt
        .children(&mut cursor)
        .find(|c| GoKind::of(*c) == GoKind::ExpressionList)?;
    let mut cursor = expr_list.walk();
    let mut values = expr_list.children(&mut cursor).filter(|c| c.is_named());
    let value = values.next()?;
    if values.next().is_some() || GoKind::of(value) != GoKind::SelectorExpression {
        return None;
    }

    let operand = value.child_by_field_name("operand")?;
    let field = value.child_by_field_name("field")?;
    if GoKind::of(operand) != GoKind::Identifier {
        return None;
    }
    Some((operand.utf8_text(src).ok()?, field.utf8_text(src).ok()?))
}

/// Resolves `type_name.field_name`'s declared type node within `root`'s file — looks for
/// a top-level `type type_name struct { ... field_name Type ... }` declaration. `None`
/// if the type isn't declared in this file, isn't a struct, or has no such field —
/// deliberately not a cross-file lookup (see module doc comment).
fn struct_field_type<'a>(
    root: Node<'a>,
    src: &'a [u8],
    type_name: &str,
    field_name: &str,
) -> Option<Node<'a>> {
    let struct_ty = find_struct_type_decl(root, src, type_name)?;
    let field_list = struct_ty
        .children(&mut struct_ty.walk())
        .find(|c| GoKind::of(*c) == GoKind::FieldDeclarationList)?;

    let mut cursor = field_list.walk();
    for field_decl in field_list
        .children(&mut cursor)
        .filter(|c| GoKind::of(*c) == GoKind::FieldDeclaration)
    {
        let mut name_cursor = field_decl.walk();
        let declares_field = field_decl
            .children_by_field_name("name", &mut name_cursor)
            .any(|n| n.utf8_text(src) == Ok(field_name));
        if declares_field {
            return field_decl.child_by_field_name("type");
        }
    }
    None
}

fn find_struct_type_decl<'a>(node: Node<'a>, src: &'a [u8], type_name: &str) -> Option<Node<'a>> {
    if GoKind::of(node) == GoKind::TypeSpec {
        let name = node.child_by_field_name("name")?;
        let ty = node.child_by_field_name("type")?;
        if name.utf8_text(src) == Ok(type_name) && GoKind::of(ty) == GoKind::StructType {
            return Some(ty);
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(found) = find_struct_type_decl(child, src, type_name) {
            return Some(found);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go_findings(src: &str) -> Vec<Finding> {
        crate::test_support::check_go_source(&EncapsulateCollectionChecker, src).unwrap()
    }

    #[test]
    fn flags_a_slice_getter_returning_the_field_directly() {
        let src = "package p\n\ntype S struct {\n\titems []int\n}\n\nfunc (s *S) Items() []int {\n\treturn s.items\n}\n";
        let findings = go_findings(src);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("`Items`"));
        assert!(findings[0].message.contains("`items`"));
        assert!(findings[0].message.contains("slice"));
    }

    #[test]
    fn flags_a_map_getter_returning_the_field_directly() {
        let src = "package p\n\ntype S struct {\n\tindex map[string]int\n}\n\nfunc (s *S) Index() map[string]int {\n\treturn s.index\n}\n";
        let findings = go_findings(src);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("map"));
    }

    #[test]
    fn does_not_flag_a_value_receiver_getter_for_a_non_collection_field() {
        let src = "package p\n\ntype S struct {\n\tcount int\n}\n\nfunc (s S) Count() int {\n\treturn s.count\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn does_not_flag_a_getter_that_returns_a_defensive_copy() {
        let src = "package p\n\ntype S struct {\n\titems []int\n}\n\nfunc (s *S) Items() []int {\n\tout := append([]int(nil), s.items...)\n\treturn out\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn does_not_flag_a_getter_on_a_different_field_than_the_receiver_var() {
        let src = "package p\n\ntype S struct {\n\titems []int\n}\n\nfunc (s *S) Other(o *S) []int {\n\treturn o.items\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn does_not_flag_when_struct_type_is_unresolvable_in_this_file() {
        let src = "package p\n\nfunc (s *S) Items() []int {\n\treturn s.items\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn does_not_flag_a_free_function() {
        let src = "package p\n\nfunc Items(s *S) []int {\n\treturn s.items\n}\n";
        assert!(go_findings(src).is_empty());
    }

    #[test]
    fn generated_file_is_skipped() {
        let src = "// Code generated by mockgen. DO NOT EDIT.\npackage p\n\ntype S struct {\n\titems []int\n}\n\nfunc (s *S) Items() []int {\n\treturn s.items\n}\n";
        assert!(go_findings(src).is_empty());
    }
}
