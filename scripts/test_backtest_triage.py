import os
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest


spec = importlib.util.spec_from_file_location("triage", Path(__file__).with_name("backtest-triage.py"))
triage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(triage)


def git(root, *args):
    subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True)


from unittest.mock import patch

class CorpusFilesTests(unittest.TestCase):
    def test_git_enumeration_is_hardened_and_bounded(self):
        with patch.dict(os.environ, {"GIT_DIR": "/untrusted", "GIT_CONFIG_COUNT": "1"}):
            with patch.object(triage.subprocess, "run") as run:
                triage._corpus_git(Path("/repo"), "ls-files")
                args, options = run.call_args
                self.assertEqual(args[0][:3], ["git", "-c", "core.fsmonitor=false"])
                self.assertFalse(any(key.startswith("GIT_") for key in options["env"]))
                self.assertEqual(options["timeout"], 30)

    def test_path_globs_and_ignored_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            (root / "src").mkdir()
            (root / "src/a.go").write_text("package a\n")
            (root / "a.go").write_text("package a\n")
            for pattern in ["*.go", "**/*.go", "src/*.go", "**/src/*.go"]:
                with self.subTest(pattern=pattern):
                    expected = sorted(str(p.relative_to(root)) for p in root.rglob(pattern) if p.is_file())
                    self.assertEqual(triage.corpus_files(root, pattern), expected)
            (root / ".gitignore").write_text("snapshot/\n")
            (root / "snapshot").mkdir()
            (root / "snapshot/a.go").write_text("package a\n")
            self.assertEqual(triage.corpus_files(root / "snapshot", "*.go"), ["a.go"])

    def test_nested_git_roots_respect_ignores_and_path_globs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            git(root, "init", "-q")
            child = root / "module"
            child.mkdir()
            git(child, "init", "-q")
            (child / "a.go").write_text("package a\n")
            (child / ".gitignore").write_text("ignored.go\n")
            (child / "ignored.go").write_text("package a\n")
            git(child, "add", ".")
            git(child, "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-qm", "initial")
            for staged in [False, True]:
                if staged:
                    git(root, "add", "module")
                with self.subTest(staged=staged):
                    self.assertEqual(triage.corpus_files(root, "*.go"), ["module/a.go"])
                    self.assertEqual(triage.corpus_files(root, "module/*.go"), ["module/a.go"])

    def test_external_symlink_is_not_scanned(self):
        with tempfile.TemporaryDirectory() as temporary, tempfile.TemporaryDirectory() as external:
            root = Path(temporary)
            git(root, "init", "-q")
            outside = Path(external) / "a.go"
            outside.write_text("package a\n")
            (root / "link.go").symlink_to(outside)
            self.assertEqual(triage.corpus_files(root, "*.go"), [])


if __name__ == "__main__":
    unittest.main()
