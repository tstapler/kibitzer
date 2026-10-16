//! `ReportEnvelope` (Task 2.3.1e): everything in a report that is not a finding. Built once from the analysis and
//! rendered by both the text and the JSON renderer, so the two cannot diverge. `schema_version` 1 is declared here.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::test_effectiveness::adapters::sidecar::{MutationRun, MutationScope};
pub use crate::test_effectiveness::confidence::ReportMode;
use crate::test_effectiveness::exclusions::ExclusionReason;
use crate::test_effectiveness::matrix::{Warning, WarningClass};

/// Frozen with this task; additive keys keep it, removing or retyping a key requires 2.
pub const SCHEMA_VERSION: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
    Brief,
}

impl Format {
    pub fn parse(s: &str) -> Option<Format> {
        match s {
            "text" => Some(Self::Text),
            "json" => Some(Self::Json),
            "brief" => Some(Self::Brief),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Json => "json",
            Self::Brief => "brief",
        }
    }
}

/// First output line for each mode (`design/ux.md` section 3.2a). The `tiered` text names no arm count because no
/// G2 decision exists to supply one.
pub fn banner(mode: ReportMode) -> &'static str {
    match mode {
        ReportMode::Experimental => {
            "experimental: signals not yet validated (see docs/test-effectiveness.md)"
        }
        ReportMode::Descriptive => {
            "descriptive: signals not validated as removal predictors; see docs/test-effectiveness.md"
        }
        ReportMode::ReportOnly => {
            "report_only: signals measured, ordering not validated as removal advice; backing: docs/test-effectiveness.md#results"
        }
        ReportMode::Tiered => {
            "tiered: confidence tiers come from triaged precision on the measured arms; backing: docs/test-effectiveness.md#results"
        }
    }
}

/// The non-endorsement statements every report carries. `descriptive` and `report_only` say "listed".
pub fn non_claims(mode: ReportMode) -> Vec<&'static str> {
    let first = if mode.shows_confidence() {
        "Absence of a flag does not mean a test is valuable."
    } else {
        "Absence from this list does not mean a test is valuable."
    };
    vec![
        first,
        "A flagged test may be a regression pin, a spec, or guard a distinct failure message.",
        "Per-test values are alone-values; they do not hold once an overlapping flagged test is gone.",
        "The joint coverage check is computed on the data the set was chosen from and says nothing about fault detection.",
    ]
}

/// Scalar parameter of a warning (`params` holds values a script would otherwise parse out of `message`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Param {
    Str(String),
    Bool(bool),
}

impl Param {
    fn to_json(&self) -> Value {
        match self {
            Self::Str(s) => json!(s),
            Self::Bool(b) => json!(b),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Str(s) => s.clone(),
            Self::Bool(b) => b.to_string(),
        }
    }
}

/// `lowers` of a warning: which signal's confidence it lowered, and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lowers {
    /// `coverage_derived`, `kill_derived`, `static` or `all`.
    pub signal: &'static str,
    /// `excluded`, `capped_low`, `capped_medium` or `informational`.
    pub effect: &'static str,
    pub tests: Option<usize>,
    pub files: Option<usize>,
}

/// A class W or I message with a stable id (never the design-table row number).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarningDoc {
    pub id: String,
    pub class: WarningClass,
    pub lowers: Lowers,
    pub params: Vec<(String, Param)>,
    pub message: String,
}

impl WarningDoc {
    pub fn class_letter(&self) -> &'static str {
        match self.class {
            WarningClass::W => "W",
            WarningClass::I => "I",
        }
    }

    /// The text-mode line: `warning:` for class W, `note:` for class I.
    pub fn line(&self) -> String {
        let prefix = match self.class {
            WarningClass::W => "warning",
            WarningClass::I => "note",
        };
        format!(
            "{prefix}: {} (lowers: {})",
            self.message, self.lowers.signal
        )
    }

    fn to_json(&self) -> Value {
        let mut params = Map::new();
        for (k, v) in &self.params {
            params.insert(k.clone(), v.to_json());
        }
        json!({
            "id": self.id,
            "class": self.class_letter(),
            "lowers": {
                "signal": self.lowers.signal,
                "effect": self.lowers.effect,
                "tests": self.lowers.tests,
                "files": self.lowers.files,
            },
            "params": Value::Object(params),
            "message": self.message,
        })
    }
}

/// Which dimension carried a matrix warning, so `lowers.signal` can name it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Coverage,
    Kills,
    Run,
}

impl Origin {
    fn signal(self) -> &'static str {
        match self {
            Self::Coverage => "coverage_derived",
            Self::Kills => "kill_derived",
            Self::Run => "all",
        }
    }
}

