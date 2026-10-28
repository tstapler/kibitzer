# Label: true negative. Mock-verification is the oracle.


def test_mock_verify(repo):
    service(repo).create("x")
    repo.save.assert_called_once()


def test_mock_alias():
    m = make_mock()
    run(m)
    m.assert_called_with("x")
