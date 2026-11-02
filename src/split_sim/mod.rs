//! Read-only feasibility simulation for splitting one Go package into several: given a
//! partition of its files, reports import cycles the split would create, the identifiers
//! that would need exporting, who must be rewritten, hard blockers, and the before/after
//! reverse-dependency fan-out — all from the in-memory [`ArchModel`], without editing
//! anything.
//!
//! Go-only. Output is deterministic (sorted collections only). Every part is assumed to
//! become a *new* package; none keeps the original import path. Anything the model cannot
//! see is listed in [`CoverageDiagnostics`] instead of being guessed.

mod cycles;
mod refs;
mod scan;
mod sections;

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::arch_model::{ArchModel, looks_generated};
use refs::{Ctx, norm};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PartitionSpec {
    pub name: String,
    pub files: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SplitSimOpts {
    /// Cap on `file:line` sites listed per symbol or edge (counts are never capped).
    pub max_ref_sites: usize,
}

impl Default for SplitSimOpts {
    fn default() -> Self {
        Self { max_ref_sites: 20 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleSeverity {
    Production,
    /// Only closes when a `_test.go` file is compiled; production builds are unaffected.
    TestOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleEdge {
    pub from: String,
    pub to: String,
    pub symbols: Vec<String>,
    pub sites: Vec<String>,
    pub test_only: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CycleFinding {
    pub severity: CycleSeverity,
    pub nodes: Vec<String>,
    pub edges: Vec<CycleEdge>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceEntry {
    pub symbol: String,
    pub kind: String,
    pub defined_in: String,
    pub referenced_from: Vec<String>,
    pub sites: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExportedSurfaceDelta {
    /// Unexported identifiers referenced across a part boundary: must be exported or moved.
    pub must_export: Vec<SurfaceEntry>,
    pub already_exported: Vec<SurfaceEntry>,
    /// Unexported identifiers never referenced across a boundary: can stay unexported.
    pub part_local_unexported: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallersToRewrite {
    pub symbol: String,
    pub defined_in: String,
    pub external_packages: Vec<String>,
    pub referencing_parts: Vec<String>,
    pub production_count: usize,
    pub test_count: usize,
    pub production_sites: Vec<String>,
    pub test_sites: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerSeverity {
    Hard,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blocker {
    pub kind: String,
    pub severity: BlockerSeverity,
    pub file: Option<String>,
    pub line: Option<usize>,
    pub part: Option<String>,
    pub detail: String,
}

/// `direct` and `transitive` count distinct importing packages (never edges), excluding
/// the node itself.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DepCount {
    pub node: String,
    pub direct: usize,
    pub transitive: usize,
    pub transitive_packages: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReverseDepImpact {
    pub before: DepCount,
    pub after: Vec<DepCount>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CoverageDiagnostics {
    pub private_symbols_skipped: usize,
    pub generated_files_skipped: usize,
    pub unresolved_call_sites_in_package: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitSimResult {
    pub package: String,
    /// Set (and every section left empty) when the model cannot support an answer.
    pub abstained: Option<String>,
    /// No hard blocker, no production cycle, and not abstained.
    pub feasible: bool,
    pub parts: BTreeMap<String, Vec<String>>,
    pub cycles: Vec<CycleFinding>,
    pub exported_surface_delta: ExportedSurfaceDelta,
    pub callers_to_rewrite: Vec<CallersToRewrite>,
    pub blockers: Vec<Blocker>,
    pub reverse_dep_impact: ReverseDepImpact,
    pub coverage: CoverageDiagnostics,
}

impl SplitSimResult {
    fn abstain(package: &str, reason: String, model: &ArchModel) -> Self {
        Self {
            package: package.to_string(),
            abstained: Some(reason),
            coverage: base_coverage(model),
            ..Self::default()
        }
    }
}

fn base_coverage(model: &ArchModel) -> CoverageDiagnostics {
    let mut notes = vec![
        "Shared-state detection is syntactic (unexported fields accessed across parts, package-level locks); sharing through parameters, interface fields or unexported types is not detected.".to_string(),
        "Package-level var/const uses are found by identifier match, not name resolution; a shadowing local is miscounted.".to_string(),
        "Every part is assumed to become a new package path; importers are attributed to the parts whose symbols they call, and importers with no attributable use are assumed to need every part.".to_string(),
    ];
    if model.pruning.private_symbols_skipped > 0 {
        notes.insert(0, format!(
            "{} unexported symbols were pruned from the model: unexported cross-boundary references may be undercounted. Build the model with include_private=true.",
            model.pruning.private_symbols_skipped
        ));
    }
    if model.pruning.generated_files_skipped > 0 {
        notes.push(format!(
            "{} generated files are absent from the model; references to or from them are not analysed.",
            model.pruning.generated_files_skipped
        ));
    }
    CoverageDiagnostics {
        private_symbols_skipped: model.pruning.private_symbols_skipped,
        generated_files_skipped: model.pruning.generated_files_skipped,
        unresolved_call_sites_in_package: 0,
        notes,
    }
}

/// Maps `spec` to a model file: exact, `./`-insensitive, repo-root-relative, or (for a bare
/// name) relative to the package directory.
fn resolve_file(pkg_files: &BTreeSet<String>, spec: &str, pkg_dir: &str) -> Option<String> {
    let spec = spec.strip_prefix("./").unwrap_or(spec);
    [spec.to_string(), format!("{pkg_dir}/{spec}")]
        .into_iter()
        .find(|c| pkg_files.contains(c))
}

struct Assignment {
    file_part: BTreeMap<String, String>,
    warnings: Vec<Blocker>,
}

fn assign(
    model: &ArchModel,
    package: &str,
    specs: &[PartitionSpec],
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<Assignment, String> {
    let pkg = model
        .packages
        .get(package)
        .ok_or_else(|| format!("package {package} is not in the model"))?;
    if pkg
        .files
        .iter()
        .any(|f| !f.to_string_lossy().ends_with(".go"))
    {
        return Err("only Go packages are supported".into());
    }
    if specs.len() < 2 {
        return Err("a split needs at least two parts".into());
    }
    let names: BTreeSet<&str> = specs.iter().map(|s| s.name.as_str()).collect();
    if names.len() != specs.len() || names.contains("") {
        return Err("part names must be non-empty and distinct".into());
    }
    if let Some(empty) = specs.iter().find(|s| s.files.is_empty()) {
        return Err(format!("part {} has no files", empty.name));
    }
    let pkg_files: BTreeSet<String> = pkg
        .files
        .iter()
        .map(|f| norm(&model.repo_root, f))
        .collect();
    let pkg_dir = pkg_files
        .iter()
        .next()
        .and_then(|f| f.rsplit_once('/'))
        .map_or(String::new(), |(d, _)| d.to_string());
    let mut file_part = BTreeMap::new();
    let mut warnings = Vec::new();
    for spec in specs {
        for raw in &spec.files {
            let Some(file) = resolve_file(&pkg_files, raw, &pkg_dir) else {
                warnings.push(generated_or_unknown(model, raw, &spec.name, read)?);
                continue;
            };
            if let Some(prev) = file_part.insert(file.clone(), spec.name.clone()) {
                return Err(format!(
                    "{file} is assigned to both {prev} and {}",
                    spec.name
                ));
            }
        }
    }
    let unassigned: Vec<&String> = pkg_files
        .iter()
        .filter(|f| !file_part.contains_key(*f))
        .collect();
    if !unassigned.is_empty() {
        return Err(format!(
            "files not assigned to any part: {}",
            unassigned
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(Assignment {
        file_part,
        warnings,
    })
}

/// A partition file absent from the model is acceptable only if it is a generated file
/// (which the model deliberately skips); anything else is a typo or a foreign file.
fn generated_or_unknown(
    model: &ArchModel,
    raw: &str,
    part: &str,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<Blocker, String> {
    let path = raw.strip_prefix("./").unwrap_or(raw);
    let on_disk = read(path).or_else(|| {
        model
            .packages
            .values()
            .flat_map(|p| &p.files)
            .next()
            .and_then(|f| {
                norm(&model.repo_root, f)
                    .rsplit_once('/')
                    .and_then(|(d, _)| read(&format!("{d}/{path}")))
            })
    });
    match on_disk {
        Some(src) if looks_generated(&src) => Ok(Blocker {
            kind: "generated_file_in_partition".into(),
            severity: BlockerSeverity::Warning,
            file: Some(path.to_string()),
            line: None,
            part: Some(part.to_string()),
            detail: "generated file is absent from the model: references to and from it are not analysed, and it must be regenerated for the new package".into(),
        }),
        Some(_) => Err(format!("{raw} is not in the model and is not a generated file")),
        None => Err(format!("{raw} is not in package files and could not be read")),
    }
}

/// Parses repeated `NAME=file1.go,file2.go` CLI flags into specs.
pub fn parse_partition_flags(flags: &[String]) -> anyhow::Result<Vec<PartitionSpec>> {
    flags
        .iter()
        .map(|flag| {
            let (name, files) = flag
                .split_once('=')
                .ok_or_else(|| anyhow::anyhow!("--partition '{flag}' must look like NAME=file1.go,file2.go"))?;
            let files: Vec<String> = files.split(',').filter(|f| !f.is_empty()).map(str::to_string).collect();
            if name.is_empty() || files.is_empty() || files.iter().any(|f| f.contains('=')) {
                anyhow::bail!("--partition '{flag}' needs a name and file names without ',' or '='; use the MCP split_sim tool for unusual paths");
            }
            Ok(PartitionSpec { name: name.to_string(), files })
        })
        .collect()
}

/// Runs the simulation reading source text from `model.repo_root`.
pub fn run(
    model: &ArchModel,
    package: &str,
    specs: &[PartitionSpec],
    opts: &SplitSimOpts,
) -> SplitSimResult {
    let root = model.repo_root.clone();
    run_with_sources(model, package, specs, opts, &|rel| {
        std::fs::read_to_string(root.join(rel)).ok()
    })
}

pub fn run_with_sources(
    model: &ArchModel,
    package: &str,
    specs: &[PartitionSpec],
    opts: &SplitSimOpts,
    read: &dyn Fn(&str) -> Option<String>,
) -> SplitSimResult {
    let assignment = match assign(model, package, specs, read) {
        Ok(a) => a,
        Err(reason) => return SplitSimResult::abstain(package, reason, model),
    };
    let pkg = &model.packages[package];
    let parse_errors: Vec<String> = model
        .pruning
        .files_with_parse_errors
        .iter()
        .map(|p| norm(&model.repo_root, p))
        .filter(|f| assignment.file_part.contains_key(f))
        .collect();
    if !parse_errors.is_empty() {
        return SplitSimResult::abstain(
            package,
            format!(
                "files failed to parse, so the model is incomplete: {}",
                parse_errors.join(", ")
            ),
            model,
        );
    }
    let mut scans = BTreeMap::new();
    let mut unreadable = Vec::new();
    for f in assignment.file_part.keys() {
        match read(f) {
            Some(src) => {
                scans.insert(f.clone(), scan::scan(&src));
            }
            None => unreadable.push(f.clone()),
        }
    }
    if !unreadable.is_empty() {
        return SplitSimResult::abstain(
            package,
            format!("could not read source files: {}", unreadable.join(", ")),
            model,
        );
    }
    let ctx = Ctx {
        model,
        package,
        pkg,
        file_part: &assignment.file_part,
        scans: &scans,
    };
    let collected = refs::collect(&ctx);

    let part_names: Vec<String> = specs
        .iter()
        .map(|s| s.name.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut blockers = sections::blockers(&ctx, &collected.refs);
    blockers.extend(assignment.warnings);
    let cycles = sections::cycles(&ctx, &collected.refs, opts.max_ref_sites);
    let feasible = !blockers.iter().any(|b| b.severity == BlockerSeverity::Hard)
        && !cycles
            .iter()
            .any(|c| c.severity == CycleSeverity::Production);
    let mut coverage = base_coverage(model);
    coverage.unresolved_call_sites_in_package = collected.unresolved_call_sites;
    SplitSimResult {
        package: package.to_string(),
        abstained: None,
        feasible,
        parts: part_names
            .iter()
            .map(|p| {
                (
                    p.clone(),
                    assignment
                        .file_part
                        .iter()
                        .filter(|(_, part)| *part == p)
                        .map(|(f, _)| f.clone())
                        .collect(),
                )
            })
            .collect(),
        cycles,
        exported_surface_delta: sections::exported_surface(
            &ctx,
            &collected.refs,
            opts.max_ref_sites,
        ),
        callers_to_rewrite: sections::callers(&ctx, &collected.refs, opts.max_ref_sites),
        blockers,
        reverse_dep_impact: sections::reverse_deps(&ctx, &part_names, &collected),
        coverage,
    }
}

/// Human-readable summary of `result`; the JSON form is the machine contract.
pub fn render_text(r: &SplitSimResult) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "split-sim {}", r.package);
    if let Some(reason) = &r.abstained {
        let _ = writeln!(s, "ABSTAINED: {reason}");
        return s;
    }
    let _ = writeln!(
        s,
        "{}",
        if r.feasible {
            "FEASIBLE: no hard blockers, no production cycles"
        } else {
            "NOT FEASIBLE (see blockers/cycles)"
        }
    );
    for c in &r.cycles {
        let label = if c.severity == CycleSeverity::Production {
            "BLOCKER: production import cycle"
        } else {
            "WARNING: test-only import cycle"
        };
        let _ = writeln!(s, "{label} among {}", c.nodes.join(", "));
        for e in &c.edges {
            let _ = writeln!(s, "  {} -> {} via {}", e.from, e.to, e.symbols.join(", "));
        }
    }
    for b in &r.blockers {
        let label = if b.severity == BlockerSeverity::Hard {
            "BLOCKER"
        } else {
            "WARNING"
        };
        let at = b
            .file
            .as_deref()
            .map(|f| {
                format!(
                    " {f}{}",
                    b.line.map(|l| format!(":{l}")).unwrap_or_default()
                )
            })
            .unwrap_or_default();
        let _ = writeln!(s, "{label} {}{at}: {}", b.kind, b.detail);
    }
    let _ = writeln!(
        s,
        "must export ({}):",
        r.exported_surface_delta.must_export.len()
    );
    for e in &r.exported_surface_delta.must_export {
        let _ = writeln!(
            s,
            "  {} (defined in {}, used from {})",
            e.symbol,
            e.defined_in,
            e.referenced_from.join(", ")
        );
    }
    let d = &r.reverse_dep_impact;
    let _ = writeln!(
        s,
        "reverse deps (distinct packages): before {} direct / {} transitive",
        d.before.direct, d.before.transitive
    );
    for a in &d.after {
        let _ = writeln!(
            s,
            "  after {}: {} direct / {} transitive",
            a.node, a.direct, a.transitive
        );
    }
    for n in &r.coverage.notes {
        let _ = writeln!(s, "note: {n}");
    }
    s
}
