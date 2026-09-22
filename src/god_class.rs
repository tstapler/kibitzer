//! God-class detection: WMC + ATFD + TCC combined — issue #38's "medium" tier item,
//! deliberately held back from `LcomChecker`'s single-metric shape. `LcomChecker`'s own
//! backtest against `kubernetes/kubernetes` (925 → 360 findings after excluding
//! state-independent methods, still noisy on plain accessor-heavy data holders) is the
//! concrete evidence behind the literature's insistence that WMC/ATFD/TCC be combined,
//! not used alone: a class can have high complexity (WMC) *or* touch foreign data
//! (ATFD) *or* look internally fragmented (TCC) for perfectly ordinary reasons, but all
//! three at once is a much stronger God Class signal (PMD's `GodClassRule`, iPlasma).
//!
//! Go-only for v1 — see [`GodClassChecker`]'s doc comment for the full scope/ceiling
//! list, especially ATFD's syntactic-type-only detection. ATFD's `pkg.Foo`-qualified
//! locals resolve cross-package via `ArchModel::file_import_aliases`; same-package
//! resolution needed no such step in the first place.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use tree_sitter::{Node, Tree};

use crate::arch_model::{ArchModel, PackageNode, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker, methods_by_type};
use crate::checker::{GrammarCache, Language};
use crate::checkers::complexity::{SubtestHandling, cyclomatic_complexity};
use crate::config::ArchitectureConfig;

/// PMD's default God Class thresholds (PMD's `GodClassRule`: `WMC > 47`, `ATFD > 5`,
/// `TCC < 0.33`). The issue that scoped this checker (#38) quoted these as `WMC ≤ 47`;
/// that reads backward for a "this class is too big" detector (a class comfortably under
/// 47 is exactly the non-God-Class case), and true PMD/Lanza-Marinescu God Class
/// detection flags classes *exceeding* the WMC threshold, not staying under it — so this
/// implements `WMC > 47`, treating the issue's `≤` as a transcription inversion rather
/// than following it literally into a threshold that would flag nearly every ordinary
/// class instead of the unusual large ones.
const WMC_THRESHOLD: usize = 47;
const ATFD_THRESHOLD: usize = 5;
const TCC_THRESHOLD: f64 = 0.33;

/// Below this many methods, WMC/ATFD naturally can't clear their thresholds and TCC's
/// pair-count denominator is too small to mean anything (a 2-method type has exactly one
/// possible pair) — same "too small to be worth evaluating" status as `LCOM_MIN_METHODS`.
const GOD_CLASS_MIN_METHODS: usize = 4;

/// God-class detector combining three metrics the literature requires together, not any
/// alone (see this module's doc comment):
///
/// - **WMC** (Weighted Method Count): sum of each method's McCabe cyclomatic complexity
///   ([`crate::checkers::complexity::cyclomatic_complexity`], reused rather than reimplemented).
///   Requires re-parsing each method's source file — `ArchModel` doesn't retain a parsed
///   `Tree` or per-method complexity, only declarations (same "no other source for this
///   data" tradeoff `PackageSizeChecker` already makes reading file contents directly).
/// - **ATFD** (Access To Foreign Data): count of distinct *foreign* `(type, field)` pairs
///   accessed across the type's methods, via a selector on a syntactically-typed local —
///   a parameter or a `var`/`:=`-declared local whose type is written out explicitly at
///   the declaration site. **Known ceiling, not fixed for v1**: this is not real type
///   inference. It cannot see through a variable whose type comes from a function's
///   return value, an interface's dynamic type, or a generic instantiation — only a
///   syntactically visible type name at the point a local is introduced. A `pkg.Foo`-
///   qualified local's field accesses now count too, resolved to `"{target_package}::
///   {name}"` via that file's `ArchModel::file_import_aliases` entry (`resolve_qualified_
///   type`) — dropped, not fabricated under raw alias text, when the alias didn't resolve
///   (an external/stdlib import, or a dot/blank import `file_import_aliases` never
///   populates an entry for). No cross-package *field validity* check exists either way —
///   same-package ATFD never validated a selector's field name against the target's real
///   fields either (see `walk_selectors_on_typed_locals`), only the package/type identity
///   changed here.
/// - **TCC** (Tight Class Cohesion, Bieman & Kang): the fraction of method pairs that
///   directly share at least one field access, reusing `ArchModel::field_accesses` —
///   the same data `LcomChecker` uses, just aggregated as a ratio of connected pairs
///   over all possible pairs instead of connected-components.
///
/// Flags a type only when **all three** cross their threshold together
/// ([`WMC_THRESHOLD`]/[`ATFD_THRESHOLD`]/[`TCC_THRESHOLD`]) — this checker exists
/// specifically because any one of these alone is noisy (see this module's doc comment).
/// Opt-in only via `model_registry()`, same as `InstabilityChecker`/`DipConcreteCouplingChecker`/`LcomChecker` — not wired into `default_checks()`.
///
/// Known false-positive ceiling, confirmed backtesting against `kubernetes/kubernetes`
/// (3 findings total across the whole repo — the triple-metric AND is far quieter than
/// `LcomChecker`'s single-metric shape, which needed a dedicated fix after a 925-finding
/// storm on the same corpus): a fluent-builder/options struct with many independent
/// boolean toggle fields and one setter method per field (`ErrorMatcher` in
/// `k8s.io/apimachinery`) reports a real, technically-correct God Class signal — high
/// WMC from comparison logic, real ATFD from comparing foreign struct fields, low TCC
/// because each setter only touches its own field — even though "split this up" is
/// debatable advice for an intentional options pattern. Same "worth a second look, not
/// proof" treatment as `LcomChecker`'s accessor-heavy-DTO ceiling.
pub struct GodClassChecker;

