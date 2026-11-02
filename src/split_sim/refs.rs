//! Collects every reference that crosses a part boundary or reaches into the package from
//! outside, from the call/type edges in `ArchModel` plus package-level `var`/`const` uses
//! found by the source scan.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::scan::{FileScan, ValueDecl};
use crate::arch_model::{ArchModel, PackageNode, SymbolNode};

/// `path` as the model stores it (relative to the scan root, possibly `./`-prefixed or
/// prefixed with the scan root itself), reduced to a root-relative form.
pub(super) fn norm(root: &Path, path: &Path) -> String {
    let path = path.strip_prefix(root).unwrap_or(path);
    let s = path.to_string_lossy();
    s.strip_prefix("./").unwrap_or(&s).to_string()
}

pub(super) fn is_test_file(file: &str) -> bool {
    file.ends_with("_test.go")
}

#[derive(Debug, Clone)]
pub(super) struct Ref {
    /// Part name for a reference from inside the package, package key for one from outside.
    pub from: String,
    /// Part that defines the referenced symbol.
    pub to: String,
    pub symbol: String,
    pub kind: String,
    pub exported: bool,
    pub file: String,
    pub line: usize,
    pub test: bool,
    /// From an external test package (`package foo_test`): imports the package under test,
    /// so it can never close an import cycle.
    pub external_test: bool,
    pub from_outside: bool,
}

pub(super) struct Ctx<'a> {
    pub model: &'a ArchModel,
    pub package: &'a str,
    pub pkg: &'a PackageNode,
    pub file_part: &'a BTreeMap<String, String>,
    pub scans: &'a BTreeMap<String, FileScan>,
}

impl Ctx<'_> {
    pub fn symbols(&self) -> HashMap<&str, &SymbolNode> {
        self.pkg
            .symbols
            .iter()
            .map(|s| (s.id.as_str(), s))
            .collect()
    }

    pub fn part_of(&self, file: &str) -> Option<&str> {
        self.file_part.get(file).map(String::as_str)
    }

    pub fn external_test(&self, file: &str) -> bool {
        self.scans.get(file).is_some_and(|s| s.external_test)
    }
}

#[derive(Default)]
pub(super) struct Collected {
    pub refs: Vec<Ref>,
    /// (part, imported package) for imports made by files of the package.
    pub imports_out: BTreeSet<(String, String)>,
    /// (importing file, importing package) for importers with no attributable symbol use.
    pub unattributed_importers: BTreeSet<(String, String)>,
    pub unresolved_call_sites: usize,
}

fn file_packages(model: &ArchModel) -> HashMap<String, &str> {
    let mut map = HashMap::new();
    for (key, pkg) in &model.packages {
        for f in &pkg.files {
            map.insert(norm(&model.repo_root, f), key.as_str());
        }
    }
    map
}

/// Where a reference at `file` originates: a part (inside the package) or a package key.
///
/// An outside file only counts if it really imports the package: the model's call
/// resolution can match a TS/JS call, or a Go call through an unrelated package, to a
/// same-named symbol here, and such a phantom edge would inflate fan-out.
fn origin(
    ctx: &Ctx,
    file_pkgs: &HashMap<String, &str>,
    importers: &HashSet<String>,
    file: &str,
) -> Option<(String, bool)> {
    match file_pkgs.get(file) {
        Some(&p) if p == ctx.package => ctx.part_of(file).map(|part| (part.to_string(), false)),
        Some(&p) if importers.contains(file) => Some((p.to_string(), true)),
        _ => None,
    }
}

fn importing_files(ctx: &Ctx) -> HashSet<String> {
    ctx.model
        .import_edges
        .iter()
        .filter(|e| e.to == ctx.package && e.from != ctx.package)
        .map(|e| norm(&ctx.model.repo_root, &e.file))
        .collect()
}

fn add_ref(
    ctx: &Ctx,
    out: &mut Collected,
    sym: &SymbolNode,
    from: (String, bool),
    site: (&Path, usize),
) {
    let file = norm(&ctx.model.repo_root, site.0);
    let Some(to) = ctx.part_of(&norm(&ctx.model.repo_root, &sym.file)) else {
        return;
    };
    if !from.1 && from.0 == to {
        return;
    }
    out.refs.push(Ref {
        from: from.0,
        to: to.to_string(),
        symbol: sym.id.clone(),
        kind: format!("{:?}", sym.kind).to_lowercase(),
        exported: sym.exported,
        test: is_test_file(&file),
        external_test: !from.1 && ctx.external_test(&file),
        from_outside: from.1,
        file,
        line: site.1,
    });
}

