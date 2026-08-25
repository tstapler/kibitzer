//! Domain types for the shared, repo-scoped architecture model — `ArchModel` and its
//! constituent types. Pure, serde-derived data; no file I/O and no tree-sitter walking
//! lives here (that's `symbol_extract.rs` and the later `build_model` orchestration
//! function). Every consumer (CLI `architecture export`, MCP query tools, LSP symbol
//! mapper, the diagram renderer) shares these types instead of reimplementing them.
//!
//! Epic 1.1 (this file) is types-only: nothing constructs these yet. `build_model`
//! (Epic 1.3) is the first real caller, hence the blanket `dead_code` allow below.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::import_graph::ImportEdge;

/// The kind of a single extracted symbol. Exhaustively matched by every consumer (the
/// diagram renderer, the LSP `SymbolKind` mapper, the MCP `kind` filter) so a new kind
/// can't silently be missed by a stringly-typed comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymbolKind {
    Type,
    Interface,
    Function,
    Method,
}

/// How deep a consumer wants to look: package/component granularity, or down to
/// individual symbols. Threaded through every consumer's "how deep" parameter (CLI
/// `--level`, MCP `level` field, diagram renderer) instead of each interface inventing
/// its own depth flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelLevel {
    Component,
    Code,
}

/// One extracted type/interface/function/method declaration.
///
/// `id` is deterministic and owner-qualified (`"{package_path}::{parent}.{name}"` for
/// methods, else `"{package_path}::{name}"`) so it's re-derivable across runs without a
/// lookup table, and so two same-named methods on different types in one package don't
/// collide.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SymbolNode {
    pub id: String,
    pub name: String,
    pub kind: SymbolKind,
    pub file: PathBuf,
    pub line: usize,
    pub exported: bool,
    /// Set for methods: the name of the owning type. `None` for types, interfaces, and
    /// free functions.
    pub parent: Option<String>,
}

/// One package/module-directory node in `ArchModel` — a path (matching `ImportGraph`'s
/// node key), the files under it, and its `SymbolNode`s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackageNode {
    pub path: String,
    pub files: Vec<PathBuf>,
    pub symbols: Vec<SymbolNode>,
}

/// What a `build_model` run excluded and why, embedded in `ArchModel` so a consumer
/// never mistakes "pruned" for "doesn't exist," and never mistakes "no supported
/// language in this file" for "no code here."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PruningSummary {
    pub include_private: bool,
    pub excluded_dirs: Vec<String>,
    pub generated_files_skipped: usize,
    pub private_symbols_skipped: usize,
    /// The `SymbolNode::id` of every symbol excluded specifically by
    /// `include_private: false` (the ids counted in `private_symbols_skipped`).
    pub pruned_symbol_ids: Vec<String>,
    /// Files that were attempted but didn't parse cleanly — a path list (not just a
    /// count) so a consumer can identify *which* files failed.
    pub files_with_parse_errors: Vec<PathBuf>,
    /// Files under scope with no recognized extension at all — distinct from a parse
    /// error, since the file was never even attempted.
    pub unsupported_language_files: usize,
    /// Denominator so a consumer can compute the unsupported fraction without a second
    /// pass over the repo.
    pub total_files_scanned: usize,
}

/// The shared, repo-scoped architecture model: packages (keyed by path) plus the import
/// edges between them. The one model all consumer interfaces (CLI export, MCP query,
/// LSP symbols, diagram) read from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchModel {
    pub repo_root: PathBuf,
    pub packages: BTreeMap<String, PackageNode>,
    pub import_edges: Vec<ImportEdge>,
    pub pruning: PruningSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_pruning() -> PruningSummary {
        PruningSummary {
            include_private: false,
            excluded_dirs: vec![],
            generated_files_skipped: 0,
            private_symbols_skipped: 0,
            pruned_symbol_ids: vec![],
            files_with_parse_errors: vec![],
            unsupported_language_files: 0,
            total_files_scanned: 0,
        }
    }

    fn empty_package(path: &str) -> PackageNode {
        PackageNode {
            path: path.to_string(),
            files: vec![],
            symbols: vec![],
        }
    }

    #[test]
    fn arch_model_serializes_packages_in_btreemap_order_not_insertion_order() {
        let mut packages = BTreeMap::new();
        packages.insert("b".to_string(), empty_package("b"));
        packages.insert("a".to_string(), empty_package("a"));

        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            pruning: empty_pruning(),
        };

        let json = serde_json::to_string(&model).expect("serializes");
        let a_pos = json.find("\"a\"").expect("contains \"a\" key");
        let b_pos = json.find("\"b\"").expect("contains \"b\" key");
        assert!(
            a_pos < b_pos,
            "expected \"a\" before \"b\" in {json}, got a_pos={a_pos} b_pos={b_pos}"
        );
    }

    #[test]
    fn symbol_node_round_trips_through_serde_without_field_loss() {
        let original = SymbolNode {
            id: "pkg/foo::Bar".to_string(),
            name: "Bar".to_string(),
            kind: SymbolKind::Type,
            file: PathBuf::from("pkg/foo/bar.go"),
            line: 12,
            exported: true,
            parent: None,
        };

        let json = serde_json::to_string(&original).expect("serializes");
        let round_tripped: SymbolNode = serde_json::from_str(&json).expect("deserializes");

        assert_eq!(original, round_tripped);
    }

    #[test]
    fn symbol_kind_serializes_as_lowercase_string() {
        let json = serde_json::to_string(&SymbolKind::Function).expect("serializes");
        assert_eq!(json, "\"function\"");
    }

    #[test]
    fn model_level_serializes_as_lowercase_string() {
        let json = serde_json::to_string(&ModelLevel::Component).expect("serializes");
        assert_eq!(json, "\"component\"");

        let json = serde_json::to_string(&ModelLevel::Code).expect("serializes");
        assert_eq!(json, "\"code\"");
    }

    #[test]
    fn import_edge_serializes_with_no_new_fields() {
        let edge = ImportEdge {
            from: "a".to_string(),
            to: "b".to_string(),
            file: PathBuf::from("a.go"),
            line: 3,
        };

        let json = serde_json::to_string(&edge).expect("serializes");
        assert_eq!(json, r#"{"from":"a","to":"b","file":"a.go","line":3}"#);
    }
}