impl ArchModelChecker for GodClassChecker {
    fn name(&self) -> &str {
        "god-class"
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        model
            .packages
            .values()
            .flat_map(|pkg| god_class_findings_for_package(pkg, model))
            .collect()
    }
}

/// Caches parsed files across every type in one package, since a Go type's methods
/// usually all live in the same file (and often several types in a package share files).
///
/// `GrammarCache` caches by [`Language`] only — one tree per language, reused across
/// every `parse` call on that same cache instance (its own doc comment: "reusing a prior
/// parse of the same language for this cache instance if one already happened"). It's
/// built for the single-file-per-invocation shape every other caller uses (one file, one
/// `GrammarCache`), not a multi-file cache. A backtest against `kubernetes/kubernetes`
/// caught this the hard way: sharing one `GrammarCache` here across every file in a
/// package meant every file after the first silently got back the *first* file's stale
/// tree paired with the new file's source string, producing byte-index-out-of-bounds
/// panics as soon as a later file was shorter than the first one parsed. A fresh
/// `GrammarCache::new()` per file avoids the cross-file reuse entirely.
pub(crate) struct FileCache {
    cache: HashMap<PathBuf, Option<(String, Tree)>>,
}

impl FileCache {
    pub(crate) fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    pub(crate) fn get(&mut self, file: &Path) -> Option<&(String, Tree)> {
        self.cache
            .entry(file.to_path_buf())
            .or_insert_with(|| {
                let source = std::fs::read_to_string(file).ok()?;
                let tree = GrammarCache::new().parse(Language::Go, &source).ok()?;
                Some((source, tree))
            })
            .as_ref()
    }
}

fn god_class_findings_for_package(pkg: &PackageNode, model: &ArchModel) -> Vec<ArchFinding> {
    let symbols_by_id: HashMap<&str, &SymbolNode> =
        pkg.symbols.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut files = FileCache::new();

    methods_by_type(pkg)
        .into_iter()
        .filter_map(|(type_name, ids)| {
            if ids.len() < GOD_CLASS_MIN_METHODS {
                return None;
            }
            let wmc = total_wmc(&ids, &symbols_by_id, &mut files);
            let atfd =
                distinct_foreign_field_accesses(&ids, &symbols_by_id, type_name, &mut files, model);
            let tcc = tight_class_cohesion(&ids, model);

            if wmc <= WMC_THRESHOLD || atfd <= ATFD_THRESHOLD || tcc >= TCC_THRESHOLD {
                return None;
            }
            Some(ArchFinding {
                file: None,
                line: None,
                message: format!(
                    "[god-class] {}::{type_name} has WMC={wmc} (over {WMC_THRESHOLD}), \
                     ATFD={atfd} (over {ATFD_THRESHOLD}), TCC={tcc:.2} (under \
                     {TCC_THRESHOLD}) across {} methods — high complexity, heavy foreign- \
                     data access, and low internal cohesion together are a God Class \
                     signal; consider Extract Class or moving foreign-data logic closer \
                     to the data it uses",
                    pkg.path,
                    ids.len()
                ),
                severity_override: None,
            })
        })
        .collect()
}