pub(super) fn collect(ctx: &Ctx) -> Collected {
    let mut out = Collected::default();
    let file_pkgs = file_packages(ctx.model);
    let syms = ctx.symbols();
    collect_edges(ctx, &file_pkgs, &importing_files(ctx), &syms, &mut out);
    collect_imports(ctx, &mut out);
    collect_values(ctx, &mut out);
    out.refs.sort_by(|a, b| {
        (&a.symbol, &a.file, a.line, &a.from).cmp(&(&b.symbol, &b.file, b.line, &b.from))
    });
    out
}

fn collect_edges(
    ctx: &Ctx,
    file_pkgs: &HashMap<String, &str>,
    importers: &HashSet<String>,
    syms: &HashMap<&str, &SymbolNode>,
    out: &mut Collected,
) {
    for e in &ctx.model.call_edges {
        let file = norm(&ctx.model.repo_root, &e.file);
        if !e.resolved {
            if file_pkgs.get(&file) == Some(&ctx.package) {
                out.unresolved_call_sites += 1;
            }
            continue;
        }
        if let (Some(sym), Some(from)) = (
            syms.get(e.to.as_str()),
            origin(ctx, file_pkgs, importers, &file),
        ) {
            add_ref(ctx, out, sym, from, (&e.file, e.line));
        }
    }
    for e in ctx.model.type_edges.iter().filter(|e| e.resolved) {
        let file = norm(&ctx.model.repo_root, &e.file);
        if let (Some(sym), Some(from)) = (
            syms.get(e.to.as_str()),
            origin(ctx, file_pkgs, importers, &file),
        ) {
            add_ref(ctx, out, sym, from, (&e.file, e.line));
        }
    }
}

fn collect_imports(ctx: &Ctx, out: &mut Collected) {
    let mut attributed: BTreeSet<(&str, &str)> = BTreeSet::new();
    for r in out.refs.iter().filter(|r| r.from_outside) {
        attributed.insert((r.file.as_str(), r.from.as_str()));
    }
    let attributed: BTreeSet<(String, String)> = attributed
        .into_iter()
        .map(|(f, p)| (f.to_string(), p.to_string()))
        .collect();
    for edge in &ctx.model.import_edges {
        let file = norm(&ctx.model.repo_root, &edge.file);
        if edge.from == ctx.package && edge.to != ctx.package {
            if let Some(part) = ctx.part_of(&file) {
                out.imports_out.insert((part.to_string(), edge.to.clone()));
            }
        } else if edge.to == ctx.package
            && edge.from != ctx.package
            && !attributed.contains(&(file.clone(), edge.from.clone()))
        {
            out.unattributed_importers.insert((file, edge.from.clone()));
        }
    }
}

/// Package-level `var`/`const` names are not `SymbolNode`s, so cross-part uses of them come
/// from the source scan: another part's file mentioning the identifier. Heuristic — a local
/// variable that shadows the name is miscounted.
fn collect_values(ctx: &Ctx, out: &mut Collected) {
    for (decl_file, scan) in ctx.scans {
        let Some(decl_part) = ctx.part_of(decl_file) else {
            continue;
        };
        for value in &scan.values {
            push_value_uses(ctx, decl_part, value, out);
        }
    }
}

fn push_value_uses(ctx: &Ctx, decl_part: &str, value: &ValueDecl, out: &mut Collected) {
    for (use_file, use_scan) in ctx.scans {
        let use_part = ctx.part_of(use_file).filter(|p| *p != decl_part);
        let line = use_scan.idents.get(&value.name);
        let (Some(use_part), Some(&line)) = (use_part, line) else {
            continue;
        };
        out.refs.push(Ref {
            from: use_part.to_string(),
            to: decl_part.to_string(),
            symbol: format!("{}::{}", ctx.package, value.name),
            kind: if value.is_lock { "lock" } else { "value" }.to_string(),
            exported: value.name.starts_with(char::is_uppercase),
            file: use_file.clone(),
            line,
            test: is_test_file(use_file),
            external_test: use_scan.external_test,
            from_outside: false,
        });
    }
}
