# Label: `self.helper` resolves in its own class, then in same-file bases (template-method idiom).
import unittest


class Base(unittest.TestCase):
    def run_wrapper_test(self, name):
        self.assertEqual(200, fetch(name))


class TestWorkers(Base):
    def test_worker_html(self):
        self.run_wrapper_test("foo.worker.html")

    def test_own_helper(self):
        self.check_it()

    def check_it(self):
        self.assertTrue(fetch("x"))


class TestOtherSameName(unittest.TestCase):
    def check_it(self):
        fetch("y")

    def test_name_collision_is_not_masked(self):
        self.check_it()
