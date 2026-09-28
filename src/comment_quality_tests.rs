use super::*;
use crate::checker::{GrammarCache, run_checker_with_cache};
use std::path::PathBuf;

fn run(lang: Language, source: &str) -> Vec<Finding> {
    let checker = CommentQualityChecker::new(lang);
    let cache = GrammarCache::new();
    run_checker_with_cache(&checker, &PathBuf::from("f"), source, &cache).unwrap()
}

fn assert_has_finding(findings: &[Finding], marker: &str) {
    assert!(
        findings.iter().any(|f| f.message.contains(marker)),
        "expected {marker} in findings: {findings:?}"
    );
}

fn assert_no_finding(findings: &[Finding], marker: &str) {
    assert!(
        !findings.iter().any(|f| f.message.contains(marker)),
        "unexpected {marker} in findings: {findings:?}"
    );
}

#[test]
fn flags_marketing_language() {
    let src = "package main\n\n// leverage this seamlessly\nfunc F() {}\n";
    let findings = run(Language::Go, src);
    assert!(
        findings
            .iter()
            .any(|f| f.message.contains(FINDING_VERBOSE_COMMENT) && f.message.contains("leverage"))
    );
}

#[test]
fn flags_wordy_filler_phrase() {
    let src = "package main\n\n// We check this in order to validate the input.\nfunc F() {}\n";
    let findings = run(Language::Go, src);
    assert!(
        findings
            .iter()
            .any(|f| f.message.contains(FINDING_VERBOSE_COMMENT)
                && f.message.contains("in order to")),
        "findings: {findings:?}"
    );
}

