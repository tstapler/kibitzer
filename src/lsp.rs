use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tower_lsp::jsonrpc::Result as RpcResult;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

use crate::arch_model::{self, ArchModel, ModelCache, PackageNode, SymbolNode};
use crate::check::{CheckResult, run_checks_for_trigger};
use crate::checker::GrammarCache;
use crate::config::{Severity, find_config};
use crate::symbol_extract::extract_symbols_for_file;

/// Trigger name checks opt into via `.claude/inspect.json`'s `triggers` field to run under
/// `kibitzer lsp` specifically; a check with no `triggers` runs under every trigger,
/// including this one, same as "PostToolUse" and "batch".
const LSP_TRIGGER: &str = "lsp";

/// The single synthetic `workspace/symbol` result returned while the background index is
/// still `Building` — see `Backend::symbol`.
const STILL_INDEXING_MESSAGE: &str =
    "⏳ kibitzer: still indexing this workspace — try again shortly";

fn severity_to_lsp(severity: Severity) -> DiagnosticSeverity {
    match severity {
        Severity::Blocking => DiagnosticSeverity::ERROR,
        Severity::Advisory => DiagnosticSeverity::WARNING,
    }
}

/// Translate one check's result into LSP diagnostics. When the output follows the
/// `{file}:{line}: message` convention (see `check::scope_output_to_changed_lines`),
/// each matched line becomes its own diagnostic at that line; otherwise (a whole-repo
/// check, or output that just doesn't follow the convention) the whole result collapses
/// into a single diagnostic on line 1 using `describe()`, so a failure is never dropped
/// just because it can't be attributed to a specific line.
fn diagnostics_from_result(result: &CheckResult, file_path: &Path) -> Vec<Diagnostic> {
    if result.passed {
        return Vec::new();
    }

    let severity = severity_to_lsp(result.severity);
    let prefix = format!("{}:", file_path.display());
    let mut diagnostics = Vec::new();

    for line in result.output.lines() {
        if let Some(rest) = line.strip_prefix(&prefix)
            && let Some((line_no_str, message)) = rest.split_once(':')
            && let Ok(line_no) = line_no_str.trim().parse::<u32>()
        {
            let zero_indexed = line_no.saturating_sub(1);
            diagnostics.push(Diagnostic {
                range: Range {
                    start: Position::new(zero_indexed, 0),
                    end: Position::new(zero_indexed, u32::MAX),
                },
                severity: Some(severity),
                source: Some(result.check_name.clone()),
                message: message.trim().to_string(),
                ..Default::default()
            });
        }
    }

    if diagnostics.is_empty() {
        diagnostics.push(Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, u32::MAX)),
            severity: Some(severity),
            source: Some(result.check_name.clone()),
            message: result.describe(),
            ..Default::default()
        });
    }

    diagnostics
}

/// Run every in-scope check against `path` (as it currently exists on disk — see the
/// module-level caveat about `did_change`) and translate the results into diagnostics.
fn diagnostics_for_file(path: &Path) -> anyhow::Result<Vec<Diagnostic>> {
    let Some((config, repo_root)) = find_config(path)? else {
        return Ok(Vec::new());
    };
    let results = run_checks_for_trigger(&config.checks, LSP_TRIGGER, &repo_root, path, None)?;
    Ok(results
        .iter()
        .flat_map(|r| diagnostics_from_result(r, path))
        .collect())
}

// ---------------------------------------------------------------------------------
// Epic 4.2: document_symbol — per-file, disk-based, no whole-repo build
// ---------------------------------------------------------------------------------

/// Maps `arch_model::SymbolKind` to the closest `lsp_types::SymbolKind` an editor
/// understands. Shared by `document_symbol` and `symbol`.
fn symbol_kind_to_lsp(kind: arch_model::SymbolKind) -> SymbolKind {
    match kind {
        arch_model::SymbolKind::Type => SymbolKind::STRUCT,
        arch_model::SymbolKind::Interface => SymbolKind::INTERFACE,
        arch_model::SymbolKind::Function => SymbolKind::FUNCTION,
        arch_model::SymbolKind::Method => SymbolKind::METHOD,
    }
}

/// One symbol's whole-line `Range`. `SymbolNode` only carries a start line (no end
/// line/column), so — mirroring `diagnostics_from_result`'s whole-line-range convention
/// above — this covers the entire line rather than inventing precise column data the
/// extractor doesn't have.
fn whole_line_range(line: usize) -> Range {
    let zero_indexed = line.saturating_sub(1) as u32;
    Range::new(
        Position::new(zero_indexed, 0),
        Position::new(zero_indexed, u32::MAX),
    )
}

#[allow(deprecated)]
fn to_document_symbol(symbol: &SymbolNode) -> DocumentSymbol {
    let range = whole_line_range(symbol.line);
    DocumentSymbol {
        name: symbol.name.clone(),
        detail: None,
        kind: symbol_kind_to_lsp(symbol.kind),
        tags: None,
        deprecated: None,
        range,
        selection_range: range,
        children: None,
    }
}

