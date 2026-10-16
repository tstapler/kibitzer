# Label: true negative. Assertions in same-file helpers and closures (pitfall P5).


def check_ok(value):
    assert value


def test_helper():
    check_ok(parse(""))


def test_closure():
    def inner():
        assert parse("") == 1

    inner()


def test_then_callback():
    run(lambda result: check_ok(result))


class TestMethodHelper:
    def _verify(self, value):
        assert value

    def test_method_helper(self):
        self._verify(parse(""))
