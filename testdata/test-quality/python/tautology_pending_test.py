# Label: test-tautology true positives (Story 1.1.3, not built yet). From
# test-no-assertion's view each test has an assertion, so zero findings.


def test_literal_true():
    assert True


def test_self_compare():
    got = parse("")
    assert got == got