/// Nests `Method`-kind symbols under their parent type (`SymbolNode.parent`) via a
/// name→index lookup built from the non-method symbols first. A method whose declared
/// parent isn't found among them (e.g. the type's own declaration fell on the far side of
/// a parse error and got dropped) is kept at the top level rather than silently lost.
fn nest_document_symbols(symbols: &[SymbolNode]) -> Vec<DocumentSymbol> {
    let mut top_level: Vec<DocumentSymbol> = Vec::new();
    let mut index_by_name: HashMap<&str, usize> = HashMap::new();
    let mut methods: Vec<&SymbolNode> = Vec::new();

    for symbol in symbols {
        if symbol.kind == arch_model::SymbolKind::Method {
            methods.push(symbol);
            continue;
        }
        index_by_name.insert(symbol.name.as_str(), top_level.len());
        top_level.push(to_document_symbol(symbol));
    }

    for method in methods {
        let doc_symbol = to_document_symbol(method);
        match method.parent.as_deref().and_then(|p| index_by_name.get(p)) {
            Some(&idx) => top_level[idx]
                .children
                .get_or_insert_with(Vec::new)
                .push(doc_symbol),
            None => top_level.push(doc_symbol),
        }
    }

    top_level
}

/// Reads `path` off disk (matching the diagnostics disk-snapshot precedent in
/// `Backend::check_and_publish` below), parses it with a fresh `GrammarCache`, and maps its
/// symbols into a nested `DocumentSymbol` tree — no whole-repo `ArchModel` build for a
/// single-file request. Returns `None` for a file kibitzer has no `Language` mapping for
/// (e.g. `.md`) rather than panicking, or if the file can't be read/parsed at all.
///
/// If the parse tree has an error node, this still returns whatever symbols extract
/// cleanly rather than skipping the whole file the way `build_model` does (Story 1.3.1) —
/// a deliberate divergence scoped to this handler only: a single open file with one syntax
/// typo shouldn't blank the whole Outline panel the way a whole-repo model export skips a
/// broken file for correctness. There's no `PruningSummary` here to record the divergence
/// in.
fn document_symbols_for_file(path: &Path) -> Option<DocumentSymbolResponse> {
    let language = arch_model::language_for_path(path)?;
    let source = std::fs::read_to_string(path).ok()?;
    let cache = GrammarCache::new();
    let tree = cache.parse(language, &source).ok()?;
    let symbols = extract_symbols_for_file(language, &source, &tree, "");
    Some(DocumentSymbolResponse::Nested(nest_document_symbols(
        &symbols,
    )))
}

// ---------------------------------------------------------------------------------
// Epic 4.3: background-indexed workspace/symbol
// ---------------------------------------------------------------------------------

/// Whole-repo architecture index state backing `workspace/symbol` (Epic 4.3). Populated by
/// a background build kicked off at `initialized()` time and refreshed on `did_save` —
/// never built inline on a `symbol` request (see Story 4.3.0's redesign rationale in
/// `plan.md`: a synchronous inline build would both make the calling client wait on a cold
/// full-repo walk and block the async runtime's worker thread for other concurrent LSP
/// requests). `Building` is the only state that means "no snapshot exists yet"; a
/// `did_save`-triggered rebuild running while `Ready` doesn't change this variant — that's
/// what the separate `Backend::rebuilding` flag tracks — until it completes and swaps a new
/// snapshot in.
enum IndexState {
    Building,
    Ready(Arc<ArchModel>),
    Failed(String),
}

/// Walks `repo_root`, builds the whole-repo `ArchModel` (exported symbols only —
/// `PruneConfig::default()`, matching `workspace/symbol`'s pruning default — see the Phase
/// 4 pruning-asymmetry note on `Backend::symbol`), through the same `ModelCache` every
/// other consumer of this data uses. Blocking: file walk, disk reads, and tree-sitter
/// parsing all happen here, so this must only ever run inside `tokio::task::spawn_blocking`
/// (see `Backend::spawn_indexed_build`), never directly on the async runtime.
fn build_index(model_cache: &ModelCache, repo_root: &Path) -> anyhow::Result<Arc<ArchModel>> {
    arch_model::load_cached_model(model_cache, repo_root, false)
}

#[allow(deprecated)]
fn still_indexing_symbol_information(workspace_root: Option<PathBuf>) -> SymbolInformation {
    let uri = workspace_root
        .and_then(|root| Url::from_directory_path(root).ok())
        .unwrap_or_else(|| Url::parse("file:///").expect("static URL parses"));
    SymbolInformation {
        name: STILL_INDEXING_MESSAGE.to_string(),
        kind: SymbolKind::NULL,
        tags: None,
        deprecated: None,
        location: Location::new(uri, Range::default()),
        container_name: None,
    }
}

