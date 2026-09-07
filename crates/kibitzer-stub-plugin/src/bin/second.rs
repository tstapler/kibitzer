//! Second throwaway plugin binary (Story 5.1.2) proving `registered_plugin_checks()`
//! synthesizes one independent `Check` per installed plugin rather than colliding on name
//! — a near-copy of `main.rs` with only the rule id/tool name changed. Always emits one
//! canned SARIF 2.1.0 finding for the file passed as the first CLI argument.
//!
//! Exits `1` after printing, for the same reason `main.rs` does — see its doc comment.

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
                    "name": "kibitzer-stub-plugin-second",
                    "informationUri": "https://github.com/tstapler/kibitzer",
                    "version": env!("CARGO_PKG_VERSION"),
                    "rules": [{
                        "id": "kibitzer-stub-plugin-second-finding",
                        "shortDescription": {
                            "text": "Canned finding emitted unconditionally by kibitzer-stub-plugin-second."
                        }
                    }]
                }
            },
            "results": [{
                "ruleId": "kibitzer-stub-plugin-second-finding",
                "level": "warning",
                "message": {
                    "text": "This is a canned finding from kibitzer-stub-plugin-second, proving a second independently-registered plugin surfaces distinctly."
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
