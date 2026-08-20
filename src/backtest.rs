//! Backtests a native [`Checker`](crate::checker::Checker) against real historical
//! edits recorded in Claude Code session transcripts (`~/.claude/projects/*/*.jsonl`),
//! without needing the historical file content to still exist on disk.
//!
//! See `docs/backtesting.md` for the methodology, limitations, and the recommended
//! workflow for validating a new or changed checker before it ships.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::checker::{Finding, GrammarCache};
use crate::glob::matches_scope;

/// A file mutation reconstructed from a transcript's Edit/Write/MultiEdit tool_use
/// events — the content kibitzer's live hook would have seen right after that tool
/// ran, reconstructed without touching disk (the real historical content is long
/// gone by the time we backtest).
struct Snapshot {
    file_path: PathBuf,
    /// Content immediately before this mutation, when known. `None` for a `Write`
    /// (always treated as a full, unscoped rewrite, matching the live hook) and for
    /// an `Edit`/`MultiEdit` whose prior state couldn't be reconstructed.
    before: Option<String>,
    after: String,
    seq: usize,
}

/// One finding from replaying a checker against reconstructed history.
#[derive(Debug, Clone)]
pub struct BacktestFinding {
    pub transcript: PathBuf,
    pub file_path: PathBuf,
    pub seq: usize,
    pub checker: String,
    pub line: usize,
    pub message: String,
    /// `true` when the identical message also fired against `before` — i.e. this
    /// finding predates the edit under test, the same "was this already true"
    /// distinction `check.rs`'s git-HEAD baseline downgrade makes for live checks.
    /// `false` means the edit is what introduced it (or it's a `Write`, always
    /// treated as newly-introduced since there's no prior state to compare).
    pub pre_existing: bool,
}

#[derive(Debug, Default, Clone)]
pub struct BacktestStats {
    pub transcripts_scanned: usize,
    pub snapshots_checked: usize,
    pub edits_unreconstructable: usize,
}

pub struct BacktestReport {
    pub findings: Vec<BacktestFinding>,
    pub stats: BacktestStats,
}

/// Finds transcript `.jsonl` files under `dir`, accepting either shape a caller might
/// point this at: a projects root containing one subdirectory per project (each full
/// of `.jsonl` files, matching `~/.claude/projects`), or a single project directory
/// with `.jsonl` files directly inside it. Both are scanned in the same pass — a
/// `.jsonl` at the top level is collected directly, and any subdirectory is descended
/// into one level for its own `.jsonl` files. Returns an empty list (not an error) if
/// `dir` doesn't exist.
pub fn discover_transcripts(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            for entry in
                std::fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))?
            {
                let inner = entry?.path();
                if inner.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    out.push(inner);
                }
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            out.push(path);
        }
    }
    Ok(out)
}

pub fn default_projects_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude/projects"))
}

/// A cheap fingerprint of a file's on-disk state, used to invalidate [`BacktestCache`]
/// entries without re-reading and re-hashing a transcript's full content. Deliberately
/// separate from `cache.rs`'s identically-shaped `Stamp` — that one keys check results
/// by `(file, config, trigger)` for the live daemon; this one keys backtest findings by
/// `(transcript, checker selection)`, an unrelated cache with its own lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Stamp {
    mtime_secs: u64,
    mtime_nanos: u32,
    len: u64,
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    Some(Stamp {
        mtime_secs: dur.as_secs(),
        mtime_nanos: dur.subsec_nanos(),
        len: meta.len(),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedFinding {
    file_path: PathBuf,
    seq: usize,
    checker: String,
    line: usize,
    message: String,
    pre_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TranscriptCacheEntry {
    stamp: Stamp,
    /// Sorted, comma-joined checker names this entry was computed against — a
    /// different checker selection invalidates it even when the transcript itself
    /// is unchanged, since a cached run may simply never have looked at it with
    /// that checker.
    checkers_key: String,
    snapshots_checked: usize,
    edits_unreconstructable: usize,
    findings: Vec<CachedFinding>,
}

/// Persistent, transcript-fingerprint-keyed cache of backtest results. The corpus is
/// thousands of append-only, effectively-immutable `.jsonl` files, so a repeat
/// `check backtest` run only has to reconstruct and check whichever transcripts
/// changed (or haven't been run against this checker selection before) since the
/// cache was last saved — everything else is a lookup.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct BacktestCache {
    entries: HashMap<String, TranscriptCacheEntry>,
}

impl BacktestCache {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string(self)?)?;
        Ok(())
    }
}

