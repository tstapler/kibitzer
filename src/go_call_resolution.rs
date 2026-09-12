use std::path::{Path, PathBuf};

use tree_sitter::{Node, Tree};

/// Resolves a same-module, package-qualified Go function call (`pkg.Func(...)`) to its
/// real declaration on disk and reports whether its last return type is literally
/// `error` — used by `go_ignored_error.rs` to avoid flagging a discarded non-error last
/// return value from a real, resolvable call, without needing any actual type-checker.
/// Deliberately narrow: only handles free functions reached via an import alias bound
/// to a path under the current file's own module (per `go.mod`) — a method call on a
/// local variable (`m.LoadOrStore(...)`), or any call into `vendor/`/stdlib/an external
/// module, is out of scope and always resolves to `None` (unresolved). Every failure
/// mode returns `None` rather than guessing, so a caller that only suppresses on
/// `Some(false)` never gets a false suppression from a botched resolution.
/// Walks up from `file` looking for the nearest ancestor directory containing `go.mod`
/// — the Go module root.
pub fn find_module_root(file: &Path) -> Option<PathBuf> {
    let mut dir = file.parent()?;
    loop {
        if dir.join("go.mod").is_file() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}

/// Extracts the `module` directive's path from a `go.mod`'s contents — the first
/// non-blank line starting with `module `, per the `go.mod` file format. Ignores
/// everything else (`go` version, `require`/`replace` blocks): only the module's own
/// import-path prefix is needed to map an import path back to a local directory.
pub fn parse_module_path(go_mod_src: &str) -> Option<String> {
    go_mod_src
        .lines()
        .find_map(|line| line.trim().strip_prefix("module "))
        .map(|rest| rest.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// One `import_spec`'s binding: the identifier code in this file uses to refer to the
/// imported package, and the import path it resolves to. Blank (`_`) and dot (`.`)
/// imports are never returned — a blank import has no usable alias, and a dot import
/// makes every reference ambiguous without real name resolution.
struct ImportBinding {
    alias: String,
    import_path: String,
}

/// Walks the file's `import_spec` nodes (both `import "path"` and grouped `import
/// (...)` forms parse identically at this node kind) collecting alias -> import-path
/// bindings. The alias is the `name` field's text when present, otherwise Go's default
/// convention: the import path's last `/`-separated segment.
fn parse_file_imports(tree: &Tree, src: &[u8]) -> Vec<ImportBinding> {
    let mut out = Vec::new();
    collect_import_specs(tree.root_node(), src, &mut out);
    out
}

fn collect_import_specs(node: Node, src: &[u8], out: &mut Vec<ImportBinding>) {
    if node.kind() == "import_spec" {
        if let Some(binding) = import_binding(node, src) {
            out.push(binding);
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_import_specs(child, src, out);
    }
}

fn import_binding(spec: Node, src: &[u8]) -> Option<ImportBinding> {
    let path_node = spec.child_by_field_name("path")?;
    let raw = path_node.utf8_text(src).ok()?;
    let import_path = raw.trim_matches(|c| c == '"' || c == '`').to_string();
    if import_path.is_empty() {
        return None;
    }

    let alias = match spec.child_by_field_name("name") {
        Some(name) if name.kind() == "package_identifier" => name.utf8_text(src).ok()?.to_string(),
        Some(_) => return None, // blank_identifier or dot import — no usable/unambiguous alias
        None => import_path.rsplit('/').next()?.to_string(),
    };
    Some(ImportBinding { alias, import_path })
}

/// A resolved Go module: the directory `go.mod` lives in, plus the module's own
/// import-path prefix (its `module` directive) — everything needed to map an import
/// path back to a local directory without a real Go toolchain.
pub struct GoModule<'a> {
    pub root: &'a Path,
    pub path: &'a str,
}

/// Resolves `pkg.func_name`'s last return type, when `pkg` is a package-qualified
/// import alias whose import path lives under `module`. Returns `None` for anything
/// unresolvable: the alias isn't bound to an import in this file, the import isn't
/// under this module (stdlib, vendor, or another module entirely), the target
/// directory/files can't be read, or no matching top-level function declaration is
/// found.
pub fn resolve_last_return_is_error(
    module: &GoModule,
    tree: &Tree,
    src: &[u8],
    pkg_alias: &str,
    func_name: &str,
) -> Option<bool> {
    let bindings = parse_file_imports(tree, src);
    let import_path = bindings
        .iter()
        .find(|b| b.alias == pkg_alias)
        .map(|b| b.import_path.as_str())?;

    let rel = import_path
        .strip_prefix(module.path)?
        .trim_start_matches('/');
    let pkg_dir = module.root.join(rel);
    if !pkg_dir.is_dir() {
        return None;
    }

    let entries = std::fs::read_dir(&pkg_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        let is_go_file = path.extension().is_some_and(|e| e == "go")
            && !path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with("_test.go"));
        if !is_go_file {
            continue;
        }
        let Ok(file_src) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(last_is_error) = find_function_last_return_is_error(&file_src, func_name) {
            return Some(last_is_error);
        }
    }
    None
}

fn find_function_last_return_is_error(file_src: &str, func_name: &str) -> Option<bool> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    let tree = parser.parse(file_src, None)?;
    let src = file_src.as_bytes();
    find_top_level_function(tree.root_node(), src, func_name)
}

fn find_top_level_function(root: Node, src: &[u8], func_name: &str) -> Option<bool> {
    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        if child.kind() != "function_declaration" {
            continue;
        }
        let Some(name) = child.child_by_field_name("name") else {
            continue;
        };
        if name.utf8_text(src) != Ok(func_name) {
            continue;
        }
        return Some(last_result_type_is_error(child, src));
    }
    None
}

/// A function with no `result` field at all (no return values) can't discard a
/// last-value error either, but that shape never reaches this function — the caller
/// already confirmed the call site's LHS has 2+ names, which requires 2+ return
/// values, so `result` is always present here in practice; treat a missing field as
/// "not error" defensively rather than panicking.
fn last_result_type_is_error(func_decl: Node, src: &[u8]) -> bool {
    let Some(result) = func_decl.child_by_field_name("result") else {
        return false;
    };
    if result.kind() != "parameter_list" {
        // A single unnamed return type with no parens, e.g. `func f() error` (only
        // possible for a single-value return, which the caller's 2+-name LHS check
        // already rules out for the shapes this resolver is used against) — no
        // multi-value list to inspect.
        return result.utf8_text(src) == Ok("error");
    }
    let mut cursor = result.walk();
    let declarations: Vec<Node> = result
        .named_children(&mut cursor)
        .filter(|n| n.kind() == "parameter_declaration")
        .collect();
    let Some(last) = declarations.last() else {
        return false;
    };
    last.child_by_field_name("type")
        .and_then(|t| t.utf8_text(src).ok())
        == Some("error")
}