#[allow(deprecated)]
fn symbol_information_for(pkg: &PackageNode, symbol: &SymbolNode) -> Option<SymbolInformation> {
    let uri = Url::from_file_path(&symbol.file).ok()?;
    Some(SymbolInformation {
        name: symbol.name.clone(),
        kind: symbol_kind_to_lsp(symbol.kind),
        tags: None,
        deprecated: None,
        location: Location::new(uri, whole_line_range(symbol.line)),
        container_name: Some(pkg.path.clone()),
    })
}

/// Substring-filters `model`'s (already-pruned, exported-only-by-default) symbols against
/// `query` and maps matches to `SymbolInformation`.
fn symbols_matching(model: &ArchModel, query: &str) -> Vec<SymbolInformation> {
    model
        .packages
        .values()
        .flat_map(|pkg| pkg.symbols.iter().map(move |s| (pkg, s)))
        .filter(|(_, s)| s.name.contains(query))
        .filter_map(|(pkg, s)| symbol_information_for(pkg, s))
        .collect()
}

/// Resolves the workspace root from `InitializeParams`: the first workspace folder, falling
/// back to the deprecated `root_uri`, falling back to the further-deprecated `root_path`.
/// `None` if the client supplied none of the three (e.g. single-file mode).
#[allow(deprecated)]
fn resolve_workspace_root(params: &InitializeParams) -> Option<PathBuf> {
    if let Some(path) = params
        .workspace_folders
        .as_ref()
        .and_then(|folders| folders.first())
        .and_then(|folder| folder.uri.to_file_path().ok())
    {
        return Some(path);
    }
    if let Some(path) = params
        .root_uri
        .as_ref()
        .and_then(|uri| uri.to_file_path().ok())
    {
        return Some(path);
    }
    params.root_path.as_ref().map(PathBuf::from)
}

struct Backend {
    client: Client,
    /// Set once, from `initialize()`'s `InitializeParams` (workspace folder / root URI /
    /// deprecated root path, in that preference order). `None` if the client never told us
    /// a workspace root (e.g. single-file mode) — the background index then never leaves
    /// `Building`, and `symbol` keeps returning the still-indexing sentinel, which is the
    /// correct degrade: there's no repo to index.
    workspace_root: Mutex<Option<PathBuf>>,
    model_cache: Arc<ModelCache>,
    index_state: Arc<Mutex<IndexState>>,
    /// Whether a `did_save`-triggered rebuild is currently in flight while `index_state` is
    /// `Ready`. Deliberately not a third `IndexState` variant (see `IndexState`'s doc
    /// comment) so `symbol`'s dispatch stays a plain 3-way match.
    rebuilding: Arc<AtomicBool>,
    /// Incremented every time a background build (the initial one in `initialized()`, or
    /// any `did_save`-triggered rebuild) is spawned; see `spawn_indexed_build`.
    build_generation: Arc<AtomicU64>,
}