pub fn default_cache_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("kibitzer").join("backtest-cache.json");
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home)
        .join(".cache")
        .join("kibitzer")
        .join("backtest-cache.json")
}

/// Runs `checker_names` (every registered checker if empty) against every file
/// mutation reconstructed from `transcripts`. `only_new` drops findings that also
/// fired against the pre-edit content — i.e. keeps only findings the edit itself
/// introduced. `cache` is consulted per transcript and updated with fresh results;
/// callers own persisting it (see `BacktestCache::save`).
pub fn run_backtest(
    transcripts: &[PathBuf],
    checker_names: &[String],
    only_new: bool,
    cache: &mut BacktestCache,
) -> Result<BacktestReport> {
    let checkers: Vec<Box<dyn crate::checker::Checker>> = if checker_names.is_empty() {
        crate::checker::registry()
    } else {
        checker_names
            .iter()
            .map(|name| {
                crate::checker::lookup(name)
                    .ok_or_else(|| anyhow::anyhow!("no checker named '{name}' registered"))
            })
            .collect::<Result<_>>()?
    };
    let mut sorted_names: Vec<&str> = checkers.iter().map(|c| c.name()).collect();
    sorted_names.sort_unstable();
    let checkers_key = sorted_names.join(",");

    let mut findings = Vec::new();
    let mut stats = BacktestStats::default();

    for transcript in transcripts {
        stats.transcripts_scanned += 1;
        let key = transcript.to_string_lossy().into_owned();
        let current_stamp = stamp(transcript);

        if let Some(current_stamp) = current_stamp {
            if let Some(entry) = cache.entries.get(&key) {
                if entry.stamp == current_stamp && entry.checkers_key == checkers_key {
                    stats.snapshots_checked += entry.snapshots_checked;
                    stats.edits_unreconstructable += entry.edits_unreconstructable;
                    findings.extend(
                        entry
                            .findings
                            .iter()
                            .filter(|f| !(only_new && f.pre_existing))
                            .map(|f| BacktestFinding {
                                transcript: transcript.clone(),
                                file_path: f.file_path.clone(),
                                seq: f.seq,
                                checker: f.checker.clone(),
                                line: f.line,
                                message: f.message.clone(),
                                pre_existing: f.pre_existing,
                            }),
                    );
                    continue;
                }
            }
        }

        let (snapshots, unreconstructable) = reconstruct_snapshots(transcript)
            .with_context(|| format!("reconstructing {}", transcript.display()))?;
        stats.edits_unreconstructable += unreconstructable;

        let mut transcript_snapshots_checked = 0usize;
        let mut transcript_findings: Vec<BacktestFinding> = Vec::new();

        for snapshot in &snapshots {
            let path_str = snapshot.file_path.to_string_lossy();
            let matching: Vec<&Box<dyn crate::checker::Checker>> = checkers
                .iter()
                .filter(|c| {
                    let globs: Vec<String> = c.file_globs().iter().map(|g| g.to_string()).collect();
                    matches_scope(&path_str, &globs)
                })
                .collect();
            if matching.is_empty() {
                continue;
            }

            // One cache per source string for this snapshot, shared across every
            // matching checker below — checkers sharing a language (e.g. all the
            // Go checkers) reuse the same parse instead of each re-parsing `after`
            // (and `before`, when present) from scratch.
            let after_cache = GrammarCache::new();
            let before_cache = GrammarCache::new();

            for checker in matching {
                transcript_snapshots_checked += 1;

                let after_findings = crate::checker::run_checker_with_cache(
                    checker.as_ref(),
                    &snapshot.file_path,
                    &snapshot.after,
                    &after_cache,
                )?;
                let before_findings = match &snapshot.before {
                    Some(before) => crate::checker::run_checker_with_cache(
                        checker.as_ref(),
                        &snapshot.file_path,
                        before,
                        &before_cache,
                    )?,
                    None => Vec::new(),
                };

                for finding in after_findings {
                    let pre_existing = before_findings
                        .iter()
                        .any(|f: &Finding| f.message == finding.message);
                    transcript_findings.push(BacktestFinding {
                        transcript: transcript.clone(),
                        file_path: snapshot.file_path.clone(),
                        seq: snapshot.seq,
                        checker: checker.name().to_string(),
                        line: finding.line,
                        message: finding.message,
                        pre_existing,
                    });
                }
            }
        }

        stats.snapshots_checked += transcript_snapshots_checked;

        if let Some(current_stamp) = current_stamp {
            cache.entries.insert(
                key,
                TranscriptCacheEntry {
                    stamp: current_stamp,
                    checkers_key: checkers_key.clone(),
                    snapshots_checked: transcript_snapshots_checked,
                    edits_unreconstructable: unreconstructable,
                    findings: transcript_findings
                        .iter()
                        .map(|f| CachedFinding {
                            file_path: f.file_path.clone(),
                            seq: f.seq,
                            checker: f.checker.clone(),
                            line: f.line,
                            message: f.message.clone(),
                            pre_existing: f.pre_existing,
                        })
                        .collect(),
                },
            );
        }

        findings.extend(
            transcript_findings
                .into_iter()
                .filter(|f| !(only_new && f.pre_existing)),
        );
    }

    Ok(BacktestReport { findings, stats })
}

