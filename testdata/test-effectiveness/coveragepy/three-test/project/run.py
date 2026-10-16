"""Regenerates ../coverage.json: runs each test function in order under coverage.py dynamic contexts.

Run from this directory: /usr/bin/python3 run.py
"""

import coverage

import test_app

cov = coverage.Coverage(data_file=None, source=["."], omit=["test_app.py", "run.py"])
cov.set_option("run:dynamic_context", "test_function")
cov.start()
for name in ("test_pos", "test_neg", "test_noassert"):
    getattr(test_app, name)()
cov.stop()
cov.json_report(outfile="../coverage.json", show_contexts=True)
