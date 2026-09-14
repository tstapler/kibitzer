//! `kibitzer schema` — emits the JSON Schema for `.claude/inspect.json`, generated from
//! `config::Config`'s own type definitions via `schemars` so it can't drift from what
//! kibitzer actually parses. Field doc comments on `Config`/`Check`/etc. become the
//! schema's per-property `description`, making this the single source of truth for both.
//! See issue #7 and `schema/README.md`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};

use crate::config::Config;

/// Renders the JSON Schema for [`Config`] as pretty-printed JSON with a trailing newline
/// (matching `install.rs`/`arch_export.rs`'s pretty-print convention).
pub(crate) fn render() -> Result<String> {
    Ok(serde_json::to_string_pretty(&schemars::schema_for!(Config))? + "\n")
}

/// Runs `kibitzer schema`: prints the rendered schema, or writes it to `out`.
pub fn run_schema(out: Option<PathBuf>) -> Result<ExitCode> {
    let rendered = render()?;
    match out {
        Some(path) => {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, &rendered)
                .with_context(|| format!("writing {}", path.display()))?;
            println!("[kibitzer] wrote {}", path.display());
        }
        None => print!("{rendered}"),
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::run_kibitzer;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kibitzer-schema-test-{}-{name}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Catches a checked-in `schema/inspect.schema.json` that's gone stale after a
    /// `Config`/`Check`/etc. field or doc-comment change — regenerate it with
    /// `cargo run -- schema --out schema/inspect.schema.json` (see `schema/README.md`).
    #[test]
    fn checked_in_schema_matches_generated_schema() {
        let generated = render().unwrap();
        let checked_in = std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/inspect.schema.json"),
        )
        .expect("schema/inspect.schema.json should exist — run `cargo run -- schema --out schema/inspect.schema.json`");
        assert_eq!(
            generated, checked_in,
            "schema/inspect.schema.json is stale — regenerate with \
             `cargo run -- schema --out schema/inspect.schema.json`"
        );
    }

    #[test]
    fn run_schema_writes_pretty_printed_schema_with_trailing_newline_and_creates_parent_dir() {
        let dir = tmp_dir("writes-file");
        // Nested, not-yet-existing parent dir: exercises the create_dir_all path a bare
        // `schema/` write (already existing) never touches.
        let out = dir.join("nested/inspect.schema.json");

        let result = run_schema(Some(out.clone())).unwrap();
        assert_eq!(result, ExitCode::SUCCESS);

        let contents = std::fs::read_to_string(&out).unwrap();
        assert!(contents.ends_with('\n'));
        assert!(!contents.ends_with("\n\n"));

        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["title"], "Config");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_schema_cli_prints_to_stdout_when_no_out() {
        let output = run_kibitzer(&["schema"]);
        assert!(output.status.success());

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed: serde_json::Value =
            serde_json::from_str(&stdout).expect("stdout is the Config JSON Schema");
        assert_eq!(parsed["title"], "Config");
    }
}
