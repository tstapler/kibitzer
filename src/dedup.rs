//! Guards against a single tool call firing kibitzer's `PostToolUse` hook more than
//! once — e.g. when the same `kibitzer hook` command is registered both in a user's
//! global `~/.claude/settings.json` and in a project's checked-in `.claude/settings.json`.
//! Claude Code invokes every matching hook registration independently, each with an
//! identical `tool_use_id` in its stdin payload, so that ID is what we dedupe on:
//! whichever invocation claims it first runs checks and reports; any other invocation
//! for the same ID is a guaranteed duplicate and exits silently.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Entries older than this are swept on each call — `tool_use_id`s are never reused,
/// so nothing legitimate is ever this stale; it just bounds the directory's size.
const MAX_AGE: Duration = Duration::from_secs(3600);

fn dedup_dir() -> PathBuf {
    crate::cache::default_cache_path()
        .parent()
        .map(|p| p.join("hook-dedup"))
        .unwrap_or_else(|| std::env::temp_dir().join("kibitzer-hook-dedup"))
}

/// Attempts to atomically claim `tool_use_id` as "being handled by this invocation".
/// Returns `true` the first time a given ID is claimed (the caller should proceed),
/// `false` on every subsequent claim of the same ID (the caller should no-op). Fails
/// open — an IO error (e.g. an unwritable cache dir) returns `true` rather than
/// silently dropping a real invocation.
pub fn claim(tool_use_id: &str) -> bool {
    let dir = dedup_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return true;
    }
    sweep(&dir);

    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(tool_use_id))
    {
        Ok(_) => true,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(_) => true,
    }
}

fn sweep(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(age) = meta
            .modified()
            .and_then(|m| now.duration_since(m).map_err(std::io::Error::other))
        else {
            continue;
        };
        if age > MAX_AGE {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_claim_succeeds_second_fails() {
        let id = format!("test-{:?}", std::thread::current().id());
        let dir = dedup_dir();
        let _ = std::fs::remove_file(dir.join(&id));

        assert!(claim(&id), "first claim of a fresh id should succeed");
        assert!(!claim(&id), "second claim of the same id should fail");

        let _ = std::fs::remove_file(dir.join(&id));
    }

    #[test]
    fn distinct_ids_do_not_interfere() {
        let a = format!("test-a-{:?}", std::thread::current().id());
        let b = format!("test-b-{:?}", std::thread::current().id());
        let dir = dedup_dir();
        let _ = std::fs::remove_file(dir.join(&a));
        let _ = std::fs::remove_file(dir.join(&b));

        assert!(claim(&a));
        assert!(claim(&b));

        let _ = std::fs::remove_file(dir.join(&a));
        let _ = std::fs::remove_file(dir.join(&b));
    }
}
