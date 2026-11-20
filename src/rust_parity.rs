//! Rust check-parity table: which default checks cover Rust, which do not and why.
//!
//! Source of truth for `docs/rust-parity.md`. Shipped rows are derived from
//! `config::default_checks()` (every check whose scope has a `*.rs` glob), so a new Rust check
//! cannot be missing from the doc. Gaps are listed by hand in [`GAPS`]; a test fails when a
//! default check for another programming language is neither Rust-scoped nor listed there.
//! Regenerate the doc with `KIBITZER_UPDATE_DOCS=1 cargo test rust_parity`.

use std::fmt::Write as _;

use crate::config::{Check, default_checks};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A default check with a `*.rs` glob covers it.
    Shipped,
    /// Intentional gap: no sound Rust analogue; the rationale says why.
    NotApplicable,
    /// A sound analogue exists but is not built; tracked on the umbrella issue.
    Planned,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Shipped => "Shipped",
            Status::NotApplicable => "N/A",
            Status::Planned => "Planned",
        }
    }
}

pub struct Gap {
    pub check: &'static str,
    pub status: Status,
    pub rationale: &'static str,
}

const TRACKER: &str = "https://github.com/tstapler/kibitzer/issues/184";
const NO_INHERITANCE: &str = "Rust has no implementation inheritance, so the subclass/superclass hierarchies this check rewrites do not exist; trait supertraits compose behaviour and are not a class hierarchy (docs/hierarchy-cleanup-checks.md).";

pub const GAPS: &[Gap] = &[
    Gap {
        check: "remove-subclass",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "extract-superclass",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "collapse-hierarchy",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "push-down-method",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "push-down-field",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "pull-up-method",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "pull-up-constructor-body",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "pull-up-field",
        status: Status::NotApplicable,
        rationale: NO_INHERITANCE,
    },
    Gap {
        check: "go-blank-imports",
        status: Status::NotApplicable,
        rationale: "Go `_` imports run package `init` side effects; Rust has no import-time side effects (`use .. as _` only brings a trait into scope).",
    },
    Gap {
        check: "go-type-switch-density",
        status: Status::NotApplicable,
        rationale: "Go `switch x.(type)` on an open interface; a Rust `match` over an enum is compiler-checked for exhaustiveness, which is the fix this check suggests.",
    },
    Gap {
        check: "go-type-switch-clustering",
        status: Status::NotApplicable,
        rationale: "Same as `go-type-switch-density`: enum `match` is the idiomatic, exhaustive form.",
    },
    Gap {
        check: "introduce-special-case",
        status: Status::NotApplicable,
        rationale: "Targets repeated Go `nil` checks; Rust encodes absence in `Option`, which is already the special-case type.",
    },
    Gap {
        check: "go-encapsulate-collection",
        status: Status::NotApplicable,
        rationale: "Targets Go getters that hand out an internal slice or map; a Rust `&[T]`/`&Vec<T>` return is read-only and the borrow checker prevents the aliasing the check guards against.",
    },
    Gap {
        check: "go-encapsulate-variable",
        status: Status::NotApplicable,
        rationale: "Targets exported mutable Go package variables; Rust global mutable state needs `static mut` (unsafe) or interior mutability, both visible at the declaration.",
    },
    Gap {
        check: "java-swallowed-interrupt",
        status: Status::NotApplicable,
        rationale: "Targets swallowed `InterruptedException`; Rust threads have no interruption exception.",
    },
    Gap {
        check: "go-bulk-fetch-linear-scan",
        status: Status::Planned,
        rationale: "The pattern (fetch everything, then scan for one item) is language-independent; no Rust port yet.",
    },
    Gap {
        check: "go-table-driven-test-candidate",
        status: Status::Planned,
        rationale: "Near-identical `#[test]` functions are a `rstest`/loop candidate; no Rust port yet.",
    },
    Gap {
        check: "java-lost-exception-cause",
        status: Status::Planned,
        rationale: "Analogue is `map_err(|_| NewError)` dropping the source error; no Rust port yet.",
    },
    Gap {
        check: "parameter-object",
        status: Status::Planned,
        rationale: "Parameter data exists for Go and TypeScript only (docs/parameter-object-false-positives.md); Rust function parameters are not extracted into the architecture model yet.",
    },
    Gap {
        check: "go-package-size",
        status: Status::Planned,
        rationale: "Go package aggregate size; Rust has `rust-file-size` per file but no crate/module aggregate yet.",
    },
];

/// Per-language variants of one concept are not gaps: they map to the Rust-scoped check named
/// here (or, for the general `-file-size`/`syntax-rules`/`comment-quality` families, to the
/// `-rust` member of the same family).
const COUNTERPARTS: &[(&str, &str)] = &[
    ("go-ignored-error", "rust-ignored-error"),
    ("java-ignored-error", "rust-ignored-error"),
    ("go-error-context", "rust-error-context"),
    ("java-error-context", "rust-error-context"),
    ("sentinel-return-java", "sentinel-return-rust"),
    ("sentinel-return-kotlin", "sentinel-return-rust"),
    ("sentinel-return-javascript", "sentinel-return-rust"),
    ("primitive-obsession", "primitive-obsession-rust"),
    ("file-complexity", "file-complexity-rust"),
];

