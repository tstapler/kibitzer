//! Signal 2 of issue #45's "Remove Dead Code" pair (signal 1 is `rules.rs`'s
//! `unreachable-code` rule): an unexported (private) function or method the repo's call
//! graph never resolves a call to — Fowler's *Remove Dead Code* applied to whole symbols
//! instead of statements. Needs #33's call-graph edges (`ArchModel::call_edges`) to tell
//! "imported and used" apart from "imported and dead"; a plain import-graph edge can't.
//!
//! Deliberately narrow in scope:
//! - **Function/Method only.** `Type`/`Interface` symbols have no usage-edge in `ArchModel`
//!   at all (no field/local/parameter type-annotation tracking) — flagging them off
//!   `call_edges` alone would be pure guesswork, so they're skipped rather than
//!   misjudged, same "no evidence, not zero usage" stance `isp_fat_interface` takes for a
//!   consumer-less interface.
//! - **Exported symbols are never candidates.** A private symbol's only possible caller is
//!   somewhere in this repo (this scan's whole universe); an exported one may be called by
//!   an external consumer this repo can't see, so it's out of scope entirely — same
//!   carve-out the issue's scope notes call for.
//! - **`main`/`init` are never candidates.** Both are unexported-by-Go-convention runtime
//!   entrypoints invoked by the language runtime, not by any call site this graph would
//!   ever see — flagging them would be a false positive on every single Go/Rust binary.
//! - **Ambiguous-name fallback.** `resolve_one_call_edge` resolves a call by name; when two
//!   symbols share a name (e.g. two types both implementing an interface's `String()`
//!   method under different, unexported names) the call stays `resolved: false` with the
//!   raw callee text kept, rather than guessing which one it meant. Treating "some call
//!   site in the repo used this exact name and never resolved" as evidence of use (rather
//!   than silence) is the same false-negative-over-false-positive tradeoff every other
//!   `ArchModelChecker` in this codebase makes when its signal runs out.
//! - **No test-file carve-out.** Unlike the issue's scope note anticipated, `ArchModel`'s
//!   call graph is built from every scanned file, tests included — a private helper used
//!   only from its own package's test file already gets a real resolved `CallEdge` from
//!   that test, so no separate exclusion is needed here.
//! - **Go/TypeScript/Tsx/JavaScript only.** `symbol_extract::call_graph_supports` doesn't
//!   extract call sites for Rust, Python, Java, or Kotlin at all (confirmed backtesting
//!   this exact checker against kibitzer's own — Rust — source: with zero call edges for
//!   the whole language, *every* private function/method looked "unreferenced," including
//!   ones called all over the codebase). A symbol in an uncovered language has no usage
//!   signal to judge it by, same "no evidence" reasoning as the `Type`/`Interface` skip
//!   above — so this checker restricts its candidates to files in a call-graph-covered
//!   language rather than producing wall-to-wall false positives everywhere else.
//! - **Raw-text occurrence fallback.** `call_edges` only ever captures a *call*
//!   (`f()`/`recv.Method()`) — a symbol referenced as a bare value (`mux.HandleFunc("/x",
//!   h.handleFoo)`, a callback table, a method passed by name to a test-table entry) has no
//!   `call_expression` at all and is invisible to the call graph. Confirmed backtesting
//!   against `tstapler/stapler-squad`: every HTTP-handler method registered this way (the
//!   dominant Go `net/http` idiom) was a false positive on the call-graph signal alone. So
//!   this checker also does its own cheap word-boundary scan of every scanned file's raw
//!   source for the candidate's bare name; a second textual occurrence anywhere (beyond the
//!   declaration itself) counts as a reference regardless of syntactic shape. This can
//!   under-flag (an unrelated identically-named identifier elsewhere in the repo silences a
//!   real dead symbol) but never over-flags — the same false-negative-over-false-positive
//!   bias as the ambiguous-name fallback above. **Cost**: this re-reads every scanned file
//!   from disk a second time (`ArchModel` doesn't retain source text after parsing) —
//!   confirmed expensive enough on a large real-world monorepo (`tstapler/stapler-squad`)
//!   that this is opt-in/periodic-CI only, not something to wire into a per-edit hook.
//! - **Malformed-name guard.** A handful of JS/TS extraction edge cases (a computed/
//!   string-quoted object-literal method key, e.g. ESLint visitor rules'
//!   `{"FunctionDeclaration:exit"(n){}}`) produce a `SymbolNode::name` that isn't a real
//!   identifier — confirmed against `stapler-squad`'s `eslint-plugin-analytics` rules.
//!   `is_plausible_identifier` filters these out rather than reporting the extractor's own
//!   confusion as a dead-code finding.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use regex::Regex;