impl Backend {
    fn new(client: Client) -> Self {
        Backend {
            client,
            workspace_root: Mutex::new(None),
            model_cache: Arc::new(ModelCache::new()),
            index_state: Arc::new(Mutex::new(IndexState::Building)),
            rebuilding: Arc::new(AtomicBool::new(false)),
            build_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Re-run checks against `uri` and publish the result, replacing whatever
    /// diagnostics that file previously had.
    ///
    /// Every check reads `path` off disk (shell commands substitute `{file}`; native
    /// checkers `std::fs::read_to_string` it) rather than the editor's in-memory buffer,
    /// so diagnostics reflect the last-saved content, not unsaved keystrokes — the same
    /// disk-based model `run_checks_for_trigger` already uses for the Claude Code hook
    /// and batch mode. Wiring in the live buffer (via LSP's incremental sync) is real
    /// future work, called out in issue #11, not done here.
    async fn check_and_publish(&self, uri: Url) {
        let Ok(path) = uri.to_file_path() else {
            return;
        };
        match diagnostics_for_file(&path) {
            Ok(diagnostics) => {
                self.client
                    .publish_diagnostics(uri, diagnostics, None)
                    .await
            }
            Err(err) => {
                self.client
                    .log_message(MessageType::ERROR, format!("kibitzer: {err:#}"))
                    .await;
            }
        }
    }

    /// Spawns `build` (blocking work — disk walk, reads, tree-sitter parsing) onto tokio's
    /// blocking thread pool and returns immediately, never awaiting it inline — this is
    /// what lets `initialized()`/`did_save()` kick off a whole-repo index build without
    /// blocking the async runtime's worker thread for other concurrent LSP requests (see
    /// Story 4.3.0's redesign rationale in `plan.md`).
    ///
    /// Generation-gated: `build_generation` is incremented before spawning, and the
    /// spawned task only swaps its result into `index_state` (and clears `rebuilding`) if
    /// `build_generation`'s value is still what was captured at spawn time when the build
    /// completes — otherwise a newer rebuild has since been spawned, and this one's result
    /// is silently discarded. This is what stops an out-of-order-completing older rebuild
    /// from clobbering a newer rebuild's already-swapped-in snapshot.
    fn spawn_indexed_build(
        &self,
        build: impl FnOnce() -> anyhow::Result<Arc<ArchModel>> + Send + 'static,
    ) {
        let generation = self.build_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let index_state = Arc::clone(&self.index_state);
        let build_generation = Arc::clone(&self.build_generation);
        let rebuilding = Arc::clone(&self.rebuilding);
        tokio::task::spawn_blocking(move || {
            // Catch a panic from `build()` (rather than letting it propagate and silently
            // drop this task) so a bug in the walk/parse path fails the build visibly via
            // `IndexState::Failed` instead of leaving `index_state` stuck in `Building`
            // forever with only a stderr backtrace as a clue.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(build))
                .unwrap_or_else(|panic_payload| {
                    let message = panic_payload
                        .downcast_ref::<&str>()
                        .map(|s| s.to_string())
                        .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                        .unwrap_or_else(|| "non-string panic payload".to_string());
                    Err(anyhow::anyhow!("panicked: {message}"))
                });

            // Check-and-write must happen under one lock hold: checking outside the lock
            // left a window where an older rebuild could pass its check, get preempted,
            // let a newer rebuild swap in, then resume and overwrite it. Under the lock,
            // whichever check-and-write runs last always sees the current generation, so
            // an older generation can never win regardless of completion order.
            let mut state = index_state.lock().expect("index_state mutex poisoned");
            if build_generation.load(Ordering::SeqCst) == generation {
                *state = match result {
                    Ok(model) => IndexState::Ready(model),
                    Err(err) => IndexState::Failed(err.to_string()),
                };
                rebuilding.store(false, Ordering::SeqCst);
            }
        });
    }

    /// Triggered by `did_save`: spawns a background rebuild if the index is `Ready` and no
    /// rebuild is already in flight (`rebuilding`, gated via `compare_exchange` so a burst
    /// of rapid saves spawns at most one rebuild). No-op while `Building`/`Failed`, or with
    /// no known workspace root. A dropped trigger isn't lost: `index_state` stays
    /// `Ready(old_model)` until the in-flight rebuild swaps in, and that rebuild reads
    /// whatever's on disk, so a save landing mid-rebuild is picked up by the *next* one.
    fn trigger_rebuild_if_ready(&self) {
        let is_ready = matches!(
            &*self.index_state.lock().expect("index_state mutex poisoned"),
            IndexState::Ready(_)
        );
        if !is_ready {
            return;
        }
        let root = self
            .workspace_root
            .lock()
            .expect("workspace_root mutex poisoned")
            .clone();
        let Some(root) = root else {
            return;
        };
        if self
            .rebuilding
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            // A rebuild is already in flight; it (or whichever rebuild it's superseded by)
            // will pick up this save on a later trigger.
            return;
        }
        let model_cache = Arc::clone(&self.model_cache);
        self.spawn_indexed_build(move || build_index(&model_cache, &root));
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> RpcResult<InitializeResult> {
        if let Some(root) = resolve_workspace_root(&params) {
            *self
                .workspace_root
                .lock()
                .expect("workspace_root mutex poisoned") = Some(root);
        }
        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "kibitzer".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                document_symbol_provider: Some(OneOf::Left(true)),
                workspace_symbol_provider: Some(OneOf::Left(true)),
                ..Default::default()
            },
        })
    }

    /// Kicks off the whole-repo background index build used by `symbol` (Story 4.3.0). Does
    /// not await the build — `spawn_indexed_build` hands it to `tokio::task::spawn_blocking`
    /// and this handler returns immediately, well before the build completes.
    async fn initialized(&self, _: InitializedParams) {
        let root = self
            .workspace_root
            .lock()
            .expect("workspace_root mutex poisoned")
            .clone();
        let Some(root) = root else {
            return;
        };
        let model_cache = Arc::clone(&self.model_cache);
        self.spawn_indexed_build(move || build_index(&model_cache, &root));
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        self.check_and_publish(params.text_document.uri).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        self.check_and_publish(params.text_document.uri).await;
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        self.check_and_publish(params.text_document.uri).await;
        self.trigger_rebuild_if_ready();
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.client
            .publish_diagnostics(params.text_document.uri, Vec::new(), None)
            .await;
    }

    /// `textDocument/documentSymbol`: the editor's "Outline" for one open file. See
    /// `document_symbols_for_file` for the read/parse/extract/nest pipeline.
    ///
    /// Unlike `symbol` (`workspace/symbol`, Epic 4.3), this handler includes **private**
    /// symbols: a file-scoped Outline shows everything in the file you're already editing,
    /// so there's no cross-repo noise for the whole-repo "exported-only by default"
    /// pruning rule to guard against — see the Phase 4 pruning-asymmetry note in
    /// `plan.md` and `symbol`'s doc comment below.
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> RpcResult<Option<DocumentSymbolResponse>> {
        let Ok(path) = params.text_document.uri.to_file_path() else {
            return Ok(None);
        };
        Ok(document_symbols_for_file(&path))
    }

    /// `workspace/symbol`: cross-repo "Go to Symbol in Workspace" search against the
    /// background-built index (Story 4.3.0) — never builds or blocks itself; it only reads
    /// whatever `IndexState` the background build has reached so far.
    ///
    /// Unlike `document_symbol` (Epic 4.2), which includes private symbols because a
    /// file-scoped Outline has no cross-repo noise to prune, `symbol` matches only against
    /// **pruned** (exported-only by default) symbols — the same "public surface by
    /// default" rule every other whole-repo consumer of `ArchModel` uses. See the Phase 4
    /// pruning-asymmetry note in `plan.md`.
    async fn symbol(
        &self,
        params: WorkspaceSymbolParams,
    ) -> RpcResult<Option<Vec<SymbolInformation>>> {
        // Snapshot the state and drop the (non-async-safe) std Mutex guard before any
        // `.await` below — `IndexState::Failed`'s error string is logged to the client,
        // which requires awaiting `Client::log_message`.
        let outcome = {
            let state = self.index_state.lock().expect("index_state mutex poisoned");
            match &*state {
                IndexState::Building => None,
                IndexState::Ready(model) => Some(Ok(Arc::clone(model))),
                IndexState::Failed(err) => Some(Err(err.clone())),
            }
        };
        match outcome {
            None => {
                let root = self
                    .workspace_root
                    .lock()
                    .expect("workspace_root mutex poisoned")
                    .clone();
                Ok(Some(vec![still_indexing_symbol_information(root)]))
            }
            Some(Ok(model)) => Ok(Some(symbols_matching(&model, &params.query))),
            Some(Err(err)) => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        format!("kibitzer: architecture index build failed: {err}"),
                    )
                    .await;
                Ok(None)
            }
        }
    }
}

