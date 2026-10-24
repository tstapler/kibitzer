//! Plain-text rendering of a [`PackageFocusReport`] into an `ArchFinding` message.

use super::{Level, PackageFocusReport};

const LIST_SEP: &str = ", ";
const CLAUSE_SEP: &str = "; ";

pub(super) fn render(report: &PackageFocusReport) -> String {
    let c = &report.coverage;
    let tag = if report.level == Level::Advisory {
        "(advisory)"
    } else {
        "(informational)"
    };
    let head = if report.components.is_empty() {
        format!(
            "[go-package-focus] [informational] {}: {}",
            report.package, report.hypothesis
        )
    } else if report.level == Level::Advisory {
        format!(
            "[go-package-focus] Candidate split in {}: {} {tag}",
            report.package, report.hypothesis
        )
    } else {
        format!(
            "[go-package-focus] [informational] Candidate split in {}: {} (likely wiring/entry-point package)",
            report.package, report.hypothesis
        )
    };
    let mut lines = vec![head];
    for (n, comp) in report.components.iter().enumerate() {
        lines.push(format!(
            "  component {} '{}': types [{}]; {} declarations; state [{}]; unresolved calls [{}]",
            n + 1,
            comp.label,
            comp.types.join(LIST_SEP),
            comp.symbols.len(),
            comp.state_fields.join(LIST_SEP),
            comp.external_calls.join(LIST_SEP)
        ));
    }
    if !report.components.is_empty() {
        lines.push(format!(
            "  cross-boundary edges: {}",
            if report.cross_boundary_edges.is_empty() {
                "none".to_string()
            } else {
                report.cross_boundary_edges.join(CLAUSE_SEP)
            }
        ));
        lines.push(format!("  import direction: {}", report.import_direction));
        lines.push(format!(
            "  required new exports: {}",
            if report.required_new_exports.is_empty() {
                "none".to_string()
            } else {
                report.required_new_exports.join(LIST_SEP)
            }
        ));
        lines.push(format!(
            "  cycle risks: {}",
            if report.cycle_risks.is_empty() {
                "none found".to_string()
            } else {
                report.cycle_risks.join(CLAUSE_SEP)
            }
        ));
    }
    for item in &report.contrary_evidence {
        lines.push(format!("  contrary evidence: {item}"));
    }
    lines.push(format!(
        "  Coverage: {} private symbols not modeled; {} generated files excluded; test imports excluded ({}); {} unresolved edges{}",
        c.private_symbols_skipped,
        c.generated_files_skipped,
        c.test_import_edges_excluded,
        c.unresolved_edges,
        if c.notes.is_empty() { String::new() } else { format!("; {}", c.notes.join(CLAUSE_SEP)) }
    ));
    lines.join("\n")
}
