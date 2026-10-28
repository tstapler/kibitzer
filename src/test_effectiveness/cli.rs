//! Argument surface of the hidden `kibitzer tests report` command (Task 2.3.1a) and the `--kill-completeness`
//! flag with its precedence against the run sidecar (Task 2.2.9b).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Subcommand, ValueEnum};

use crate::checker::{CheckContext, GrammarCache};
use crate::inline_ignores::sanitize::escape_path;
use crate::test_code::{TestCodeProfile, TestKind};

use super::adapters::guard::{DEFAULT_MAX_INPUT_BYTES, GuardError};
use super::adapters::pit_xml::{self, PitOptions};
use super::adapters::sidecar::{self, MutationRun, Sidecar, SidecarRead};
use super::adapters::{coveragepy, gocover, mutation_elements};
use super::confidence::G2_VERDICTS;
use super::locate::{Location, RawTestId, TestLocator};
use super::matrix::{
    CoverageAttribution, CoverageDimension, Dimension, KillCompleteness, KillDimension, TestId,
    WARN_INPUT_COMMIT_UNKNOWN, Warning,
};
use super::report::envelope::{self, Format, HeaderInput, Scope};
use super::report::json::{self, ClassE};
use super::report::{self, AnalyzeOptions, InputInfo, Loaded, Located, Report, export, text};

#[derive(Subcommand)]
pub enum TestsCommand {
    /// Rank tests whose coverage or kills no other test needs, from ingested third-party reports.
    Report(ReportArgs),
}

/// `--kill-completeness` spelling (hyphenated; JSON values are snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum KillCompletenessArg {
    Full,
    FirstKiller,
    Sampled,
    BudgetLimited,
}

impl KillCompletenessArg {
    pub fn into_domain(self) -> KillCompleteness {
        match self {
            Self::Full => KillCompleteness::Full,
            Self::FirstKiller => KillCompleteness::FirstKillerOnly,
            Self::Sampled => KillCompleteness::Sampled,
            Self::BudgetLimited => KillCompleteness::BudgetLimited,
        }
    }
}

/// `--attribution` spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AttributionArg {
    Isolated,
    InOrder,
    LeaveOneOut,
}