pub async fn run_lsp_server() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{PruningSummary, SymbolKind as ArchSymbolKind};
    use crate::config::Severity;
    use std::sync::atomic::AtomicU64 as TestAtomicU64;

    fn result(severity: Severity, passed: bool, output: &str) -> CheckResult {
        CheckResult {
            check_name: "test-check".to_string(),
            severity,
            passed,
            output: output.to_string(),
            message: None,
            command: "true".to_string(),
        }
    }

    #[test]
    fn passing_result_has_no_diagnostics() {
        let path = PathBuf::from("src/main.go");
        let r = result(Severity::Blocking, true, "");
        assert!(diagnostics_from_result(&r, &path).is_empty());
    }

    #[test]
    fn line_attributed_output_becomes_one_diagnostic_per_line() {
        let path = PathBuf::from("src/main.go");
        let r = result(
            Severity::Advisory,
            false,
            "src/main.go:12: something's off\nsrc/main.go:40: something else",
        );
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].range.start.line, 11);
        assert_eq!(diags[0].message, "something's off");
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::WARNING));
        assert_eq!(diags[1].range.start.line, 39);
        assert_eq!(diags[1].message, "something else");
    }

    #[test]
    fn blocking_severity_maps_to_error() {
        let path = PathBuf::from("f.go");
        let r = result(Severity::Blocking, false, "f.go:1: bad");
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags[0].severity, Some(DiagnosticSeverity::ERROR));
    }

    #[test]
    fn output_without_line_attribution_collapses_to_one_diagnostic() {
        let path = PathBuf::from("f.go");
        let r = result(Severity::Advisory, false, "some whole-repo failure text");
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "some whole-repo failure text");
        assert_eq!(diags[0].range.start.line, 0);
    }

    #[test]
    fn message_and_output_are_combined_via_describe() {
        let path = PathBuf::from("f.go");
        let mut r = result(Severity::Advisory, false, "why this is bad");
        r.message = Some("rule explanation".to_string());
        let diags = diagnostics_from_result(&r, &path);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].message, "rule explanation\nwhy this is bad");
    }

    // --- test helpers ---

    static TMP_COUNTER: TestAtomicU64 = TestAtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-lsp-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture(dir: &Path, rel: &str, contents: &str) -> PathBuf {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, contents).unwrap();
        path
    }

    /// Captures a real `Client` (via a throwaway `LspService`) to build a real `Backend`
    /// in tests without going through full JSON-RPC transport.
    fn test_backend() -> Backend {
        let holder: Arc<Mutex<Option<Client>>> = Arc::new(Mutex::new(None));
        let holder2 = Arc::clone(&holder);
        let (_service, _socket) = LspService::new(move |client| {
            *holder2.lock().unwrap() = Some(client.clone());
            Backend::new(client)
        });
        let client = holder.lock().unwrap().take().expect("client captured");
        Backend::new(client)
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

    /// A small synthetic `ArchModel` with one package holding one `Function` symbol per
    /// name in `names`, in file `<repo_root>/pkg/<name>.go` — enough shape for
    /// `symbols_matching`/`symbol_information_for` to exercise filtering + mapping without
    /// touching disk.
    fn arch_model_with_symbols(repo_root: &Path, names: &[&str]) -> ArchModel {
        let mut pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols: vec![],
        };
        for name in names {
            pkg.symbols.push(SymbolNode {
                id: format!("pkg::{name}"),
                name: name.to_string(),
                kind: ArchSymbolKind::Function,
                file: repo_root.join("pkg").join(format!("{name}.go")),
                line: 1,
                exported: true,
                parent: None,
            });
        }
        let mut packages = std::collections::BTreeMap::new();
        packages.insert("pkg".to_string(), pkg);
        ArchModel {
            repo_root: repo_root.to_path_buf(),
            packages,
            import_edges: vec![],
            pruning: empty_pruning(),
        }
    }

    fn workspace_symbol_params(query: &str) -> WorkspaceSymbolParams {
        WorkspaceSymbolParams {
            partial_result_params: Default::default(),
            work_done_progress_params: Default::default(),
            query: query.to_string(),
        }
    }

    async fn wait_until<F: Fn() -> bool>(check: F) {
        for _ in 0..200 {
            if check() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        panic!("condition never became true within timeout");
    }

    // --- Epic 4.1: initialize() capabilities ---

    #[tokio::test]
    async fn initialize_advertises_document_and_workspace_symbol_capabilities() {
        let backend = test_backend();
        let result = backend
            .initialize(InitializeParams::default())
            .await
            .unwrap();
        assert_eq!(
            result.capabilities.document_symbol_provider,
            Some(OneOf::Left(true))
        );
        assert_eq!(
            result.capabilities.workspace_symbol_provider,
            Some(OneOf::Left(true))
        );
    }

    // --- Epic 4.2: document_symbol ---

    #[test]
    fn document_symbol_nests_method_under_parent_type() {
        let dir = tmp_dir("doc-symbol-nest");
        let path = write_fixture(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{}\n\nfunc (t T) M() {}\n",
        );
        let response = document_symbols_for_file(&path).expect("Go file has a mapping");
        let DocumentSymbolResponse::Nested(top_level) = response else {
            panic!("expected a Nested response");
        };
        assert_eq!(top_level.len(), 1);
        assert_eq!(top_level[0].name, "T");
        assert_eq!(top_level[0].kind, SymbolKind::STRUCT);
        let children = top_level[0]
            .children
            .as_ref()
            .expect("T has one method child");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "M");
        assert_eq!(children[0].kind, SymbolKind::METHOD);
    }

    #[test]
    fn document_symbol_returns_none_for_unmapped_language() {
        let dir = tmp_dir("doc-symbol-md");
        let path = write_fixture(&dir, "notes.md", "# just some notes\n");
        assert!(document_symbols_for_file(&path).is_none());
    }

    #[test]
    fn document_symbol_still_returns_partial_symbols_when_parse_tree_has_errors() {
        let dir = tmp_dir("doc-symbol-partial");
        // Deliberately broken Go: an unterminated function body. tree-sitter still
        // recovers a partial tree with an error node, and `Widget` should still show up.
        let path = write_fixture(
            &dir,
            "broken.go",
            "package pkg\n\nfunc Widget() {\n\nfunc Other(",
        );
        let response =
            document_symbols_for_file(&path).expect("partial extraction still returns Some");
        let DocumentSymbolResponse::Nested(top_level) = response else {
            panic!("expected a Nested response");
        };
        assert!(
            top_level.iter().any(|s| s.name == "Widget"),
            "expected Widget to survive partial extraction, got {top_level:?}"
        );
    }

    #[test]
    fn document_symbol_includes_private_symbols_unlike_build_model() {
        let dir = tmp_dir("doc-symbol-private");
        let path = write_fixture(
            &dir,
            "p.go",
            "package pkg\n\nfunc exported() {}\nfunc unexported() {}\n",
        );
        let response = document_symbols_for_file(&path).expect("Go file has a mapping");
        let DocumentSymbolResponse::Nested(top_level) = response else {
            panic!("expected a Nested response");
        };
        assert!(top_level.iter().any(|s| s.name == "exported"));
        assert!(
            top_level.iter().any(|s| s.name == "unexported"),
            "private symbols must be included, unlike build_model's default pruning"
        );
    }

    // --- Epic 4.3, Story 4.3.0: background index build + generation gating ---

    #[tokio::test]
    async fn initialized_spawns_background_build_and_returns_before_it_completes() {
        let backend = test_backend();

        // `spawn_indexed_build` is the exact background-build entry point `initialized()`
        // delegates to (see its trait-method body above) — exercising it directly with a
        // synthetic, externally-controlled closure lets this test assert "returns before
        // the build completes" deterministically, without depending on real build_model
        // wall-clock timing.
        let (unblock_tx, unblock_rx) = std::sync::mpsc::channel::<()>();
        let repo_root = tmp_dir("initialized-bg-build");
        let repo_root_for_build = repo_root.clone();
        backend.spawn_indexed_build(move || {
            unblock_rx.recv().expect("test unblock channel");
            Ok(Arc::new(arch_model_with_symbols(
                &repo_root_for_build,
                &["Widget"],
            )))
        });

        // The call above returned immediately (spawn_blocking hands the closure to the
        // blocking pool without waiting for it) — the closure is parked on `recv()` and
        // hasn't run to completion, so index_state must still be Building.
        assert!(matches!(
            &*backend.index_state.lock().unwrap(),
            IndexState::Building
        ));

        unblock_tx.send(()).unwrap();
        wait_until(|| !matches!(&*backend.index_state.lock().unwrap(), IndexState::Building)).await;
        assert!(matches!(
            &*backend.index_state.lock().unwrap(),
            IndexState::Ready(_)
        ));

        // End-to-end confidence check: the real `initialized()` handler, wired to a tiny
        // real fixture, also reaches `Ready` (it uses the same `spawn_indexed_build`
        // mechanism just proven non-blocking above, so no timing assertion is needed
        // here — just that the wiring is correct).
        let backend2 = test_backend();
        let fixture_root = tmp_dir("initialized-e2e");
        write_fixture(&fixture_root, "a.go", "package pkg\n\nfunc A() {}\n");
        *backend2.workspace_root.lock().unwrap() = Some(fixture_root);
        backend2.initialized(InitializedParams {}).await;
        wait_until(|| !matches!(&*backend2.index_state.lock().unwrap(), IndexState::Building))
            .await;
        assert!(matches!(
            &*backend2.index_state.lock().unwrap(),
            IndexState::Ready(_)
        ));
    }

    #[tokio::test]
    async fn did_save_rebuild_serves_stale_snapshot_until_swap_completes() {
        let backend = test_backend();
        let repo_root = tmp_dir("did-save-stale-snapshot");
        let model_v1 = Arc::new(arch_model_with_symbols(&repo_root, &["Reader"]));
        *backend.index_state.lock().unwrap() = IndexState::Ready(Arc::clone(&model_v1));

        let (unblock_tx, unblock_rx) = std::sync::mpsc::channel::<()>();
        let repo_root_for_build = repo_root.clone();
        backend.rebuilding.store(true, Ordering::SeqCst);
        backend.spawn_indexed_build(move || {
            unblock_rx.recv().expect("test unblock channel");
            Ok(Arc::new(arch_model_with_symbols(
                &repo_root_for_build,
                &["Writer"],
            )))
        });

        // Rebuild is in flight but hasn't swapped in yet: `symbol` must still serve the
        // pre-rebuild snapshot.
        let during = backend
            .symbol(workspace_symbol_params(""))
            .await
            .unwrap()
            .unwrap();
        assert!(during.iter().any(|s| s.name == "Reader"));
        assert!(!during.iter().any(|s| s.name == "Writer"));

        unblock_tx.send(()).unwrap();
        wait_until(|| {
            let state = backend.index_state.lock().unwrap();
            matches!(&*state, IndexState::Ready(m) if m.find_symbol("Writer").len() == 1)
        })
        .await;

        let after = backend
            .symbol(workspace_symbol_params(""))
            .await
            .unwrap()
            .unwrap();
        assert!(after.iter().any(|s| s.name == "Writer"));
        assert!(!after.iter().any(|s| s.name == "Reader"));
    }

    #[tokio::test]
    async fn did_save_is_noop_while_index_state_building() {
        let backend = test_backend();
        assert!(matches!(
            &*backend.index_state.lock().unwrap(),
            IndexState::Building
        ));
        let generation_before = backend.build_generation.load(Ordering::SeqCst);
        backend.trigger_rebuild_if_ready();
        assert_eq!(
            backend.build_generation.load(Ordering::SeqCst),
            generation_before,
            "did_save must not spawn a build while index_state is Building"
        );
        assert!(matches!(
            &*backend.index_state.lock().unwrap(),
            IndexState::Building
        ));
    }

    #[tokio::test]
    async fn trigger_rebuild_if_ready_dedupes_rapid_calls_while_rebuild_in_flight() {
        let backend = test_backend();
        let repo_root = tmp_dir("dedup-rebuild-burst");
        *backend.index_state.lock().unwrap() =
            IndexState::Ready(Arc::new(arch_model_with_symbols(&repo_root, &["Initial"])));
        *backend.workspace_root.lock().unwrap() = Some(repo_root.clone());

        let generation_before = backend.build_generation.load(Ordering::SeqCst);

        // Simulate a burst of `did_save` events (e.g. an editor's "save all") firing back to
        // back, synchronously — before the first spawned rebuild has had a chance to run on
        // the blocking pool and clear `rebuilding`. Only the first call should win the
        // `compare_exchange` and actually spawn a build; the rest must no-op.
        for _ in 0..5 {
            backend.trigger_rebuild_if_ready();
        }

        assert_eq!(
            backend.build_generation.load(Ordering::SeqCst),
            generation_before + 1,
            "a burst of rapid did_save triggers must spawn exactly one rebuild, not one per call"
        );
        assert!(backend.rebuilding.load(Ordering::SeqCst));

        wait_until(|| !backend.rebuilding.load(Ordering::SeqCst)).await;
        assert!(matches!(
            &*backend.index_state.lock().unwrap(),
            IndexState::Ready(_)
        ));
    }

    /// `build_generation` is incremented synchronously in `spawn_indexed_build`, before
    /// either closure below runs — so by the time B is spawned, A's captured generation is
    /// already stale, regardless of completion order. This proves a stale-generation
    /// completion is discarded (A never clobbers B), not that the lock-then-check ordering
    /// specifically closes a race between two closures completing concurrently — that
    /// narrower claim would need a hook forcing A's check to run *during* B's write, which
    /// this test doesn't do.
    #[tokio::test]
    async fn stale_generation_completion_is_discarded_after_newer_build_completes() {
        let backend = test_backend();
        let repo_root = tmp_dir("out-of-order-rebuild");
        *backend.index_state.lock().unwrap() =
            IndexState::Ready(Arc::new(arch_model_with_symbols(&repo_root, &["Initial"])));

        let (a_release_tx, a_release_rx) = std::sync::mpsc::channel::<()>();
        let a_root = repo_root.clone();
        // Rebuild A (older generation) — parked until we explicitly release it below.
        backend.spawn_indexed_build(move || {
            a_release_rx.recv().expect("A's unblock channel");
            Ok(Arc::new(arch_model_with_symbols(&a_root, &["FromA"])))
        });

        // Rebuild B (newer generation) — completes immediately.
        let b_root = repo_root.clone();
        backend.spawn_indexed_build(move || {
            Ok(Arc::new(arch_model_with_symbols(&b_root, &["FromB"])))
        });

        wait_until(|| {
            let state = backend.index_state.lock().unwrap();
            matches!(&*state, IndexState::Ready(m) if m.find_symbol("FromB").len() == 1)
        })
        .await;

        // Now let A (the older, out-of-order-completing rebuild) finish. Its result must
        // be discarded because a newer generation has already swapped in.
        a_release_tx.send(()).unwrap();
        // Give A's completion a moment to run (it should be a no-op).
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let state = backend.index_state.lock().unwrap();
        match &*state {
            IndexState::Ready(model) => {
                assert_eq!(model.find_symbol("FromB").len(), 1, "B's result must win");
                assert_eq!(
                    model.find_symbol("FromA").len(),
                    0,
                    "A's out-of-order completion must not clobber B's already-swapped-in result"
                );
            }
            IndexState::Building => panic!("expected Ready, got Building"),
            IndexState::Failed(err) => panic!("expected Ready, got Failed({err})"),
        }
    }

    // --- Epic 4.3, Story 4.3.1: symbol ---

    #[tokio::test]
    async fn symbol_returns_synthetic_still_indexing_entry_while_building() {
        let backend = test_backend();
        assert!(matches!(
            &*backend.index_state.lock().unwrap(),
            IndexState::Building
        ));
        let response = backend
            .symbol(workspace_symbol_params("anything"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.len(), 1);
        assert_eq!(response[0].name, STILL_INDEXING_MESSAGE);
    }

    #[tokio::test]
    async fn symbol_filters_query_substring_against_pruned_names() {
        let backend = test_backend();
        let repo_root = tmp_dir("symbol-filter");
        *backend.index_state.lock().unwrap() = IndexState::Ready(Arc::new(
            arch_model_with_symbols(&repo_root, &["Reader", "Writer", "Closer"]),
        ));
        let response = backend
            .symbol(workspace_symbol_params("Re"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.len(), 1);
        assert_eq!(response[0].name, "Reader");
    }

    #[tokio::test]
    async fn symbol_returns_none_when_index_state_failed() {
        let backend = test_backend();
        *backend.index_state.lock().unwrap() = IndexState::Failed("no config found".to_string());
        let response = backend
            .symbol(workspace_symbol_params("anything"))
            .await
            .unwrap();
        assert!(response.is_none());
    }
}