/// Sum of each method's cyclomatic complexity. A method whose file fails to re-parse (
/// moved/deleted between model build and this check, or a real I/O error) contributes 0
/// rather than aborting the whole type's evaluation — same "skip what we can't read"
/// tolerance `PackageSizeChecker` shows for an unreadable file.
fn total_wmc(
    ids: &[&str],
    symbols_by_id: &HashMap<&str, &SymbolNode>,
    files: &mut FileCache,
) -> usize {
    ids.iter()
        .filter_map(|id| symbols_by_id.get(id).copied())
        .filter_map(|sym| {
            let (source, tree) = files.get(&sym.file)?;
            let node = find_method_node(tree.root_node(), sym.line, &sym.name, source)?;
            Some(cyclomatic_complexity(
                node,
                source.as_bytes(),
                SubtestHandling::IncludeAll,
            ))
        })
        .sum()
}

/// Finds the `method_declaration` or `function_declaration` node whose declared name and
/// 1-indexed start line match `line`/`name` — `SymbolNode` records a declaration site,
/// not a `Node` handle (it outlives the `Tree` it came from), so re-locating it after a
/// fresh parse is the only way back to real AST structure for per-symbol analysis.
/// Matches both kinds since callers need both: `god_class.rs`'s own WMC/ATFD only ever
/// look up `Method` symbols, but `isp_fat_interface.rs` looks up interface *consumers*,
/// which are just as often free functions (`func UseThing(t Thing)`) as methods.
pub(crate) fn find_method_node<'a>(
    node: Node<'a>,
    line: usize,
    name: &str,
    source: &str,
) -> Option<Node<'a>> {
    if matches!(node.kind(), "method_declaration" | "function_declaration")
        && node.start_position().row + 1 == line
        && node
            .child_by_field_name("name")
            .is_some_and(|n| node_text(n, source) == name)
    {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find_map(|child| find_method_node(child, line, name, source))
}

pub(crate) fn node_text<'a>(node: Node, source: &'a str) -> &'a str {
    &source[node.start_byte()..node.end_byte()]
}

/// TCC (Bieman & Kang): the fraction of method pairs that directly share at least one
/// field access, out of every possible pair. Reuses `ArchModel::field_accesses` — the
/// same data `LcomChecker` unions into connected components, here counted as a ratio of
/// pairwise connections instead. Unlike `LcomChecker`, this doesn't first filter out
/// state-independent methods: TCC's denominator (`n * (n-1) / 2`) is over *all* of the
/// type's methods, marker stubs included, matching the metric's own definition rather
/// than borrowing LCOM's own-signal filter, which would understate `n` and inflate TCC.
fn tight_class_cohesion(ids: &[&str], model: &ArchModel) -> f64 {
    let n = ids.len();
    if n < 2 {
        return 1.0; // Vacuously cohesive — never reached given GOD_CLASS_MIN_METHODS.
    }
    let index: HashMap<&str, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
    let mut methods_by_field: HashMap<&str, Vec<usize>> = HashMap::new();
    for access in &model.field_accesses {
        if let Some(&i) = index.get(access.from.as_str()) {
            methods_by_field
                .entry(access.to.as_str())
                .or_default()
                .push(i);
        }
    }
    let mut connected_pairs: HashSet<(usize, usize)> = HashSet::new();
    for members in methods_by_field.values() {
        for a in 0..members.len() {
            for b in (a + 1)..members.len() {
                connected_pairs.insert((members[a].min(members[b]), members[a].max(members[b])));
            }
        }
    }
    let total_pairs = n * (n - 1) / 2;
    connected_pairs.len() as f64 / total_pairs as f64
}

