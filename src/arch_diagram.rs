//! `kibitzer architecture diagram` — renders the shared `ArchModel` as a text tree plus a
//! Mermaid `graph TD` diagram, inspired by (but explicitly not conformant to) C4 notation
//! (Story 2.2.1). Real Mermaid `C4Component`/`C4Dynamic` notation was rejected per the
//! plan's Pattern Decisions table: GitHub's built-in Mermaid renderer doesn't support
//! Mermaid's C4 extension, which would defeat this diagram's PR-paste purpose.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::ValueEnum;

use crate::arch_model::{ArchModel, ModelLevel, PruneConfig, SymbolKind, build_model};
use crate::check::walk_and_collect_files;
use crate::config::find_config;
use crate::import_graph;
use crate::mermaid::slugify;

/// Diagram granularity — the CLI-facing mirror of `arch_model::ModelLevel`, kept as its own
/// `clap::ValueEnum` type rather than deriving `ValueEnum` on `ModelLevel` itself, since
/// this phase is scoped to leave `arch_model.rs` untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DiagramLevel {
    Component,
    Code,
}

impl From<DiagramLevel> for ModelLevel {
    fn from(level: DiagramLevel) -> Self {
        match level {
            DiagramLevel::Component => ModelLevel::Component,
            DiagramLevel::Code => ModelLevel::Code,
        }
    }
}

/// Mirrors `mermaid.rs::MAX_NODES` — past this many nodes a Mermaid diagram stops being
/// readable, so `render_component_diagram` falls back to a text note instead.
const MAX_NODES: usize = 150;

const DISCLAIMER_COMMENT: &str = "# Component/Code diagram — inspired by C4, not a standards-conformant C4 Context/Container diagram";
const DISCLAIMER_MERMAID: &str =
    "%% inspired by C4 — not a standards-conformant C4 Context/Container diagram";

fn symbol_display_name(symbol: &crate::arch_model::SymbolNode) -> String {
    match &symbol.parent {
        Some(parent) => format!("{parent}.{}", symbol.name),
        None => symbol.name.clone(),
    }
}

fn symbol_kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::Type => "type",
        SymbolKind::Interface => "interface",
        SymbolKind::Function => "function",
        SymbolKind::Method => "method",
    }
}

/// Renders a plain-text, always-full-detail package → symbol tree — the accessible-text
/// equivalent of `render_component_diagram`'s Mermaid output (which has no reliable
/// accessible-text form of its own). Deliberately ignores `_level`: unlike the Mermaid
/// diagram, whose granularity is gated by `--level`, the text tree always lists every
/// symbol so it's never *less* informative than the diagram it accompanies (the
/// accessibility rationale behind Story 2.2.1's "always includes a text-tree section" AC).
pub fn render_text_tree(model: &ArchModel, _level: ModelLevel) -> String {
    let mut out = String::from(DISCLAIMER_COMMENT);
    out.push('\n');
    for (path, pkg) in &model.packages {
        out.push_str(path);
        out.push('\n');
        for symbol in &pkg.symbols {
            out.push_str(&format!(
                "  - {} {}\n",
                symbol_kind_label(symbol.kind),
                symbol_display_name(symbol)
            ));
        }
    }
    out
}

/// Renders `model` as a Mermaid `graph TD` diagram with C4-*like* visual grouping
/// (`subgraph` per package, only at `ModelLevel::Code` — `Component` level renders
/// package-to-package boxes with no symbol detail, per Story 2.2.1 AC4). Falls back to a
/// text-only cap note, mirroring `mermaid.rs`'s `MAX_NODES` pattern, once the node count
/// (component count at `Component` level, symbol count at `Code` level) exceeds
/// [`MAX_NODES`] — the returned string in that case does not start with `"graph TD"`, so
/// callers can tell diagram output from cap-note output without a separate flag.
pub fn render_component_diagram(model: &ArchModel, level: ModelLevel) -> String {
    let node_count = match level {
        ModelLevel::Component => model.packages.len(),
        ModelLevel::Code => model.packages.values().map(|p| p.symbols.len()).sum(),
    };
    if node_count > MAX_NODES {
        return format!(
            "{node_count} nodes, over the {MAX_NODES}-node diagram cap — pass a narrower \
             `--scope` to render a subgraph instead"
        );
    }

    let mut out = String::from("graph TD\n");
    out.push_str(DISCLAIMER_MERMAID);
    out.push('\n');

    for (path, pkg) in &model.packages {
        let pkg_id = slugify(path);
        if level == ModelLevel::Code && !pkg.symbols.is_empty() {
            out.push_str(&format!("    subgraph {pkg_id}[\"{path}\"]\n"));
            for symbol in &pkg.symbols {
                let sym_name = symbol_display_name(symbol);
                let sym_id = slugify(&format!("{path}::{sym_name}"));
                out.push_str(&format!("        {sym_id}[\"{sym_name}\"]\n"));
            }
            out.push_str("    end\n");
        } else {
            out.push_str(&format!("    {pkg_id}[\"{path}\"]\n"));
        }
    }

    for edge in &model.import_edges {
        let from_id = slugify(&edge.from);
        let to_id = slugify(&edge.to);
        out.push_str(&format!("    {from_id} --> {to_id}\n"));
    }

    out
}

