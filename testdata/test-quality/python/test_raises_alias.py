# Label: `<x>.raises` / `<x>.warns` aliases (py.test.raises) are oracles, directly or via a helper.
import py


def parse_a_error(input):
    return py.test.raises(ValueError, parse, input)


def test_alias_via_helper():
    parse_a_error("!!")


def test_alias_direct():
    with py.test.raises(ValueError):
        parse("!!")


def test_warns_alias():
    with compat.pytest.warns(UserWarning):
        parse("!!")


def test_not_an_oracle():
    parse("!!")
