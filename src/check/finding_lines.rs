//! Rendered native-check output lines, each remembering the finding it came from, so
//! diff-scoping and `accepted/` filtering decide per finding and the surviving anchors are
//! read back by identity instead of by matching rendered text.

use std::path::Path;

use crate::checker::Finding;

pub(super) struct FindingLines {
    lines: Vec<OwnedLine>,
    finding_count: usize,
    filtered: bool,
}

struct OwnedLine {
    text: String,
    finding: usize,
    /// First line of its finding's rendering; a multi-line message continues on later lines.
    first: bool,
}

impl FindingLines {
    /// Renders each finding and splits its own text into lines, tagged by finding index.
    /// `text()` equals the findings joined with `\n` and re-split like `str::lines`.
    pub(super) fn new(findings: &[Finding], file_path: &Path) -> Self {
        let mut lines: Vec<OwnedLine> = Vec::new();
        for (index, finding) in findings.iter().enumerate() {
            let rendered = super::render_finding(file_path, finding);
            for (n, piece) in rendered.split('\n').enumerate() {
                lines.push(OwnedLine {
                    text: piece.strip_suffix('\r').unwrap_or(piece).to_string(),
                    finding: index,
                    first: n == 0,
                });
            }
        }
        // `str::lines` drops the empty piece after a final newline.
        if lines.last().is_some_and(|l| l.text.is_empty() && !l.first) {
            lines.pop();
        }
        FindingLines {
            lines,
            finding_count: findings.len(),
            filtered: false,
        }
    }

    pub(super) fn texts(&self) -> Vec<&str> {
        self.lines.iter().map(|l| l.text.as_str()).collect()
    }

    /// Keeps the lines whose flag is true; `keep` is parallel to `texts()`.
    pub(super) fn retain(&mut self, keep: &[bool]) {
        let mut flags = keep.iter();
        self.lines.retain(|_| flags.next().copied().unwrap_or(true));
        self.filtered |= keep.contains(&false);
    }

    /// Whether any `retain` dropped a line, so `text()` can differ from the original rendering.
    pub(super) fn is_filtered(&self) -> bool {
        self.filtered
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
        let lines = FindingLines::new(findings, file);
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

    const RUST_CHECKER: &str = "syntax-rules-rust";

    /// Two string literals, each repeated three times, each with a newline inside: two
    /// `replace-magic-literal` findings whose messages span lines.
    fn multiline_literals_source() -> String {
        let body = |lit: &str| format!("fn f() {{\n    let x = {lit};\n}}\n");
        let (lit1, lit2) = ("\"one\ntwo words\"", "\"three\nfour words\"");
        [lit1, lit1, lit1, lit2, lit2, lit2]
            .iter()
            .map(|lit| body(lit))
            .collect()
    }

    fn rust_check() -> crate::config::Check {
        crate::config::Check {
            name: RUST_CHECKER.to_string(),
            command: None,
            checker: Some(RUST_CHECKER.to_string()),
            architecture_checker: None,
            severity: crate::config::Severity::Advisory,
            scope: vec![],
            triggers: vec![],
            message: None,
            output_format: None,
            options: None,
        }
    }

    /// Output must equal what text-level scoping produces, even when a message spans lines.
    #[test]
    fn run_native_check_should_MatchTextScoping_When_MessageSpansLines() {
        use crate::check::{
            NativeRun, run_checker_against_file, run_native_check, scope_output_to_changed_lines,
        };
        use crate::config::Severity;
        use crate::run_context::RunContext;
        let dir = std::env::temp_dir().join(format!("kibitzer-multiline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("main.rs");
        std::fs::write(&file, multiline_literals_source()).unwrap();
        let ctx = crate::inline_ignores::InlineIgnoreContext::default();
        let run = NativeRun {
            checker_name: RUST_CHECKER,
            options: None,
            severity: Severity::Advisory,
            inline_ctx: &ctx,
        };
        let raw = run_checker_against_file(run, &file).unwrap();
        assert!(raw.combined.contains('\n'), "{:?}", raw.combined);
        let ranges = [(14usize, 14usize)];
        let result = run_native_check(
            &rust_check(),
            RUST_CHECKER,
            &dir,
            &file,
            Some(&ranges),
            &RunContext::default(),
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
