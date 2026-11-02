//! `sync-policy`: rewrite repository-declared paths (`.kibitzer/inspect.json` scopes,
//! component paths, dependency-guard configs) after a move. Always a separate plan from
//! the move itself so a reviewer can accept one without the other.

use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::plan::sha256_hex;
use super::types::{FileTransform, MovePlan, OperationKind, PathMapping, PlanStep, PlanStepKind};

/// Keys whose string values (or array elements) are repo paths/globs.
const PATH_KEYS: &[&str] = &[
    "scope", "paths", "path", "include", "exclude", "files", "packages", "dirs", "roots", "root",
    "from", "to", "allow", "deny",
];

/// Plain-text guard configs that name import paths.
const GUARD_FILES: &[&str] = &[
    ".golangci.yml",
    ".golangci.yaml",
    ".go-arch-lint.yml",
    ".depguard.yml",
];

const GLOB_PREFIXES: &[&str] = &["./", "**/", ""];

fn remap(s: &str, maps: &[&PathMapping]) -> Option<String> {
    for m in maps {
        for pre in GLOB_PREFIXES {
            let Some(rest) = s.strip_prefix(pre) else {
                continue;
            };
            if let Some(tail) = rest.strip_prefix(m.from.as_str())
                && (tail.is_empty() || tail.starts_with(['/', '*']))
            {
                return Some(format!("{pre}{}{tail}", m.to));
            }
        }
    }
    None
}

fn rewrite(
    v: &mut Value,
    key_is_path: bool,
    maps: &[&PathMapping],
    changed: &mut Vec<(String, String)>,
) {
    match v {
        Value::String(s) if key_is_path => {
            if let Some(new) = remap(s, maps) {
                changed.push((s.clone(), new.clone()));
                *s = new;
            }
        }
        Value::Array(a) => a
            .iter_mut()
            .for_each(|x| rewrite(x, key_is_path, maps, changed)),
        Value::Object(o) => {
            for (k, x) in o.iter_mut() {
                rewrite(x, PATH_KEYS.contains(&k.as_str()), maps, changed);
            }
        }
        _ => {}
    }
}

fn replace_import_paths(text: &str, imports: &[&PathMapping]) -> String {
    let mut out = text.to_string();
    for m in imports {
        let mut res = String::new();
        let mut rest = out.as_str();
        while let Some(i) = rest.find(&m.from) {
            let after = rest[i + m.from.len()..].chars().next();
            let before_ok = rest[..i].chars().next_back().is_none_or(|c| {
                !(c.is_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_')
            });
            let after_ok = after.is_none_or(|c| !(c.is_alphanumeric() || c == '_' || c == '-'));
            res.push_str(&rest[..i]);
            res.push_str(if before_ok && after_ok {
                &m.to
            } else {
                &m.from
            });
            rest = &rest[i + m.from.len()..];
        }
        res.push_str(rest);
        out = res;
    }
    out
}

pub fn sync_policy(root: &Path, source: &MovePlan) -> Result<MovePlan> {
    let mut maps: Vec<&PathMapping> = source
        .path_map
        .iter()
        .filter(|m| !m.from.is_empty())
        .collect();
    maps.sort_by_key(|m| std::cmp::Reverse(m.from.len()));
    let module_prefix = format!("{}/", source.module);
    let import_maps: Vec<&PathMapping> = maps
        .iter()
        .copied()
        .filter(|m| m.from == source.module || m.from.starts_with(&module_prefix))
        .collect();
    let mut steps = vec![];
    let mut warnings = vec![];

    let inspect = ".kibitzer/inspect.json";
    if let Ok(raw) = std::fs::read_to_string(root.join(inspect)) {
        let mut v: Value =
            serde_json::from_str(&raw).with_context(|| format!("parsing {inspect}"))?;
        let canonical = format!("{}\n", serde_json::to_string_pretty(&v)?);
        let mut changed = vec![];
        rewrite(&mut v, false, &maps, &mut changed);
        if !changed.is_empty() {
            if canonical != raw {
                warnings.push(format!(
                    "{inspect} is not in canonical 2-space JSON; the rewrite reformats it"
                ));
            }
            let content = format!("{}\n", serde_json::to_string_pretty(&v)?);
            let reason = changed
                .iter()
                .map(|(a, b)| format!("`{a}` -> `{b}`"))
                .collect::<Vec<_>>()
                .join(", ");
            steps.push(PlanStep {
                kind: PlanStepKind::PolicyUpdate,
                transform: FileTransform {
                    old_path: Some(inspect.into()),
                    old_sha256: Some(sha256_hex(raw.as_bytes())),
                    new_path: Some(inspect.into()),
                    new_content: Some(content),
                },
                reason: format!("path changed: {reason}"),
            });
        }
    }
    for name in GUARD_FILES {
        let Ok(raw) = std::fs::read_to_string(root.join(name)) else {
            continue;
        };
        let new = replace_import_paths(&raw, &import_maps);
        if new != raw {
            steps.push(PlanStep {
                kind: PlanStepKind::PolicyUpdate,
                transform: FileTransform {
                    old_path: Some((*name).into()),
                    old_sha256: Some(sha256_hex(raw.as_bytes())),
                    new_path: Some((*name).into()),
                    new_content: Some(new),
                },
                reason: "import path changed in dependency-guard config".into(),
            });
        }
    }
    Ok(MovePlan {
        operation: OperationKind::SyncPolicy,
        repo_root: source.repo_root.clone(),
        module: source.module.clone(),
        from: source.from.clone(),
        to: source.to.clone(),
        steps,
        // A policy plan derived from a blocked move plan is itself blocked.
        blockers: source.blockers.clone(),
        warnings,
        surface_changes: vec![],
        path_map: source.path_map.clone(),
        affected_packages: vec![],
        simulation_result: source.simulation_result.clone(),
    })
}