use crate::arch_model::{ArchModel, SymbolKind, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker};
use crate::config::ArchitectureConfig;

/// Runtime entrypoints that are unexported by language convention but never explicitly
/// called from anywhere this repo's call graph could observe.
const ENTRYPOINT_NAMES: &[&str] = &["main", "init"];

pub struct UnreferencedPrivateSymbolChecker;

impl ArchModelChecker for UnreferencedPrivateSymbolChecker {
    fn name(&self) -> &str {
        "unreferenced-private-symbol"
    }

    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        unreferenced_findings_for_model(model)
    }
}

fn last_segment(text: &str) -> &str {
    text.rsplit_once('.').map_or(text, |(_, last)| last)
}

fn has_call_graph_coverage(file: &Path) -> bool {
    crate::arch_model::language_for_path(file)
        .is_some_and(crate::symbol_extract::call_graph_supports)
}

/// A handful of JS/TS extraction edge cases (e.g. a computed/string-quoted method key
/// in an object literal, like ESLint visitor rules' `{"FunctionDeclaration:exit"(n){}}`)
/// produce a `SymbolNode::name` that isn't a real identifier — empty, or carrying quote/
/// punctuation characters `symbol_extract` never intended as a name. Confirmed backtesting
/// against `tstapler/stapler-squad`'s `eslint-plugin-analytics` rules. Reporting one of
/// these as "dead code" would just be surfacing the extractor's own confusion as a finding,
/// so they're filtered out here rather than fixing extraction (a pre-existing, unrelated
/// gap this checker happens to be the first consumer to notice at scale).
fn is_plausible_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn is_candidate(symbol: &SymbolNode) -> bool {
    !symbol.exported
        && matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
        && !ENTRYPOINT_NAMES.contains(&symbol.name.as_str())
        && is_plausible_identifier(&symbol.name)
        && has_call_graph_coverage(&symbol.file)
}

fn finding_for(symbol: &SymbolNode) -> ArchFinding {
    let kind = match symbol.kind {
        SymbolKind::Method => "method",
        _ => "function",
    };
    ArchFinding {
        file: Some(symbol.file.clone()),
        line: Some(symbol.line),
        message: format!(
            "[unreferenced-private-symbol] private {kind} `{}` has no resolved caller \
             anywhere in the repo — likely dead code (Fowler, Remove Dead Code)",
            symbol.name
        ),
        severity_override: None,
    }
}

fn word_regex() -> &'static Regex {
    static WORD_RE: OnceLock<Regex> = OnceLock::new();
    WORD_RE.get_or_init(|| Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").unwrap())
}

/// How many times each identifier-shaped word occurs across every file the model
/// scanned. A file that fails to re-read from disk (already gone, permissions) is
/// silently skipped — this signal only ever adds suppressions, never findings, so an
/// undercount here just means one less place a real reference could be found, not a
/// false finding.
fn build_word_occurrence_counts(model: &ArchModel) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut seen = HashSet::new();
    for file in model.packages.values().flat_map(|pkg| pkg.files.iter()) {
        if !seen.insert(file.as_path()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(file) else {
            continue;
        };
        for word in word_regex().find_iter(&content) {
            *counts.entry(word.as_str().to_string()).or_insert(0) += 1;
        }
    }
    counts
}

