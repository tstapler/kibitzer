//! Package facts the `ArchModel` drops: shared package-level state, generics, build tags.

use crate::arch_model::{ArchModel, PackageNode};

/// Facts read straight from the package's non-test `.go` files; unreadable files only
/// shrink coverage, they never fail the check.
#[derive(Default)]
pub(super) struct PackageSource {
    pub(super) shared_state_vars: Vec<String>,
    pub(super) has_generics: bool,
    pub(super) has_build_tags: bool,
    pub(super) unreadable: usize,
}

impl PackageSource {
    pub(super) fn read(model: &ArchModel, pkg: &PackageNode) -> Self {
        let mut out = Self::default();
        for file in pkg.files.iter().filter(|f| !super::is_test_path(model, f)) {
            let Ok(text) = std::fs::read_to_string(file) else {
                out.unreadable += 1;
                continue;
            };
            out.scan(&text);
        }
        out
    }

    pub(super) fn has_shared_state(&self) -> bool {
        !self.shared_state_vars.is_empty()
    }

    pub(super) fn scan(&mut self, text: &str) {
        let mut in_var_block = false;
        for line in text.lines() {
            self.note_build_tag(line);
            self.note_generics(line);
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

    fn note_build_tag(&mut self, line: &str) {
        if line.starts_with("//go:build") || line.starts_with("// +build") {
            self.has_build_tags = true;
        }
    }

    fn note_generics(&mut self, line: &str) {
        if !(line.starts_with("func ") || line.starts_with("type ")) || !line.contains('[') {
            return;
        }
        let head = line.split(['(', '{']).next().unwrap_or(line);
        if ["[T ", "[K ", "any]", "comparable]"]
            .iter()
            .any(|marker| head.contains(marker))
        {
            self.has_generics = true;
        }
    }

    fn note_var(&mut self, decl: &str) {
        let code = strip_comment_and_strings(decl);
        let shares = code.contains("sync.")
            || code.contains("atomic.")
            || code
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|w| w == "chan");
        let name = code
            .split_whitespace()
            .next()
            .map(|n| n.trim_end_matches(','));
        if shares && let Some(name) = name.filter(|n| !n.is_empty() && *n != "_") {
            self.shared_state_vars.push(name.to_string());
        }
    }
}

/// Drops a trailing `//` comment and the contents of `"..."` literals so neither can
/// match the shared-state keywords.
fn strip_comment_and_strings(line: &str) -> String {
    let mut out = String::new();
    let mut in_str = false;
    let mut prev = '\0';
    for c in line.chars() {
        if !in_str && c == '/' && prev == '/' {
            out.pop();
            break;
        }
        if c == '"' {
            in_str = !in_str;
        } else if !in_str {
            out.push(c);
        }
        prev = c;
    }
    out
}