#[test]
fn flags_commented_out_code() {
    let src = "package main\n\nfunc F() {\n\t// x = doSomething(1, 2);\n}\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

/// Regression guard for docs/comment-quality-false-positives.md's first entry:
/// SonarQube's S125 has the same documented gap (a bare `ends_with(';')` treats any
/// semicolon-terminated clause as code), and a semicolon-separated bullet list is a
/// common real doc-comment style.
#[test]
fn does_not_flag_a_semicolon_terminated_bullet_list() {
    let src = "package main\n\n// Normalize does three things:\n// - validates input;\n// - normalizes casing;\n// - returns the result;\nfunc Normalize(s string) string {\n\treturn s\n}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

#[test]
fn does_not_flag_ordinary_prose_comment() {
    let src = "package main\n\n// Parse validates the input and returns an error if it's malformed.\nfunc Parse() {}\n";
    let findings = run(Language::Go, src);
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn flags_over_commented_function() {
    let src = "package main\n\n// This function adds two numbers together.\n// It takes a and b as parameters.\n// It returns the sum of a and b.\n// It never returns anything else.\n// It has no side effects.\n// There is nothing more to say about it.\nfunc Add(a, b int) int {\n\treturn a + b\n}\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn does_not_flag_proportionate_explanation_over_short_body() {
    let src = "package main\n\n// Retry calls fn up to attempts times, waiting delay between failures.\n// Returns the first successful result, or the last error if every attempt\n// fails — callers that need cancellation should wrap fn themselves, since\n// Retry does not accept a context.\nfunc Retry(fn func() (int, error)) (int, error) {\n\tresult, err := fn()\n\treturn result, err\n}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

/// A 7-line WHY comment over an 8-line body has ratio ~0.9 — well under
/// `COMMENT_TO_CODE_RATIO` — so `[over-commented]` correctly stays silent. This is
/// the real gap `[comment-too-long]` closes: a comment can be perfectly
/// "proportionate" to its function and still be too long on its own terms.
#[test]
fn flags_absolute_length_even_when_proportionate() {
    let src = "package main\n\n// ProcessBatch runs validation before writing, because the legacy importer\n// upstream sometimes emits rows with a trailing null byte that corrupts the\n// downstream parser if written as-is. We considered stripping it at the\n// source instead, but that importer is owned by another team and a fix\n// there would take a full quarter to land, so this is the accepted\n// workaround until that migration completes.\nfunc ProcessBatch(rows []string) []string {\n\tout := make([]string, 0, len(rows))\n\tfor _, r := range rows {\n\t\tif r == \"\" {\n\t\t\tcontinue\n\t\t}\n\t\tout = append(out, r)\n\t}\n\treturn out\n}\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, FINDING_COMMENT_TOO_LONG);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn does_not_flag_a_five_line_comment() {
    let src = "package main\n\n// Retry calls fn up to attempts times, waiting delay between failures.\n// Returns the first successful result, or the last error if every attempt\n// fails — callers that need cancellation should wrap fn themselves, since\n// Retry does not accept a context, delay is not jittered, and there is no\n// backoff between attempts.\nfunc Retry(fn func() (int, error)) (int, error) {\n\tresult, err := fn()\n\treturn result, err\n}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_COMMENT_TOO_LONG);
}

#[test]
fn rust_flags_marketing_language() {
    let src = "/// This seamlessly leverages a robust approach.\nfn f() {}\n";
    let findings = run(Language::Rust, src);
    assert!(
        findings
            .iter()
            .any(|f| f.message.contains(FINDING_VERBOSE_COMMENT) && f.message.contains("leverage")),
        "findings: {findings:?}"
    );
}

#[test]
fn rust_flags_commented_out_code() {
    let src = "fn f() {\n    // x = do_something(1, 2);\n}\n";
    let findings = run(Language::Rust, src);
    assert_has_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

#[test]
fn rust_does_not_flag_ordinary_prose_comment() {
    let src = "/// Parses the input and returns an error if it's malformed.\nfn parse() {}\n";
    let findings = run(Language::Rust, src);
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn rust_flags_over_commented_function() {
    let src = "/// This function adds two numbers together.\n/// It takes a and b as parameters.\n/// It returns the sum of a and b.\n/// It never returns anything else.\n/// It has no side effects.\n/// There is nothing more to say about it.\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let findings = run(Language::Rust, src);
    assert_has_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn rust_checks_impl_methods_too() {
    let src =
        "struct S;\nimpl S {\n    /// This seamlessly does the thing.\n    fn m(&self) {}\n}\n";
    let findings = run(Language::Rust, src);
    assert_has_finding(&findings, FINDING_VERBOSE_COMMENT);
}

// --- Regression tests from the 2026-09-06 Kubernetes/Cassandra/Servo backtest ---

#[test]
fn short_banned_phrase_does_not_match_inside_an_unrelated_longer_word() {
    let src = "// This prevents the race and properly handles the process.\nfunc f() {}\n";
    let findings = run(Language::Go, src);
    assert!(
        !findings.iter().any(|f| f.message.contains("\"this pr\"")),
        "findings: {findings:?}"
    );
}

#[test]
fn short_banned_phrase_still_matches_as_its_own_word() {
    let src = "// For purpose of this PR, report only the failure count.\nfunc f() {}\n";
    let findings = run(Language::Go, src);
    assert!(
        findings.iter().any(|f| f.message.contains("\"this pr\"")),
        "findings: {findings:?}"
    );
}

#[test]
fn license_header_style_comment_is_not_flagged_as_commented_out_code() {
    let src = "// Licensed under the Apache License, Version 2.0 (the \"License\");\npackage main\n\nfunc f() {}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

#[test]
fn spec_quoting_blockquote_ending_in_semicolon_is_not_flagged_as_commented_out_code() {
    let src = "// > or if the command is the fontSize command;\nfunc f() {}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

#[test]
fn narrative_arithmetic_comment_is_not_flagged_as_commented_out_code() {
    let src = "// FQDN=15 + 1(dot) + 55 = 71 chars\nfunc f() {}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

#[test]
fn fenced_doc_comment_code_example_is_not_flagged_as_commented_out_code() {
    let src = "/// Example:\n///\n/// ```\n/// let x = f(1, 2);\n/// ```\nfunc f() {}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_COMMENTED_OUT_CODE);
}

#[test]
fn adjacent_commented_out_stub_is_not_folded_into_the_next_functions_ratio() {
    // The two `//`-commented function stubs immediately above `Real` look like
    // dead code, not `Real`'s own leading doc comment — they must not inflate
    // `Real`'s comment-to-code ratio (a real backtest finding against Servo).
    let src = "// fn Dead1() { return 1; }\n// fn Dead2() { return 2; }\nfunc Real() int {\n\treturn 3\n}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn rust_safety_doc_section_is_exempt_from_over_commented() {
    let src = "/// Derefs a raw pointer.\n///\n/// # Safety\n///\n/// The caller must ensure the pointer is non-null, properly aligned, and\n/// points to a live, initialized value of type `T` for the duration of the\n/// borrow — violating any of these is immediate undefined behavior.\npub unsafe fn deref<T>(p: *const T) -> &'static T {\n    &*p\n}\n";
    let findings = run(Language::Rust, src);
    assert_no_finding(&findings, "[over-commented]");
    // This comment is 7 lines — past MAX_LEADING_COMMENT_LINES (5) — so it also
    // proves the `# Safety` exemption covers `[comment-too-long]`, not just the ratio.
    assert_no_finding(&findings, "[comment-too-long]");
}

/// The delegating-single-statement-body exemption only excuses the *ratio* check
/// (`[over-commented]`) — its rationale is the near-zero denominator for a one-line
/// body, which says nothing about absolute comment length. A long comment over a
/// delegating body must still trigger `[comment-too-long]`.
#[test]
fn delegating_body_does_not_exempt_comment_too_long() {
    let src = "// LimitWriter is a copy of the standard library ioutils.LimitReader,\n// applied to the writer interface. LimitWriter returns a Writer that\n// writes to w but stops with EOF after n bytes. The underlying\n// implementation is a *LimitedWriter, which tracks remaining capacity\n// and returns io.EOF once that capacity is exhausted, matching the\n// semantics callers already expect from LimitReader on the read side.\nfunc LimitWriter(w Writer, n int64) Writer { return &LimitedWriter{w, n} }\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, "[over-commented]");
    assert_has_finding(&findings, "[comment-too-long]");
}

#[test]
fn flags_a_six_line_comment() {
    let src = "package main\n\n// Retry calls fn up to attempts times, waiting delay between failures.\n// Returns the first successful result, or the last error if every attempt\n// fails — callers that need cancellation should wrap fn themselves, since\n// Retry does not accept a context, delay is not jittered, there is no\n// backoff between attempts, and errors are not wrapped with attempt\n// count context for callers that want to log it.\nfunc Retry(fn func() (int, error)) (int, error) {\n\tresult, err := fn()\n\treturn result, err\n}\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, "[comment-too-long]");
}

// --- Regression tests for the two 2026-09-06 backtest-informed enforcements:
// delegating-single-statement-body exemption, and parameter-count-scaled ratio ---

#[test]
fn delegating_body_with_struct_construction_is_exempt_from_over_commented() {
    // Mirrors kubernetes/kubernetes's pkg/kubelet/util/ioutils/ioutils.go
    // LimitWriter almost verbatim.
    let src = "// LimitWriter is a copy of the standard library ioutils.LimitReader,\n// applied to the writer interface.\n// LimitWriter returns a Writer that writes to w\n// but stops with EOF after n bytes.\n// The underlying implementation is a *LimitedWriter.\nfunc LimitWriter(w Writer, n int64) Writer { return &LimitedWriter{w, n} }\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn delegating_body_with_qualified_multi_arg_call_is_exempt_from_over_commented() {
    // Mirrors apache/cassandra's ColumnFamilyStore.sstablesRewrite: several
    // opaque boolean/numeric parameters, a thorough per-parameter explanation,
    // over a body that's one delegating call — spread across multiple lines
    // (unlike the crammed-one-line test above) to prove the exemption is
    // AST-based (named-child count), not line-count-based.
    let src = "// Rewrite rewrites all SSTables according to specified parameters.\n//\n// skipIfCurrentVersion, if true, rewrites only SSTables older than current.\n// skipIfNewerThanTimestamp excludes SSTables created after this timestamp.\n// skipIfCompressionMatches, if true, rewrites only SSTables whose compression differs.\nfunc Rewrite(skipIfCurrentVersion bool, skipIfNewerThanTimestamp int64, skipIfCompressionMatches bool, jobs int) error {\n\treturn other.PerformRewrite(skipIfCurrentVersion, skipIfNewerThanTimestamp, skipIfCompressionMatches, jobs)\n}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn rust_delegating_method_chain_is_exempt_from_over_commented() {
    // Mirrors servo/servo's ServoLayoutNode::dangerous_first_child: a method
    // chain (not a bare call or brace construction) as the sole statement.
    let src = "/// Get the first child of this node.\n///\n/// This node should never be exposed directly to the layout interface, as\n/// that may allow mutating a node that is being laid out on another thread.\npub(super) unsafe fn dangerous_first_child(&self) -> Option<Self> {\n    self.node.first_child_ref().map(Into::into)\n}\n";
    let findings = run(Language::Rust, src);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn multi_statement_body_is_not_exempt_even_when_the_last_statement_delegates() {
    // Mirrors apache/cassandra's ColumnFamilyStore.addSSTable: a precondition
    // check PLUS a delegating call is two statements, not one — the
    // single-statement gate must not treat this as "just a delegation" the way
    // it correctly does for a bare one-statement body.
    // Body is 4 lines (open-brace-with-signature, two statements, close brace),
    // so 8 comment lines are needed to clear the flat 2x ratio (8 >= 2.0*4).
    let src = "// Add validates and adds the given item to the store.\n// This should be called after ensuring the item's checksum matches, since\n// items with mismatched checksums silently corrupt the on-disk index and\n// there is no way to detect this after the fact — the corruption surfaces\n// only much later, in an unrelated request against unrelated data, by\n// which point the original cause is impossible to trace back.\n// This line and the next exist only to reach the required comment count.\n// Final padding line.\nfunc Add(item Item) {\n\tvalidate(item)\n\tstore.Add(item)\n}\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn higher_parameter_count_raises_the_over_commented_threshold() {
    // 6 parameters, a single non-delegating statement (plain arithmetic — no
    // call/construction shape, so the delegating-body exemption correctly
    // doesn't apply here). At the flat 2x ratio this would fire, since 7
    // comment lines over a 3-line body clears a 2x threshold of 6. Scaled for
    // 6 params (2 plus a 0.25 bonus per param over the baseline of 2, giving
    // 3x here) it must not, since 7 no longer clears a 3x threshold of 9.
    // Body is 3 lines (open-brace-with-signature, one return, close brace); 7
    // comment lines clear the flat 2x threshold of 6 but not the scaled 3x
    // threshold of 9.
    let src = "// f validates a, b, c, d, e, and g against their expected ranges before use,\n// since callers frequently pass swapped or stale values here and the\n// resulting corruption is silent until much later in an unrelated request.\n// This line and the next two exist only to reach the required comment count.\n// Padding line two.\n// Padding line three.\n// Padding line four.\nfunc f(a, b, c, d, e, g int) int {\n\treturn a + b + c + d + e + g\n}\n";
    let findings = run(Language::Go, src);
    assert_no_finding(&findings, FINDING_OVER_COMMENTED);
}

#[test]
fn low_parameter_count_does_not_get_a_ratio_bonus() {
    // Same shape as the test above but with only 2 parameters (at the
    // baseline, so no bonus applies) and the same comment/body line counts —
    // this must still fire at the plain 2.0x ratio.
    let src = "// f validates a and b against their expected ranges before use,\n// since callers frequently pass swapped or stale values here and the\n// resulting corruption is silent until much later in an unrelated request.\n// This line and the next two exist only to reach the required comment count.\n// Padding line two.\n// Padding line three.\n// Padding line four.\nfunc f(a, b int) int {\n\treturn a + b + a + b + a + b\n}\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, FINDING_OVER_COMMENTED);
}

/// A one-line body sharing its line with an inline comment makes `body_code_lines`
/// compute to 0 (`saturating_sub` of equal totals) — that must not exempt
/// `[comment-too-long]`, whose whole point is being independent of body size.
#[test]
fn zero_body_code_lines_does_not_exempt_comment_too_long() {
    let src = "// F does something, for reasons that take more than five lines to explain.\n// Line two of the rationale.\n// Line three of the rationale.\n// Line four of the rationale.\n// Line five of the rationale.\n// Line six of the rationale, which pushes this over the ceiling.\nfunc F() { /* trivial */ }\n";
    let findings = run(Language::Go, src);
    assert_has_finding(&findings, FINDING_COMMENT_TOO_LONG);
}