/// Distinct `(foreign_type, field_name)` pairs accessed, across all of `ids`' method
/// bodies, through a syntactically-typed local that isn't this type's own receiver — see
/// [`GodClassChecker`]'s doc comment for what "syntactically-typed" and "foreign" mean
/// here (no real type inference). A qualified (`pkg.Foo`) local resolves to its real
/// cross-package target via `model.file_import_aliases` (per `resolve_qualified_type`)
/// when that file's imports resolved one; otherwise it's dropped, same as before
/// cross-package resolution existed — never counted under its raw, ambiguous
/// `"alias.Foo"` text.
fn distinct_foreign_field_accesses(
    ids: &[&str],
    symbols_by_id: &HashMap<&str, &SymbolNode>,
    own_type: &str,
    files: &mut FileCache,
    model: &ArchModel,
) -> usize {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    for id in ids {
        let Some(sym) = symbols_by_id.get(id).copied() else {
            continue;
        };
        let Some((source, tree)) = files.get(&sym.file) else {
            continue;
        };
        let Some(node) = find_method_node(tree.root_node(), sym.line, &sym.name, source) else {
            continue;
        };
        let file_aliases = model.file_import_aliases.get(&sym.file);
        collect_foreign_field_accesses(node, source, own_type, file_aliases, &mut seen);
    }
    seen.len()
}

/// One syntactically-typed local's type name, as introduced by a parameter, a `var
/// x T`, or a `x := T{}`-shaped composite literal — see `simple_type_name` for which
/// type-expression shapes are recognized.
pub(crate) fn collect_typed_locals(method: Node, source: &str, out: &mut HashMap<String, String>) {
    // The method's own (non-receiver) parameters.
    if let Some(params) = method.child_by_field_name("parameters") {
        let mut cursor = params.walk();
        for decl in params
            .children(&mut cursor)
            .filter(|c| c.kind() == "parameter_declaration")
        {
            let Some(ty) = decl
                .child_by_field_name("type")
                .and_then(|t| simple_type_name(t, source))
            else {
                continue;
            };
            let mut nc = decl.walk();
            for name_node in decl.children_by_field_name("name", &mut nc) {
                out.insert(node_text(name_node, source).to_string(), ty.clone());
            }
        }
    }
    walk_typed_locals(method, source, out);
}

