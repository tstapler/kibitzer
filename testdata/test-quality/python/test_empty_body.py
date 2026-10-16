# Label: true positives. Empty bodies (pass, docstring only).


def test_pass():
    pass


def test_docstring():
    """Placeholder."""


def test_real():
    assert parse("x") == 1
