//! Run sidecar (`<output>.run.json`, or `<output-dir>/run.json`): the schema of design/ux.md
//! section 8.1 rule 6, frozen in `testdata/test-effectiveness/sidecar/sidecar.schema.json`.
//!
//! The adapters read it as a value the input itself carries. Unknown keys are ignored; a
//! missing or wrongly typed required key is a class W warning and caps the dimension at Low.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::guard::{self, GuardError, JsonError};
use crate::test_effectiveness::matrix::{
    CoverageAttribution, DimensionMeta, KillCompleteness, Provenance, Tier, Warning, WarningClass,
};

pub const SCHEMA_VERSION: u64 = 1;
pub const SIDECAR_FILE_IN_DIR: &str = "run.json";

pub const WARN_SIDECAR_MISSING_KEY: &str = "sidecar-missing-key";
pub const WARN_SIDECAR_INVALID_VALUE: &str = "sidecar-invalid-value";
pub const WARN_SIDECAR_SCHEMA_VERSION: &str = "sidecar-schema-version-unsupported";
pub const WARN_KILL_COMPLETENESS_CONFLICT: &str = "kill-completeness-conflict";

const REQUIRED_KEYS: [&str; 5] = [
    "schema_version",
    "tool",
    "tool_version",
    "commit",
    "attribution",
];

/// How complete the mutation run was; `Unknown` when the input does not say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationScope {
    Full,
    Sampled,
    BudgetLimited,
    Unknown,
}

/// Typed `header.inputs[].mutation_run`; counts are `None` when the input does not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MutationRun {
    pub scope: MutationScope,
    pub mutants_run: Option<u64>,
    pub mutants_total: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sidecar {
    pub schema_version: Option<u64>,
    pub tool: Option<String>,
    pub tool_version: Option<String>,
    pub commit: Option<String>,
    pub attribution: Option<CoverageAttribution>,
    pub tests_total: Option<u64>,
    pub mutants_total: Option<u64>,
    pub mutants_run: Option<u64>,
    pub sampled_fraction: Option<f64>,
    pub budget_seconds: Option<f64>,
    pub budget_exhausted: bool,
    pub partial: bool,
}

impl Sidecar {
    /// True when the sidecar says anything about a mutation run (a coverage sidecar does not).
    fn describes_mutation_run(&self) -> bool {
        self.mutants_total.is_some()
            || self.mutants_run.is_some()
            || self.sampled_fraction.is_some()
            || self.budget_exhausted
            || self.partial
    }

    /// A partial run must never read as complete: budget stop, then sampling, then any other early stop.
    pub fn kill_completeness(&self) -> KillCompleteness {
        let ran_fewer =
            matches!((self.mutants_run, self.mutants_total), (Some(r), Some(t)) if r < t);
        if self.budget_exhausted {
            KillCompleteness::BudgetLimited
        } else if self.sampled_fraction.is_some_and(|f| f < 1.0) || ran_fewer {
            KillCompleteness::Sampled
        } else if self.partial {
            KillCompleteness::BudgetLimited
        } else {
            KillCompleteness::Full
        }
    }

    pub fn mutation_run(&self) -> MutationRun {
        let scope = if !self.describes_mutation_run() {
            MutationScope::Unknown
        } else {
            match self.kill_completeness() {
                KillCompleteness::Full | KillCompleteness::FirstKillerOnly => MutationScope::Full,
                KillCompleteness::Sampled => MutationScope::Sampled,
                KillCompleteness::BudgetLimited => MutationScope::BudgetLimited,
            }
        };
        MutationRun {
            scope,
            mutants_run: self.mutants_run,
            mutants_total: self.mutants_total,
        }
    }
}

/// A parsed sidecar plus the class W warnings and structural cap it implies.
#[derive(Debug, Clone, PartialEq)]
pub struct SidecarRead {
    pub sidecar: Sidecar,
    pub warnings: Vec<Warning>,
    /// `Some(Low)` when a required key is missing or invalid, or the run was not complete.
    pub cap: Option<Tier>,
}

#[derive(Debug)]
pub enum SidecarError {
    Guard(GuardError),
    Json(JsonError),
    NotAnObject,
}

impl std::fmt::Display for SidecarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Guard(e) => e.fmt(f),
            Self::Json(e) => e.fmt(f),
            Self::NotAnObject => f.write_str("run sidecar is not a JSON object"),
        }
    }
}

impl std::error::Error for SidecarError {}

fn warn(id: &'static str, key: &str) -> Warning {
    Warning {
        id,
        class: WarningClass::W,
        params: vec![("key".to_string(), key.to_string())],
    }
}

