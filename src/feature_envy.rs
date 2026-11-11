//! Fowler's **Feature Envy** → **Move Function** / **Move Field**: a method that reaches into
//! another type's data far more than its own (Tsantalis & Chatzigeorgiou, IEEE TSE 2009).
//! Foreign field accesses come from `god_class`'s ATFD walker, foreign calls from
//! `ArchModel::call_edges`, own accesses from `ArchModel::field_accesses`. Go only; opt-in via
//! `model_registry()`. Skips test files, delegation-named types, pure delegations, tiny methods.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::arch_model::{ArchModel, PackageNode, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker, methods_by_type};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;
use crate::god_class::{FileCache, collect_foreign_field_accesses, find_method_node};
use crate::middle_man::is_intentional_delegation_type;

/// Minimum foreign share of all of a method's counted accesses.
const FEATURE_ENVY_RATIO_THRESHOLD: f64 = 0.33;
/// Minimum foreign accesses before flagging; noise guard for tiny methods. Raised from 3
/// after the public-corpus backtest (see `project_plans/feature-envy-detection/decisions/validation.md`).
const FEATURE_ENVY_MIN_FOREIGN: usize = 5;
/// Minimum foreign + own accesses before a method is evaluated at all.
const FEATURE_ENVY_MIN_TOTAL: usize = 4;

pub struct FeatureEnvyChecker;

impl ArchModelChecker for FeatureEnvyChecker {
    fn name(&self) -> &str {
        "feature-envy"
    }

    /// The subject is usually an unexported helper method, which pruning would drop.
    fn needs_private_symbols(&self) -> bool {
        true
    }

    fn check(&self, model: &ArchModel, _config: &ArchitectureConfig) -> Vec<ArchFinding> {
        let index = EdgeIndex::new(model);
        let mut findings: Vec<ArchFinding> = model
            .packages
            .values()
            .flat_map(|pkg| feature_envy_findings_for_package(pkg, model, &index))
            .collect();
        findings.sort_by(|a, b| (&a.file, a.line).cmp(&(&b.file, b.line)));
        findings
    }
}

/// Foreign type -> count of distinct accessed fields, call targets excluded.
fn foreign_field_accesses_per_type(
    method: Node,
    source: &str,
    own_type: &str,
    file_aliases: Option<&HashMap<String, String>>,
) -> HashMap<String, usize> {
    let mut pairs: HashSet<(String, String)> = HashSet::new();
    collect_foreign_field_accesses(method, source, own_type, file_aliases, &mut pairs);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (ty, _field) in pairs {
        *counts.entry(ty).or_insert(0) += 1;
    }
    counts
}

/// Per-method views of the model's edge lists, built once so each method lookup is O(1)
/// instead of a scan over every edge in the repo.
struct EdgeIndex<'a> {
    resolved_callees: HashMap<&'a str, Vec<&'a str>>,
    own_field_accesses: HashMap<&'a str, usize>,
}

impl<'a> EdgeIndex<'a> {
    fn new(model: &'a ArchModel) -> Self {
        let mut resolved_callees: HashMap<&str, Vec<&str>> = HashMap::new();
        for edge in model.call_edges.iter().filter(|e| e.resolved) {
            resolved_callees
                .entry(edge.from.as_str())
                .or_default()
                .push(edge.to.as_str());
        }
        let mut own_field_accesses: HashMap<&str, usize> = HashMap::new();
        for access in &model.field_accesses {
            *own_field_accesses.entry(access.from.as_str()).or_insert(0) += 1;
        }
        Self {
            resolved_callees,
            own_field_accesses,
        }
    }

    fn callees(&self, method_id: &str) -> &[&'a str] {
        self.resolved_callees
            .get(method_id)
            .map_or(&[], Vec::as_slice)
    }

    fn local_field_access_count(&self, method_id: &str) -> usize {
        self.own_field_accesses.get(method_id).copied().unwrap_or(0)
    }
}

/// Calls from `method_id` to methods of a different type, keyed `"{pkg}::{Type}"`, plus the
/// number of calls to its own type. Method ids are `{pkg}::{Type}.{name}`; free-function
/// targets have no type prefix and are not counted.
fn call_counts_by_type(method_id: &str, callees: &[&str]) -> (HashMap<String, usize>, usize) {
    let own_prefix = method_id.rsplit_once('.').map(|(prefix, _)| prefix);
    let mut foreign: HashMap<String, usize> = HashMap::new();
    let mut own_calls = 0;
    for callee in callees {
        let Some((callee_type, _)) = callee.rsplit_once('.') else {
            continue;
        };
        // A package path can contain '.', so a free function `k8s.io/x::F` splits to `k8s`.
        if !callee_type.contains("::") {
            continue;
        }
        if Some(callee_type) == own_prefix {
            own_calls += 1;
        } else {
            *foreign.entry(callee_type.to_string()).or_insert(0) += 1;
        }
    }
    (foreign, own_calls)
}