fn unreferenced_findings_for_model(model: &ArchModel) -> Vec<ArchFinding> {
    let resolved_targets: HashSet<&str> = model
        .call_edges
        .iter()
        .filter(|edge| edge.resolved)
        .map(|edge| edge.to.as_str())
        .collect();
    let unresolved_names: HashSet<&str> = model
        .call_edges
        .iter()
        .filter(|edge| !edge.resolved)
        .map(|edge| last_segment(&edge.to))
        .collect();
    let word_counts = build_word_occurrence_counts(model);

    let mut findings: Vec<ArchFinding> = model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter())
        .filter(|symbol| is_candidate(symbol))
        .filter(|symbol| !resolved_targets.contains(symbol.id.as_str()))
        .filter(|symbol| !unresolved_names.contains(symbol.name.as_str()))
        .filter(|symbol| word_counts.get(&symbol.name).copied().unwrap_or(0) <= 1)
        .map(finding_for)
        .collect();

    findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{PruneConfig, build_model};
    use crate::import_graph::ImportGraph;
    use std::path::PathBuf;

    fn model_from(files: Vec<(&str, &str)>) -> ArchModel {
        let files: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|(path, src)| (PathBuf::from(path), src.to_string()))
            .collect();
        build_model(
            &PathBuf::from("/repo"),
            &files,
            &ImportGraph::default(),
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap()
    }

    /// Writes real files to a temp dir and builds a real `ArchModel` from disk (unlike
    /// `model_from`'s fake, unreadable paths) — needed whenever a test's assertion depends
    /// on `build_word_occurrence_counts`'s `std::fs::read_to_string` actually reading
    /// something, not silently no-op'ing on a path that was never written.
    fn model_from_disk(label: &str, files: Vec<(&str, &str)>) -> ArchModel {
        let dir = crate::test_support::unique_temp_dir(label);
        let paths: Vec<PathBuf> = files
            .into_iter()
            .map(|(name, src)| {
                let path = dir.join(name);
                std::fs::write(&path, src).unwrap();
                path
            })
            .collect();
        crate::arch_model::build_model_from_files(
            &dir,
            &paths,
            &PruneConfig {
                include_private: true,
            },
        )
        .unwrap()
    }

    #[test]
    fn flags_an_unreferenced_private_function() {
        // Uses a real file on disk (not `model_from`'s fake paths) so the raw-text
        // fallback's word count for "dead" is the real value (1, its own declaration)
        // rather than `model_from`'s always-0 (unreadable path) — the `<= 1` boundary in
        // `unreferenced_findings_for_model` is only actually exercised this way.
        let model = model_from_disk(
            "unreferenced-symbols-flags-function",
            vec![("a.go", "package pkg\n\nfunc dead() {}\n\nfunc Live() {}\n")],
        );
        let findings = unreferenced_findings_for_model(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("private function `dead`"));
    }

    #[test]
    fn flags_an_unreferenced_private_method() {
        let model = model_from_disk(
            "unreferenced-symbols-flags-method",
            vec![(
                "a.go",
                "package pkg\n\ntype T struct{}\n\nfunc (t T) dead() {}\n\nfunc Live() {}\n",
            )],
        );
        let findings = unreferenced_findings_for_model(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("private method `dead`"));
    }

    #[test]
    fn discriminates_a_dead_symbol_from_a_live_one_in_the_same_file() {
        let model = model_from_disk(
            "unreferenced-symbols-discriminate",
            vec![(
                "a.go",
                "package pkg\n\nfunc dead() {}\n\nfunc used() {}\n\nfunc Live() {\n\tused()\n}\n",
            )],
        );
        let findings = unreferenced_findings_for_model(&model);
        assert_eq!(findings.len(), 1, "got: {findings:?}");
        assert!(findings[0].message.contains("`dead`"));
    }

    #[test]
    fn sorts_findings_by_file_then_line() {
        let model = model_from_disk(
            "unreferenced-symbols-sort",
            vec![
                ("b.go", "package pkg\n\nfunc deadB() {}\n"),
                (
                    "a.go",
                    "package pkg\n\nfunc deadA1() {}\n\nfunc deadA2() {}\n",
                ),
            ],
        );
        let findings = unreferenced_findings_for_model(&model);
        assert_eq!(findings.len(), 3, "got: {findings:?}");
        let locations: Vec<_> = findings.iter().map(|f| (f.file.clone(), f.line)).collect();
        let mut sorted = locations.clone();
        sorted.sort();
        assert_eq!(locations, sorted, "got: {findings:?}");
    }

    #[test]
    fn does_not_flag_a_private_function_that_is_called() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc used() {}\n\nfunc Live() {\n\tused()\n}\n",
        )]);
        assert!(
            unreferenced_findings_for_model(&model).is_empty(),
            "used() is called from Live() and must not be flagged"
        );
    }

    #[test]
    fn does_not_flag_exported_symbols() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\nfunc Unused() {}\n",
        )]);
        assert!(
            unreferenced_findings_for_model(&model).is_empty(),
            "exported symbols are out of scope — an external consumer may call this"
        );
    }

    #[test]
    fn does_not_flag_main_or_init() {
        let model = model_from(vec![(
            "/repo/cmd/a.go",
            "package main\n\nfunc init() {}\n\nfunc main() {}\n",
        )]);
        assert!(
            unreferenced_findings_for_model(&model).is_empty(),
            "main/init are runtime entrypoints, never explicitly called"
        );
    }

    #[test]
    fn does_not_flag_a_type_or_interface_symbol() {
        let model = model_from(vec![(
            "/repo/pkg/a.go",
            "package pkg\n\ntype unused struct{}\n",
        )]);
        assert!(
            unreferenced_findings_for_model(&model).is_empty(),
            "Type/Interface symbols have no usage-edge data to judge them by"
        );
    }

    #[test]
    fn does_not_flag_a_private_method_whose_name_collides_with_an_unresolved_call() {
        // Two packages each declare an unexported `run` method on their own type. A third
        // package calls `x.run()` on an interface-typed value; because `run` isn't unique
        // repo-wide, resolution can't tell which one it meant and leaves the edge
        // unresolved (`resolved: false`, raw text kept). Neither `run` should be flagged —
        // silence here is "ambiguous," not "provably dead."
        let model = model_from(vec![
            (
                "/repo/a/a.go",
                "package a\n\ntype T struct{}\n\nfunc (t T) run() {}\n",
            ),
            (
                "/repo/b/b.go",
                "package b\n\ntype T struct{}\n\nfunc (t T) run() {}\n",
            ),
            (
                "/repo/c/c.go",
                "package c\n\nfunc Dispatch(x interface{ run() }) {\n\tx.run()\n}\n",
            ),
        ]);
        let findings = unreferenced_findings_for_model(&model);
        assert!(findings.is_empty(), "got: {findings:?}");
    }

    #[test]
    fn does_not_flag_a_private_function_in_a_language_without_call_graph_coverage() {
        // Rust has no call-site extraction at all (`call_graph_supports`), so every
        // private fn would otherwise look unreferenced regardless of real usage —
        // confirmed against kibitzer's own source before this carve-out was added.
        let model = model_from(vec![(
            "/repo/src/a.rs",
            "fn dead() {}\n\npub fn live() {\n    dead();\n}\n",
        )]);
        assert!(
            unreferenced_findings_for_model(&model).is_empty(),
            "Rust isn't call-graph-covered — no evidence either way, must not flag"
        );
    }

    #[test]
    fn does_not_flag_a_method_only_referenced_as_a_bare_value() {
        // Go's dominant net/http idiom: the handler is registered by name
        // (`mux.HandleFunc(pattern, h.handleFoo)`), never actually called as
        // `h.handleFoo(...)` anywhere the call graph would see it. Confirmed as a real
        // false-positive class backtesting against tstapler/stapler-squad.
        let model = model_from_disk(
            "unreferenced-symbols-bare-value",
            vec![(
                "handler.go",
                "package pkg\n\n\
                 type Handler struct{}\n\n\
                 func (h *Handler) Register(mux *http.ServeMux) {\n\
                 \tmux.HandleFunc(\"/x\", h.handleFoo)\n\
                 }\n\n\
                 func (h *Handler) handleFoo(w http.ResponseWriter, r *http.Request) {}\n",
            )],
        );
        let findings = unreferenced_findings_for_model(&model);
        assert!(findings.is_empty(), "got: {findings:?}");
    }

    #[test]
    fn rejects_names_that_are_not_plausible_identifiers() {
        assert!(!is_plausible_identifier(""));
        assert!(!is_plausible_identifier("\"FunctionDeclaration"));
        assert!(!is_plausible_identifier("SwitchStatement:exit"));
        assert!(is_plausible_identifier("handleFoo"));
        assert!(is_plausible_identifier("_private"));
    }
}
