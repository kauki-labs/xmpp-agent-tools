import io
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ci"))
import changes  # noqa: E402


class Classify(unittest.TestCase):
    def test_docs_only_skips_the_code_jobs(self):
        self.assertEqual(changes.classify(["README.md", "docs/experiments.md", "docs/decisions/x.md", "LICENSE"]), {"code": False, "workflows": False})

    def test_any_source_or_test_runs_the_code_jobs(self):
        for path in ("src/xmpp_agent_client/router.py", "tests/test_router.py", "ci/scrub.py", "scripts/mutation-check.py", "pyproject.toml", "justfile", "Cargo.toml"):
            self.assertTrue(changes.classify(["README.md", path])["code"], path)

    def test_unknown_file_types_run_the_code_jobs(self):
        for path in ("data.bin", "Dockerfile", "renovate.json", "noextension", "src/x.rs", "docs/diagram.svg"):
            self.assertTrue(changes.classify([path])["code"], path)

    def test_workflow_changes_run_the_workflow_lint_only(self):
        self.assertEqual(changes.classify([".github/workflows/pr.yaml"]), {"code": False, "workflows": True})
        self.assertEqual(changes.classify([".github/CODEOWNERS"]), {"code": False, "workflows": True})

    def test_a_mixed_change_runs_both(self):
        self.assertEqual(changes.classify([".github/workflows/pr.yaml", "src/a.py"]), {"code": True, "workflows": True})

    def test_an_empty_list_runs_everything(self):
        self.assertEqual(changes.classify([]), {"code": True, "workflows": True})

    def test_blank_lines_are_ignored_but_do_not_make_an_empty_diff_pass(self):
        self.assertEqual(changes.classify(["", "  ", "README.md"]), {"code": False, "workflows": False})
        self.assertEqual(changes.classify(["", "  "]), {"code": True, "workflows": True})

    def test_markdown_outside_docs_is_still_docs(self):
        self.assertFalse(changes.classify(["CONTRIBUTING.md", "src/README.md"])["code"])

    def test_a_docs_path_with_code_inside_is_not_hidden(self):
        self.assertTrue(changes.classify(["docs/tools/build.py"])["code"])
        self.assertTrue(changes.classify(["docs/run.sh"])["code"])

    def test_paths_with_spaces_and_unicode(self):
        self.assertFalse(changes.classify(["docs/my notes.md"])["code"])
        self.assertTrue(changes.classify(["src/naïve file.py"])["code"])

    def test_traversal_looking_paths_do_not_pass_as_docs(self):
        self.assertTrue(changes.classify(["docs/../src/a.py"])["code"])


class Main(unittest.TestCase):
    def test_writes_github_output_lines(self):
        with tempfile.NamedTemporaryFile("r+", suffix=".out") as out:
            rc = changes.main(io.StringIO("README.md\n.github/workflows/pr.yaml\n"), out.name)
            out.seek(0)
            lines = sorted(out.read().split())
        self.assertEqual(rc, 0)
        self.assertEqual(lines, ["code=false", "workflows=true"])

    def test_no_output_file_prints_to_stdout(self):
        old = sys.stdout
        sys.stdout = buf = io.StringIO()
        try:
            changes.main(io.StringIO("src/a.py\n"), None)
        finally:
            sys.stdout = old
        self.assertIn("code=true", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
