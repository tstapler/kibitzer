//! Throwaway plugin binary proving kibitzer's install -> register -> run path end-to-end
//! (project_plans/checker-plugin-system). Always emits one canned SARIF 2.1.0 finding for
//! the file passed as the first CLI argument, regardless of that file's actual contents.
//!
//! Exits `1` (not `0`) after printing: `run_check`'s SARIF branch takes pass/fail from the
//! process's exit status alone (`src/check.rs`, `output.status.success()`), independent of
//! SARIF content, and every renderer that surfaces check output (`run_batch`, the hook, the
//! MCP `run_checks` tool) skips a `CheckResult` outright once it's marked `passed`. A
//! SARIF-emitting check that exits `0` therefore never has its finding shown anywhere,
//! matching this repo's documented convention for real SARIF-emitting linters
//! (`docs/output-formats.md`'s `eslint --format eslint-formatter-sarif` example, which
//! likewise relies on ESLint's own nonzero exit on a lint violation). Exiting `1` here is
//! what makes this binary's canned finding actually demonstrate the plugin mechanism's
//! stated success metric ("the finding appears in kibitzer's normal check output") rather
//! than only proving install/registration silently succeeded. The manifest that registers
//! this plugin sets `severity: "advisory"`, so a "failing" (exit 1) run still doesn't block
//! a `kibitzer run --trigger batch` invocation overall — see `has_blocking_finding` in
//! `src/run.rs`.

use serde_json::{Value, json};

fn main() -> std::process::ExitCode {
    let file_path = std::env::args().nth(1).unwrap_or_default();
    println!("{}", build_sarif(&file_path));
    std::process::ExitCode::from(1)
}

fn build_sarif(file_path: &str) -> Value {
    json!({
        "version": "2.1.0",
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "kibitzer-stub-plugin",
                    "informationUri": "https://github.com/tstapler/kibitzer",
                    "version": env!("CARGO_PKG_VERSION"),
                    "rules": [{
                        "id": "kibitzer-stub-plugin-finding",
                        "shortDescription": {
                            "text": "Canned finding emitted unconditionally by kibitzer-stub-plugin."
                        }
                    }]
                }
            },
            "results": [{
                "ruleId": "kibitzer-stub-plugin-finding",
                "level": "warning",
                "message": {
                    "text": "This is a canned finding from kibitzer-stub-plugin, proving the plugin install/register/run path works end-to-end."
                },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": file_path
                        },
                        "region": {
                            "startLine": 1
                        }
                    }
                }]
            }]
        }]
    })
}