/// Where the Rust-scoped check came from, shown in the Shipped table.
const ORIGIN: &[(&str, &str)] = &[
    (
        "test-no-assertion-rust",
        "test-no-assertion (Go/Python/Kotlin)",
    ),
    ("primitive-obsession-rust", "primitive-obsession (Go)"),
    ("file-complexity-rust", "file-complexity (Go)"),
    (
        "sentinel-return-rust",
        "sentinel-return-* (Java/Kotlin/JS), reshaped for `?`",
    ),
    (
        "rust-ignored-error",
        "go-ignored-error / java-ignored-error",
    ),
    (
        "rust-error-context",
        "go-error-context / java-error-context",
    ),
    ("split-loop-rust", "split-loop-go"),
    ("rust-file-size", "`<lang>-file-size` family"),
    ("syntax-rules-rust", "`syntax-rules-<lang>` family"),
    ("comment-quality-rust", "`comment-quality-<lang>` family"),
];

fn has_rust_glob(check: &Check) -> bool {
    check.scope.iter().any(|g| g.ends_with("*.rs"))
}

/// Globs of a non-Rust programming-language check; markdown/Claude-KB checks are not code.
fn is_other_language_code(check: &Check) -> bool {
    const CODE: &[&str] = &[
        ".go", ".ts", ".tsx", ".js", ".java", ".kt", ".py", ".sh", "_test.go",
    ];
    check
        .scope
        .iter()
        .any(|g| CODE.iter().any(|ext| g.ends_with(&format!("*{ext}"))))
}

pub fn shipped_checks(checks: &[Check]) -> Vec<&Check> {
    let mut v: Vec<&Check> = checks.iter().filter(|c| has_rust_glob(c)).collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn render_parity_table() -> String {
    let checks = default_checks();
    let mut out = String::new();
    out.push_str("# Rust check parity\n\n");
    out.push_str("<!-- Generated from src/rust_parity.rs. Do not edit by hand: run `KIBITZER_UPDATE_DOCS=1 cargo test rust_parity`. -->\n\n");
    let _ = writeln!(
        out,
        "Which of kibitzer's default checks cover Rust. Tracking issue: {TRACKER}.\n"
    );
    out.push_str("## Shipped\n\nDefault checks whose scope includes `**/*.rs`.\n\n");
    out.push_str("| Check | Status | Ported from |\n|---|---|---|\n");
    for c in shipped_checks(&checks) {
        let origin = ORIGIN
            .iter()
            .find(|(n, _)| *n == c.name)
            .map_or("language-agnostic", |(_, o)| *o);
        let _ = writeln!(
            out,
            "| `{}` | {} | {} |",
            c.name,
            Status::Shipped.label(),
            origin
        );
    }
    out.push_str("\n## Gaps\n\nChecks for other languages with no Rust counterpart. N/A is intentional; Planned has a sound analogue that is not built yet and is tracked on the issue above.\n\n");
    out.push_str("| Check | Status | Rationale |\n|---|---|---|\n");
    for g in GAPS {
        let _ = writeln!(
            out,
            "| `{}` | {} | {} |",
            g.check,
            g.status.label(),
            g.rationale
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/docs/rust-parity.md");

    #[test]
    fn parity_table_matches_committed_doc() {
        let rendered = render_parity_table();
        if std::env::var_os("KIBITZER_UPDATE_DOCS").is_some() {
            std::fs::write(DOC, &rendered).unwrap();
        }
        let committed = std::fs::read_to_string(DOC).expect("docs/rust-parity.md is missing");
        assert_eq!(
            rendered.trim(),
            committed.trim(),
            "docs/rust-parity.md is stale: run `KIBITZER_UPDATE_DOCS=1 cargo test rust_parity`"
        );
    }

    #[test]
    fn every_rust_scoped_default_check_is_listed_as_shipped() {
        let checks = default_checks();
        let table = render_parity_table();
        let shipped = shipped_checks(&checks);
        assert!(!shipped.is_empty());
        for c in shipped {
            assert!(
                table.contains(&format!("| `{}` | Shipped |", c.name)),
                "{} has a *.rs glob but no Shipped row",
                c.name
            );
        }
    }

    #[test]
    fn no_gap_row_names_a_check_that_has_a_rust_glob() {
        let checks = default_checks();
        for g in GAPS {
            assert!(
                !checks.iter().any(|c| c.name == g.check && has_rust_glob(c)),
                "{} is a gap row but is Rust-scoped",
                g.check
            );
        }
    }

    #[test]
    fn every_non_rust_code_check_is_covered_or_explained() {
        let checks = default_checks();
        for c in &checks {
            let other_language = is_other_language_code(c) || c.scope.is_empty();
            if has_rust_glob(c) || !other_language {
                continue;
            }
            let family = COUNTERPARTS.iter().any(|(n, _)| *n == c.name)
                || c.name.ends_with("-file-size")
                || c.name.starts_with("syntax-rules-")
                || c.name.starts_with("comment-quality-")
                || c.name.starts_with("test-no-assertion-");
            let has_gap = GAPS.iter().any(|g| g.check == c.name);
            assert!(
                family || has_gap,
                "`{}` covers another language but is neither Rust-scoped, mapped in COUNTERPARTS, nor in GAPS",
                c.name
            );
        }
    }

    #[test]
    fn counterparts_exist_and_are_rust_scoped() {
        let checks = default_checks();
        for (from, to) in COUNTERPARTS {
            assert!(
                checks.iter().any(|c| c.name == *from),
                "unknown check {from}"
            );
            assert!(
                checks.iter().any(|c| c.name == *to && has_rust_glob(c)),
                "{to} must be a Rust-scoped default check"
            );
        }
    }

    #[test]
    fn gap_rows_have_rationales_and_planned_rows_cite_a_reason() {
        for g in GAPS {
            assert!(g.rationale.len() > 20, "{} needs a real rationale", g.check);
            assert_ne!(
                g.status,
                Status::Shipped,
                "{} cannot be Shipped here",
                g.check
            );
        }
    }
}
