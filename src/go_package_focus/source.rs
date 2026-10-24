//! Package facts the `ArchModel` drops: shared package-level state, generics, build tags.

use crate::arch_model::PackageNode;

/// Facts read straight from the package's non-test `.go` files; unreadable files only
/// shrink coverage, they never fail the check.
pub(super) struct PackageSource {
    pub(super) shared_state: bool,
    pub(super) shared_state_vars: Vec<String>,
    pub(super) has_generics: bool,
    pub(super) has_build_tags: bool,
    pub(super) unreadable: usize,
}

impl PackageSource {
    pub(super) fn read(pkg: &PackageNode) -> Self {
        let mut out = PackageSource {
            shared_state: false,
            shared_state_vars: vec![],
            has_generics: false,
            has_build_tags: false,
            unreadable: 0,
        };
        for file in pkg
            .files
            .iter()
            .filter(|f| !f.to_string_lossy().ends_with("_test.go"))
        {
            let Ok(text) = std::fs::read_to_string(file) else {
                out.unreadable += 1;
                continue;
            };
            out.scan(&text);
        }
        out.shared_state = !out.shared_state_vars.is_empty();
        out
    }

    fn scan(&mut self, text: &str) {
        let mut in_var_block = false;
        for line in text.lines() {
            if line.starts_with("//go:build") || line.starts_with("// +build") {
                self.has_build_tags = true;
            }
            if (line.starts_with("func ") || line.starts_with("type ")) && line.contains('[') {
                let head = line.split(['(', '{']).next().unwrap_or(line);
                if head.contains("[T ")
                    || head.contains("[K ")
                    || head.contains("any]")
                    || head.contains("comparable]")
                {
                    self.has_generics = true;
                }
            }
            if in_var_block {
                if line.starts_with(')') {
                    in_var_block = false;
                } else {
                    self.note_var(line.trim());
                }
            } else if line.starts_with("var (") {
                in_var_block = true;
            } else if let Some(rest) = line.strip_prefix("var ") {
                self.note_var(rest.trim());
            }
        }
    }

    fn note_var(&mut self, decl: &str) {
        let shares = decl.contains("sync.") || decl.contains("atomic.") || decl.contains("chan ");
        if shares && let Some(name) = decl.split_whitespace().next() {
            self.shared_state_vars.push(name.to_string());
        }
    }
}
