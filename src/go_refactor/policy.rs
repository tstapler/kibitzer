//! `sync-policy`: rewrite repository-declared paths (`.kibitzer/inspect.json` scopes,
//! component paths, dependency-guard configs) after a move. Always a separate plan from
//! the move itself so a reviewer can accept one without the other.

use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::types::{FileTransform, MovePlan, OperationKind, PathMapping, PlanStep, PlanStepKind};
use crate::refactor_core::sha256_hex;

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

/// Import and directory paths only: these strings are pasted into YAML and JSON policy files, so
/// a newline or quote in one would inject structure.
fn plain_path(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/-~+@*".contains(c))
}

/// Reads a policy file; `None` only when it does not exist. A symlink is refused so a plan can
/// never embed (or later write through to) a file outside the repo.
fn read_policy_file(root: &Path, rel: &str) -> Result<Option<String>> {
    let path = root.join(rel);
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_symlink() => bail!("{rel} is a symlink; refusing to read it"),
        Ok(_) => std::fs::read_to_string(&path)
            .map(Some)
            .with_context(|| format!("reading {rel}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {rel}")),
    }
}

pub fn sync_policy(root: &Path, source: &MovePlan) -> Result<MovePlan> {
    if let Some(m) = source.path_map.iter().find(|m| {
        !(plain_path(&m.from) && plain_path(&m.to) || m.from.is_empty() && plain_path(&m.to))
    }) {
        bail!(
            "path mapping `{}` -> `{}` has characters that are not valid in a path; refusing to write it into policy files",
            m.from,
            m.to
        );
    }
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
    if let Some(raw) = read_policy_file(root, inspect)? {
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
        let Some(raw) = read_policy_file(root, name)? else {
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
        plan_id: String::new(),
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
    }
    .sealed())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(from: &str, to: &str) -> PathMapping {
        PathMapping {
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn remap_respects_path_boundaries_and_glob_prefixes() {
        let m = map("lib", "core");
        let maps = [&m];
        assert_eq!(remap("lib", &maps).as_deref(), Some("core"));
        assert_eq!(remap("lib/**", &maps).as_deref(), Some("core/**"));
        assert_eq!(remap("./lib/x", &maps).as_deref(), Some("./core/x"));
        assert_eq!(remap("**/lib/**", &maps).as_deref(), Some("**/core/**"));
        assert_eq!(
            remap("libx/a", &maps),
            None,
            "sibling prefix must not match"
        );
        assert_eq!(remap("other/lib", &maps), None);
    }

    #[test]
    fn longest_mapping_wins() {
        let (a, b) = (map("lib/sub", "x"), map("lib", "core"));
        assert_eq!(remap("lib/sub/f.go", &[&a, &b]).as_deref(), Some("x/f.go"));
    }

    #[test]
    fn import_replacement_honors_word_boundaries() {
        let m = map("example.com/m/lib", "example.com/m/core");
        let out = replace_import_paths(
            "a: example.com/m/lib\nb: example.com/m/lib-extra\nc: \"example.com/m/lib\"\n",
            &[&m],
        );
        assert_eq!(
            out,
            "a: example.com/m/core\nb: example.com/m/lib-extra\nc: \"example.com/m/core\"\n"
        );
    }
}
