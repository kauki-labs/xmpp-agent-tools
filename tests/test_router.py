import unittest

from xmpp_agent_client.router import Harness, Local, Prompt, Rejected, route

# From the claude init event (docs/experiments.md E6): plugin skills carry a colon, terminal-only commands are listed apart.
HARNESS = {"simplify", "loop", "code-review:code-review", "init", "security-review", "cost", "context", "compact", "clear", "status"}
TERMINAL = {"doctor", "color", "focus", "reload-plugins"}


def r(text):
    return route(text, harness_commands=HARNESS, terminal_commands=TERMINAL)


class PlainText(unittest.TestCase):
    def test_plain_text_is_a_prompt(self):
        self.assertEqual(r("run the tests"), Prompt("run the tests"))

    def test_surrounding_whitespace_is_trimmed(self):
        self.assertEqual(r("  run the tests \n"), Prompt("run the tests"))

    def test_empty_is_rejected(self):
        for text in ("", "   ", "\n\t"):
            self.assertEqual(r(text), Rejected("empty"))

    def test_slash_inside_text_is_not_a_command(self):
        self.assertEqual(r("see /etc/hosts for details"), Prompt("see /etc/hosts for details"))

    def test_file_paths_are_prompts_not_commands(self):
        self.assertEqual(r("/etc/hosts"), Prompt("/etc/hosts"))
        self.assertEqual(r("/srv/x/notes.md please summarise"), Prompt("/srv/x/notes.md please summarise"))

    def test_bang_without_a_name_is_text(self):
        for text in ("!!!", "! hello", "!", "!?"):
            self.assertEqual(r(text), Prompt(text))


class LocalCommands(unittest.TestCase):
    def test_each_local_command(self):
        self.assertEqual(r("/stop"), Local("stop", ""))
        self.assertEqual(r("/new"), Local("new", ""))
        self.assertEqual(r("/status"), Local("status", ""))
        self.assertEqual(r("/help"), Local("help", ""))
        self.assertEqual(r("/mode plan"), Local("mode", "plan"))
        self.assertEqual(r("/verbose on"), Local("verbose", "on"))

    def test_local_names_are_case_insensitive(self):
        self.assertEqual(r("/HELP"), Local("help", ""))
        self.assertEqual(r("/Mode acceptEdits"), Local("mode", "acceptEdits"))  # args keep their case

    def test_local_wins_over_a_harness_command_of_the_same_name(self):
        self.assertIn("status", HARNESS)
        self.assertEqual(r("/status"), Local("status", ""))

    def test_args_are_trimmed_but_kept_whole(self):
        self.assertEqual(r("/mode   plan  "), Local("mode", "plan"))


class Escapes(unittest.TestCase):
    """Gajim has its own / commands and may eat a leading slash; // and ! reach the bridge unchanged."""

    def test_double_slash_is_a_single_slash(self):
        self.assertEqual(r("//stop"), Local("stop", ""))
        self.assertEqual(r("//simplify x"), Harness("simplify", "x", "/simplify x"))

    def test_bang_is_a_slash(self):
        self.assertEqual(r("!status"), Local("status", ""))
        self.assertEqual(r("!code-review:code-review 12"), Harness("code-review:code-review", "12", "/code-review:code-review 12"))


class HarnessCommands(unittest.TestCase):
    def test_known_command_is_passed_through_with_its_args(self):
        self.assertEqual(r("/simplify tighten the loop"), Harness("simplify", "tighten the loop", "/simplify tighten the loop"))

    def test_plugin_skill_names_with_a_colon(self):
        self.assertEqual(r("/code-review:code-review 12"), Harness("code-review:code-review", "12", "/code-review:code-review 12"))

    def test_command_without_args(self):
        self.assertEqual(r("/cost"), Harness("cost", "", "/cost"))

    def test_multiline_args_are_preserved(self):
        self.assertEqual(r("/loop 5m\n/check the build"), Harness("loop", "5m\n/check the build", "/loop 5m\n/check the build"))

    def test_harness_names_are_case_sensitive(self):
        self.assertEqual(r("/Simplify x").__class__, Rejected)


class Rejections(unittest.TestCase):
    def test_unknown_command_is_rejected_not_sent_to_the_model(self):
        # claude would take "/nonexistent foo" as plain text (experiments E6), so the bridge must stop it here.
        got = r("/nonexistent foo")
        self.assertEqual(got.reason, "unknown")
        self.assertEqual(got.detail, "nonexistent")

    def test_close_names_are_suggested(self):
        got = r("/simplfy x")
        self.assertEqual(got.reason, "unknown")
        self.assertIn("simplify", got.suggestions)

    def test_terminal_only_commands_are_rejected_with_a_reason(self):
        for name in TERMINAL:
            got = r("/" + name)
            self.assertEqual(got, Rejected("interactive", name))

    def test_lone_slash_is_unknown(self):
        self.assertEqual(r("/").reason, "unknown")
        self.assertEqual(r("//").reason, "unknown")

    def test_no_harness_commands_known_yet_means_every_non_local_command_is_unknown(self):
        self.assertEqual(route("/simplify x").reason, "unknown")


if __name__ == "__main__":
    unittest.main()