/// Walks `repo_root`, resolves the `ImportGraph`, and reads every file's contents that
/// `read_to_string` succeeds on (binary/non-UTF8 files are silently skipped, not fatal).
fn collect_files(
    repo_root: &std::path::Path,
) -> Result<(import_graph::ImportGraph, Vec<(PathBuf, String)>)> {
    let all_files = walk_and_collect_files(repo_root)
        .with_context(|| format!("walking {}", repo_root.display()))?;

    let graph = import_graph::build(repo_root, &all_files)
        .with_context(|| format!("building import graph for {}", repo_root.display()))?;

    let files: Vec<(PathBuf, String)> = all_files
        .into_iter()
        .filter_map(|f| std::fs::read_to_string(&f).ok().map(|s| (f, s)))
        .collect();

    Ok((graph, files))
}

/// Runs `kibitzer architecture diagram`: builds the model, renders the text tree +
/// Mermaid diagram, and writes the combined output to `--out` or stdout.
pub fn run_diagram(
    path: PathBuf,
    scope: Option<String>,
    level: DiagramLevel,
    out: Option<PathBuf>,
) -> Result<ExitCode> {
    let repo_root = match find_config(&path)? {
        Some((_, root)) => root,
        None => path.clone(),
    };

    let (graph, files) = collect_files(&repo_root)?;
    let model = build_model(&repo_root, &files, &graph, &PruneConfig::default())?;

    let model = match &scope {
        // Always filter with `ModelLevel::Code` here — `--level` gates only the Mermaid
        // diagram's granularity (via the `level` argument passed to the render functions
        // below), not whether scope-filtered symbol data is even available to
        // `render_text_tree`, which always wants full detail.
        Some(s) => model.filtered(std::slice::from_ref(s), ModelLevel::Code),
        None => model,
    };

    let model_level: ModelLevel = level.into();
    let text_tree = render_text_tree(&model, model_level);
    let diagram = render_component_diagram(&model, model_level);

    let rendered = if diagram.starts_with("graph TD") {
        format!("{text_tree}\n```mermaid\n{diagram}```\n")
    } else {
        format!("{text_tree}\n{diagram}\n")
    };

    match out {
        Some(out_path) => {
            std::fs::write(&out_path, &rendered)
                .with_context(|| format!("writing {}", out_path.display()))?;
            println!("[kibitzer] wrote {}", out_path.display());
        }
        None => {
            print!("{rendered}");
        }
    }

    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{PackageNode, PruningSummary, SymbolNode};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-arch-diagram-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture(dir: &std::path::Path, rel: &str, contents: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn kibitzer_bin_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            })
            .join("kibitzer")
    }

    fn run_kibitzer(args: &[&str]) -> std::process::Output {
        std::process::Command::new(kibitzer_bin_path())
            .args(args)
            .output()
            .expect("kibitzer binary runs (run `cargo build` first if this fails)")
    }

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

    fn symbol(name: &str, kind: SymbolKind, parent: Option<&str>) -> SymbolNode {
        SymbolNode {
            id: format!("pkg::{name}"),
            name: name.to_string(),
            kind,
            file: PathBuf::from("pkg/f"),
            line: 1,
            exported: true,
            parent: parent.map(str::to_string),
        }
    }

    fn model_with_one_package_two_symbols() -> ArchModel {
        let mut pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols: vec![
                symbol("Widget", SymbolKind::Type, None),
                symbol("Do", SymbolKind::Method, Some("Widget")),
            ],
        };
        pkg.symbols[1].id = "pkg::Widget.Do".to_string();

        let mut packages = BTreeMap::new();
        packages.insert("pkg".to_string(), pkg);

        ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            pruning: empty_pruning(),
        }
    }

    // --- Story 2.2.1 AC2: render_text_tree ---

    #[test]
    fn render_text_tree_lists_every_package_and_symbol_at_code_level() {
        let model = model_with_one_package_two_symbols();
        let tree = render_text_tree(&model, ModelLevel::Code);

        let mut lines = tree.lines();
        assert_eq!(lines.next().unwrap(), DISCLAIMER_COMMENT);

        assert!(tree.contains("pkg"));
        assert!(tree.contains("Widget"));
        assert!(tree.contains("Widget.Do"));
    }

    #[test]
    fn render_text_tree_still_lists_symbols_at_component_level() {
        // The text tree is the accessible-text fallback for the Mermaid diagram, so it's
        // deliberately always full-detail, unlike render_component_diagram.
        let model = model_with_one_package_two_symbols();
        let tree = render_text_tree(&model, ModelLevel::Component);

        assert!(tree.contains("Widget"));
        assert!(tree.contains("Widget.Do"));
    }

    // --- Story 2.2.1 AC4: render_component_diagram level gating ---

    #[test]
    fn render_component_diagram_omits_symbol_names_at_component_level() {
        let model = model_with_one_package_two_symbols();
        let diagram = render_component_diagram(&model, ModelLevel::Component);

        assert!(diagram.starts_with("graph TD\n"));
        assert!(!diagram.contains("Widget"));
        assert!(!diagram.contains("subgraph"));
    }

    #[test]
    fn render_component_diagram_nests_symbols_in_subgraph_at_code_level() {
        let model = model_with_one_package_two_symbols();
        let diagram = render_component_diagram(&model, ModelLevel::Code);

        assert!(diagram.contains("subgraph"));
        assert!(diagram.contains("Widget"));
        assert!(diagram.contains("Widget.Do"));
    }

    // --- Story 2.2.1 AC2 (inline disclaimer): render_component_diagram ---

    #[test]
    fn render_component_diagram_places_disclaimer_immediately_after_graph_td() {
        let model = model_with_one_package_two_symbols();
        let diagram = render_component_diagram(&model, ModelLevel::Component);

        let mut lines = diagram.lines();
        assert_eq!(lines.next().unwrap(), "graph TD");
        assert_eq!(lines.next().unwrap(), DISCLAIMER_MERMAID);
    }

    // --- Story 2.2.1 AC5: node cap fallback ---

    #[test]
    fn render_component_diagram_falls_back_to_text_tree_note_over_node_cap() {
        let mut packages = BTreeMap::new();
        for i in 0..200 {
            packages.insert(
                format!("pkg{i}"),
                PackageNode {
                    path: format!("pkg{i}"),
                    files: vec![],
                    symbols: vec![],
                },
            );
        }
        let model = ArchModel {
            repo_root: PathBuf::from("/repo"),
            packages,
            import_edges: vec![],
            pruning: empty_pruning(),
        };

        let diagram = render_component_diagram(&model, ModelLevel::Component);
        assert!(!diagram.starts_with("graph TD"));
        assert!(diagram.contains("over the"));
        assert!(diagram.contains("200"));

        // The text-tree section is unaffected by the Mermaid cap — every package still
        // appears there in full.
        let tree = render_text_tree(&model, ModelLevel::Component);
        for i in 0..200 {
            assert!(tree.contains(&format!("pkg{i}")));
        }
    }

    // --- Story 2.2.1 AC1: --help disclaimer ---

    #[test]
    fn diagram_cli_help_contains_not_standards_conformant_c4_substring() {
        let output = run_kibitzer(&["architecture", "diagram", "--help"]);
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("not a standards-conformant C4"),
            "expected --help output to contain the disclaimer substring, got {stdout:?}"
        );
    }

    // --- Story 2.2.1 AC3 + AC6: text-tree + Mermaid always present, --out writes to file ---

    #[test]
    fn diagram_cli_writes_text_tree_and_mermaid_to_out_file() {
        let dir = tmp_dir("cli-out-file");
        write_fixture(
            &dir,
            "pkg/a.go",
            "package pkg\n\ntype Widget struct{}\n\nfunc (w Widget) Do() {}\n",
        );
        let out = dir.join("diagram.md");

        let output = run_kibitzer(&[
            "architecture",
            "diagram",
            "--path",
            dir.to_str().unwrap(),
            "--level",
            "code",
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(output.status.success());

        let contents = std::fs::read_to_string(&out).unwrap();
        assert!(contents.starts_with(DISCLAIMER_COMMENT));
        assert!(contents.contains("```mermaid"));
        assert!(contents.contains(DISCLAIMER_MERMAID));
        assert!(contents.contains("Widget"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diagram_cli_default_level_is_component_and_stdout_contains_text_tree_and_mermaid() {
        let dir = tmp_dir("cli-stdout-default");
        write_fixture(
            &dir,
            "pkg/a.go",
            "package pkg\n\ntype Widget struct{}\n\nfunc (w Widget) Do() {}\n",
        );

        let output = run_kibitzer(&["architecture", "diagram", "--path", dir.to_str().unwrap()]);
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains(DISCLAIMER_COMMENT));
        assert!(stdout.contains("```mermaid"));
        // Default level is Component, but the text-tree section is always full detail
        // (Story 2.2.1 AC3's "stdout contains ... a text line per symbol").
        assert!(stdout.contains("Widget"));
        // ...while the Mermaid section under it omits symbol detail at Component level.
        let mermaid_section = stdout.split("```mermaid").nth(1).unwrap();
        assert!(!mermaid_section.contains("subgraph"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
