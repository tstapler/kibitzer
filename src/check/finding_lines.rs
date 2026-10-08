//! Rendered native-check output lines, each remembering the finding it came from, so
//! diff-scoping and `accepted/` filtering decide per finding and the surviving anchors are
//! read back by identity instead of by matching rendered text.

use std::path::Path;

use crate::checker::Finding;

pub(super) struct FindingLines {
    lines: Vec<OwnedLine>,
    finding_count: usize,
}

struct OwnedLine {
    text: String,
    finding: usize,
    /// First line of its finding's rendering; a multi-line message continues on later lines.
    first: bool,
}

impl FindingLines {
    /// Splits `combined` (the findings rendered and joined with `\n`) the way `str::lines`
    /// does, tagging each line by the byte span of the finding that produced it.
    pub(super) fn new(findings: &[Finding], file_path: &Path, combined: &str) -> Self {
        let mut starts = Vec::with_capacity(findings.len());
        let mut offset = 0;
        for finding in findings {
            starts.push(offset);
            offset += super::render_finding(file_path, finding).len() + 1;
        }
        let base = combined.as_ptr() as usize;
        let lines = combined
            .lines()
            .map(|line| {
                let at = line.as_ptr() as usize - base;
                let finding = starts.partition_point(|&s| s <= at).saturating_sub(1);
                OwnedLine {
                    text: line.to_string(),
                    finding,
                    first: starts.get(finding) == Some(&at),
                }
            })
            .collect();
        FindingLines {
            lines,
            finding_count: findings.len(),
        }
    }

    pub(super) fn texts(&self) -> Vec<&str> {
        self.lines.iter().map(|l| l.text.as_str()).collect()
    }

    /// Keeps the lines whose flag is true; `keep` is parallel to `texts()`.
    pub(super) fn retain(&mut self, keep: &[bool]) {
        let mut flags = keep.iter();
        self.lines.retain(|_| flags.next().copied().unwrap_or(true));
    }

    pub(super) fn text(&self) -> String {
        self.texts().join("\n")
    }

    /// Per finding: whether its first rendered line is still present.
    pub(super) fn visible(&self) -> Vec<bool> {
        let mut visible = vec![false; self.finding_count];
        for line in self.lines.iter().filter(|l| l.first) {
            visible[line.finding] = true;
        }
        visible
    }
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;

    fn finding(line: usize, message: &str) -> Finding {
        Finding {
            line,
            message: message.to_string(),
        }
    }

    fn lines_of(findings: &[Finding]) -> (String, FindingLines) {
        let file = Path::new("f.rs");
        let combined = findings
            .iter()
            .map(|f| super::super::render_finding(file, f))
            .collect::<Vec<_>>()
            .join("\n");
        let lines = FindingLines::new(findings, file, &combined);
        (combined, lines)
    }

    #[test]
    fn text_should_RoundTripCombined_When_FindingsAreMultiLine() {
        let findings = [finding(1, "a\n  b"), finding(5, "c"), finding(9, "d\ne\nf")];
        let (combined, lines) = lines_of(&findings);
        assert_eq!(lines.text(), combined);
        assert_eq!(lines.visible(), vec![true, true, true]);
    }

    #[test]
    fn visible_should_DropFinding_When_OnlyContinuationLinesRemain() {
        let findings = [finding(1, "a\nb"), finding(5, "c")];
        let (_, mut lines) = lines_of(&findings);
        lines.retain(&[false, true, true]);
        assert_eq!(lines.visible(), vec![false, true]);
        assert_eq!(lines.text(), "b\nf.rs:5: c");
    }

    #[test]
    fn visible_should_TrackEachFinding_When_RenderedLinesAreIdentical() {
        let findings = [finding(1, "same"), finding(1, "same")];
        let (_, mut lines) = lines_of(&findings);
        lines.retain(&[false, true]);
        assert_eq!(lines.visible(), vec![false, true]);
    }

    #[test]
    fn new_should_AttributeBlankLine_When_MessageEndsWithNewline() {
        let findings = [finding(1, "a\n"), finding(2, "b")];
        let (combined, lines) = lines_of(&findings);
        assert_eq!(
            lines.text(),
            combined.lines().collect::<Vec<_>>().join("\n")
        );
        assert_eq!(lines.visible(), vec![true, true]);
    }

    /// A string literal repeated three times fires `replace-magic-literal` with a message
    /// that itself spans lines; output must equal what text-level scoping produces.
    #[test]
    fn run_native_check_should_MatchTextScoping_When_MessageSpansLines() {
        use crate::accepted_findings::AcceptedFindings;
        use crate::check::{
            NativeRun, run_checker_against_file, run_native_check, scope_output_to_changed_lines,
        };
        use crate::config::{Check, Severity};
        let dir = std::env::temp_dir().join(format!("kibitzer-multiline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        let (lit1, lit2) = ("\"one\ntwo words\"", "\"three\nfour words\"");
        let body = |lit: &str| format!("fn f() {{\n    let x = {lit};\n}}\n");
        let source = format!(
            "{}{}{}{}{}{}",
            body(lit1),
            body(lit1),
            body(lit1),
            body(lit2),
            body(lit2),
            body(lit2)
        );
        std::fs::write(&file, &source).unwrap();
        let ctx = crate::inline_ignores::InlineIgnoreContext::default();
        let run = NativeRun {
            checker_name: "syntax-rules-rust",
            options: None,
            severity: Severity::Advisory,
            inline_ctx: &ctx,
        };
        let raw = run_checker_against_file(run, &file).unwrap();
        assert!(raw.combined.contains('\n'), "{:?}", raw.combined);
        let check = Check {
            name: "syntax-rules-rust".to_string(),
            command: None,
            checker: Some("syntax-rules-rust".to_string()),
            architecture_checker: None,
            severity: Severity::Advisory,
            scope: vec![],
            triggers: vec![],
            message: None,
            output_format: None,
            options: None,
        };
        let ranges = [(14usize, 14usize)];
        let result = run_native_check(
            &check,
            "syntax-rules-rust",
            &dir,
            &file,
            Some(&ranges),
            &AcceptedFindings::default(),
        )
        .unwrap();
        let (expected, expected_passed) =
            scope_output_to_changed_lines(&raw.combined, &file, &ranges, raw.passed);
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(result.output, expected);
        assert_eq!(result.passed, expected_passed);
        assert_eq!(result.inline.shown.len(), 1);
        assert_eq!(result.inline.shown[0].line.get(), 14);
        assert_eq!(result.inline.kept.len(), 2);
    }
}