fn walk_typed_locals(node: Node, source: &str, out: &mut HashMap<String, String>) {
    match node.kind() {
        "var_spec" => {
            if let Some(ty) = node
                .child_by_field_name("type")
                .and_then(|t| simple_type_name(t, source))
            {
                let mut nc = node.walk();
                for name_node in node.children_by_field_name("name", &mut nc) {
                    out.insert(node_text(name_node, source).to_string(), ty.clone());
                }
            }
        }
        "short_var_declaration" => {
            if let (Some(left), Some(right)) = (
                node.child_by_field_name("left"),
                node.child_by_field_name("right"),
            ) && left.kind() == "expression_list"
                && right.kind() == "expression_list"
                && left.named_child_count() == 1
                && right.named_child_count() == 1
                && let Some(name_node) = left.named_child(0)
                && name_node.kind() == "identifier"
                && let Some(value) = right.named_child(0)
                && value.kind() == "composite_literal"
                && let Some(ty) = value
                    .child_by_field_name("type")
                    .and_then(|t| simple_type_name(t, source))
            {
                out.insert(node_text(name_node, source).to_string(), ty);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_typed_locals(child, source, out);
    }
}

/// The type name for a type expression, or `None` when the shape isn't one this checker
/// resolves without real type inference: `type_identifier` (`Foo`) and `pointer_type`
/// wrapping one (`*Foo`) resolve to a bare, local-package name; `qualified_type`
/// (`pkg.Foo`, a different package) resolves too, but to the raw `"pkg.Foo"` text
/// verbatim rather than a bare name — callers that need the real target package
/// (`resolve_qualified_type`) split on the first `.`, which is safe because a Go type
/// identifier can never itself contain one. A local whose type comes from anything else
/// (a function's return value, an interface's dynamic type, a generic instantiation)
/// still resolves to `None` — that ceiling is unaffected by cross-package resolution.
pub(crate) fn simple_type_name(ty: Node, source: &str) -> Option<String> {
    match ty.kind() {
        "type_identifier" => Some(node_text(ty, source).to_string()),
        "pointer_type" => ty.named_child(0).and_then(|c| simple_type_name(c, source)),
        "qualified_type" => Some(node_text(ty, source).to_string()),
        _ => None,
    }
}

/// Resolves a `simple_type_name` result to `"{target_package}::{bare_name}"` when it's a
/// `pkg.Foo`-qualified name whose `pkg` alias resolves in `file_aliases` (that file's
/// `ArchModel::file_import_aliases` entry) — `None` for a same-package bare name (no `.`)
/// or a qualified name whose alias didn't resolve (an external/stdlib import, or a `.`
/// that isn't actually a package qualifier — Go's grammar guarantees it is, for a real
/// `qualified_type` node, so this is defensive rather than expected). Split rather than
/// resolved eagerly in `simple_type_name` itself, since only some callers (ATFD, ISP
/// consumer detection) need the target package — same-type-name comparisons (`local_type
/// != own_type`) work fine on the raw text either way.
pub(crate) fn resolve_qualified_type(
    type_name: &str,
    file_aliases: Option<&HashMap<String, String>>,
) -> Option<String> {
    let (alias, name) = type_name.split_once('.')?;
    let target_package = file_aliases?.get(alias)?;
    Some(format!("{target_package}::{name}"))
}

/// `local_type`'s foreign-type identity for ATFD purposes, or `None` when it isn't
/// foreign: a same-package bare name equal to `own_type` is the receiver's own type, not
/// foreign data; a qualified (`pkg.Foo`) name resolves via `resolve_qualified_type` (see
/// its own doc comment for why an unresolved qualified name — an external/stdlib import —
/// is dropped rather than kept under its raw, ambiguous text).
fn foreign_type_of(
    local_type: &str,
    own_type: &str,
    file_aliases: Option<&HashMap<String, String>>,
) -> Option<String> {
    if local_type.contains('.') {
        resolve_qualified_type(local_type, file_aliases)
    } else if local_type != own_type {
        Some(local_type.to_string())
    } else {
        None
    }
}

/// Threaded context for `walk_selectors_on_typed_locals`'s recursive walk — bundled
/// rather than passed as separate parameters once cross-package resolution added a
/// fourth thing every recursive call needs to carry unchanged.
struct ForeignAccessCtx<'a> {
    own_type: &'a str,
    typed_locals: &'a HashMap<String, String>,
    file_aliases: Option<&'a HashMap<String, String>>,
}

fn collect_foreign_field_accesses(
    method: Node,
    source: &str,
    own_type: &str,
    file_aliases: Option<&HashMap<String, String>>,
    out: &mut HashSet<(String, String)>,
) {
    let mut typed_locals = HashMap::new();
    collect_typed_locals(method, source, &mut typed_locals);
    if typed_locals.is_empty() {
        return;
    }
    let ctx = ForeignAccessCtx {
        own_type,
        typed_locals: &typed_locals,
        file_aliases,
    };
    walk_selectors_on_typed_locals(method, source, &ctx, out);
}

fn walk_selectors_on_typed_locals(
    node: Node,
    source: &str,
    ctx: &ForeignAccessCtx,
    out: &mut HashSet<(String, String)>,
) {
    if node.kind() == "selector_expression"
        && !is_call_target(node)
        && let Some(operand) = node.child_by_field_name("operand")
        && operand.kind() == "identifier"
        && let Some(local_type) = ctx.typed_locals.get(node_text(operand, source))
        && let Some(foreign_type) = foreign_type_of(local_type, ctx.own_type, ctx.file_aliases)
        && let Some(field) = node.child_by_field_name("field")
    {
        out.insert((foreign_type, node_text(field, source).to_string()));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_selectors_on_typed_locals(child, source, ctx, out);
    }
}

/// Same "a selector that's really a method call doesn't count as data access" exclusion
/// `symbol_extract::selector_is_call_target` applies to receiver field accesses — ATFD
/// (Lanza & Marinescu's original definition) is about foreign *attribute* access, not
/// foreign method calls.
pub(crate) fn is_call_target(selector: Node) -> bool {
    selector.parent().is_some_and(|p| {
        p.kind() == "call_expression"
            && p.child_by_field_name("function").is_some_and(|f| {
                f.start_byte() == selector.start_byte() && f.end_byte() == selector.end_byte()
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{AccessKind, FieldAccessEdge, SymbolKind};
    use std::collections::BTreeMap;

    fn write_go_file(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn method_symbol(
        pkg: &str,
        type_name: &str,
        name: &str,
        file: &Path,
        line: usize,
    ) -> SymbolNode {
        SymbolNode {
            id: format!("{pkg}::{type_name}.{name}"),
            name: name.to_string(),
            kind: SymbolKind::Method,
            file: file.to_path_buf(),
            line,
            exported: true,
            parent: Some(type_name.to_string()),
        }
    }

    fn field_access(from: &str, type_name: &str, field: &str, pkg: &str) -> FieldAccessEdge {
        FieldAccessEdge {
            from: from.to_string(),
            to: format!("{pkg}::{type_name}.{field}"),
            access: AccessKind::Read,
            file: PathBuf::new(),
            line: 0,
        }
    }

    #[test]
    fn file_cache_parses_a_second_shorter_file_correctly_after_a_longer_first_one() {
        // Regression for a real panic found backtesting against kubernetes/kubernetes:
        // sharing one `GrammarCache` across files meant a file parsed after a longer one
        // got that longer file's stale tree back, paired with its own (shorter) source —
        // `node_text` then indexed past the end of the second file's string. Parsing a
        // long file first, then a short one, and confirming the short one's own content
        // comes back (not a byte-index panic, not the long file's leftover content) is
        // the direct regression check.
        let dir = crate::test_support::unique_temp_dir("god-class-file-cache");
        let long = write_go_file(
            &dir,
            "long.go",
            &format!(
                "package pkg\n\nfunc Long() {{\n{}\n}}\n",
                "\t_ = 1\n".repeat(200)
            ),
        );
        let short = write_go_file(&dir, "short.go", "package pkg\n\nfunc Short() {}\n");

        let mut files = FileCache::new();
        {
            let (long_source, _) = files.get(&long).expect("long.go parses");
            assert_eq!(long_source.matches("Long").count(), 1);
        }

        let (short_source, short_tree) = files.get(&short).expect("short.go parses");
        assert_eq!(short_source, "package pkg\n\nfunc Short() {}\n");
        let node = find_method_node(short_tree.root_node(), 3, "Short", short_source)
            .expect("Short is found in short.go's own (correctly re-parsed) tree");
        // The real assertion is that this matched a node inside `short_source`'s own
        // byte range, not a leftover node from the (much longer) cached `long.go` tree —
        // indexing past `short_source`'s end is exactly the panic this regression covers.
        assert!(node.end_byte() <= short_source.len());
    }

    #[test]
    fn tcc_is_one_when_every_pair_shares_a_field() {
        let ids = ["m1", "m2", "m3"];
        let model = ArchModel {
            repo_root: PathBuf::new(),
            packages: BTreeMap::new(),
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![
                field_access("m1", "T", "x", "pkg"),
                field_access("m2", "T", "x", "pkg"),
                field_access("m3", "T", "x", "pkg"),
            ],
            file_import_aliases: BTreeMap::new(),
            pruning: crate::arch_model::PruningSummary::default(),
        };
        assert_eq!(tight_class_cohesion(&ids, &model), 1.0);
    }

    #[test]
    fn tcc_is_zero_when_no_pair_shares_a_field() {
        let ids = ["m1", "m2", "m3"];
        let model = ArchModel {
            repo_root: PathBuf::new(),
            packages: BTreeMap::new(),
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![
                field_access("m1", "T", "x", "pkg"),
                field_access("m2", "T", "y", "pkg"),
                field_access("m3", "T", "z", "pkg"),
            ],
            file_import_aliases: BTreeMap::new(),
            pruning: crate::arch_model::PruningSummary::default(),
        };
        assert_eq!(tight_class_cohesion(&ids, &model), 0.0);
    }

    #[test]
    fn wmc_sums_cyclomatic_complexity_across_methods() {
        let dir = crate::test_support::unique_temp_dir("god-class");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{}\n\nfunc (t T) A() {\n\tif true {\n\t}\n}\n\nfunc (t T) B() {\n\tif true {\n\t} else if false {\n\t}\n}\n",
        );
        let symbols_by_id: HashMap<&str, &SymbolNode> = HashMap::new();
        let a = method_symbol("pkg", "T", "A", &file, 5);
        let b = method_symbol("pkg", "T", "B", &file, 10);
        let mut map = symbols_by_id;
        map.insert("a", &a);
        map.insert("b", &b);
        let mut files = FileCache::new();
        // A: 1 (base) + 1 (if) = 2. B: 1 (base) + 1 (if) + 1 (else if) = 3. Total 5.
        assert_eq!(total_wmc(&["a", "b"], &map, &mut files), 5);
    }

    fn model_with_aliases(aliases: BTreeMap<PathBuf, HashMap<String, String>>) -> ArchModel {
        ArchModel {
            repo_root: PathBuf::new(),
            packages: BTreeMap::new(),
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![],
            file_import_aliases: aliases,
            pruning: crate::arch_model::PruningSummary::default(),
        }
    }

    fn empty_test_model() -> ArchModel {
        model_with_aliases(BTreeMap::new())
    }

    #[test]
    fn atfd_counts_distinct_foreign_fields_through_a_typed_parameter() {
        let dir = crate::test_support::unique_temp_dir("god-class");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{}\n\ntype Other struct{ Y int }\n\nfunc (t T) M(o *Other) {\n\t_ = o.Y\n\t_ = o.Y\n}\n",
        );
        let sym = method_symbol("pkg", "T", "M", &file, 7);
        let mut map: HashMap<&str, &SymbolNode> = HashMap::new();
        map.insert("m", &sym);
        let mut files = FileCache::new();
        assert_eq!(
            distinct_foreign_field_accesses(&["m"], &map, "T", &mut files, &empty_test_model()),
            1,
            "same (Other, Y) pair accessed twice must count once"
        );
    }

    #[test]
    fn atfd_ignores_access_through_the_receivers_own_type() {
        let dir = crate::test_support::unique_temp_dir("god-class");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{ X int }\n\nfunc (t T) M(other T) {\n\t_ = other.X\n}\n",
        );
        let sym = method_symbol("pkg", "T", "M", &file, 5);
        let mut map: HashMap<&str, &SymbolNode> = HashMap::new();
        map.insert("m", &sym);
        let mut files = FileCache::new();
        assert_eq!(
            distinct_foreign_field_accesses(&["m"], &map, "T", &mut files, &empty_test_model()),
            0
        );
    }

    #[test]
    fn atfd_ignores_a_foreign_method_call_not_a_field_access() {
        let dir = crate::test_support::unique_temp_dir("god-class");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{}\n\ntype Other struct{}\n\nfunc (o Other) Helper() int { return 0 }\n\nfunc (t T) M(o *Other) {\n\t_ = o.Helper()\n}\n",
        );
        let sym = method_symbol("pkg", "T", "M", &file, 9);
        let mut map: HashMap<&str, &SymbolNode> = HashMap::new();
        map.insert("m", &sym);
        let mut files = FileCache::new();
        assert_eq!(
            distinct_foreign_field_accesses(&["m"], &map, "T", &mut files, &empty_test_model()),
            0
        );
    }

    #[test]
    fn atfd_drops_a_cross_package_qualified_type_with_no_resolvable_alias() {
        // No `file_import_aliases` entry for this file at all (as if the import were
        // stdlib/external, or simply unmodeled) — must drop the access rather than
        // fabricate a foreign-type identity from the raw, ambiguous `"other.Other"` text.
        let dir = crate::test_support::unique_temp_dir("god-class");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{}\n\nfunc (t T) M(o other.Other) {\n\t_ = o.Y\n}\n",
        );
        let sym = method_symbol("pkg", "T", "M", &file, 5);
        let mut map: HashMap<&str, &SymbolNode> = HashMap::new();
        map.insert("m", &sym);
        let mut files = FileCache::new();
        assert_eq!(
            distinct_foreign_field_accesses(&["m"], &map, "T", &mut files, &empty_test_model()),
            0,
            "an unresolvable qualified type must not be counted under its raw alias text"
        );
    }

    #[test]
    fn atfd_resolves_a_cross_package_qualified_type_via_the_files_import_alias() {
        let dir = crate::test_support::unique_temp_dir("god-class");
        let file = write_go_file(
            &dir,
            "t.go",
            "package pkg\n\ntype T struct{}\n\nfunc (t T) M(o other.Other) {\n\t_ = o.Y\n}\n",
        );
        let sym = method_symbol("pkg", "T", "M", &file, 5);
        let mut map: HashMap<&str, &SymbolNode> = HashMap::new();
        map.insert("m", &sym);
        let mut files = FileCache::new();
        let mut aliases = BTreeMap::new();
        aliases.insert(
            file.clone(),
            HashMap::from([("other".to_string(), "some/other".to_string())]),
        );
        assert_eq!(
            distinct_foreign_field_accesses(
                &["m"],
                &map,
                "T",
                &mut files,
                &model_with_aliases(aliases)
            ),
            1,
            "a resolvable cross-package qualified type must now count toward ATFD"
        );
    }
}