/// Replays a single transcript's Read/Write/Edit/MultiEdit tool_use events in order,
/// maintaining a per-file "known content" map, and emits one [`Snapshot`] per
/// mutation (Write/Edit/MultiEdit). Read results seed `known content` for later
/// Edits but never produce a snapshot themselves — the live hook never fires on a
/// Read, so backtesting one would test a mutation that never happened.
///
/// This is necessarily best-effort: a partial `Read` (an `offset`/`limit` window)
/// can't seed a reliable full-file baseline and is skipped; an `Edit` whose
/// `old_string` isn't found in the last-known content (no prior `Read`/`Write` seed,
/// or an intervening tool this reconstruction doesn't model) is counted as
/// unreconstructable and skipped rather than guessed at.
fn reconstruct_snapshots(transcript: &Path) -> Result<(Vec<Snapshot>, usize)> {
    let raw = std::fs::read_to_string(transcript)
        .with_context(|| format!("reading {}", transcript.display()))?;
    let lines: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();

    let tool_results = index_tool_results(&lines);

    let mut known: HashMap<PathBuf, String> = HashMap::new();
    let mut snapshots = Vec::new();
    let mut unreconstructable = 0usize;
    let mut seq = 0usize;

    for line in &lines {
        if line.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = line
            .pointer("/message/content")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                continue;
            }
            let name = block.get("name").and_then(Value::as_str).unwrap_or_default();
            let input = block.get("input").cloned().unwrap_or(Value::Null);
            let Some(file_path) = input
                .get("file_path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
            else {
                continue;
            };

            match name {
                "Read" => {
                    // Only a whole-file read is a trustworthy seed; a windowed read
                    // (offset/limit) would silently truncate the reconstructed file.
                    if input.get("offset").is_some() || input.get("limit").is_some() {
                        continue;
                    }
                    let Some(id) = block.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(text) = tool_results.get(id).and_then(|r| parse_cat_n(r)) {
                        known.insert(file_path, text);
                    }
                }
                "Write" => {
                    let Some(new_content) = input.get("content").and_then(Value::as_str) else {
                        continue;
                    };
                    seq += 1;
                    known.insert(file_path.clone(), new_content.to_string());
                    snapshots.push(Snapshot {
                        file_path,
                        before: None,
                        after: new_content.to_string(),
                        seq,
                    });
                }
                "Edit" => {
                    let Some(old_string) = input.get("old_string").and_then(Value::as_str) else {
                        continue;
                    };
                    let Some(new_string) = input.get("new_string").and_then(Value::as_str) else {
                        continue;
                    };
                    match apply_edit(known.get(&file_path), old_string, new_string) {
                        Some((before, after)) => {
                            seq += 1;
                            known.insert(file_path.clone(), after.clone());
                            snapshots.push(Snapshot {
                                file_path,
                                before: Some(before),
                                after,
                                seq,
                            });
                        }
                        None => unreconstructable += 1,
                    }
                }
                "MultiEdit" => {
                    let Some(edits) = input.get("edits").and_then(Value::as_array) else {
                        continue;
                    };
                    let Some(mut current) = known.get(&file_path).cloned() else {
                        unreconstructable += edits.len().max(1);
                        continue;
                    };
                    let before = current.clone();
                    let mut ok = true;
                    for edit in edits {
                        let (Some(old_string), Some(new_string)) = (
                            edit.get("old_string").and_then(Value::as_str),
                            edit.get("new_string").and_then(Value::as_str),
                        ) else {
                            ok = false;
                            break;
                        };
                        match apply_edit(Some(&current), old_string, new_string) {
                            Some((_, after)) => current = after,
                            None => {
                                ok = false;
                                break;
                            }
                        }
                    }
                    if ok {
                        seq += 1;
                        known.insert(file_path.clone(), current.clone());
                        snapshots.push(Snapshot {
                            file_path,
                            before: Some(before),
                            after: current,
                            seq,
                        });
                    } else {
                        unreconstructable += 1;
                    }
                }
                _ => {}
            }
        }
    }

    Ok((snapshots, unreconstructable))
}

