//! In-memory overlay over a Go module's `.go` files. Operations edit the overlay; the plan
//! is the diff against disk, so composing operations (split = repeated move) needs no
//! intermediate writes.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use anyhow::{Context, Result, bail};

use super::go_syntax;

pub struct Workspace {
    pub module: String,
    orig: BTreeMap<String, String>,
    edits: BTreeMap<String, Option<String>>,
    /// new path → original path, for files relocated by an operation.
    moves: BTreeMap<String, String>,
    /// Directories holding non-Go sources a package move would leave behind (cgo, asm).
    pub native_dirs: BTreeSet<String>,
    imports_memo: std::cell::RefCell<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Clone)]
pub struct PkgInfo {
    pub import: String,
    /// Non-test package name.
    pub name: String,
    pub files: Vec<String>,
}

const SKIP_DIRS: &[&str] = &["vendor", "testdata", "node_modules", "target"];
const NATIVE_EXTS: &[&str] = &["s", "c", "h", "cc", "cpp", "cxx", "m", "syso", "swig"];

pub fn dir_of(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

pub fn base_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub fn join(dir: &str, base: &str) -> String {
    if dir.is_empty() {
        base.to_string()
    } else {
        format!("{dir}/{base}")
    }
}

pub fn is_test_file(path: &str) -> bool {
    path.ends_with("_test.go")
}

impl Workspace {
    pub fn load(root: &Path) -> Result<Self> {
        let gomod = root.join("go.mod");
        let text = std::fs::read_to_string(&gomod).with_context(|| {
            format!(
                "no go.mod at {} (a Go module root is required)",
                root.display()
            )
        })?;
        let module = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("module "))
            .map(|m| m.trim().trim_matches('"').to_string())
            .filter(|m| !m.is_empty());
        let Some(module) = module else {
            bail!("{} has no `module` directive", gomod.display())
        };
        let mut ws = Workspace {
            module,
            orig: BTreeMap::new(),
            edits: BTreeMap::new(),
            moves: BTreeMap::new(),
            native_dirs: BTreeSet::new(),
            imports_memo: Default::default(),
        };
        ws.walk(root, "")?;
        Ok(ws)
    }

    fn walk(&mut self, abs: &Path, rel: &str) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(abs)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let name = e.file_name().to_string_lossy().to_string();
            let child_rel = join(rel, &name);
            let ty = e.file_type()?;
            if ty.is_dir() {
                if name.starts_with('.')
                    || name.starts_with('_')
                    || SKIP_DIRS.contains(&name.as_str())
                {
                    continue;
                }
                if e.path().join("go.mod").exists() {
                    continue; // nested module: cross-module moves are out of scope
                }
                self.walk(&e.path(), &child_rel)?;
            } else if ty.is_file() {
                if name.ends_with(".go") {
                    if let Ok(s) = std::fs::read_to_string(e.path()) {
                        self.orig.insert(child_rel, s);
                    }
                } else if let Some(ext) = name.rsplit('.').next()
                    && NATIVE_EXTS.contains(&ext)
                {
                    self.native_dirs.insert(rel.to_string());
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, path: &str) -> Option<&str> {
        match self.edits.get(path) {
            Some(Some(s)) => Some(s),
            Some(None) => None,
            None => self.orig.get(path).map(String::as_str),
        }
    }

    pub fn original(&self, path: &str) -> Option<&str> {
        self.orig.get(path).map(String::as_str)
    }

    pub fn set(&mut self, path: &str, content: String) {
        self.imports_memo.borrow_mut().clear();
        self.edits.insert(path.to_string(), Some(content));
    }

    pub fn remove(&mut self, path: &str) {
        self.imports_memo.borrow_mut().clear();
        self.edits.insert(path.to_string(), None);
    }

    /// Relocate `old` to `new` with `content`, remembering the lineage for the plan.
    pub fn move_file(&mut self, old: &str, new: &str, content: String) {
        let origin = self.moves.remove(old).unwrap_or_else(|| old.to_string());
        self.remove(old);
        self.set(new, content);
        self.moves.insert(new.to_string(), origin);
    }

    pub fn moves(&self) -> &BTreeMap<String, String> {
        &self.moves
    }

    pub fn paths(&self) -> Vec<String> {
        let mut v: BTreeSet<String> = self.orig.keys().cloned().collect();
        for (k, val) in &self.edits {
            if val.is_some() {
                v.insert(k.clone());
            } else {
                v.remove(k);
            }
        }
        v.into_iter().collect()
    }

    pub fn changed_paths(&self) -> Vec<String> {
        self.edits.keys().cloned().collect()
    }

    pub fn import_of_dir(&self, dir: &str) -> String {
        if dir.is_empty() {
            self.module.clone()
        } else {
            format!("{}/{}", self.module, dir)
        }
    }

    pub fn dir_of_import(&self, import: &str) -> Option<String> {
        if import == self.module {
            return Some(String::new());
        }
        import
            .strip_prefix(&format!("{}/", self.module))
            .map(str::to_string)
    }

    pub fn files_in_dir(&self, dir: &str) -> Vec<String> {
        self.paths()
            .into_iter()
            .filter(|p| dir_of(p) == dir)
            .collect()
    }

    pub fn pkg_info(&self, dir: &str) -> Option<PkgInfo> {
        let files = self.files_in_dir(dir);
        let mut name = None;
        for f in &files {
            let pn = go_syntax::package_name(self.get(f)?)?;
            if !is_test_file(f) {
                name = Some(pn);
                break;
            }
            name.get_or_insert_with(|| pn.trim_end_matches("_test").to_string());
        }
        Some(PkgInfo {
            import: self.import_of_dir(dir),
            name: name?,
            files,
        })
    }

    pub fn files_containing(&self, needle: &str) -> Vec<String> {
        self.paths()
            .into_iter()
            .filter(|p| self.get(p).is_some_and(|s| s.contains(needle)))
            .collect()
    }

    /// Module-local import paths a directory's non-external-test files import.
    pub fn local_imports_of_dir(&self, dir: &str) -> Vec<String> {
        if let Some(v) = self.imports_memo.borrow().get(dir) {
            return v.clone();
        }
        let mut out = BTreeSet::new();
        for f in self.files_in_dir(dir) {
            let Some(src) = self.get(&f) else { continue };
            if is_test_file(&f)
                && go_syntax::package_name(src).is_some_and(|n| n.ends_with("_test"))
            {
                continue;
            }
            for i in go_syntax::imports(src) {
                if self.dir_of_import(&i.path).is_some() {
                    out.insert(i.path);
                }
            }
        }
        let v: Vec<String> = out.into_iter().collect();
        self.imports_memo
            .borrow_mut()
            .insert(dir.to_string(), v.clone());
        v
    }

    /// An import cycle through `start`, as the path of import paths, if one exists.
    pub fn cycle_through(&self, start: &str) -> Option<Vec<String>> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![(start.to_string(), vec![start.to_string()])];
        while let Some((cur, path)) = stack.pop() {
            let Some(dir) = self.dir_of_import(&cur) else {
                continue;
            };
            for next in self.local_imports_of_dir(&dir) {
                if next == start {
                    let mut p = path.clone();
                    p.push(next);
                    return Some(p);
                }
                if seen.insert(next.clone()) {
                    let mut p = path.clone();
                    p.push(next.clone());
                    stack.push((next, p));
                }
            }
        }
        None
    }
}

