//! Fowler's **Feature Envy** → **Move Function** / **Move Field**: a method that reaches into
//! another type's data far more than its own is a candidate to move to that type
//! (Tsantalis & Chatzigeorgiou, "Identification of Move Method Refactoring Opportunities",
//! IEEE TSE 2009; ATFD from Lanza & Marinescu, as used by `god_class.rs`).
//!
//! Foreign *field* accesses come from `god_class`'s ATFD walker (syntactically-typed locals
//! only — no type inference, same ceiling as `GodClassChecker`). Foreign *calls* come from
//! `ArchModel::call_edges`. Own accesses are `ArchModel::field_accesses` from the method.
//! Go only (via `methods_by_type`). Opt-in through `model_registry()`, not `default_checks()`.
//!
//! Skipped on purpose: test files, `*Adapter`/`*Wrapper`/`*Proxy`/`*Decorator` types, pure
//! delegations, and methods with too few total accesses to carry a ratio.

use std::collections::{HashMap, HashSet};

use tree_sitter::Node;

use crate::arch_model::{ArchModel, PackageNode, SymbolNode};
use crate::architecture_checks::{ArchFinding, ArchModelChecker, methods_by_type};
use crate::checkers::is_test_file;
use crate::config::ArchitectureConfig;
use crate::god_class::{FileCache, collect_foreign_field_accesses, find_method_node};

/// Minimum foreign share of all of a method's counted accesses.
const FEATURE_ENVY_RATIO_THRESHOLD: f64 = 0.33;
/// Minimum foreign accesses before flagging; noise guard for tiny methods.
const FEATURE_ENVY_MIN_FOREIGN: usize = 3;
/// Minimum foreign + own accesses before a method is evaluated at all.
const FEATURE_ENVY_MIN_TOTAL: usize = 4;
const INTENTIONAL_DELEGATION_SUFFIXES: [&str; 4] = ["Adapter", "Wrapper", "Proxy", "Decorator"];

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
        let mut findings: Vec<ArchFinding> = model
            .packages
            .values()
            .flat_map(|pkg| feature_envy_findings_for_package(pkg, model))
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

/// Resolved calls from `method_id` to methods of a different type, keyed `"{pkg}::{Type}"`.
/// Method ids are `{pkg}::{Type}.{name}`; free-function targets have no type prefix and are
/// not counted.
fn foreign_call_counts_per_type(method_id: &str, model: &ArchModel) -> HashMap<String, usize> {
    let own_prefix = method_id.rsplit_once('.').map(|(prefix, _)| prefix);
    let mut counts: HashMap<String, usize> = HashMap::new();
    for edge in &model.call_edges {
        if !edge.resolved || edge.from != method_id {
            continue;
        }
        let Some((callee_type, _)) = edge.to.rsplit_once('.') else {
            continue;
        };
        // A package path can contain '.', so a free function `k8s.io/x::F` splits to `k8s`.
        if !callee_type.contains("::") || Some(callee_type) == own_prefix {
            continue;
        }
        *counts.entry(callee_type.to_string()).or_insert(0) += 1;
    }
    counts
}

