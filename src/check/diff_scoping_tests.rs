use super::*;
use std::path::PathBuf;

fn file() -> PathBuf {
    PathBuf::from("src/foo.go")
}

#[test]
fn substitute_command_fills_changed_lines() {
    let cmd = substitute_command(
        "kibitzer check native primitive-obsession {file} --lines={changed_lines}",
        &file(),
        Some(&[(12, 15), (40, 40)]),
    );
    assert_eq!(
        cmd,
        "kibitzer check native primitive-obsession src/foo.go --lines=12-15,40-40"
    );
}

#[test]
fn substitute_command_empty_changed_lines_when_none() {
    let cmd = substitute_command("cmd {file} {changed_lines}", &file(), None);
    assert_eq!(cmd, "cmd src/foo.go ");
}

#[test]
fn scope_output_keeps_findings_inside_changed_ranges() {
    let output = "src/foo.go:5: unrelated finding\nsrc/foo.go:13: newtype me\n";
    let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
    assert!(!passed);
    assert_eq!(filtered, "src/foo.go:13: newtype me");
}

#[test]
fn scope_output_passes_when_all_findings_outside_changed_ranges() {
    let output = "src/foo.go:5: pre-existing finding\n";
    let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
    assert!(passed);
    assert_eq!(filtered, "");
}

#[test]
fn scope_output_leaves_unconventional_output_untouched() {
    let output = "some linter crashed with no file:line prefix\n";
    let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
    assert!(!passed);
    assert_eq!(filtered, output);
}

#[test]
fn scope_output_noop_when_already_passing() {
    let (filtered, passed) = scope_output_to_changed_lines("", &file(), &[(12, 15)], true);
    assert!(passed);
    assert_eq!(filtered, "");
}

#[test]
fn scope_output_empty_ranges_suppresses_all_findings() {
    // Regression for docs/go-primitive-obsession-false-positives.md's
    // "deletion-only edit flagged" entry: `changed_lines` present but empty
    // (a pure-deletion edit — see `hook::compute_changed_lines`) must suppress
    // every finding, not fall back to raw whole-file output.
    let output = "src/foo.go:5: pre-existing finding\n";
    let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[], false);
    assert!(passed);
    assert_eq!(filtered, "");
}

#[test]
fn scope_output_counts_malformed_prefixed_line_as_failure() {
    // Has the file prefix but the text after it isn't a line number — can't be
    // attributed to a range, so it must count toward failure, not just be displayed.
    let output = "src/foo.go:note: continued from previous finding\n";
    let (filtered, passed) = scope_output_to_changed_lines(output, &file(), &[(12, 15)], false);
    assert!(!passed);
    assert_eq!(filtered, output);
}