fn parse_attribution(label: &str) -> Option<CoverageAttribution> {
    [
        CoverageAttribution::Isolated,
        CoverageAttribution::InOrder,
        CoverageAttribution::LeaveOneOut,
    ]
    .into_iter()
    .find(|a| a.label() == label)
}

/// Looks up an optional key; `null` and absent are both "not stated". A wrong type is flagged and dropped.
fn optional<T>(
    obj: &Map<String, Value>,
    key: &str,
    convert: impl Fn(&Value) -> Option<T>,
    warnings: &mut Vec<Warning>,
) -> Option<T> {
    match obj.get(key) {
        None | Some(Value::Null) => None,
        Some(v) => {
            let converted = convert(v);
            if converted.is_none() {
                warnings.push(warn(WARN_SIDECAR_INVALID_VALUE, key));
            }
            converted
        }
    }
}

fn as_string(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

pub fn parse_sidecar(bytes: &[u8]) -> Result<SidecarRead, SidecarError> {
    let value: Value = guard::parse_json_bounded(bytes).map_err(SidecarError::Json)?;
    let Value::Object(obj) = value else {
        return Err(SidecarError::NotAnObject);
    };
    let mut warnings = Vec::new();

    for key in REQUIRED_KEYS {
        if !matches!(obj.get(key), Some(v) if !v.is_null()) {
            warnings.push(warn(WARN_SIDECAR_MISSING_KEY, key));
        }
    }

    let schema_version = optional(&obj, "schema_version", Value::as_u64, &mut warnings);
    if schema_version.is_some_and(|v| v != SCHEMA_VERSION) {
        warnings.push(warn(WARN_SIDECAR_SCHEMA_VERSION, "schema_version"));
    }
    let sidecar = Sidecar {
        schema_version,
        tool: optional(&obj, "tool", as_string, &mut warnings),
        tool_version: optional(&obj, "tool_version", as_string, &mut warnings),
        commit: optional(&obj, "commit", as_string, &mut warnings),
        attribution: optional(
            &obj,
            "attribution",
            |v| v.as_str().and_then(parse_attribution),
            &mut warnings,
        ),
        tests_total: optional(&obj, "tests_total", Value::as_u64, &mut warnings),
        mutants_total: optional(&obj, "mutants_total", Value::as_u64, &mut warnings),
        mutants_run: optional(&obj, "mutants_run", Value::as_u64, &mut warnings),
        sampled_fraction: optional(&obj, "sampled_fraction", Value::as_f64, &mut warnings),
        budget_seconds: optional(&obj, "budget_seconds", Value::as_f64, &mut warnings),
        budget_exhausted: optional(&obj, "budget_exhausted", Value::as_bool, &mut warnings)
            .unwrap_or(false),
        partial: optional(&obj, "partial", Value::as_bool, &mut warnings).unwrap_or(false),
    };

    let (_, completeness_cap) = sidecar.kill_completeness().derive(&Provenance::default());
    let cap = (!warnings.is_empty())
        .then_some(Tier::Low)
        .or(completeness_cap);
    Ok(SidecarRead {
        sidecar,
        warnings,
        cap,
    })
}

/// Reads a sidecar through the size guard.
pub fn read_sidecar(path: &Path, max_input_bytes: u64) -> Result<SidecarRead, SidecarError> {
    let bytes = guard::read_bounded(path, max_input_bytes).map_err(SidecarError::Guard)?;
    parse_sidecar(&bytes)
}

/// Where the sidecar for a producer output lives: `<dir>/run.json` for a directory, else `<output>.run.json`.
pub fn sidecar_path_for(output: &Path) -> PathBuf {
    if output.is_dir() {
        return output.join(SIDECAR_FILE_IN_DIR);
    }
    let mut name = output.as_os_str().to_owned();
    name.push(".run.json");
    PathBuf::from(name)
}

/// Lower is less complete.
fn completeness_rank(c: KillCompleteness) -> u8 {
    match c {
        KillCompleteness::Full => 3,
        KillCompleteness::FirstKillerOnly => 2,
        KillCompleteness::Sampled => 1,
        KillCompleteness::BudgetLimited => 0,
    }
}

/// Precedence of Story 2.2.9: a sidecar and the `--kill-completeness` flag that disagree
/// resolve to the less complete value with a class W warning naming both; either alone wins;
/// neither stated is `Full` (the run's scope is then `Unknown`, see [`MutationRun`]).
pub fn resolve_kill_completeness(
    sidecar: Option<&Sidecar>,
    flag: Option<KillCompleteness>,
) -> (KillCompleteness, Vec<Warning>) {
    let from_sidecar = sidecar.map(Sidecar::kill_completeness);
    match (from_sidecar, flag) {
        (Some(s), Some(f)) if s != f => {
            let less = if completeness_rank(s) <= completeness_rank(f) {
                s
            } else {
                f
            };
            let warning = Warning {
                id: WARN_KILL_COMPLETENESS_CONFLICT,
                class: WarningClass::W,
                params: vec![
                    ("sidecar".to_string(), s.label().to_string()),
                    ("flag".to_string(), f.label().to_string()),
                ],
            };
            (less, vec![warning])
        }
        (Some(v), _) | (None, Some(v)) => (v, Vec::new()),
        (None, None) => (KillCompleteness::Full, Vec::new()),
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    const MAX: u64 = 1 << 20;

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("testdata/test-effectiveness/sidecar")
            .join(rel)
    }

    fn read(rel: &str) -> SidecarRead {
        read_sidecar(&fixture(rel), MAX).unwrap()
    }

    fn ids(r: &SidecarRead) -> Vec<&'static str> {
        r.warnings.iter().map(|w| w.id).collect()
    }

    #[test]
    fn sidecar_should_SetBudgetLimitedAndCopyCounts_When_HandWrittenSidecarBudgetExhausted() {
        // Given a hand-written sidecar with budget_exhausted, 412 of 1930
        let r = read("valid/mutation-budget-exhausted.json");
        // Then kill_completeness is budget_limited and the counts are copied
        assert_eq!(
            r.sidecar.kill_completeness(),
            KillCompleteness::BudgetLimited
        );
        let run = r.sidecar.mutation_run();
        assert_eq!(run.scope, MutationScope::BudgetLimited);
        assert_eq!(
            (run.mutants_run, run.mutants_total),
            (Some(412), Some(1930))
        );
        assert!(r.warnings.is_empty());
    }

    #[test]
    fn adapter_should_ReturnTypedMutationRunAndCapLow_When_SidecarBudgetExhausted() {
        let r = read("valid/mutation-budget-exhausted.json");
        assert_eq!(
            r.sidecar.mutation_run(),
            MutationRun {
                scope: MutationScope::BudgetLimited,
                mutants_run: Some(412),
                mutants_total: Some(1930),
            }
        );
        assert_eq!(r.cap, Some(Tier::Low));
    }

    #[test]
    fn sidecar_should_BeFullWithNoCap_When_FullRunSidecar() {
        let r = read("valid/mutation-full.json");
        assert_eq!(r.sidecar.mutation_run().scope, MutationScope::Full);
        assert_eq!(r.cap, None);
        assert_eq!(r.sidecar.attribution, Some(CoverageAttribution::Isolated));
    }

    #[test]
    fn sidecar_should_BeSampled_When_SampledFractionBelowOne() {
        let r = read("valid/mutation-sampled.json");
        assert_eq!(r.sidecar.kill_completeness(), KillCompleteness::Sampled);
        assert_eq!(r.cap, Some(Tier::Low));
    }

    #[test]
    fn sidecar_should_HaveUnknownScopeAndNoCap_When_CoverageSidecarCarriesNoMutationFields() {
        let r = read("valid/coverage-python.json");
        assert_eq!(r.sidecar.mutation_run().scope, MutationScope::Unknown);
        assert_eq!(r.sidecar.attribution, Some(CoverageAttribution::InOrder));
        assert_eq!(r.sidecar.tests_total, Some(312));
        assert_eq!(r.cap, None);
    }

    #[test]
    fn sidecar_should_IgnoreUnknownKeys_When_FutureFieldsPresent() {
        let r = read("valid/unknown-keys.json");
        assert!(r.warnings.is_empty());
        assert_eq!(r.cap, None);
    }

    #[test]
    fn sidecar_should_WarnAndCapLow_When_RequiredKeyMissing() {
        let r = read("invalid/missing-required.json");
        let missing: Vec<_> = r
            .warnings
            .iter()
            .filter(|w| w.id == WARN_SIDECAR_MISSING_KEY)
            .map(|w| w.params[0].1.as_str())
            .collect();
        assert_eq!(missing, ["commit", "attribution"]);
        assert!(r.warnings.iter().all(|w| w.class == WarningClass::W));
        assert_eq!(r.cap, Some(Tier::Low));
    }

    #[test]
    fn sidecar_should_TreatModeAsUnknownAndWarnAttributionMissing_When_LegacyModeKey() {
        let r = read("invalid/legacy-mode-key.json");
        assert_eq!(r.sidecar.attribution, None);
        assert_eq!(ids(&r), [WARN_SIDECAR_MISSING_KEY]);
        assert_eq!(r.cap, Some(Tier::Low));
    }

    #[test]
    fn sidecar_should_WarnInvalidValueAndCapLow_When_KeyHasWrongType() {
        let r = read("invalid/wrong-type.json");
        let bad: Vec<_> = r.warnings.iter().map(|w| w.params[0].1.as_str()).collect();
        assert_eq!(bad, ["mutants_total", "budget_exhausted"]);
        assert!(!r.sidecar.budget_exhausted);
        assert_eq!(r.cap, Some(Tier::Low));
    }

    #[test]
    fn sidecar_should_WarnAndCapLow_When_SchemaVersionUnsupported() {
        let r = read("invalid/unsupported-schema-version.json");
        assert_eq!(ids(&r), [WARN_SIDECAR_SCHEMA_VERSION]);
        assert_eq!(r.cap, Some(Tier::Low));
    }

    #[test]
    fn sidecar_should_ReturnError_When_NotAnObjectOrTruncated() {
        assert!(matches!(
            read_sidecar(&fixture("invalid/not-an-object.json"), MAX),
            Err(SidecarError::NotAnObject)
        ));
        assert!(matches!(
            read_sidecar(&fixture("invalid/truncated.json"), MAX),
            Err(SidecarError::Json(JsonError::Invalid(_)))
        ));
        assert!(matches!(
            parse_sidecar(b""),
            Err(SidecarError::Json(JsonError::Invalid(_)))
        ));
    }

    #[test]
    fn sidecar_should_RefuseWithGuardError_When_FileOverCap() {
        let err = read_sidecar(&fixture("valid/mutation-full.json"), 10).unwrap_err();
        assert!(matches!(err, SidecarError::Guard(GuardError::TooLarge(_))));
    }

    #[test]
    fn sidecar_path_should_FollowOutputKind_When_FileOrDirectory() {
        let dir = fixture("valid");
        assert_eq!(sidecar_path_for(&dir), dir.join("run.json"));
        let file = Path::new("out/mutations.json");
        assert_eq!(
            sidecar_path_for(file),
            PathBuf::from("out/mutations.json.run.json")
        );
    }

    #[test]
    fn schema_file_should_RequireAttributionAndNeverMention_mode_When_Inspected() {
        let text = std::fs::read_to_string(fixture("sidecar.schema.json")).unwrap();
        let schema: Value = serde_json::from_str(&text).unwrap();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|k| k == "attribution"));
        assert!(schema["properties"].get("mode").is_none());
        for key in REQUIRED_KEYS {
            assert!(required.iter().any(|k| k == key), "{key}");
        }
    }

    #[test]
    fn resolve_should_ChooseLessCompleteWithWarningNamingBoth_When_SidecarFullAndFlagSampled() {
        let full = read("valid/mutation-full.json").sidecar;
        let (value, warnings) =
            resolve_kill_completeness(Some(&full), Some(KillCompleteness::Sampled));
        assert_eq!(value, KillCompleteness::Sampled);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].id, WARN_KILL_COMPLETENESS_CONFLICT);
        assert_eq!(warnings[0].class, WarningClass::W);
        assert!(
            warnings[0]
                .params
                .contains(&("sidecar".into(), "full".into()))
        );
        assert!(
            warnings[0]
                .params
                .contains(&("flag".into(), "sampled".into()))
        );
    }

    #[test]
    fn resolve_should_ChooseSidecarSide_When_SidecarLessCompleteThanFlag() {
        let budget = read("valid/mutation-budget-exhausted.json").sidecar;
        let (value, warnings) =
            resolve_kill_completeness(Some(&budget), Some(KillCompleteness::Full));
        assert_eq!(value, KillCompleteness::BudgetLimited);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn resolve_should_UseWhicheverIsStatedWithoutWarning_When_OnlyOneSource() {
        let sampled = read("valid/mutation-sampled.json").sidecar;
        assert_eq!(
            resolve_kill_completeness(Some(&sampled), None),
            (KillCompleteness::Sampled, vec![])
        );
        assert_eq!(
            resolve_kill_completeness(None, Some(KillCompleteness::FirstKillerOnly)),
            (KillCompleteness::FirstKillerOnly, vec![])
        );
        assert_eq!(
            resolve_kill_completeness(None, None),
            (KillCompleteness::Full, vec![])
        );
    }

    #[test]
    fn resolve_should_NotWarn_When_SidecarAndFlagAgree() {
        let sampled = read("valid/mutation-sampled.json").sidecar;
        let (value, warnings) =
            resolve_kill_completeness(Some(&sampled), Some(KillCompleteness::Sampled));
        assert_eq!(value, KillCompleteness::Sampled);
        assert!(warnings.is_empty());
    }
}
