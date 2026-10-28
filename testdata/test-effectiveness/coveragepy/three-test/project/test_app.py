from app import classify


def test_pos():
    assert classify(1) == "pos"


def test_neg():
    assert classify(-1) == "neg"


def test_noassert():
    classify(2)