impl AttributionArg {
    pub fn into_domain(self) -> CoverageAttribution {
        match self {
            Self::Isolated => CoverageAttribution::Isolated,
            Self::InOrder => CoverageAttribution::InOrder,
            Self::LeaveOneOut => CoverageAttribution::LeaveOneOut,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub struct ReportArgs {
    /// coverage.py JSON report produced with `--show-contexts`.
    #[arg(long, value_name = "FILE")]
    pub coveragepy: Option<PathBuf>,
    /// Directory of per-test Go coverage profiles plus `manifest.json`.
    #[arg(long, value_name = "DIR")]
    pub go_profiles: Option<PathBuf>,
    /// Stryker mutation-testing-elements JSON report.
    #[arg(long, value_name = "FILE")]
    pub stryker: Option<PathBuf>,
    /// PIT `mutations.xml`.
    #[arg(long, value_name = "FILE")]
    pub pit: Option<PathBuf>,
    /// Output format: text, json or brief (validated by the command so a bad value gets the documented error).
    #[arg(long, default_value = "text")]
    pub format: String,
    /// Rows shown in text output; 0 means all. JSON is always complete.
    #[arg(long, default_value = "20")]
    pub top: String,
    /// How per-test coverage was attributed, when the input does not say.
    #[arg(long, value_enum)]
    pub attribution: Option<AttributionArg>,
    /// How complete the kill data is, when no run sidecar says; the less complete of flag and sidecar wins.
    #[arg(long, value_enum)]
    pub kill_completeness: Option<KillCompletenessArg>,
    /// Commit the inputs were produced at, when the input does not carry one.
    #[arg(long, value_name = "SHA")]
    pub input_commit: Option<String>,
    /// Largest input file read, in bytes. Defaults to the built-in ceiling.
    #[arg(long, default_value_t = DEFAULT_MAX_INPUT_BYTES)]
    pub max_input_bytes: u64,
    /// Also list signals outside the main-list rule, in their own section.
    #[arg(long)]
    pub experimental: bool,
    /// Write `[tests]` diagnostics to stderr.
    #[arg(long)]
    pub verbose: bool,
    #[arg(long, hide = true, value_name = "PATH")]
    pub emit_experiment_input: Option<PathBuf>,
}

/// Story 2.2.9 precedence: a sidecar and the flag that disagree resolve to the less complete value with a class W
/// warning naming both; either alone wins; neither stated is `Full`.
pub fn resolve_kill_flag(
    sidecar: Option<&Sidecar>,
    flag: Option<KillCompletenessArg>,
) -> (KillCompleteness, Vec<Warning>) {
    sidecar::resolve_kill_completeness(sidecar, flag.map(KillCompletenessArg::into_domain))
}

/// Runs a `kibitzer tests` subcommand. Exit codes: 0 report produced, 2 input error (class E), 1 internal error.
pub fn run(command: TestsCommand) -> anyhow::Result<ExitCode> {
    match command {
        TestsCommand::Report(args) => Ok(run_report(&args)),
    }
}

const RECIPES_HINT: &str = "recipes: docs/test-effectiveness-report.md";
const EXIT_INPUT_ERROR: u8 = 2;
const EXIT_INTERNAL: u8 = 1;
const STORY_BRIEF: &str = "2.3.4";

/// Largest unit (decimal) that keeps the number at or above one, e.g. `3.2 GB`.
fn human_bytes(n: u64) -> String {
    const UNITS: [(&str, f64); 3] = [("GB", 1e9), ("MB", 1e6), ("KB", 1e3)];
    UNITS
        .iter()
        .find(|(_, scale)| n as f64 >= *scale)
        .map_or_else(
            || format!("{n} B"),
            |(unit, scale)| format!("{:.1} {unit}", n as f64 / scale),
        )
}

/// Any adapter error becomes class E: R26 for an input over the cap, `input-unreadable` for the rest. Every
/// input-derived string goes through the shared escape.
fn input_error(
    flag: &str,
    path: &Path,
    err: &dyn std::error::Error,
    guard: Option<&GuardError>,
) -> ClassE {
    let shown = escape_path(&path.display().to_string());
    if let Some(GuardError::TooLarge(big)) = guard {
        return ClassE {
            id: "input-over-size-cap",
            message: format!(
                "{shown} is {}, over the {} input cap (--max-input-bytes).",
                human_bytes(big.size),
                human_bytes(big.cap)
            ),
            hint: Some(
                "raise --max-input-bytes or split the input; see docs/test-effectiveness-report.md"
                    .to_string(),
            ),
        };
    }
    ClassE {
        id: "input-unreadable",
        message: format!("{flag} {shown}: {}", escape_path(&err.to_string())),
        hint: Some(format!(
            "check that the file is the report {flag} expects; {RECIPES_HINT}"
        )),
    }
}

fn plain_error(id: &'static str, message: String, hint: Option<&str>) -> ClassE {
    ClassE {
        id,
        message,
        hint: hint.map(str::to_owned),
    }
}

/// Class E output: nothing on stdout in text; the error object on stdout for json and brief; `error:` on stderr.
fn fail(format: Option<Format>, e: &ClassE) -> ExitCode {
    let mut err = io::stderr().lock();
    let _ = writeln!(err, "error: {}", e.message);
    if let Some(hint) = &e.hint {
        let _ = writeln!(err, "  {hint}");
    }
    if let Some(f @ (Format::Json | Format::Brief)) = format {
        let _ = io::stdout()
            .lock()
            .write_all(json::error_doc(f, e).as_bytes());
    }
    ExitCode::from(EXIT_INPUT_ERROR)
}

fn internal_error(what: &str) -> ExitCode {
    let _ = writeln!(io::stderr(), "error: internal: {what}");
    ExitCode::from(EXIT_INTERNAL)
}

fn has_no_input(a: &ReportArgs) -> bool {
    a.coveragepy.is_none() && a.go_profiles.is_none() && a.stryker.is_none() && a.pit.is_none()
}

/// Applies `--input-commit` where the input carried none, and drops the matching unknown-commit warning.
fn fill_commit<U, M>(d: &mut Dimension<U, M>, commit: Option<&str>) {
    if let (None, Some(c)) = (&d.provenance.commit, commit) {
        d.provenance.commit = Some(c.to_owned());
        d.warnings.retain(|w| w.id != WARN_INPUT_COMMIT_UNKNOWN);
    }
}

fn read_sidecar_for(path: &Path, max: u64) -> Result<Option<SidecarRead>, ClassE> {
    let side = sidecar::sidecar_path_for(path);
    if !side.is_file() {
        return Ok(None);
    }
    sidecar::read_sidecar(&side, max).map(Some).map_err(|e| {
        let guard = match &e {
            sidecar::SidecarError::Guard(g) => Some(g),
            _ => None,
        };
        input_error("--sidecar", &side, &e, guard)
    })
}

fn coverage_header(flag: &'static str, d: &CoverageDimension) -> HeaderInput {
    HeaderInput {
        flag,
        tool: d.provenance.tool.clone(),
        tool_version: d.provenance.version.clone(),
        commit: d.provenance.commit.clone(),
        run: d.provenance.run_timestamp.clone(),
        attribution: Some(d.meta.label()),
        attribution_assumed: d.provenance.attribution.is_some_and(|a| a.assumed),
        kill_completeness: None,
        mutation_run: None,
    }
}

fn kills_header(flag: &'static str, d: &KillDimension, run: MutationRun) -> HeaderInput {
    HeaderInput {
        flag,
        tool: d.provenance.tool.clone(),
        tool_version: d.provenance.version.clone(),
        commit: d.provenance.commit.clone(),
        run: d.provenance.run_timestamp.clone(),
        attribution: None,
        attribution_assumed: false,
        kill_completeness: Some(d.meta.label()),
        mutation_run: Some(run),
    }
}

fn push_input(loaded: &mut Loaded, header: HeaderInput, path: &Path) -> usize {
    loaded.inputs.push(InputInfo {
        header,
        path: path.to_owned(),
    });
    loaded.inputs.len() - 1
}

fn load_coverage(a: &ReportArgs, loaded: &mut Loaded) -> Result<(), ClassE> {
    let max = a.max_input_bytes;
    if let Some(path) = &a.coveragepy {
        let flag = "--coveragepy";
        let side = read_sidecar_for(path, max)?;
        let mut d = coveragepy::read_coveragepy(
            path,
            max,
            a.attribution.map(AttributionArg::into_domain),
            side.as_ref(),
        )
        .map_err(|e| {
            let g = match &e {
                coveragepy::CoveragePyError::Guard(g) => Some(g),
                _ => None,
            };
            input_error(flag, path, &e, g)
        })?;
        fill_commit(&mut d, a.input_commit.as_deref());
        loaded.coverage_input = Some(push_input(loaded, coverage_header(flag, &d), path));
        loaded.matrix.coverage = Some(d);
    }
    if let Some(dir) = &a.go_profiles {
        let flag = "--go-profiles";
        let mut read = gocover::parse_dir(dir, max).map_err(|e| {
            let g = match &e {
                gocover::GoCoverError::Guard(g) => Some(g),
                _ => None,
            };
            input_error(flag, dir, &e, g)
        })?;
        fill_commit(&mut read.dimension, a.input_commit.as_deref());
        loaded.failing.extend(read.failing);
        loaded.go_tests.extend(read.dimension.tests.iter().cloned());
        loaded.coverage_input = Some(push_input(
            loaded,
            coverage_header(flag, &read.dimension),
            dir,
        ));
        loaded.matrix.coverage = Some(read.dimension);
    }
    Ok(())
}

fn load_kills(a: &ReportArgs, loaded: &mut Loaded) -> Result<(), ClassE> {
    let max = a.max_input_bytes;
    let flag_value = a.kill_completeness.map(KillCompletenessArg::into_domain);
    if let Some(path) = &a.stryker {
        let flag = "--stryker";
        let side = read_sidecar_for(path, max)?;
        let mut report = mutation_elements::parse_report(
            path,
            max,
            side.as_ref().map(|s| &s.sidecar),
            flag_value,
        )
        .map_err(|e| {
            let g = match &e {
                mutation_elements::MutationElementsError::Guard(g) => Some(g),
                _ => None,
            };
            input_error(flag, path, &e, g)
        })?;
        fill_commit(&mut report.kills, a.input_commit.as_deref());
        let header = kills_header(flag, &report.kills, report.mutation_run.clone());
        loaded.kills_input = Some(push_input(loaded, header, path));
        loaded.matrix.kills = Some(report.kills);
    }
    if let Some(path) = &a.pit {
        let flag = "--pit";
        let opts = PitOptions {
            max_input_bytes: max,
            kill_completeness: flag_value,
            commit: a.input_commit.clone(),
            ..PitOptions::default()
        };
        let mut report = pit_xml::parse_file(path, &opts).map_err(|e| {
            let g = match &e {
                pit_xml::PitError::Guard(g) => Some(g),
                _ => None,
            };
            input_error(flag, path, &e, g)
        })?;
        let header_kills = report
            .matrix
            .kills
            .take()
            .expect("a PIT report always has a kill dimension");
        let header = kills_header(flag, &header_kills, report.mutation_run.clone());
        let index = push_input(loaded, header, path);
        loaded.kills_input = Some(index);
        loaded.matrix.kills = Some(header_kills);
        if let (None, Some(site_coverage)) =
            (&loaded.matrix.coverage, report.matrix.coverage.take())
        {
            loaded.coverage_input = Some(index);
            loaded.matrix.coverage = Some(site_coverage);
        }
    }
    Ok(())
}

fn load(a: &ReportArgs) -> Result<Loaded, ClassE> {
    if a.coveragepy.is_some() && a.go_profiles.is_some() {
        return Err(plain_error(
            "conflicting-inputs",
            "--coveragepy and --go-profiles both supply coverage; pass one per run.".to_string(),
            Some("run once per language"),
        ));
    }
    if a.stryker.is_some() && a.pit.is_some() {
        return Err(plain_error(
            "conflicting-inputs",
            "--stryker and --pit both supply mutation kills; pass one per run.".to_string(),
            Some("run once per mutation report"),
        ));
    }
    let mut loaded = Loaded::default();
    load_coverage(a, &mut loaded)?;
    load_kills(a, &mut loaded)?;
    Ok(loaded)
}

/// `(line, kind)` of each test function by name, per source file, parsed once.
struct KindIndex {
    root: PathBuf,
    parsed: HashMap<String, Vec<(usize, TestKind)>>,
}

impl KindIndex {
    fn kind_at(&mut self, file: &str, line: usize) -> TestKind {
        let root = &self.root;
        let fns = self.parsed.entry(file.to_owned()).or_insert_with(|| {
            let path = root.join(file);
            let Some(profile) = TestCodeProfile::for_path(&path) else {
                return Vec::new();
            };
            let Ok(source) = std::fs::read_to_string(&path) else {
                return Vec::new();
            };
            let Ok(tree) = GrammarCache::new().parse(profile.language, &source) else {
                return Vec::new();
            };
            let ctx = CheckContext {
                source: &source,
                tree: Some(&tree),
            };
            profile
                .test_functions(&ctx)
                .into_iter()
                .map(|f| (f.line, f.kind))
                .collect()
        });
        fns.iter()
            .find(|(l, _)| *l == line)
            .map_or(TestKind::Unit, |(_, k)| *k)
    }
}

/// Resolves every test of the input universe against the tree under `root`; tests that do not resolve to one
/// file and line are left out and become `no-source` exclusions.
fn locate_all(loaded: &Loaded, root: &Path) -> std::io::Result<BTreeMap<TestId, Located>> {
    let mut locator = TestLocator::new(root)?;
    let mut kinds = KindIndex {
        root: root.canonicalize()?,
        parsed: HashMap::new(),
    };
    let universe: BTreeSet<&TestId> = loaded
        .matrix
        .coverage
        .iter()
        .flat_map(|d| d.tests.iter())
        .chain(loaded.matrix.kills.iter().flat_map(|d| d.tests.iter()))
        .collect();
    let mut out = BTreeMap::new();
    for t in universe {
        let raw = if loaded.go_tests.contains(t) {
            RawTestId::Go {
                package: Some(t.file.clone()),
                name: t.name.clone(),
            }
        } else {
            RawTestId::FileName {
                file: t.file.clone(),
                name: t.name.clone(),
            }
        };
        if let Location::Located { file, line } = locator.locate(&raw) {
            let kind = kinds.kind_at(&file, line);
            out.insert(t.clone(), Located { file, line, kind });
        }
    }
    Ok(out)
}

fn write_stdout(text: &str) -> std::io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.flush()
}

fn verbose_lines(report: &Report, loaded: &Loaded) {
    let mut err = io::stderr().lock();
    for input in &loaded.inputs {
        let _ = writeln!(
            err,
            "[tests] input {}: {}",
            input.header.flag,
            escape_path(&input.path.display().to_string())
        );
    }
    for (reason, n) in report
        .envelope
        .header
        .scope
        .by_reason
        .iter()
        .filter(|(_, n)| *n > 0)
    {
        let _ = writeln!(err, "[tests] excluded: {}={n}", Scope::reason_key(*reason));
    }
}

fn run_report(a: &ReportArgs) -> ExitCode {
    let Some(format) = Format::parse(&a.format) else {
        return fail(
            None,
            &plain_error(
                "unknown-format",
                format!(
                    "unknown --format \"{}\"; expected text, json, or brief.",
                    escape_path(&a.format)
                ),
                Some("valid values: text, json, brief"),
            ),
        );
    };
    let Ok(top) = a.top.parse::<usize>() else {
        return fail(
            Some(format),
            &plain_error(
                "bad-top",
                format!(
                    "--top expects a non-negative integer; got \"{}\".",
                    escape_path(&a.top)
                ),
                Some("--top 0 means all"),
            ),
        );
    };
    if has_no_input(a) {
        return fail(
            Some(format),
            &plain_error(
                "no-input",
                "no input given. Supply at least one of --coveragepy, --go-profiles, --lcov, --stryker, --pit."
                    .to_string(),
                Some(RECIPES_HINT),
            ),
        );
    }
    let mut loaded = match load(a) {
        Ok(l) => l,
        Err(e) => return fail(Some(format), &e),
    };
    if format == Format::Brief {
        loaded
            .extra_warnings
            .push(envelope::gated_flag_warning("--format brief", STORY_BRIEF));
    }
    let root = match std::env::current_dir() {
        Ok(r) => r,
        Err(e) => return internal_error(&format!("cannot read the working directory: {e}")),
    };
    let locations = match locate_all(&loaded, &root) {
        Ok(l) => l,
        Err(e) => return internal_error(&format!("cannot scan {}: {e}", root.display())),
    };
    let opts = AnalyzeOptions {
        format,
        experimental: a.experimental,
        verdicts: G2_VERDICTS,
    };
    let report = report::analyze(&loaded, &locations, &opts);
    if a.verbose {
        verbose_lines(&report, &loaded);
    }
    if let Some(path) = &a.emit_experiment_input {
        let inputs: Vec<(&'static str, PathBuf)> = loaded
            .inputs
            .iter()
            .map(|i| (i.header.flag, i.path.clone()))
            .collect();
        if let Err(e) = export::write(path, &report, &loaded, &inputs) {
            return internal_error(&format!("cannot write the experiment input: {e}"));
        }
    }
    let body = match format {
        Format::Text => text::render(&report, top),
        Format::Json | Format::Brief => {
            let mut err = io::stderr().lock();
            for w in &report.envelope.warnings {
                let _ = writeln!(err, "{}", w.line());
            }
            json::render(&report)
        }
    };
    match write_stdout(&body) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => internal_error(&format!("cannot write the report to stdout: {e}")),
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn sidecar_saying(completeness: &str) -> Sidecar {
        let body = match completeness {
            "full" => {
                r#"{"schema_version":1,"tool":"stryker","tool_version":"8","commit":"abc1234","mutants_total":10,"mutants_run":10}"#
            }
            _ => {
                r#"{"schema_version":1,"tool":"stryker","tool_version":"8","commit":"abc1234","mutants_total":10,"mutants_run":3}"#
            }
        };
        sidecar::parse_sidecar(body.as_bytes()).unwrap().sidecar
    }

    #[test]
    fn kill_completeness_flag_should_ResolveToLessCompleteValue_When_SidecarAndFlagDisagree() {
        // Given a sidecar saying full and the flag saying sampled
        let side = sidecar_saying("full");
        // When resolved
        let (value, warnings) = resolve_kill_flag(Some(&side), Some(KillCompletenessArg::Sampled));
        // Then the less complete value wins and a warning names both
        assert_eq!(value, KillCompleteness::Sampled);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].id, sidecar::WARN_KILL_COMPLETENESS_CONFLICT);
        let params: Vec<_> = warnings[0].params.iter().map(|(_, v)| v.as_str()).collect();
        assert_eq!(params, ["full", "sampled"]);
    }

    #[test]
    fn kill_completeness_flag_should_YieldToSidecar_When_FlagIsMoreComplete() {
        // Given a sidecar saying sampled and the flag saying full
        let side = sidecar_saying("sampled");
        let (value, warnings) = resolve_kill_flag(Some(&side), Some(KillCompletenessArg::Full));
        // Then the less complete sidecar value wins, still with the warning
        assert_eq!(value, KillCompleteness::Sampled);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn kill_completeness_flag_should_StandAlone_When_NoSidecar() {
        let (value, warnings) = resolve_kill_flag(None, Some(KillCompletenessArg::FirstKiller));
        assert_eq!(value, KillCompleteness::FirstKillerOnly);
        assert!(warnings.is_empty());
        let (value, _) = resolve_kill_flag(None, None);
        assert_eq!(value, KillCompleteness::Full);
    }

    #[test]
    fn input_error_should_EscapeControlCharacters_And_UseR26Id_When_OverCap() {
        let io = std::io::Error::other("boom\u{1b}[0m\nforged: line");
        let generic = input_error("--pit", Path::new("a\u{1b}b.xml"), &io, None);
        assert_eq!(generic.id, "input-unreadable");
        assert!(!generic.message.contains('\u{1b}') && !generic.message.contains('\n'));
        let big = GuardError::TooLarge(super::super::adapters::guard::InputTooLarge {
            file: PathBuf::from("x.json"),
            size: 3_200_000_000,
            cap: 1_000_000_000,
        });
        let r26 = input_error("--stryker", Path::new("x.json"), &io, Some(&big));
        assert_eq!(r26.id, "input-over-size-cap");
        assert_eq!(
            r26.message,
            "x.json is 3.2 GB, over the 1.0 GB input cap (--max-input-bytes)."
        );
    }

    #[test]
    fn human_bytes_should_PickLargestUnit() {
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500), "1.5 KB");
        assert_eq!(human_bytes(2_000_000), "2.0 MB");
    }

    #[test]
    fn flag_spellings_should_MapToDomainValues() {
        assert_eq!(
            KillCompletenessArg::BudgetLimited.into_domain(),
            KillCompleteness::BudgetLimited
        );
        assert_eq!(
            AttributionArg::LeaveOneOut.into_domain(),
            CoverageAttribution::LeaveOneOut
        );
        assert_eq!(AttributionArg::InOrder.into_domain().label(), "in-order");
    }
}
