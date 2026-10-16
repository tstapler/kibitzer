//! Hidden experiment-input export (Task 2.3.1h): one streamed JSON Lines file that carries the matrix to the Phase
//! 3 harness, so the harness never re-implements an adapter, `joint_safe_subset` or the test-id tie-break.
//!
//! Every line has a `rec` discriminator first. This file exists only when `--emit-experiment-input` is passed;
//! no user-facing output names it (Principle 7: `JointSafe` never reaches a report).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::test_effectiveness::analysis::joint_safe_subset;
use crate::test_effectiveness::confidence::Signal;
use crate::test_effectiveness::matrix::{
    CoverageUnitKind, Dimension, DimensionMeta, Provenance, TestId,
};

use super::envelope::{SCHEMA_VERSION, Scope, snake};
use super::{Loaded, Report};

const KIND_EXPERIMENT_INPUT: &str = "experiment_input";

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hash_file(h: &mut Sha256, path: &Path) -> io::Result<()> {
    let mut f = File::open(path)?;
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        h.update(&buf[..n]);
    }
}

/// SHA-256 of a file, or of a directory's files (sorted by name, each preceded by its name).
pub fn digest_path(path: &Path) -> io::Result<String> {
    let mut h = Sha256::new();
    if path.is_dir() {
        let mut names: Vec<PathBuf> = std::fs::read_dir(path)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        names.sort();
        for p in names {
            h.update(p.file_name().unwrap_or_default().as_encoded_bytes());
            h.update([0u8]);
            hash_file(&mut h, &p)?;
        }
    } else {
        hash_file(&mut h, path)?;
    }
    Ok(hex(h.finalize().as_ref()))
}

fn provenance_json(
    dimension: &str,
    p: &Provenance,
    attribution: Option<&str>,
    kill_completeness: Option<&str>,
    unit_kind: &str,
) -> Value {
    json!({
        "dimension": dimension,
        "tool": p.tool,
        "tool_version": p.version,
        "commit": p.commit,
        "attribution": attribution.map(snake),
        "kill_completeness": kill_completeness.map(snake),
        "unit_kind": unit_kind,
    })
}

fn provenance_lines(loaded: &Loaded) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(d) = &loaded.matrix.coverage {
        let kind = match d.unit_kind {
            Some(CoverageUnitKind::MutantSite) => "mutant_site",
            _ => "line",
        };
        out.push(provenance_json(
            "coverage",
            &d.provenance,
            Some(d.meta.label()),
            None,
            kind,
        ));
    }
    if let Some(d) = &loaded.matrix.kills {
        out.push(provenance_json(
            "kills",
            &d.provenance,
            None,
            Some(d.meta.label()),
            "mutant",
        ));
    }
    out
}

fn put(out: &mut impl Write, v: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *out, v).map_err(io::Error::other)?;
    out.write_all(b"\n")
}

/// One line per unit, with test ordinals translated into the universe order of the `test` lines.
fn write_units<U: Ord, M: DimensionMeta>(
    out: &mut impl Write,
    (dimension, unit_kind): (&str, &str),
    d: &Dimension<U, M>,
    name: impl Fn(&U) -> String,
    ordinal_in_universe: &HashMap<&TestId, u32>,
) -> io::Result<usize> {
    for (unit, posting) in &d.index {
        let tests: Vec<u32> = posting
            .tests
            .iter()
            .map(|&t| ordinal_in_universe[d.test(t)])
            .collect();
        put(
            out,
            &json!({
                "rec": "unit",
                "dimension": dimension,
                "unit": name(unit),
                "unit_kind": unit_kind,
                "tests": tests,
            }),
        )?;
    }
    Ok(d.index.len())
}

fn coverage_unit_string(u: &crate::test_effectiveness::matrix::CoverageUnit) -> String {
    format!("{}:{}", u.file, u.line)
}

/// Writes the export. The caller must not call this when the run failed with a class E error.
pub fn write(
    path: &Path,
    report: &Report,
    loaded: &Loaded,
    inputs: &[(&'static str, PathBuf)],
) -> io::Result<()> {
    let mut out = BufWriter::new(File::create(path)?);
    let digests: Vec<Value> = inputs
        .iter()
        .map(|(flag, p)| Ok(json!({"flag": flag, "sha256": digest_path(p)?})))
        .collect::<io::Result<_>>()?;
    put(
        &mut out,
        &json!({
            "rec": "header",
            "schema_version": SCHEMA_VERSION,
            "kind": KIND_EXPERIMENT_INPUT,
            "kibitzer_version": env!("CARGO_PKG_VERSION"),
            "input_digests": digests,
            "provenance": provenance_lines(loaded),
        }),
    )?;

    let ordinal: HashMap<&TestId, u32> = report
        .universe
        .iter()
        .enumerate()
        .map(|(i, t)| (&t.test, i as u32))
        .collect();
    let id_of: HashMap<&TestId, &str> = report
        .universe
        .iter()
        .map(|t| (&t.test, t.id.as_str()))
        .collect();
    for t in &report.universe {
        put(
            &mut out,
            &json!({
                "rec": "test",
                "id": t.id,
                "package": t.package,
                "kind": format!("{:?}", t.kind).to_lowercase(),
            }),
        )?;
    }

    let mut units = 0;
    if let Some(d) = &loaded.matrix.coverage {
        let kind = match d.unit_kind {
            Some(CoverageUnitKind::MutantSite) => "mutant_site",
            _ => "line",
        };
        units += write_units(
            &mut out,
            ("coverage", kind),
            d,
            coverage_unit_string,
            &ordinal,
        )?;
    }
    if let Some(d) = &loaded.matrix.kills {
        units += write_units(&mut out, ("kills", "mutant"), d, |m| m.0.clone(), &ordinal)?;
    }

    for n in &report.not_assessed {
        put(
            &mut out,
            &json!({"rec": "excluded", "id": n.id, "reason": Scope::reason_key(n.reason)}),
        )?;
    }

    let mut flagged_rules = 0;
    for (signal, tests) in &report.flagged {
        flagged_rules += 1;
        put(&mut out, &flagged_line(*signal, tests, loaded, &id_of))?;
    }
    put(
        &mut out,
        &json!({
            "rec": "end",
            "counts": {
                "tests": report.universe.len(),
                "units": units,
                "excluded": report.not_assessed.len(),
                "flagged_rules": flagged_rules,
            },
        }),
    )?;
    out.flush()
}

fn flagged_line(
    signal: Signal,
    tests: &[TestId],
    loaded: &Loaded,
    id_of: &HashMap<&TestId, &str>,
) -> Value {
    let safe = joint_safe_subset(&loaded.matrix, tests).safe;
    let ids = |ts: &[TestId]| -> Vec<String> {
        let mut v: Vec<String> = ts.iter().map(|t| id_of[t].to_string()).collect();
        v.sort();
        v
    };
    json!({
        "rec": "flagged",
        "rule": signal.rule_id(),
        "individually_flagged": ids(tests),
        "joint_safe": ids(&safe),
    })
}
