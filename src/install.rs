use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

const MATCHER: &str = "Edit|Write";

/// Installs kibitzer's `PostToolUse` hook into a Claude Code `settings.json`,
/// merging with whatever is already there. `global` targets `~/.claude/settings.json`
/// (all projects); otherwise `<cwd>/.claude/settings.json` (this project only, matching
/// the convention documented in `docs/checking-invocations.md`). `dry_run` prints the
/// resulting file instead of writing it.
pub fn run_install(global: bool, dry_run: bool) -> Result<ExitCode> {
    let path = if global {
        home_settings_path()?
    } else {
        std::env::current_dir()
            .context("resolving current directory")?
            .join(".claude")
            .join("settings.json")
    };

    let mut settings = read_settings(&path)?;
    let command = hook_command()?;

    if !merge_hook(&mut settings, &command)? {
        println!(
            "[kibitzer] already installed: a PostToolUse hook already runs `kibitzer hook` in {}",
            path.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    let rendered = serde_json::to_string_pretty(&settings)? + "\n";
    if dry_run {
        print!("[kibitzer] would write {}:\n{rendered}", path.display());
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&path, rendered).with_context(|| format!("writing {}", path.display()))?;
    println!(
        "[kibitzer] installed PostToolUse hook (matcher \"{MATCHER}\", command `{command}`) into {}",
        path.display()
    );
    println!(
        "[kibitzer] a Claude Code session already watching that settings file picks this up \
         automatically; otherwise open /hooks once to reload, or restart."
    );
    Ok(ExitCode::SUCCESS)
}

fn home_settings_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .context("resolving home directory (neither HOME nor USERPROFILE is set)")?;
    Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

/// The full command line Claude Code should invoke — this binary's own resolved path
/// plus `hook`, so the install survives whether kibitzer was reached via `cargo install`,
/// Homebrew, or a plain `PATH` entry.
fn hook_command() -> Result<String> {
    let exe = std::env::current_exe().context("resolving kibitzer's own executable path")?;
    let exe = exe.canonicalize().unwrap_or(exe);
    Ok(format!("{} hook", exe.display()))
}

fn read_settings(path: &std::path::Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let raw =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    let settings: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    if !settings.is_object() {
        bail!(
            "{} does not contain a JSON object at its root",
            path.display()
        );
    }
    Ok(settings)
}

/// Merges a `PostToolUse` hook running `command` under matcher `"Edit|Write"` into
/// `settings`, preserving everything else already there (key order included, since
/// `serde_json`'s `preserve_order` feature is enabled). Returns `Ok(false)` — no change
/// made — if a `PostToolUse` hook already invokes `kibitzer hook`, so running install
/// twice doesn't run the hook twice per edit.
fn merge_hook(settings: &mut Value, command: &str) -> Result<bool> {
    let root = settings
        .as_object_mut()
        .context("settings.json root must be a JSON object")?;
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("`hooks` must be a JSON object")?;
    let post_tool_use = hooks
        .entry("PostToolUse")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .context("`hooks.PostToolUse` must be a JSON array")?;

    let already_installed = post_tool_use.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(Value::as_array)
            .is_some_and(|hooks| {
                hooks.iter().any(|h| {
                    h.get("command")
                        .and_then(Value::as_str)
                        .is_some_and(|c| c.contains("kibitzer hook"))
                })
            })
    });
    if already_installed {
        return Ok(false);
    }

    let matching_entry = post_tool_use
        .iter_mut()
        .find(|e| e.get("matcher").and_then(Value::as_str) == Some(MATCHER));
    match matching_entry {
        Some(entry) => {
            let hooks_array = entry
                .as_object_mut()
                .context("a `hooks.PostToolUse` entry must be a JSON object")?
                .entry("hooks")
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .context("a `hooks.PostToolUse` entry's `hooks` must be a JSON array")?;
            hooks_array.push(json!({"type": "command", "command": command}));
        }
        None => {
            post_tool_use.push(json!({
                "matcher": MATCHER,
                "hooks": [{"type": "command", "command": command}]
            }));
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_into_empty_settings() {
        let mut settings = json!({});
        assert!(merge_hook(&mut settings, "/bin/kibitzer hook").unwrap());
        assert_eq!(
            settings,
            json!({
                "hooks": {
                    "PostToolUse": [
                        {"matcher": "Edit|Write", "hooks": [{"type": "command", "command": "/bin/kibitzer hook"}]}
                    ]
                }
            })
        );
    }

    #[test]
    fn preserves_unrelated_existing_settings() {
        let mut settings = json!({
            "model": "sonnet",
            "hooks": {
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "some-other-hook"}]}]
            }
        });
        assert!(merge_hook(&mut settings, "/bin/kibitzer hook").unwrap());
        assert_eq!(settings["model"], "sonnet");
        assert_eq!(
            settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "some-other-hook"
        );
        assert_eq!(
            settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "/bin/kibitzer hook"
        );
    }

    #[test]
    fn appends_to_an_existing_edit_write_matcher_instead_of_duplicating_it() {
        let mut settings = json!({
            "hooks": {
                "PostToolUse": [
                    {"matcher": "Edit|Write", "hooks": [{"type": "command", "command": "prettier --write"}]}
                ]
            }
        });
        assert!(merge_hook(&mut settings, "/bin/kibitzer hook").unwrap());
        let entries = settings["hooks"]["PostToolUse"].as_array().unwrap();
        assert_eq!(entries.len(), 1, "should not add a second matcher entry");
        let hooks = entries[0]["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), 2);
        assert_eq!(hooks[0]["command"], "prettier --write");
        assert_eq!(hooks[1]["command"], "/bin/kibitzer hook");
    }

    #[test]
    fn is_idempotent() {
        let mut settings = json!({});
        assert!(merge_hook(&mut settings, "/bin/kibitzer hook").unwrap());
        assert!(
            !merge_hook(&mut settings, "/bin/kibitzer hook").unwrap(),
            "second install should be a no-op"
        );
        assert_eq!(
            settings["hooks"]["PostToolUse"].as_array().unwrap().len(),
            1
        );
    }

    #[test]
    fn detects_an_already_installed_hook_under_a_different_matcher() {
        // e.g. hand-installed under a broader matcher than "Edit|Write" — still
        // recognized so install doesn't add a redundant second invocation.
        let mut settings = json!({
            "hooks": {
                "PostToolUse": [
                    {"matcher": ".*", "hooks": [{"type": "command", "command": "/usr/local/bin/kibitzer hook"}]}
                ]
            }
        });
        assert!(!merge_hook(&mut settings, "/bin/kibitzer hook").unwrap());
    }

    #[test]
    fn rejects_a_non_object_root() {
        let mut settings = json!([1, 2, 3]);
        assert!(merge_hook(&mut settings, "/bin/kibitzer hook").is_err());
    }
}
