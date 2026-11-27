import unittest

from xmpp_agent_common.redact import describe_tool, redact

# Fake secrets built from pieces so scanners do not flag this file.
AWS = "AKIA" + "ABCDEFGHIJKLMNOP"
GH = "ghp_" + "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8"
SK = "sk-ant-" + "api03-AbCdEfGhIjKlMnOpQrStUvWx"
SLACK = "xoxb-" + "1234567890-abcdefghij"
JWT = "eyJhbGciOiJIUzI1NiJ9" + "." + "eyJzdWIiOiIxMjM0NTY3ODkwIn0" + "." + "SflKxwRJSMeKKF2QT4fwpMeJf36"
PEM = "-----BEGIN " + "RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA1234\nabcdefg\n-----END " + "RSA PRIVATE KEY-----"


class Redact(unittest.TestCase):
    def test_plain_text_is_untouched(self):
        for text in ("ls -la /etc/hosts", "git checkout 3f2a9c1", "python3 -m unittest", "echo hello world", ""):
            self.assertEqual(redact(text), text)

    def test_token_shapes(self):
        for secret in (AWS, GH, SK, SLACK, JWT):
            out = redact(f"curl -H 'x: {secret}' https://example.com")
            self.assertNotIn(secret, out)
            self.assertIn("[redacted", out)

    def test_private_key_block_spanning_lines(self):
        out = redact("cat > key.pem <<EOF\n" + PEM + "\nEOF")
        self.assertNotIn("MIIEow", out)
        self.assertNotIn("abcdefg", out)
        self.assertIn("EOF", out)

    def test_key_value_pairs_keep_the_name(self):
        self.assertEqual(redact("export API_KEY=abc123"), "export API_KEY=[redacted]")
        self.assertEqual(redact("DB_PASSWORD=hunter2 psql -h db"), "DB_PASSWORD=[redacted] psql -h db")
        self.assertEqual(redact("token: abcdef"), "token: [redacted]")
        self.assertEqual(redact('client_secret="s3cr3t value"'), 'client_secret=[redacted]')
        self.assertEqual(redact("--password 'pw'"), "--password [redacted]")

    def test_authorization_headers(self):
        out = redact('curl -H "Authorization: Bearer abc.def.ghi-jkl" https://x')
        self.assertNotIn("abc.def.ghi-jkl", out)
        self.assertIn("Authorization", out)

    def test_authorization_header_in_other_schemes(self):
        # not covered by the generic bearer rule
        for header, secret in (("Authorization: Basic dXNlcjpwYXNz", "dXNlcjpwYXNz"), ("Authorization: abc123", "abc123"),
                               ("authorization:token zzz999", "zzz999")):
            out = redact(f"curl -H '{header}' https://x")
            self.assertNotIn(secret, out)
            self.assertIn("uthorization", out)

    def test_bearer_token_alone(self):
        self.assertNotIn("abcdefghij1234", redact("Bearer abcdefghij1234"))

    def test_credentials_in_urls(self):
        out = redact("git clone https://user:pw123@github.com/org/repo.git")
        self.assertNotIn("pw123", out)
        self.assertIn("github.com/org/repo.git", out)

    def test_several_secrets_in_one_line(self):
        out = redact(f"A_TOKEN=one B_SECRET=two {GH} {AWS}")
        for leaked in ("one", "two", GH, AWS):
            self.assertNotIn(leaked, out)

    def test_idempotent(self):
        once = redact(f"export TOKEN=abc {GH}")
        self.assertEqual(redact(once), once)

    def test_words_that_merely_contain_a_keyword_are_kept(self):
        # "author" and "tokenize" are not secrets
        self.assertEqual(redact("git log --author=bob"), "git log --author=bob")
        self.assertEqual(redact("python tokenize.py"), "python tokenize.py")


class Describe(unittest.TestCase):
    def test_bash_shows_the_redacted_command(self):
        out = describe_tool("Bash", {"command": "DB_PASSWORD=hunter2 psql -h db", "description": "Query the db"})
        self.assertIn("psql -h db", out)
        self.assertNotIn("hunter2", out)
        self.assertTrue(out.startswith("Bash"))

    def test_write_never_shows_content(self):
        content = "token=abcdef-secret-content"
        out = describe_tool("Write", {"file_path": "/tmp/a.txt", "content": content})
        self.assertIn("/tmp/a.txt", out)
        self.assertNotIn("abcdef-secret-content", out)
        self.assertIn(str(len(content)), out)  # the size is shown, the text is not

    def test_edit_never_shows_old_or_new_text(self):
        out = describe_tool("Edit", {"file_path": "/src/x.py", "old_string": "def helper_function_name():", "new_string": "def renamed_thing():"})
        self.assertIn("/src/x.py", out)
        for leaked in ("helper_function_name", "renamed_thing"):  # plain text, so only the omission hides it
            self.assertNotIn(leaked, out)

    def test_read_and_search_tools(self):
        self.assertIn("/etc/hosts", describe_tool("Read", {"file_path": "/etc/hosts"}))
        out = describe_tool("Grep", {"pattern": "API_KEY=abc123", "path": "src"})
        self.assertNotIn("abc123", out)
        self.assertIn("src", out)

    def test_web_fetch_redacts_url_credentials(self):
        out = describe_tool("WebFetch", {"url": "https://user:pw123@example.com/a"})
        self.assertNotIn("pw123", out)
        self.assertIn("example.com/a", out)

    def test_unknown_tools_show_redacted_arguments(self):
        out = describe_tool("mcp__stub__thing", {"name": "x", "api_key": "abc123", "nested": {"token": "zzz999"}})
        self.assertIn("mcp__stub__thing", out)
        for leaked in ("abc123", "zzz999"):
            self.assertNotIn(leaked, out)

    def test_long_values_are_cut_to_the_limit(self):
        out = describe_tool("Bash", {"command": "echo " + "x" * 5000}, max_chars=200)
        self.assertLessEqual(len(out), 200)
        self.assertIn("more", out)  # says that something was cut

    def test_a_secret_split_by_the_cut_is_still_gone(self):
        # redact first, then cut. Unredacted this is 155 chars, so a cut-first version cuts inside the token (about 14 chars of it
        # survive); redacted it is 131 chars and needs no cut.
        out = describe_tool("Bash", {"command": "curl " + "a" * 100 + " -H " + GH}, max_chars=150)
        self.assertNotIn(GH[:12], out)

    def test_a_secret_split_by_the_per_argument_cut_is_still_gone(self):
        # 91 chars unredacted: a per-argument cut at 80 keeps about 8 chars of the token (too short for the final pass to match);
        # redacted first it is 67 chars and needs no cut.
        out = describe_tool("mcp__x__y", {"blob": "x" * 50 + " " + GH})
        self.assertNotIn(GH[:6], out)

    def test_missing_fields_do_not_crash(self):
        for name in ("Bash", "Write", "Edit", "Read", "Grep", "WebFetch", "Whatever"):
            self.assertIsInstance(describe_tool(name, {}), str)


if __name__ == "__main__":
    unittest.main()
