import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ci"))
import scrub  # noqa: E402

# Forbidden strings are assembled from pieces so this file passes its own check.
DOMAIN = "kauki" + ".xyz"
HOST = "chat.apps.int." + DOMAIN
PRIV = [".".join(map(str, p)) for p in ((192, 168, 1, 241), (10, 244, 0, 1), (172, 16, 5, 4), (172, 31, 255, 1), (100, 64, 0, 3), (100, 127, 1, 1), (169, 254, 1, 1))]
OK_IPS = [".".join(map(str, p)) for p in ((192, 0, 2, 10), (198, 51, 100, 7), (203, 0, 113, 9), (8, 8, 8, 8), (172, 32, 0, 1), (172, 15, 0, 1), (100, 63, 0, 1), (100, 128, 0, 1), (127, 0, 0, 1))]


def rules(text, path="a.md"):
    return {rule for _, rule, _ in scrub.find_problems(text, path)}


class FindProblems(unittest.TestCase):
    def test_clean_text_has_no_problems(self):
        self.assertEqual(scrub.find_problems("chat.example.org on 192.0.2.1\nnothing to see\n", "a.md"), [])

    def test_internal_domain_and_subdomains(self):
        self.assertIn("internal-domain", rules(f"connect to {HOST}"))
        self.assertIn("internal-domain", rules(f"see {DOMAIN.upper()}"))

    def test_private_and_mesh_addresses(self):
        for ip in PRIV:
            self.assertIn("private-address", rules(f"host {ip} port 5223"), ip)

    def test_public_and_documentation_addresses_are_fine(self):
        for ip in OK_IPS:
            self.assertNotIn("private-address", rules(f"host {ip}"), ip)

    def test_an_address_inside_a_longer_number_is_not_flagged(self):
        self.assertEqual(rules("version 1.10.244.0.1.2 and 3.192.168.1.2.3"), set())

    def test_home_directories(self):
        self.assertIn("home-path", rules("/Us" + "ers/someone/Fun/x"))
        self.assertIn("home-path", rules("see /ho" + "me/someone/.config"))

    def test_ci_and_placeholder_home_paths_are_fine(self):
        for text in ("/home/runner/work/x", "/home/user/project", "/Users/example/project"):
            self.assertNotIn("home-path", rules(text), text)

    def test_personal_names_and_organisations(self):
        for word in ("tee" + "bor", "cso" + "ka", "hop" + "rnet", "." + "ts" + ".net"):
            self.assertIn("personal", rules(f"owner {word.upper()}"), word)

    def test_each_problem_has_its_line_number(self):
        found = scrub.find_problems(f"fine\nfine\nhost {PRIV[0]}\n", "a.md")
        self.assertEqual([(n, r) for n, r, _ in found], [(3, "private-address")])

    def test_several_problems_on_one_line_are_all_reported(self):
        got = rules(f"{HOST} at {PRIV[0]} in " + "/Us" + "ers/someone")
        self.assertEqual(got, {"internal-domain", "private-address", "home-path"})

    def test_a_snippet_is_returned_for_the_report(self):
        (_, _, snippet), = scrub.find_problems(f"x {PRIV[0]} y", "a.md")
        self.assertIn(PRIV[0], snippet)


class Scan(unittest.TestCase):
    def make_repo(self, files):
        d = tempfile.mkdtemp()
        subprocess.run(["git", "init", "-q", d], check=True)
        for name, content in files.items():
            p = os.path.join(d, name)
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "wb") as f:
                f.write(content if isinstance(content, bytes) else content.encode())
            subprocess.run(["git", "-C", d, "add", name], check=True)
        return d

    def test_scans_tracked_files_and_reports_the_path(self):
        d = self.make_repo({"docs/a.md": f"host {PRIV[0]}\n", "src/ok.py": "x = 1\n"})
        problems = scrub.scan(d)
        self.assertEqual([(p, n, r) for p, n, r, _ in problems], [("docs/a.md", 1, "private-address")])

    def test_binary_files_and_the_license_are_skipped(self):
        d = self.make_repo({"img.png": b"\x89PNG\x00" + PRIV[0].encode(), "LICENSE": "Copyright " + "tee" + "bor" + "\n", "a.md": "ok\n"})
        self.assertEqual(scrub.scan(d), [])

    def test_untracked_files_are_ignored(self):
        d = self.make_repo({"a.md": "ok\n"})
        with open(os.path.join(d, "untracked.md"), "w") as f:
            f.write(PRIV[0])
        self.assertEqual(scrub.scan(d), [])

    def test_the_exit_code_follows_the_result(self):
        clean = self.make_repo({"a.md": "ok\n"})
        dirty = self.make_repo({"a.md": f"{HOST}\n"})
        self.assertEqual(scrub.main([clean]), 0)
        self.assertEqual(scrub.main([dirty]), 1)

    def test_this_repository_is_clean(self):
        root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
        self.assertEqual(scrub.scan(root), [], "the repository holds an internal name; run python3 ci/scrub.py")


if __name__ == "__main__":
    unittest.main()