fn param<'a>(w: &'a Warning, key: &str) -> &'a str {
    w.params
        .iter()
        .find(|(k, _)| k == key)
        .map_or("?", |(_, v)| v.as_str())
}

/// Turns a typed matrix warning into its report entry. Ids not in the table fall back to a generic entry
/// (`capped_low`, the conservative reading); Task 2.3.1f adds one row per remaining message.
pub fn describe_warning(w: &Warning, origin: Origin) -> WarningDoc {
    let signal = origin.signal();
    let (effect, message, extra): (&'static str, String, Vec<(String, Param)>) = match w.id {
        "attribution-in-order" => (
            "capped_medium",
            "attribution is in-order (assumed; pass --attribution to state it); unique-coverage can over-state \
             redundancy when tests share setup. Treat as medium at most."
                .to_string(),
            vec![("assumed".to_string(), Param::Bool(true))],
        ),
        "unverified-tool-version" => (
            "capped_low",
            format!(
                "the producing tool's version or schema is not on the verified list; findings from this input \
                 are capped at low{}",
                w.params
                    .iter()
                    .map(|(k, v)| format!(" ({k} {v})"))
                    .collect::<String>()
            ),
            Vec::new(),
        ),
        "input-commit-unknown" => (
            "capped_low",
            "report has no commit; cannot check staleness. Treat results as unverified against the current tree."
                .to_string(),
            Vec::new(),
        ),
        "first-killer-only" => (
            "capped_low",
            "kill sets look truncated (bail on first kill); unique kills over-credit tests; zero-unique-kills \
             capped at low."
                .to_string(),
            Vec::new(),
        ),
        "full-matrix-not-enabled" => (
            "capped_low",
            "full mutation matrix not enabled (only killingTest present); zero-unique-kills capped at low."
                .to_string(),
            Vec::new(),
        ),
        "kill-completeness-conflict" => (
            "capped_low",
            format!(
                "run sidecar says {} but --kill-completeness says {}; using the less complete value; \
                 zero-unique-kills capped at low.",
                param(w, "sidecar"),
                param(w, "flag")
            ),
            Vec::new(),
        ),
        other => (
            "capped_low",
            format!(
                "{other}{}",
                w.params
                    .iter()
                    .map(|(k, v)| format!(" ({k} {v})"))
                    .collect::<String>()
            ),
            Vec::new(),
        ),
    };
    let mut params: Vec<(String, Param)> = w
        .params
        .iter()
        .map(|(k, v)| (k.clone(), Param::Str(v.clone())))
        .collect();
    params.extend(extra);
    WarningDoc {
        id: w.id.to_string(),
        class: w.class,
        lowers: Lowers {
            signal,
            effect,
            tests: None,
            files: None,
        },
        params,
        message,
    }
}

/// A gated flag (`--format brief`) accepted before its owning story lands.
pub fn gated_flag_warning(flag: &str, story: &str) -> WarningDoc {
    WarningDoc {
        id: "gated-flag-ignored".to_string(),
        class: WarningClass::W,
        lowers: Lowers {
            signal: "all",
            effect: "informational",
            tests: None,
            files: None,
        },
        params: vec![
            ("flag".to_string(), Param::Str(flag.to_string())),
            ("story".to_string(), Param::Str(story.to_string())),
        ],
        message: format!(
            "{flag} is accepted but not yet implemented (Story {story}); output follows --format json"
        ),
    }
}

impl Param {
    pub fn render(&self) -> String {
        self.text()
    }
}

/// One `header.inputs[]` entry.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderInput {
    pub flag: &'static str,
    pub tool: String,
    pub tool_version: Option<String>,
    pub commit: Option<String>,
    pub run: Option<String>,
    pub attribution: Option<&'static str>,
    pub attribution_assumed: bool,
    pub kill_completeness: Option<&'static str>,
    pub mutation_run: Option<MutationRun>,
}

/// Snake_case spelling of a hyphenated flag value.
pub fn snake(label: &str) -> String {
    label.replace('-', "_")
}

fn mutation_run_json(run: &MutationRun) -> Value {
    let scope = match run.scope {
        MutationScope::Full => "full",
        MutationScope::Sampled => "sampled",
        MutationScope::BudgetLimited => "budget_limited",
        MutationScope::Unknown => "unknown",
    };
    json!({
        "scope": scope,
        "mutants_in_report": run.mutants_run,
        "mutants_total": run.mutants_total,
        "sampled_fraction": Value::Null,
        "budget_seconds": Value::Null,
        "budget_exhausted": Value::Null,
    })
}

