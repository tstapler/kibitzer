# Label: true negatives. Snapshot oracle, skipped tests, explicit no-raise smoke test.
import pytest


def test_snapshot(snapshot):
    assert_match_snapshot(render(), snapshot)


@pytest.mark.skip(reason="flaky")
def test_skipped():
    parse("")


def test_skip_inline():
    pytest.skip("later")
    parse("")


def test_smoke_does_not_raise():
    try:
        parse("")
    except Exception as exc:
        pytest.fail(f"raised {exc}")
