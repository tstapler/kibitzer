# Label: helper resolution is bounded: test_shallow resolves, test_too_deep does not.


def h1():
    h2()


def h2():
    assert ready()


def test_shallow():
    h1()


def d1():
    d2()


def d2():
    d3()


def d3():
    d4()


def d4():
    assert ready()


def test_too_deep():
    d1()


def loop():
    loop()


def test_recursive():
    loop()
