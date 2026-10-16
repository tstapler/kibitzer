# Label: unittest class: test_no_assert is a true positive, test_asserts is not.
import unittest


class ParseTest(unittest.TestCase):
    def test_no_assert(self):
        parse("")

    def test_asserts(self):
        self.assertEqual(parse(""), 1)
