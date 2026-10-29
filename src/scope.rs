//! Intra-file scope map: which function/class a byte offset sits in, and how widely a set
//! of offsets is spread across them. Lets a per-file check weigh "N uses in one function"
//! differently from "N uses across several". Single-file only: `Spread::files` is always 1,
//! so nothing here depends on cross-file or cross-run state.

use std::path::Path;

use tree_sitter::{Node, Tree};

use crate::checker::Language;
use crate::checkers::rules::lang_config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScopeKind {
    Function,
    Class,
    Module,
}

/// Index into [`ScopeMap`]; index 0 is always the module scope.
pub(crate) type ScopeId = usize;

/// Lines are part of the shared API for other checks; `replace-magic-literal` doesn't read them.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct Scope {
    pub kind: ScopeKind,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub parent: Option<ScopeId>,
}

/// How a set of byte offsets is distributed over the file's scopes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Spread {
    pub total: usize,
    /// Distinct function scopes, plus one if any offset sits outside every function.
    pub distinct_scopes: usize,
    /// Offsets outside every function (top-level tables, class fields).
    pub module_level: usize,
    /// 1-based line span from the first to the last offset, inclusive.
    pub span_lines: usize,
    /// Always 1 in this single-file implementation.
    pub files: usize,
}

/// Test/generated classification, so callers stop re-deriving it from two places.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FileKind {
    pub test: bool,
    pub generated: bool,
}

impl FileKind {
    pub fn classify(path: &Path, source: &str) -> Self {
        Self {
            test: crate::checkers::is_test_file(path),
            generated: crate::checkers::file_size::is_generated(source),
        }
    }
}

pub(crate) struct ScopeMap {
    /// Pre-order, so sorted by `start_byte`; each scope nests inside its `parent`.
    scopes: Vec<Scope>,
    /// Byte offset of the start of each line, for offset-to-line lookups.
    line_starts: Vec<usize>,
    pub file: FileKind,
}

/// Node kinds that open a class-like scope. Functions come from the shared
/// `LangRuleConfig::function_kinds`; closures/lambdas that aren't listed there simply
/// belong to their nearest enclosing function.
fn class_kinds(lang: Language) -> &'static [&'static str] {
    match lang {
        Language::Go | Language::Bash => &[],
        Language::TypeScript | Language::Tsx | Language::JavaScript => {
            &["class_declaration", "class"]
        }
        Language::Python => &["class_definition"],
        Language::Java => &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "record_declaration",
        ],
        Language::Kotlin => &["class_declaration", "object_declaration"],
        Language::Rust => &["impl_item", "trait_item"],
    }
}

impl ScopeMap {
    pub fn build(lang: Language, tree: &Tree, path: &Path, src: &str) -> Self {
        let root = tree.root_node();
        let function_kinds = lang_config(lang).function_kinds;
        let mut map = Self {
            scopes: vec![Scope {
                kind: ScopeKind::Module,
                start_byte: root.start_byte(),
                end_byte: root.end_byte(),
                start_line: root.start_position().row + 1,
                end_line: root.end_position().row + 1,
                parent: None,
            }],
            line_starts: std::iter::once(0)
                .chain(src.match_indices('\n').map(|(i, _)| i + 1))
                .collect(),
            file: FileKind::classify(path, src),
        };
        map.collect(root, 0, function_kinds, class_kinds(lang));
        map
    }

    fn collect(&mut self, node: Node, parent: ScopeId, fns: &[&str], classes: &[&str]) {
        let kind = node.kind();
        let mut enclosing = parent;
        let scope_kind = if fns.contains(&kind) {
            Some(ScopeKind::Function)
        } else if classes.contains(&kind) {
            Some(ScopeKind::Class)
        } else {
            None
        };
        if let Some(scope_kind) = scope_kind {
            self.scopes.push(Scope {
                kind: scope_kind,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                parent: Some(parent),
            });
            enclosing = self.scopes.len() - 1;
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.collect(child, enclosing, fns, classes);
        }
    }

    #[allow(dead_code)]
    pub fn scope(&self, id: ScopeId) -> &Scope {
        &self.scopes[id]
    }

    /// Innermost scope of `kind` containing `byte`; `Module` always resolves to the root.
    pub fn enclosing(&self, byte: usize, kind: ScopeKind) -> Option<ScopeId> {
        let after = self.scopes.partition_point(|s| s.start_byte <= byte);
        let mut id = after.checked_sub(1)?;
        // The last scope starting at or before `byte` may have ended already; an
        // ancestor of it still contains `byte` (siblings never overlap).
        while self.scopes[id].end_byte <= byte && self.scopes[id].parent.is_some() {
            id = self.scopes[id].parent?;
        }
        loop {
            if self.scopes[id].kind == kind {
                return Some(id);
            }
            id = self.scopes[id].parent?;
        }
    }

    fn line_of(&self, byte: usize) -> usize {
        self.line_starts.partition_point(|&s| s <= byte)
    }

