//! Builders for each section of a [`SplitSimResult`](super::SplitSimResult).

use std::collections::{BTreeMap, BTreeSet};

use super::cycles::{Graph, cyclic_components};
use super::refs::{Collected, Ctx, Ref, is_test_file, norm};
use super::scan::platform_suffix;
use super::{
    Blocker, BlockerSeverity, CallersToRewrite, CycleEdge, CycleFinding, CycleSeverity, DepCount,
    ExportedSurfaceDelta, ReverseDepImpact, SurfaceEntry,
};
use crate::affected::{build_reverse_adjacency, reverse_bfs};
use crate::arch_model::SymbolKind;
use crate::import_graph::ImportEdge;

pub(super) fn node(package: &str, part: &str) -> String {
    format!("{package}#{part}")
}

fn site(r: &Ref) -> String {
    format!("{}:{}", r.file, r.line)
}

fn capped(sites: BTreeSet<String>, max: usize) -> Vec<String> {
    sites.into_iter().take(max).collect()
}

pub(super) fn cycles(ctx: &Ctx, refs: &[Ref], max_sites: usize) -> Vec<CycleFinding> {
    let build = |include_test: bool| -> Graph {
        let mut g = Graph::new();
        for r in refs.iter().filter(|r| !r.from_outside && !r.external_test) {
            if include_test || !r.test {
                g.entry(r.from.clone()).or_default().insert(r.to.clone());
            }
        }
        g
    };
    let prod = cyclic_components(&build(false));
    let all = cyclic_components(&build(true));
    let mut out = Vec::new();
    for scc in &prod {
        out.push(finding(
            ctx,
            refs,
            scc,
            CycleSeverity::Production,
            max_sites,
        ));
    }
    for scc in all.iter().filter(|s| !prod.contains(s)) {
        out.push(finding(ctx, refs, scc, CycleSeverity::TestOnly, max_sites));
    }
    out
}

