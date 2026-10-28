# Label: pytester / LineMatcher methods are the oracle (pytest's own suite, real snippets).
class TestRunResult:
    def test_stderr_lines(self, pytester):
        result = pytester.runpytest("--override-ini", "xdist_strict", "True")
        result.stderr.fnmatch_lines(["ERROR: -o/--override-ini expects option=value style"])

    def test_negative_and_regex(self, pytester):
        result = pytester.runpytest()
        result.stdout.no_fnmatch_line("*FAILED*")
        result.stdout.re_match_lines([r"\d+ passed"])
        result.stdout.fnmatch_lines_random(["*a*", "*b*"])
        result.stdout.no_re_match_line("boom")
        result.stdout.re_match_lines_random([r"x"])

    def test_outcomes(self, pytester):
        pytester.runpytest().assert_outcomes(passed=1)

    def test_contains(self, pytester):
        pytester.runpytest().assert_contains("hello")

    def test_runs_only(self, pytester):
        pytester.runpytest()