    pub fn spread(&self, bytes: &[usize]) -> Spread {
        let mut functions = std::collections::HashSet::new();
        let mut module_level = 0;
        for &b in bytes {
            match self.enclosing(b, ScopeKind::Function) {
                Some(id) => {
                    functions.insert(id);
                }
                None => module_level += 1,
            }
        }
        let lines = bytes.iter().map(|&b| self.line_of(b));
        let span_lines = match (lines.clone().min(), lines.max()) {
            (Some(lo), Some(hi)) => hi - lo + 1,
            _ => 0,
        };
        Spread {
            total: bytes.len(),
            distinct_scopes: functions.len() + usize::from(module_level > 0),
            module_level,
            span_lines,
            files: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checker::GrammarCache;

    fn map_for(lang: Language, path: &str, src: &str) -> ScopeMap {
        let tree = GrammarCache::new().parse(lang, src).unwrap();
        ScopeMap::build(lang, &tree, Path::new(path), src)
    }

    fn offset_of(src: &str, needle: &str) -> usize {
        src.find(needle).unwrap_or_else(|| panic!("no {needle}"))
    }

    fn fn_scope(map: &ScopeMap, src: &str, needle: &str) -> Option<ScopeId> {
        map.enclosing(offset_of(src, needle), ScopeKind::Function)
    }

    #[test]
    #[allow(non_snake_case)]
    fn scope_map_enclosing_should_ReturnInnermostFunction_When_ClosureNested() {
        let src =
            "fn outer() {\n    let a = 111;\n    let f = || { 222 };\n}\nfn other() { 333 }\n";
        let map = map_for(Language::Rust, "a.rs", src);
        let outer = fn_scope(&map, src, "111").unwrap();
        // closure_expression is not a function kind, so it belongs to `outer`
        assert_eq!(fn_scope(&map, src, "222"), Some(outer));
        assert_ne!(fn_scope(&map, src, "333"), Some(outer));
    }

    #[test]
    #[allow(non_snake_case)]
    fn scope_map_enclosing_should_ReturnInnermostFunction_When_FunctionNestedInFunction() {
        let src = "function outer() {\n  const a = 111;\n  function inner() { return 222; }\n  return 444;\n}\n";
        let map = map_for(Language::TypeScript, "a.ts", src);
        let outer = fn_scope(&map, src, "111").unwrap();
        let inner = fn_scope(&map, src, "222").unwrap();
        assert_ne!(inner, outer);
        assert_eq!(map.scope(inner).parent, Some(outer));
        // after the inner function ends, back in outer
        assert_eq!(fn_scope(&map, src, "444"), Some(outer));
    }

    #[test]
    #[allow(non_snake_case)]
    fn scope_map_should_TreatArrowFunctionAsFunction_When_LangIsTypeScript() {
        let src = "const top = 111;\nconst f = () => { return 222; };\n";
        let map = map_for(Language::TypeScript, "a.ts", src);
        assert_eq!(fn_scope(&map, src, "111"), None);
        let arrow = fn_scope(&map, src, "222").unwrap();
        assert_eq!(map.scope(arrow).kind, ScopeKind::Function);
    }

    #[test]
    #[allow(non_snake_case)]
    fn scope_map_should_DeriveFunctionScopes_When_LangIsGo() {
        let src = "package m\nvar top = 111\nfunc a() int { return 222 }\nfunc (r T) b() int { return 333 }\n";
        let map = map_for(Language::Go, "a.go", src);
        assert_eq!(fn_scope(&map, src, "111"), None);
        let a = fn_scope(&map, src, "222").unwrap();
        let b = fn_scope(&map, src, "333").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    #[allow(non_snake_case)]
    fn scope_map_should_DeriveFunctionAndClassScopes_When_LangIsPython() {
        let src = "class C:\n    attr = 111\n    def m(self):\n        return 222\nTOP = 333\n";
        let map = map_for(Language::Python, "a.py", src);
        assert_eq!(fn_scope(&map, src, "111"), None);
        let class = map.enclosing(offset_of(src, "111"), ScopeKind::Class);
        assert!(class.is_some());
        let m = fn_scope(&map, src, "222").unwrap();
        assert_eq!(map.scope(m).parent, class);
        assert_eq!(map.enclosing(offset_of(src, "333"), ScopeKind::Class), None);
    }

    #[test]
    #[allow(non_snake_case)]
    fn scope_map_enclosing_should_ResolveModule_When_OutsideEveryFunction() {
        let src = "package m\nvar top = 111\n";
        let map = map_for(Language::Go, "a.go", src);
        assert_eq!(
            map.enclosing(offset_of(src, "111"), ScopeKind::Module),
            Some(0)
        );
    }

    #[test]
    #[allow(non_snake_case)]
    fn spread_should_CountTopLevelUsesAsOneScope_When_AllOutsideFunctions() {
        let src = "package m\nvar a = 111\nvar b = 111\nvar c = 111\n";
        let map = map_for(Language::Go, "a.go", src);
        let bytes: Vec<usize> = src.match_indices("111").map(|(i, _)| i).collect();
        let s = map.spread(&bytes);
        assert_eq!(
            (s.total, s.distinct_scopes, s.module_level, s.files),
            (3, 1, 3, 1)
        );
        assert_eq!(s.span_lines, 3);
    }

    #[test]
    #[allow(non_snake_case)]
    fn spread_should_CountFunctionsAndModule_When_Mixed() {
        let src = "package m\nvar t = 111\nfunc a() { _ = 111 }\nfunc b() { _ = 111; _ = 111 }\n";
        let map = map_for(Language::Go, "a.go", src);
        let bytes: Vec<usize> = src.match_indices("111").map(|(i, _)| i).collect();
        let s = map.spread(&bytes);
        assert_eq!((s.total, s.distinct_scopes, s.module_level), (4, 3, 1));
    }

    #[test]
    #[allow(non_snake_case)]
    fn file_kind_should_ClassifyTestAndGenerated() {
        let tree_src = "// Code generated by x. DO NOT EDIT.\npackage m\n";
        let map = map_for(Language::Go, "a_test.go", tree_src);
        assert!(map.file.test && map.file.generated);
        let map = map_for(Language::Go, "a.go", "package m\n");
        assert!(!map.file.test && !map.file.generated);
    }
}