fn finding(
    ctx: &Ctx,
    refs: &[Ref],
    scc: &[String],
    severity: CycleSeverity,
    max_sites: usize,
) -> CycleFinding {
    let members: BTreeSet<&str> = scc.iter().map(String::as_str).collect();
    let mut groups: BTreeMap<(String, String), Vec<&Ref>> = BTreeMap::new();
    for r in refs.iter().filter(|r| !r.from_outside && !r.external_test) {
        let in_scc = members.contains(r.from.as_str()) && members.contains(r.to.as_str());
        let counts = severity == CycleSeverity::TestOnly || !r.test;
        if in_scc && counts {
            groups
                .entry((r.from.clone(), r.to.clone()))
                .or_default()
                .push(r);
        }
    }
    let edges = groups
        .into_iter()
        .map(|((from, to), rs)| CycleEdge {
            from: node(ctx.package, &from),
            to: node(ctx.package, &to),
            symbols: rs
                .iter()
                .map(|r| r.symbol.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            sites: capped(rs.iter().map(|r| site(r)).collect(), max_sites),
            test_only: rs.iter().all(|r| r.test),
        })
        .collect();
    CycleFinding {
        severity,
        nodes: scc.iter().map(|p| node(ctx.package, p)).collect(),
        edges,
    }
}

pub(super) fn exported_surface(ctx: &Ctx, refs: &[Ref], max_sites: usize) -> ExportedSurfaceDelta {
    let mut by_symbol: BTreeMap<&str, Vec<&Ref>> = BTreeMap::new();
    for r in refs.iter().filter(|r| !r.from_outside && !r.external_test) {
        by_symbol.entry(r.symbol.as_str()).or_default().push(r);
    }
    let mut delta = ExportedSurfaceDelta::default();
    let mut crossing: BTreeSet<&str> = BTreeSet::new();
    for (symbol, rs) in &by_symbol {
        crossing.insert(symbol);
        let entry = SurfaceEntry {
            symbol: symbol.to_string(),
            kind: rs[0].kind.clone(),
            defined_in: rs[0].to.clone(),
            referenced_from: rs
                .iter()
                .map(|r| r.from.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            sites: capped(rs.iter().map(|r| site(r)).collect(), max_sites),
        };
        if rs[0].exported {
            delta.already_exported.push(entry);
        } else {
            delta.must_export.push(entry);
        }
    }
    for s in &ctx.pkg.symbols {
        let defined_in_test = is_test_file(&norm(&ctx.model.repo_root, &s.file));
        if !s.exported && !defined_in_test && !crossing.contains(s.id.as_str()) {
            delta.part_local_unexported.push(s.id.clone());
        }
    }
    delta.part_local_unexported.sort();
    delta
}

pub(super) fn callers(ctx: &Ctx, refs: &[Ref], max_sites: usize) -> Vec<CallersToRewrite> {
    let mut by_symbol: BTreeMap<&str, Vec<&Ref>> = BTreeMap::new();
    for r in refs
        .iter()
        .filter(|r| r.kind != "lock" && r.kind != "value")
    {
        by_symbol.entry(r.symbol.as_str()).or_default().push(r);
    }
    by_symbol
        .into_iter()
        .map(|(symbol, rs)| {
            let (test, prod): (Vec<&Ref>, Vec<&Ref>) = rs.iter().partition(|r| r.test);

            CallersToRewrite {
                symbol: symbol.to_string(),
                defined_in: rs[0].to.clone(),
                external_packages: rs
                    .iter()
                    .filter(|r| r.from_outside)
                    .map(|r| r.from.clone())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                referencing_parts: rs
                    .iter()
                    .filter(|r| !r.from_outside)
                    .map(|r| node(ctx.package, &r.from))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect(),
                production_count: prod.len(),
                test_count: test.len(),
                production_sites: capped(prod.iter().map(|r| site(r)).collect(), max_sites),
                test_sites: capped(test.iter().map(|r| site(r)).collect(), max_sites),
            }
        })
        .collect()
}

fn hard(kind: &str, file: &str, line: Option<usize>, part: &str, detail: String) -> Blocker {
    Blocker {
        kind: kind.to_string(),
        severity: BlockerSeverity::Hard,
        file: Some(file.to_string()),
        line,
        part: Some(part.to_string()),
        detail,
    }
}

pub(super) fn blockers(ctx: &Ctx, refs: &[Ref]) -> Vec<Blocker> {
    let mut out = file_blockers(ctx);
    out.extend(method_blockers(ctx));
    out.extend(field_blockers(ctx));
    out.extend(lock_blockers(refs));
    out.sort_by(|a, b| {
        (&a.kind, &a.file, a.line, &a.detail).cmp(&(&b.kind, &b.file, b.line, &b.detail))
    });
    out
}

fn file_blockers(ctx: &Ctx) -> Vec<Blocker> {
    let mut out = Vec::new();
    for (file, scan) in ctx.scans {
        let part = ctx.part_of(file).unwrap_or_default();
        let name = file.rsplit('/').next().unwrap_or(file);
        if let Some(line) = scan.init_line {
            out.push(hard("init_function", file, Some(line), part, "init() runs at package load; ordering relative to other init() functions and package-level initialisation across the new packages is not analysed — human review required".into()));
        }
        if let Some(line) = scan.build_tag_line {
            out.push(hard(
                "build_tag",
                file,
                Some(line),
                part,
                "file carries a build constraint; its platform variants must be split consistently"
                    .into(),
            ));
        }
        if let Some(suffix) = platform_suffix(name) {
            out.push(hard(
                "platform_filename_variant",
                file,
                None,
                part,
                format!("filename suffix _{suffix} is an implicit build constraint"),
            ));
        }
        if let Some(line) = scan.cgo_line {
            out.push(hard(
                "cgo",
                file,
                Some(line),
                part,
                "file uses cgo (import \"C\"); cgo preamble and symbols must stay together".into(),
            ));
        }
    }
    out
}

/// Go forbids declaring a method outside the package that declares its receiver type.
fn method_blockers(ctx: &Ctx) -> Vec<Blocker> {
    let syms = ctx.symbols();
    let mut out = Vec::new();
    for m in ctx
        .pkg
        .symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
    {
        let Some(ty) = m
            .parent
            .as_ref()
            .and_then(|p| syms.get(format!("{}::{p}", ctx.package).as_str()))
        else {
            continue;
        };
        let (mf, tf) = (
            norm(&ctx.model.repo_root, &m.file),
            norm(&ctx.model.repo_root, &ty.file),
        );
        if ctx.part_of(&mf) != ctx.part_of(&tf) {
            out.push(hard(
                "method_separated_from_receiver_type",
                &mf,
                Some(m.line),
                ctx.part_of(&mf).unwrap_or_default(),
                format!(
                    "method {} is in a different part than its receiver type {} ({tf})",
                    m.id, ty.id
                ),
            ));
        }
    }
    out
}

fn field_blockers(ctx: &Ctx) -> Vec<Blocker> {
    let syms = ctx.symbols();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    let mut out = Vec::new();
    for fa in &ctx.model.field_accesses {
        let Some((pkg, rest)) = fa.to.split_once("::") else {
            continue;
        };
        let Some((ty, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let Some(ty_sym) = (pkg == ctx.package)
            .then(|| syms.get(format!("{pkg}::{ty}").as_str()))
            .flatten()
        else {
            continue;
        };
        let file = norm(&ctx.model.repo_root, &fa.file);
        let (ap, tp) = (
            ctx.part_of(&file),
            ctx.part_of(&norm(&ctx.model.repo_root, &ty_sym.file)),
        );
        let exported = field.starts_with(char::is_uppercase);
        if exported || ap.is_none() || ap == tp || !seen.insert((fa.to.clone(), file.clone())) {
            continue;
        }
        out.push(hard(
            "shared_unexported_state",
            &file,
            Some(fa.line),
            ap.unwrap_or_default(),
            format!(
                "unexported field {ty}.{field} (declared in part {}) is accessed from part {}",
                tp.unwrap_or_default(),
                ap.unwrap_or_default()
            ),
        ));
    }
    out
}

fn lock_blockers(refs: &[Ref]) -> Vec<Blocker> {
    refs.iter()
        .filter(|r| r.kind == "lock")
        .map(|r| {
            hard(
                "shared_lock",
                &r.file,
                Some(r.line),
                &r.from,
                format!(
                    "package-level lock {} (declared in part {}) is used from part {}",
                    r.symbol, r.to, r.from
                ),
            )
        })
        .collect()
}

/// Distinct (from, to) package pairs, as `ImportEdge`s the shared reverse-BFS accepts.
fn pairs_to_edges(pairs: &BTreeSet<(String, String)>) -> Vec<ImportEdge> {
    pairs
        .iter()
        .map(|(from, to)| ImportEdge {
            from: from.clone(),
            to: to.clone(),
            file: Default::default(),
            line: 0,
        })
        .collect()
}

fn dep_count(node: &str, edges: &[ImportEdge]) -> DepCount {
    let adjacency = build_reverse_adjacency(edges);
    let direct = adjacency
        .get(node)
        .map_or(0, |s| s.iter().filter(|p| **p != node).count());
    let mut closure = reverse_bfs(&adjacency, &BTreeSet::from([node.to_string()]));
    closure.remove(node);
    DepCount {
        node: node.to_string(),
        direct,
        transitive: closure.len(),
        transitive_packages: closure.into_iter().collect(),
    }
}

pub(super) fn reverse_deps(ctx: &Ctx, parts: &[String], collected: &Collected) -> ReverseDepImpact {
    let mut before: BTreeSet<(String, String)> = BTreeSet::new();
    let mut after: BTreeSet<(String, String)> = BTreeSet::new();
    for e in ctx.model.import_edges.iter().filter(|e| e.from != e.to) {
        before.insert((e.from.clone(), e.to.clone()));
        if e.from != ctx.package && e.to != ctx.package {
            after.insert((e.from.clone(), e.to.clone()));
        }
    }
    for (part, pkg) in &collected.imports_out {
        after.insert((node(ctx.package, part), pkg.clone()));
    }
    for r in &collected.refs {
        let from = if r.from_outside {
            r.from.clone()
        } else {
            node(ctx.package, &r.from)
        };
        after.insert((from, node(ctx.package, &r.to)));
    }
    for (_, importer) in &collected.unattributed_importers {
        for part in parts {
            after.insert((importer.clone(), node(ctx.package, part)));
        }
    }
    let after_edges = pairs_to_edges(&after);
    ReverseDepImpact {
        before: dep_count(ctx.package, &pairs_to_edges(&before)),
        after: parts
            .iter()
            .map(|p| dep_count(&node(ctx.package, p), &after_edges))
            .collect(),
    }
}