/// Maps `tool_use_id` -> raw `tool_result` content (only the shapes we can turn
/// into text: a plain string, or an array with a `type: "text"` block — an image
/// result has neither and is left unindexed).
fn index_tool_results(lines: &[Value]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in lines {
        if line.get("type").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let Some(content) = line
            .pointer("/message/content")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            let text = match block.get("content") {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Array(items)) => items.iter().find_map(|item| {
                    if item.get("type").and_then(Value::as_str) == Some("text") {
                        item.get("text").and_then(Value::as_str).map(str::to_string)
                    } else {
                        None
                    }
                }),
                _ => None,
            };
            if let Some(text) = text {
                out.insert(id.to_string(), text);
            }
        }
    }
    out
}

/// Strips the Read tool's `cat -n`-style `{n}\t{content}` prefix off each line,
/// reconstructing the raw file content. Bails (returns `None`) the first line that
/// doesn't match — trailing non-numbered content (an appended system note) means
/// the rest of the block isn't file content, and a leading mismatch means this
/// wasn't a whole-file text listing (e.g. an error message) at all.
fn parse_cat_n(raw: &str) -> Option<String> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let (_num, rest) = line.split_once('\t')?;
        out.push(rest);
    }
    Some(out.join("\n"))
}

/// Applies a single Edit's `old_string` -> `new_string` replacement onto the last
/// known content for a file. Returns `None` when there's no known prior content to
/// apply onto, or when `old_string` doesn't appear in it (an intervening mutation
/// this reconstruction doesn't model, or a `replace_all` edit — applied here as a
/// single first-occurrence replacement, since most edits aren't `replace_all` and
/// treating one as unreconstructable would undercount far more often than treating
/// a non-`replace_all` edit as one would overcount).
fn apply_edit(known: Option<&String>, old_string: &str, new_string: &str) -> Option<(String, String)> {
    let before = known?.clone();
    if !before.contains(old_string) {
        return None;
    }
    let after = before.replacen(old_string, new_string, 1);
    Some((before, after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A transcript file under the OS temp dir that deletes itself on drop — a
    /// hand-rolled stand-in for `tempfile::NamedTempFile`, since this crate has no
    /// dev-dependency on `tempfile`.
    struct TempTranscript(PathBuf);

    impl Drop for TempTranscript {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    impl TempTranscript {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    fn write_transcript(lines: &[Value]) -> TempTranscript {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "kibitzer-backtest-test-{}-{n}.jsonl",
            std::process::id()
        ));
        let body = lines
            .iter()
            .map(|l| serde_json::to_string(l).unwrap())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&path, body).unwrap();
        TempTranscript(path)
    }

    fn tool_use(id: &str, name: &str, input: Value) -> Value {
        serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "tool_use", "id": id, "name": name, "input": input}]}
        })
    }

    fn tool_result_text(id: &str, text: &str) -> Value {
        serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "tool_result", "tool_use_id": id, "content": text}]}
        })
    }

    #[test]
    fn write_produces_one_unscoped_snapshot() {
        let lines = vec![tool_use(
            "t1",
            "Write",
            serde_json::json!({"file_path": "/repo/foo.go", "content": "package main\n"}),
        )];
        let file = write_transcript(&lines);
        let (snapshots, unreconstructable) = reconstruct_snapshots(file.path()).unwrap();
        assert_eq!(unreconstructable, 0);
        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].before.is_none());
        assert_eq!(snapshots[0].after, "package main\n");
    }

    #[test]
    fn edit_without_a_seed_is_unreconstructable() {
        let lines = vec![tool_use(
            "t1",
            "Edit",
            serde_json::json!({"file_path": "/repo/foo.go", "old_string": "a", "new_string": "b"}),
        )];
        let file = write_transcript(&lines);
        let (snapshots, unreconstructable) = reconstruct_snapshots(file.path()).unwrap();
        assert_eq!(snapshots.len(), 0);
        assert_eq!(unreconstructable, 1);
    }

    #[test]
    fn read_seeds_content_for_a_later_edit() {
        let lines = vec![
            tool_use(
                "r1",
                "Read",
                serde_json::json!({"file_path": "/repo/foo.go"}),
            ),
            tool_result_text("r1", "1\tpackage main\n2\t\n3\tfunc old() {}\n"),
            tool_use(
                "e1",
                "Edit",
                serde_json::json!({
                    "file_path": "/repo/foo.go",
                    "old_string": "func old() {}",
                    "new_string": "func new() {}"
                }),
            ),
        ];
        let file = write_transcript(&lines);
        let (snapshots, unreconstructable) = reconstruct_snapshots(file.path()).unwrap();
        assert_eq!(unreconstructable, 0);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(
            snapshots[0].before.as_deref(),
            Some("package main\n\nfunc old() {}")
        );
        assert_eq!(snapshots[0].after, "package main\n\nfunc new() {}");
    }

    #[test]
    fn windowed_read_is_not_used_as_a_seed() {
        let lines = vec![
            tool_use(
                "r1",
                "Read",
                serde_json::json!({"file_path": "/repo/foo.go", "offset": 10}),
            ),
            tool_result_text("r1", "10\tsome line\n"),
            tool_use(
                "e1",
                "Edit",
                serde_json::json!({"file_path": "/repo/foo.go", "old_string": "some line", "new_string": "x"}),
            ),
        ];
        let file = write_transcript(&lines);
        let (snapshots, unreconstructable) = reconstruct_snapshots(file.path()).unwrap();
        assert_eq!(snapshots.len(), 0);
        assert_eq!(unreconstructable, 1);
    }

    #[test]
    fn multi_edit_applies_all_edits_in_sequence() {
        let lines = vec![
            tool_use(
                "w1",
                "Write",
                serde_json::json!({"file_path": "/repo/foo.go", "content": "a\nb\nc\n"}),
            ),
            tool_use(
                "m1",
                "MultiEdit",
                serde_json::json!({
                    "file_path": "/repo/foo.go",
                    "edits": [
                        {"old_string": "a", "new_string": "x"},
                        {"old_string": "c", "new_string": "z"}
                    ]
                }),
            ),
        ];
        let file = write_transcript(&lines);
        let (snapshots, unreconstructable) = reconstruct_snapshots(file.path()).unwrap();
        assert_eq!(unreconstructable, 0);
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots[1].after, "x\nb\nz\n");
    }

    #[test]
    fn run_backtest_flags_reconstructed_duplication_and_marks_it_new() {
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let content = format!("package main\n\n{block}\n{block}\n{block}");
        let lines = vec![tool_use(
            "w1",
            "Write",
            serde_json::json!({"file_path": "/repo/foo.go", "content": content}),
        )];
        let file = write_transcript(&lines);
        let path = file.path().to_path_buf();
        let report =
            run_backtest(&[path], &["duplicate-code".to_string()], false, &mut BacktestCache::default()).unwrap();
        assert_eq!(report.findings.len(), 1);
        assert!(!report.findings[0].pre_existing);
        assert_eq!(report.stats.transcripts_scanned, 1);
    }

    #[test]
    fn run_backtest_only_new_drops_pre_existing_findings() {
        // Three identical duplicate-triggering blocks, seeded then re-edited elsewhere
        // in the file: the duplication already existed before this edit, so
        // --only-new should drop it.
        let block = "func doWork(id string) error {\n\
                      \tconn := openConnection(id)\n\
                      \tdefer conn.Close()\n\
                      \tresult := conn.Fetch(id)\n\
                      \tlog.Printf(\"fetched %v\", result)\n\
                      \treturn conn.Validate(result)\n";
        let before = format!("package main\n\n{block}\n{block}\n{block}");
        let after = format!("{before}\n// trailing comment\n");
        let lines = vec![
            tool_use(
                "w1",
                "Write",
                serde_json::json!({"file_path": "/repo/foo.go", "content": before}),
            ),
            tool_use(
                "e1",
                "Edit",
                serde_json::json!({
                    "file_path": "/repo/foo.go",
                    "old_string": "package main",
                    "new_string": "package main // edited"
                }),
            ),
        ];
        let file = write_transcript(&lines);
        let path = file.path().to_path_buf();
        let report = run_backtest(&[path], &["duplicate-code".to_string()], true, &mut BacktestCache::default()).unwrap();
        // Only the Write's snapshot has no `before` to compare against, so it's the
        // only one counted as new; the Edit's identical duplication already existed.
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].seq, 1);
        let _ = after; // constructed above only to document what the Edit produces
    }

    #[test]
    fn unrelated_checker_globs_skip_the_file() {
        let lines = vec![tool_use(
            "w1",
            "Write",
            serde_json::json!({"file_path": "/repo/foo.go", "content": "package main\n"}),
        )];
        let file = write_transcript(&lines);
        let path = file.path().to_path_buf();
        let report =
            run_backtest(&[path], &["markdown-link-integrity".to_string()], false, &mut BacktestCache::default()).unwrap();
        assert_eq!(report.findings.len(), 0);
        assert_eq!(report.stats.snapshots_checked, 0);
    }
}
