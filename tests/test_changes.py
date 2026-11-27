import io
import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ci"))
import changes  # noqa: E402


class Classify(unittest.TestCase):
    def test_docs_only_skips_the_code_jobs(self):
        self.assertEqual(changes.classify(["README.md", "docs/experiments.md", "docs/decisions/x.md", "LICENSE"]), {"code": False, "workflows": False, "rust": False})

    def test_any_source_or_test_runs_the_code_jobs(self):
        for path in ("src/xmpp_agent_client/router.py", "tests/test_router.py", "ci/scrub.py", "scripts/mutation-check.py", "pyproject.toml", "justfile", "Cargo.toml"):
            self.assertTrue(changes.classify(["README.md", path])["code"], path)

    def test_unknown_file_types_run_the_code_jobs(self):
        for path in ("data.bin", "Dockerfile", "renovate.json", "noextension", "src/x.rs", "docs/diagram.svg"):
            self.assertTrue(changes.classify([path])["code"], path)

    def test_workflow_changes_run_the_workflow_lint_only(self):
        self.assertEqual(changes.classify([".github/workflows/pr.yaml"]), {"code": False, "workflows": True, "rust": False})
        self.assertEqual(changes.classify([".github/CODEOWNERS"]), {"code": False, "workflows": True, "rust": False})

    def test_a_mixed_change_runs_both(self):
        self.assertEqual(changes.classify([".github/workflows/pr.yaml", "src/a.py"]), {"code": True, "workflows": True, "rust": False})

    def test_an_empty_list_runs_everything(self):
        self.assertEqual(changes.classify([]), {"code": True, "workflows": True, "rust": True})

    def test_blank_lines_are_ignored_but_do_not_make_an_empty_diff_pass(self):
        self.assertEqual(changes.classify(["", "  ", "README.md"]), {"code": False, "workflows": False, "rust": False})
        self.assertEqual(changes.classify(["", "  "]), {"code": True, "workflows": True, "rust": True})

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


class Rust(unittest.TestCase):
    def test_rust_sources_and_manifests_run_the_rust_job(self):
        for path in ("crates/xmpp-agent-common/src/redact.rs", "build.rs", "Cargo.toml", "Cargo.lock", "crates/x/Cargo.toml", "rust-toolchain.toml",
                     "rustfmt.toml", ".rustfmt.toml", "clippy.toml", ".cargo/config.toml", "flake.nix", "flake.lock", "rust-toolchain"):
            self.assertTrue(changes.classify(["README.md", path])["rust"], path)

    def test_a_change_without_rust_files_skips_the_rust_job(self):
        for path in ("README.md", "src/xmpp_agent_client/router.py", "tests/test_router.py", ".github/workflows/pr.yaml", "docs/ci.md", "justfile"):
            self.assertFalse(changes.classify([path])["rust"], path)

    def test_lookalike_names_are_not_rust(self):
        for path in ("docs/Cargo.toml.md", "src/rs", "notes.rs.txt", "src/mycargo.toml", "Cargo.toml.bak", "docs/flake.nix.md"):
            self.assertFalse(changes.classify([path])["rust"], path)

    def test_an_unknown_diff_runs_the_rust_job(self):
        self.assertTrue(changes.classify([])["rust"])
        self.assertTrue(changes.classify(["", "  "])["rust"])

    def test_a_rust_change_is_also_a_code_change(self):
        self.assertEqual(changes.classify(["crates/x/src/lib.rs"]), {"code": True, "workflows": False, "rust": True})


class DiffCommand(unittest.TestCase):
    """The workflow feeds `git diff --name-only` to the classifier. By default git prints only the new name of a rename (the old
    `.rs` path is missing from the list) and quotes non-ASCII paths (`"na\\303\\257ve.rs"` does not end in `.rs`). Both make `rust` false
    for a Rust change, so the command must turn them off."""

    def setUp(self):
        with open(os.path.join(os.path.dirname(__file__), "..", ".github", "workflows", "pr.yaml"), encoding="utf-8") as workflow:
            self.lines = [line for line in workflow.read().splitlines() if "git" in line and "diff" in line and "ci/changes.py" in line]

    def test_the_workflow_has_one_diff_command(self):
        self.assertEqual(len(self.lines), 1, self.lines)

    def test_renames_are_listed_as_a_delete_and_an_add(self):
        self.assertIn("--no-renames", self.lines[0])

    def test_paths_are_not_quoted(self):
        self.assertIn("core.quotePath=false", self.lines[0])


class Main(unittest.TestCase):
    def test_writes_github_output_lines(self):
        with tempfile.NamedTemporaryFile("r+", suffix=".out") as out:
            rc = changes.main(io.StringIO("README.md\n.github/workflows/pr.yaml\n"), out.name)
            out.seek(0)
            lines = sorted(out.read().split())
        self.assertEqual(rc, 0)
        self.assertEqual(lines, ["code=false", "rust=false", "workflows=true"])

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