fn local_field_access_count(method_id: &str, model: &ArchModel) -> usize {
    model
        .field_accesses
        .iter()
        .filter(|a| a.from == method_id)
        .count()
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

fn feature_envy_findings_for_package(pkg: &PackageNode, model: &ArchModel) -> Vec<ArchFinding> {
    let symbols_by_id: HashMap<&str, &SymbolNode> =
        pkg.symbols.iter().map(|s| (s.id.as_str(), s)).collect();
    let mut files = FileCache::new();
    let mut findings = Vec::new();

    for (type_name, ids) in methods_by_type(pkg) {
        if INTENTIONAL_DELEGATION_SUFFIXES
            .iter()
            .any(|suffix| type_name.ends_with(suffix))
        {
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
            let call_counts = foreign_call_counts_per_type(id, model);
            let local = local_field_access_count(id, model);

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

    // Zero-own methods have ratio 1.0 by definition; require real foreign volume.
    if c.local == 0 && foreign_fn < FEATURE_ENVY_MIN_FOREIGN {
        return None;
    }
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
            delegation_field: None,
            params: vec![],
            used_as_value: false,
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
        model.call_edges.push(CallEdge {
            from: from.to_string(),
            to: to.to_string(),
            resolved: true,
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

    const CALC_LINE: usize = 6;

    fn run(
        dir_tag: &str,
        file_name: &str,
        source: &str,
        setup: impl FnOnce(&mut ArchModel),
    ) -> Vec<ArchFinding> {
        let dir = crate::test_support::unique_temp_dir(dir_tag);
        let file = dir.join(file_name);
        std::fs::write(&file, source).unwrap();
        let mut model = model_of(vec![symbol("Order", "Calc", &file, CALC_LINE)]);
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
        let findings = run("fe-fn", "order.go", &go_source(2), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            call(m, "pkg::Order.Calc", "pkg::Customer.Rate");
            call(m, "pkg::Order.Calc", "pkg::Customer.Base");
        });
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].message;
        assert!(
            msg.starts_with("[feature-envy:move-function] pkg::Order.Calc (order.go:6)")
                || msg.contains("order.go:6)"),
            "{msg}"
        );
        assert!(
            msg.contains(
                "4 foreign accesses (4 to pkg::Customer) vs 1 own (ratio 0.80, threshold 0.33); \
             dominant target: pkg::Customer — consider Move Function"
            ),
            "{msg}"
        );
        assert_eq!(findings[0].line, Some(CALC_LINE));
    }

    #[test]
    fn flags_move_field_candidate() {
        let findings = run("fe-fld", "order.go", &go_source(4), |m| {
            own_access(m, "pkg::Order.Calc", 1);
        });
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].message;
        assert!(
            msg.starts_with("[feature-envy:move-field] pkg::Order.Calc"),
            "{msg}"
        );
        assert!(
            msg.contains("4 foreign field accesses (4 to pkg::Customer) vs 1 own (ratio 0.80"),
            "{msg}"
        );
        assert!(msg.ends_with("consider Move Field"), "{msg}");
    }

    #[test]
    fn suppresses_test_file_method() {
        let findings = run("fe-test", "order_test.go", &go_source(4), |m| {
            own_access(m, "pkg::Order.Calc", 1);
        });
        assert!(findings.is_empty());
    }

    #[test]
    fn suppresses_pure_delegation_adapter_type_and_low_total() {
        let delegating = run("fe-deleg", "order.go", &go_source(4), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            m.packages.get_mut("pkg").unwrap().symbols[0].is_pure_delegation = true;
        });
        assert!(delegating.is_empty());

        let adapter = run("fe-adapter", "order.go", &go_source(4), |m| {
            own_access(m, "pkg::OrderAdapter.Calc", 1);
            let sym = &mut m.packages.get_mut("pkg").unwrap().symbols[0];
            sym.parent = Some("OrderAdapter".to_string());
            sym.id = "pkg::OrderAdapter.Calc".to_string();
        });
        assert!(adapter.is_empty());

        // 3 foreign + 0 own = total 3 < MIN_TOTAL.
        let tiny = run("fe-tiny", "order.go", &go_source(3), |_| {});
        assert!(tiny.is_empty());
    }

    #[test]
    fn below_threshold_is_a_true_negative() {
        // Too few foreign accesses.
        let few = run("fe-few", "order.go", &go_source(2), |m| {
            own_access(m, "pkg::Order.Calc", 3);
        });
        assert!(few.is_empty());
        // 3 foreign vs 7 own = ratio 0.30 < 0.33.
        let low_ratio = run("fe-ratio", "order.go", &go_source(3), |m| {
            own_access(m, "pkg::Order.Calc", 7);
        });
        assert!(low_ratio.is_empty());
    }

    #[test]
    fn bare_and_qualified_same_package_keys_merge() {
        // 2 field accesses (bare `Customer`) + 2 calls (`pkg::Customer`): neither alone
        // reaches MIN_FOREIGN=3 as a split, nor does field-only (2); merged it is 4.
        let findings = run("fe-norm", "order.go", &go_source(2), |m| {
            own_access(m, "pkg::Order.Calc", 1);
            call(m, "pkg::Order.Calc", "pkg::Customer.Rate");
            call(m, "pkg::Order.Calc", "pkg::Customer.Base");
        });
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0]
                .message
                .contains("4 foreign accesses (4 to pkg::Customer)")
        );

        let merged = merge_counts(
            &qualify_keys(HashMap::from([("Customer".to_string(), 3)]), "pkg"),
            &HashMap::from([("pkg::Customer".to_string(), 2)]),
        );
        assert_eq!(merged, HashMap::from([("pkg::Customer".to_string(), 5)]));
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
        let mut long_src = go_source(4);
        long_src.push_str(&"// padding line\n".repeat(200));
        std::fs::write(&long, &long_src).unwrap();
        let short = dir.join("b_short.go");
        std::fs::write(
            &short,
            "package pkg\n\ntype Other struct{}\n\nfunc (o Other) Tiny() {}\n",
        )
        .unwrap();
        let mut model = model_of(vec![
            symbol("Order", "Calc", &long, CALC_LINE),
            symbol("Other", "Tiny", &short, 5),
        ]);
        own_access(&mut model, "pkg::Order.Calc", 1);
        let findings = FeatureEnvyChecker.check(&model, &ArchitectureConfig::default());
        assert_eq!(findings.len(), 1);
        assert!(findings[0].message.contains("pkg::Order.Calc"));
    }
}
