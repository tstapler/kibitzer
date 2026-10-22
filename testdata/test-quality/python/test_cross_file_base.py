# Label: base class lives in another file (wptserve functional/base.py); unresolvable under
# one-parse-per-file, so the finding stands until `assertion_helpers` names the helper.
from .base import TestWrapperHandlerUsingServer


class TestWorkersHandler(TestWrapperHandlerUsingServer):
    def test_any_worker_html(self):
        self.run_wrapper_test("foo.any.worker.html", "text/html")