/// Package name a directory path implies (`.../foo-bar/v2` → `foo_bar`).
pub fn derive_pkg_name(import: &str) -> String {
    let mut segs = import.rsplit('/');
    let mut last = segs.next().unwrap_or("pkg");
    let is_version =
        |s: &str| s.len() > 1 && s.starts_with('v') && s[1..].chars().all(|c| c.is_ascii_digit());
    if is_version(last)
        && let Some(prev) = segs.next()
    {
        last = prev;
    }
    let mut name: String = last
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if name.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        name.insert(0, 'p');
    }
    name
}

/// Go's `internal` rule: the importer must live under the parent of the last
/// `internal` element of the target path.
pub fn internal_ok(importer_import: &str, target_import: &str) -> bool {
    let idx = if let Some(i) = target_import.rfind("/internal/") {
        i
    } else if target_import.ends_with("/internal") {
        target_import.len() - "/internal".len()
    } else {
        return true;
    };
    let parent = &target_import[..idx];
    importer_import == parent || importer_import.starts_with(&format!("{parent}/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_rule_table() {
        for (importer, target, ok) in [
            ("m/app", "m/lib", true),
            ("m/app", "m/lib/internal/x", false),
            ("m/lib/sub", "m/lib/internal/x", true),
            ("m/lib", "m/lib/internal", true),
            ("m/other/internal/y", "m/lib/internal/x", false),
        ] {
            assert_eq!(internal_ok(importer, target), ok, "{importer} -> {target}");
        }
    }

    #[test]
    fn derives_package_names() {
        assert_eq!(derive_pkg_name("m/foo-bar"), "foo_bar");
        assert_eq!(derive_pkg_name("m/foo/v2"), "foo");
        assert_eq!(derive_pkg_name("m/lib"), "lib");
    }
}
