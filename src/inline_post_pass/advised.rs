//! Per-file memory of which blocking suppressions the hook has already seen, so a later edit
//! reports only a suppression that is new (a finding that appeared or slid under a directive)
//! instead of re-listing every old one. Best-effort: an unreadable or unwritable store reads as
//! "never seen this file", which falls back to the conservative first-sight rules.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// Most fingerprints kept per file; past this the store resets to the current set.
const MAX_FINGERPRINTS: usize = 512;

/// Longest slice of the silenced line's text folded into a fingerprint.
const FINGERPRINT_TEXT_CHARS: usize = 120;

pub(super) struct Advised {
    path: PathBuf,
    /// `None` until a prior run stored a set for this file.
    seen: Option<BTreeSet<String>>,
}

impl Advised {
    pub(super) fn load(dir: &Path, file_path: &Path) -> Self {
        let canonical = file_path
            .canonicalize()
            .unwrap_or_else(|_| file_path.to_path_buf());
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        canonical.hash(&mut hasher);
        let path = dir.join(format!("{:016x}.json", hasher.finish()));
        let seen = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<BTreeSet<String>>(&raw).ok());
        Self { path, seen }
    }

    /// Whether an earlier run stored a baseline for this file.
    pub(super) fn is_known(&self) -> bool {
        self.seen.is_some()
    }

    pub(super) fn has_seen(&self, fingerprint: &str) -> bool {
        self.seen
            .as_ref()
            .is_some_and(|seen| seen.contains(fingerprint))
    }

    /// Stores `current` (plus what was already seen) as the new baseline.
    pub(super) fn save(&self, current: BTreeSet<String>) {
        let mut all = self.seen.clone().unwrap_or_default();
        all.extend(current.iter().cloned());
        if all.len() > MAX_FINGERPRINTS {
            all = current;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(raw) = serde_json::to_string(&all) {
            let _ = std::fs::write(&self.path, raw);
        }
    }
}

/// Identity of one suppression that survives row shifts: the rule, the reason, and the text of
/// the silenced line (omitted for file-scope rules, whose finding has no line of its own).
pub(super) fn fingerprint(rule: &str, reason: &str, line_text: Option<&str>) -> String {
    let text: String = line_text
        .map(|t| t.trim().chars().take(FINGERPRINT_TEXT_CHARS).collect())
        .unwrap_or_default();
    format!("{rule}\u{1f}{reason}\u{1f}{text}")
}