impl HeaderInput {
    fn to_json(&self) -> Value {
        json!({
            "flag": self.flag,
            "tool": self.tool,
            "tool_version": self.tool_version,
            "commit": self.commit,
            "run": self.run,
            "attribution": self.attribution.map(snake),
            "attribution_assumed": self.attribution_assumed,
            "kill_completeness": self.kill_completeness.map(snake),
            "mutation_run": self.mutation_run.as_ref().map(mutation_run_json),
            "leave_one_out": Value::Null,
        })
    }
}

/// `header.scope`: `by_reason` holds every reason, zeros included, so its parts sum to `not_assessed`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub tests_in_input: usize,
    pub by_reason: Vec<(ExclusionReason, usize)>,
}

impl Scope {
    pub fn not_assessed(&self) -> usize {
        self.by_reason.iter().map(|(_, n)| n).sum()
    }

    pub fn assessed(&self) -> usize {
        self.tests_in_input - self.not_assessed()
    }

    /// JSON key of a reason: the printed label in snake_case.
    pub fn reason_key(r: ExclusionReason) -> String {
        r.label().replace([' ', '-'], "_")
    }

    fn to_json(&self) -> Value {
        let mut by_reason = Map::new();
        for r in [
            ExclusionReason::Stale,
            ExclusionReason::NoSource,
            ExclusionReason::Flaky,
            ExclusionReason::Failing,
            ExclusionReason::Skipped,
            ExclusionReason::Generated,
            ExclusionReason::NoData,
        ] {
            let n = self
                .by_reason
                .iter()
                .find(|(x, _)| *x == r)
                .map_or(0, |(_, n)| *n);
            by_reason.insert(Self::reason_key(r), json!(n));
        }
        json!({
            "tests_in_input": self.tests_in_input,
            "assessed": self.assessed(),
            "not_assessed": self.not_assessed(),
            "by_reason": Value::Object(by_reason),
            "repo_tests_not_in_input": Value::Null,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub kibitzer_version: &'static str,
    pub head_commit: Option<String>,
    pub inputs: Vec<HeaderInput>,
    pub scope: Scope,
}

/// `summary` skeleton. `joint_check` stays `[]` until Task 2.3.1i computes it per rule class.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    pub flagged_by_rule: BTreeMap<&'static str, usize>,
    /// Main-section findings, unsuppressed.
    pub individually_flagged: usize,
    pub redundant_only_with_flagged: usize,
    /// Present only with `--experimental`.
    pub experimental: Option<BTreeMap<&'static str, usize>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReportEnvelope {
    pub format: Format,
    pub mode: ReportMode,
    pub header: Header,
    pub warnings: Vec<WarningDoc>,
    pub summary: Summary,
}

impl ReportEnvelope {
    pub fn banner(&self) -> &'static str {
        banner(self.mode)
    }

    fn summary_json(&self) -> Value {
        let s = &self.summary;
        let mut out = Map::new();
        out.insert("flagged_by_rule".to_string(), json!(s.flagged_by_rule));
        if self.mode.shows_confidence() {
            out.insert(
                "individually_flagged".to_string(),
                json!(s.individually_flagged),
            );
            out.insert(
                "redundant_only_with_flagged".to_string(),
                json!(s.redundant_only_with_flagged),
            );
        } else {
            out.insert("listed".to_string(), json!(s.individually_flagged));
        }
        out.insert("joint_check".to_string(), json!([]));
        if let Some(exp) = &s.experimental {
            out.insert(
                "experimental".to_string(),
                json!({"shown": true, "flagged_by_rule": exp}),
            );
        }
        out.insert(
            "suppressed".to_string(),
            json!({"total": 0, "non_specific_reason": 0, "malformed_directives": 0}),
        );
        out.insert("stability".to_string(), json!("single_run"));
        Value::Object(out)
    }

    /// The envelope keys in contract order, from `schema_version` through `summary`; the renderer appends
    /// `findings`, `not_assessed` and `suppressed`.
    pub fn to_json_map(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("schema_version".to_string(), json!(SCHEMA_VERSION));
        m.insert("format".to_string(), json!(self.format.as_str()));
        m.insert("mode".to_string(), json!(self.mode.as_str()));
        m.insert("banner".to_string(), json!(self.banner()));
        m.insert("advisory".to_string(), json!(true));
        m.insert("non_claims".to_string(), json!(non_claims(self.mode)));
        m.insert(
            "header".to_string(),
            json!({
                "kibitzer_version": self.header.kibitzer_version,
                "head_commit": self.header.head_commit,
                "inputs": self.header.inputs.iter().map(HeaderInput::to_json).collect::<Vec<_>>(),
                "scope": self.header.scope.to_json(),
            }),
        );
        m.insert(
            "warnings".to_string(),
            Value::Array(self.warnings.iter().map(WarningDoc::to_json).collect()),
        );
        m.insert("summary".to_string(), self.summary_json());
        m
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn matrix_warning(id: &'static str) -> Warning {
        Warning {
            id,
            class: WarningClass::W,
            params: Vec::new(),
        }
    }

    fn envelope(mode: ReportMode) -> ReportEnvelope {
        ReportEnvelope {
            format: Format::Json,
            mode,
            header: Header {
                kibitzer_version: "0.0.0",
                head_commit: None,
                inputs: Vec::new(),
                scope: Scope {
                    tests_in_input: 3,
                    by_reason: vec![(ExclusionReason::Stale, 1)],
                },
            },
            warnings: Vec::new(),
            summary: Summary::default(),
        }
    }

    #[test]
    fn envelope_should_HaveContractKeyOrder_When_Built() {
        let m = envelope(ReportMode::Experimental).to_json_map();
        let keys: Vec<_> = m.keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            [
                "schema_version",
                "format",
                "mode",
                "banner",
                "advisory",
                "non_claims",
                "header",
                "warnings",
                "summary"
            ]
        );
        assert_eq!(m["schema_version"], 1);
        assert_eq!(m["header"]["scope"]["assessed"], 2);
        assert_eq!(m["header"]["scope"]["by_reason"]["stale"], 1);
    }

    #[test]
    fn banner_should_BeFirstLineOfEachMode_And_NeverHyphenateReportOnly() {
        for mode in [
            ReportMode::Experimental,
            ReportMode::Descriptive,
            ReportMode::ReportOnly,
            ReportMode::Tiered,
        ] {
            let b = banner(mode);
            assert!(b.starts_with(&format!("{}:", mode.as_str())), "{b}");
            assert!(!b.contains("report-only"));
            assert!(!b.contains('\n'));
        }
    }

    #[test]
    fn summary_should_NameListedAndOmitRedundantOnly_When_ReportOnlyMode() {
        let m = envelope(ReportMode::ReportOnly).to_json_map();
        let s = m["summary"].as_object().unwrap();
        assert!(s.contains_key("listed"));
        assert!(!s.contains_key("individually_flagged"));
        assert!(!s.contains_key("redundant_only_with_flagged"));
        let m = envelope(ReportMode::Experimental).to_json_map();
        assert!(
            m["summary"]
                .as_object()
                .unwrap()
                .contains_key("redundant_only_with_flagged")
        );
    }

    #[test]
    fn warnings_should_CarryStableIdStructuredLowersAndParams_When_AssumedAttributionAndUnlistedVersion()
     {
        let in_order = describe_warning(&matrix_warning("attribution-in-order"), Origin::Coverage);
        assert_eq!(in_order.id, "attribution-in-order");
        assert_eq!(in_order.lowers.signal, "coverage_derived");
        assert_eq!(in_order.lowers.effect, "capped_medium");
        assert!(
            in_order
                .params
                .contains(&("assumed".to_string(), Param::Bool(true)))
        );
        let version = describe_warning(&matrix_warning("unverified-tool-version"), Origin::Kills);
        assert_eq!(version.lowers.signal, "kill_derived");
        let j = version.to_json();
        let keys: Vec<_> = j.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, ["id", "class", "lowers", "params", "message"]);
        assert_eq!(j["class"], "W");
        assert!(version.line().starts_with("warning: "));
        assert!(version.line().ends_with("(lowers: kill_derived)"));
    }

    #[test]
    fn warning_should_FallBackToConservativeEntry_When_IdUnknown() {
        let w = describe_warning(&matrix_warning("brand-new-id"), Origin::Run);
        assert_eq!(w.lowers.effect, "capped_low");
        assert_eq!(w.lowers.signal, "all");
        let note = WarningDoc {
            class: WarningClass::I,
            ..w
        };
        assert!(note.line().starts_with("note: "));
    }

    #[test]
    fn non_claims_should_SayListed_When_ModeDoesNotShowConfidence() {
        assert!(non_claims(ReportMode::Descriptive)[0].starts_with("Absence from this list"));
        assert!(non_claims(ReportMode::Experimental)[0].starts_with("Absence of a flag"));
        assert_eq!(Format::parse("brief"), Some(Format::Brief));
        assert_eq!(Format::parse("xml"), None);
    }
}