/// Same-package foreign types come back from the ATFD walker as bare names, while call
/// edges are always `pkg::Type`; qualify so both sides merge under one key.
fn qualify_keys(counts: HashMap<String, usize>, pkg_path: &str) -> HashMap<String, usize> {
    let mut out: HashMap<String, usize> = HashMap::new();
    for (key, n) in counts {
        let key = if key.contains("::") {
            key
        } else {
            format!("{pkg_path}::{key}")
        };
        *out.entry(key).or_insert(0) += n;
    }
    out
}

fn merge_counts(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> HashMap<String, usize> {
    let mut out = a.clone();
    for (k, v) in b {
        *out.entry(k.clone()).or_insert(0) += v;
    }
    out
}

/// Highest count wins; ties break lexicographically so output is deterministic.
fn dominant_type(counts: &HashMap<String, usize>) -> Option<(&str, usize)> {
    counts
        .iter()
        .min_by_key(|(name, n)| (std::cmp::Reverse(**n), name.as_str()))
        .map(|(name, n)| (name.as_str(), *n))
}

/// Whether `foreign`/`local` cross every threshold.
fn is_envious(foreign: usize, local: usize) -> bool {
    let total = foreign + local;
    total >= FEATURE_ENVY_MIN_TOTAL
        && foreign >= FEATURE_ENVY_MIN_FOREIGN
        && foreign as f64 / total as f64 >= FEATURE_ENVY_RATIO_THRESHOLD
}

fn feature_envy_findings_for_package(
    pkg: &PackageNode,
    model: &ArchModel,
    index: &EdgeIndex,
) -> Vec<ArchFinding> {
    let symbols_by_id: HashMap<&str, &SymbolNode> =
        pkg.symbols.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut files = FileCache::new();
    let mut findings = Vec::new();

    for (type_name, ids) in methods_by_type(pkg) {
        if is_intentional_delegation_type(type_name) {
            continue;
        }
        for id in ids {
            let Some(sym) = symbols_by_id.get(id).copied() else {
                continue;
            };
            if sym.is_pure_delegation
                || is_test_file(sym.file.strip_prefix(&model.repo_root).unwrap_or(&sym.file))
            {
                continue;
            }
            let Some((source, tree)) = files.get(&sym.file) else {
                continue;
            };
            let Some(node) = find_method_node(tree.root_node(), sym.line, &sym.name, source) else {
                continue;
            };
            let file_aliases = model.file_import_aliases.get(&sym.file);
            let field_counts = qualify_keys(
                foreign_field_accesses_per_type(node, source, type_name, file_aliases),
                &pkg.path,
            );
            // Call edges resolve by bare method name (`arch_model::resolve_one_call_edge`), so
            // `mu.Lock()` binds to an unrelated `noCopy.Lock`; trust a call target only when the
            // type-aware field walker also saw that type accessed in this method.
            let (mut call_counts, own_calls) = call_counts_by_type(id, index.callees(id));
            call_counts.retain(|ty, _| field_counts.contains_key(ty));
            let local = index.local_field_access_count(id);
            // A method that neither touches its own receiver's fields nor calls its own type's
            // methods is a free-standing helper, not an envious member.
            if local == 0 && own_calls == 0 {
                continue;
            }

            let counts = MethodCounts {
                field_counts,
                call_counts,
                local,
            };
            if let Some(finding) = evaluate_method(sym, &pkg.path, type_name, &counts, model) {
                findings.push(finding);
            }
        }
    }
    findings
}

/// One method's access tallies; `local` is own-receiver field accesses.
struct MethodCounts {
    field_counts: HashMap<String, usize>,
    call_counts: HashMap<String, usize>,
    local: usize,
}

/// Which variant fired, with the counts it was judged on.
struct Variant {
    label: &'static str,
    suggestion: &'static str,
    noun: &'static str,
    foreign: usize,
    counts: HashMap<String, usize>,
}

/// The narrower field-only variant wins when it holds on its own; otherwise the
/// field-plus-call (Move Function) variant is tried.
fn pick_variant(c: &MethodCounts) -> Option<Variant> {
    let merged = merge_counts(&c.field_counts, &c.call_counts);
    let foreign_fn: usize = merged.values().sum();
    let foreign_fld: usize = c.field_counts.values().sum();

    if is_envious(foreign_fld, c.local) {
        Some(Variant {
            label: "move-field",
            suggestion: "Move Field",
            noun: "foreign field accesses",
            foreign: foreign_fld,
            counts: c.field_counts.clone(),
        })
    } else if is_envious(foreign_fn, c.local) {
        Some(Variant {
            label: "move-function",
            suggestion: "Move Function",
            noun: "foreign accesses",
            foreign: foreign_fn,
            counts: merged,
        })
    } else {
        None
    }
}

fn evaluate_method(
    sym: &SymbolNode,
    pkg_path: &str,
    type_name: &str,
    c: &MethodCounts,
    model: &ArchModel,
) -> Option<ArchFinding> {
    let Variant {
        label,
        suggestion,
        noun,
        foreign,
        counts,
    } = pick_variant(c)?;
    let local = c.local;
    let (dominant, dominant_n) = dominant_type(&counts)?;
    let ratio = foreign as f64 / (foreign + local) as f64;
    let rel = sym.file.strip_prefix(&model.repo_root).unwrap_or(&sym.file);
    Some(ArchFinding {
        file: Some(sym.file.clone()),
        line: Some(sym.line),
        message: format!(
            "[feature-envy:{label}] {pkg_path}::{type_name}.{} ({}:{}): {foreign} {noun} \
             ({dominant_n} to {dominant}) vs {local} own (ratio {ratio:.2}, threshold \
             {FEATURE_ENVY_RATIO_THRESHOLD}); dominant target: {dominant} — consider {suggestion}",
            sym.name,
            rel.display(),
            sym.line,
        ),
        severity_override: None,
        operation: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arch_model::{AccessKind, CallEdge, FieldAccessEdge, PruningSummary, SymbolKind};
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    fn symbol(type_name: &str, name: &str, file: &Path, line: usize) -> SymbolNode {
        SymbolNode {
            id: format!("pkg::{type_name}.{name}"),
            name: name.to_string(),
            kind: SymbolKind::Method,
            file: file.to_path_buf(),
            line,
            exported: true,
            parent: Some(type_name.to_string()),
            is_pure_delegation: false,
            is_constant_return: false,
            delegation_field: None,
            params: vec![],
            used_as_value: false,
            end_line: None,
        }
    }

    fn model_of(symbols: Vec<SymbolNode>) -> ArchModel {
        let pkg = PackageNode {
            path: "pkg".to_string(),
            files: vec![],
            symbols,
        };
        ArchModel {
            repo_root: PathBuf::new(),
            packages: BTreeMap::from([("pkg".to_string(), pkg)]),
            import_edges: vec![],
            call_edges: vec![],
            field_accesses: vec![],
            type_edges: vec![],
            file_import_aliases: BTreeMap::new(),
            struct_field_names: Default::default(),
            struct_field_types: Default::default(),
            promoted_field_accesses: Default::default(),
            pruning: PruningSummary::default(),
        }
    }

    fn own_access(model: &mut ArchModel, from: &str, n: usize) {
        for i in 0..n {
            model.field_accesses.push(FieldAccessEdge {
                from: from.to_string(),
                to: format!("pkg::Order.f{i}"),
                access: AccessKind::Read,
                file: PathBuf::new(),
                line: 0,
            });
        }
    }

    fn call(model: &mut ArchModel, from: &str, to: &str) {
        call_edge(model, from, to, true);
    }

    fn call_edge(model: &mut ArchModel, from: &str, to: &str, resolved: bool) {
        model.call_edges.push(CallEdge {
            from: from.to_string(),
            to: to.to_string(),
            resolved,
            file: PathBuf::new(),
            line: 0,
        });
    }

    /// Go source with `Order.Calc(c Customer)` reading the first `n` Customer fields.
    fn go_source(n: usize) -> String {
        let fields = ["Tier", "Balance", "Name", "Age", "City"];
        let reads: String = fields[..n]
            .iter()
            .map(|f| format!("\t_ = c.{f}\n"))
            .collect();
        format!(
            "package pkg\n\ntype Customer struct{{ Tier, Balance, Name, Age, City int }}\n\
             type Order struct{{ n int }}\n\nfunc (o Order) Calc(c Customer) int {{\n{reads}\treturn o.n\n}}\n"
        )
    }

    /// 1-based line of the `Calc` func declaration in `source`.
    fn calc_line(source: &str) -> usize {
        source
            .lines()
            .position(|l| l.starts_with("func (o Order) Calc"))
            .expect("source declares Calc")
            + 1
    }

    fn run(
        dir_tag: &str,
        file_name: &str,
        source: &str,
        setup: impl FnOnce(&mut ArchModel),
    ) -> Vec<ArchFinding> {
        let dir = crate::test_support::unique_temp_dir(dir_tag);
        let file = dir.join(file_name);
        std::fs::write(&file, source).unwrap();
        let mut model = model_of(vec![symbol("Order", "Calc", &file, calc_line(source))]);
        setup(&mut model);
        FeatureEnvyChecker.check(&model, &ArchitectureConfig::default())
    }

    #[test]
    fn registered_in_model_registry_only_once() {
        let n = crate::architecture_checks::model_registry()
            .iter()
            .filter(|c| c.name() == "feature-envy")
            .count();
        assert_eq!(n, 1);
        assert!(FeatureEnvyChecker.needs_private_symbols());
    }

    #[test]
    fn flags_move_function_candidate_with_canonical_message() {
        let source = go_source(3);
        let line = calc_line(&source);
        let findings = run("fe-fn", "order.go", &source, |m| {
            own_access(m, "pkg::Order.Calc", 1);
            call(m, "pkg::Order.Calc", "pkg::Customer.Rate");
            call(m, "pkg::Order.Calc", "pkg::Customer.Base");
        });
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].message;
        assert!(
            msg.starts_with("[feature-envy:move-function] pkg::Order.Calc"),
            "{msg}"
        );
        // repo_root is empty in this model, so the location is the absolute temp path.
        assert!(msg.contains(&format!("/order.go:{line}):")), "{msg}");
        assert!(
            msg.contains(
                "5 foreign accesses (5 to pkg::Customer) vs 1 own (ratio 0.83, threshold 0.33); \
             dominant target: pkg::Customer — consider Move Function"
            ),
            "{msg}"
        );
        assert_eq!(findings[0].line, Some(line));
    }

    #[test]
    fn extra_call_edges_do_not_demote_a_move_field_candidate() {
        let findings = run("fe-fld-calls", "order.go", &go_source(5), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            call(m, "pkg::Order.Calc", "pkg::Customer.Rate");
            call(m, "pkg::Order.Calc", "pkg::Customer.Base");
        });
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].message;
        assert!(msg.starts_with("[feature-envy:move-field]"), "{msg}");
        assert!(msg.contains("5 foreign field accesses"), "{msg}");
    }

    #[test]
    fn unresolved_call_edges_are_not_counted() {
        let findings = run("fe-unres", "order.go", &go_source(3), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            call_edge(m, "pkg::Order.Calc", "pkg::Customer.Rate", false);
            call_edge(m, "pkg::Order.Calc", "pkg::Customer.Base", false);
        });
        assert!(findings.is_empty());
    }

    #[test]
    fn is_envious_thresholds() {
        for (foreign, local, expected) in
            [(5, 10, true), (5, 11, false), (4, 0, false), (5, 0, true)]
        {
            assert_eq!(is_envious(foreign, local), expected, "({foreign}, {local})");
        }
    }

    #[test]
    fn flags_move_field_candidate() {
        let findings = run("fe-fld", "order.go", &go_source(5), |m| {
            own_access(m, "pkg::Order.Calc", 1);
        });
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].message;
        assert!(
            msg.starts_with("[feature-envy:move-field] pkg::Order.Calc"),
            "{msg}"
        );
        assert!(
            msg.contains("5 foreign field accesses (5 to pkg::Customer) vs 1 own (ratio 0.83"),
            "{msg}"
        );
        assert!(msg.ends_with("consider Move Field"), "{msg}");
    }

    #[test]
    fn suppresses_test_file_method() {
        let findings = run("fe-test", "order_test.go", &go_source(5), |m| {
            own_access(m, "pkg::Order.Calc", 1);
        });
        assert!(findings.is_empty());
    }

    #[test]
    fn suppresses_pure_delegation() {
        let delegating = run("fe-deleg", "order.go", &go_source(5), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            m.packages.get_mut("pkg").unwrap().symbols[0].is_pure_delegation = true;
        });
        assert!(delegating.is_empty());
    }

    /// Re-homes the `Calc` symbol under `type_name` with one own access.
    fn run_as_type(tag: &str, type_name: &str) -> Vec<ArchFinding> {
        run(tag, "order.go", &go_source(5), |m| {
            let id = format!("pkg::{type_name}.Calc");
            own_access(m, &id, 1);
            let sym = &mut m.packages.get_mut("pkg").unwrap().symbols[0];
            sym.parent = Some(type_name.to_string());
            sym.id = id;
        })
    }

    #[test]
    fn suppresses_every_intentional_delegation_suffix() {
        for suffix in ["Adapter", "Wrapper", "Proxy", "Decorator"] {
            let findings = run_as_type(&format!("fe-sfx-{suffix}"), &format!("Order{suffix}"));
            assert!(findings.is_empty(), "Order{suffix} should be skipped");
        }
    }

    #[test]
    fn does_not_suppress_types_that_merely_resemble_a_delegation_suffix() {
        for type_name in ["Wrapped", "Adaptive"] {
            let findings = run_as_type(&format!("fe-near-{type_name}"), type_name);
            assert_eq!(findings.len(), 1, "{type_name} must still be flagged");
        }
    }

    #[test]
    fn below_min_foreign_with_own_signal_is_suppressed() {
        // 3 foreign + 1 own: total 4 passes MIN_TOTAL, so MIN_FOREIGN=5 is what suppresses it.
        let tiny = run("fe-tiny", "order.go", &go_source(3), |m| {
            own_access(m, "pkg::Order.Calc", 1);
        });
        assert!(tiny.is_empty());
    }

    #[test]
    fn below_threshold_is_a_true_negative() {
        // 4 foreign < MIN_FOREIGN=5 despite a high ratio.
        let few = run("fe-few", "order.go", &go_source(4), |m| {
            own_access(m, "pkg::Order.Calc", 1);
        });
        assert!(few.is_empty());
        // 5 foreign vs 11 own = ratio 0.31 < 0.33.
        let low_ratio = run("fe-ratio", "order.go", &go_source(5), |m| {
            own_access(m, "pkg::Order.Calc", 11);
        });
        assert!(low_ratio.is_empty());
    }

    #[test]
    fn bare_and_qualified_same_package_keys_merge() {
        // 3 field accesses (bare `Customer`) + 2 calls (`pkg::Customer`): neither alone
        // reaches MIN_FOREIGN=5 as a split; merged under one key it is 5.
        let findings = run("fe-norm", "order.go", &go_source(3), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            call(m, "pkg::Order.Calc", "pkg::Customer.Rate");
            call(m, "pkg::Order.Calc", "pkg::Customer.Base");
        });
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("5 foreign accesses (5 to pkg::Customer)")
        );

        let merged = merge_counts(
            &qualify_keys(HashMap::from([("Customer".to_string(), 3)]), "pkg"),
            &HashMap::from([("pkg::Customer".to_string(), 2)]),
        );
        assert_eq!(merged, HashMap::from([("pkg::Customer".to_string(), 5)]));
    }

    #[test]
    fn zero_own_signal_helper_is_suppressed_but_an_own_type_call_counts_as_signal() {
        let helper = run("fe-helper", "order.go", &go_source(5), |_| {});
        assert!(helper.is_empty());
        let member = run("fe-member", "order.go", &go_source(5), |m| {
            call(m, "pkg::Order.Calc", "pkg::Order.Helper");
        });
        assert_eq!(member.len(), 1);
    }

    #[test]
    fn name_collision_call_edges_without_field_evidence_are_ignored() {
        // `mu.Lock()`-style edges bound to an unrelated type must not tip a method over.
        let findings = run("fe-collide", "order.go", &go_source(3), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            for name in ["A", "B", "C"] {
                call(m, "pkg::Order.Calc", &format!("pkg::noCopy.{name}"));
            }
        });
        assert!(findings.is_empty());
    }

    #[test]
    fn dominant_type_is_highest_count_with_deterministic_ties() {
        let counts = HashMap::from([("pkg::A".to_string(), 3), ("pkg::B".to_string(), 5)]);
        assert_eq!(dominant_type(&counts), Some(("pkg::B", 5)));
        let tie = HashMap::from([("pkg::B".to_string(), 3), ("pkg::A".to_string(), 3)]);
        assert_eq!(dominant_type(&tie), Some(("pkg::A", 3)));
    }

    #[test]
    fn second_shorter_file_in_package_does_not_panic() {
        let dir = crate::test_support::unique_temp_dir("fe-stale");
        let long = dir.join("a_long.go");
        let mut long_src = go_source(5);
        long_src.push_str(&"// padding line\n".repeat(200));
        std::fs::write(&long, &long_src).unwrap();
        let short = dir.join("b_short.go");
        std::fs::write(
            &short,
            "package pkg\n\ntype Other struct{}\n\nfunc (o Other) Tiny() {}\n",
        )
        .unwrap();
        let mut model = model_of(vec![
            symbol("Order", "Calc", &long, calc_line(&long_src)),
            symbol("Other", "Tiny", &short, 5),
        ]);
        own_access(&mut model, "pkg::Order.Calc", 1);
        let findings = FeatureEnvyChecker.check(&model, &ArchitectureConfig::default());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("pkg::Order.Calc"));
    }
}
