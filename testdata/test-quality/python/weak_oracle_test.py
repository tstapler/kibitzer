# Label: known false negative: weak oracle. Zero findings from test-no-assertion
# and test-tautology; the gap is documented here, not silently widened.


def test_weak():
    assert parse("") is not None


def test_weak_len():
    assert len(parse("")) > 0


def test_weak_truthy():
    assert parse("")
