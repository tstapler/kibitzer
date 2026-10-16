# Label: true negative. Expected-exception oracles.
import unittest

import pytest


def test_raises():
    with pytest.raises(ValueError):
        parse("")


class TestRaises(unittest.TestCase):
    def test_assert_raises(self):
        with self.assertRaises(ValueError):
            parse("")
