import json
import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "ci"))
import merge_gate  # noqa: E402


def needs(**results):
    return {name: {"result": r, "outputs": {}} for name, r in results.items()}


class Evaluate(unittest.TestCase):
    def test_all_success_passes(self):
        ok, _ = merge_gate.evaluate(needs(tests="success", mutation="success"))
        self.assertTrue(ok)

    def test_skipped_counts_as_green(self):
        ok, _ = merge_gate.evaluate(needs(tests="success", rust="skipped"))
        self.assertTrue(ok)

    def test_everything_skipped_passes(self):
        ok, _ = merge_gate.evaluate(needs(tests="skipped", rust="skipped"))
        self.assertTrue(ok)

    def test_failure_fails(self):
        ok, report = merge_gate.evaluate(needs(tests="failure", rust="success"))
        self.assertFalse(ok)
        self.assertIn("tests", report)
        self.assertIn("failure", report)

    def test_cancelled_fails(self):
        ok, report = merge_gate.evaluate(needs(tests="success", mutation="cancelled"))
        self.assertFalse(ok)
        self.assertIn("mutation", report)

    def test_unknown_results_fail_closed(self):
        for bad in ("", "pending", "in_progress", "neutral", "timed_out", None):
            ok, _ = merge_gate.evaluate({"tests": {"result": bad}})
            self.assertFalse(ok, bad)

    def test_a_malformed_entry_fails_closed(self):
        for bad in (None, "success", 5, {}, {"outputs": {}}):
            ok, _ = merge_gate.evaluate({"tests": bad})
            self.assertFalse(ok, bad)

    def test_no_jobs_at_all_fails_closed(self):
        self.assertFalse(merge_gate.evaluate({})[0])
        self.assertFalse(merge_gate.evaluate(None)[0])

    def test_one_failure_among_many_successes_fails(self):
        ok, _ = merge_gate.evaluate(needs(a="success", b="success", c="skipped", d="failure", e="success"))
        self.assertFalse(ok)

    def test_the_report_lists_every_job(self):
        _, report = merge_gate.evaluate(needs(tests="success", rust="skipped"))
        self.assertIn("tests", report)
        self.assertIn("rust", report)


class Main(unittest.TestCase):
    def run_main(self, payload):
        env = {"NEEDS_JSON": payload} if payload is not None else {}
        return merge_gate.main([], env)

    def test_exit_zero_when_green(self):
        self.assertEqual(self.run_main(json.dumps(needs(a="success", b="skipped"))), 0)

    def test_exit_one_when_red(self):
        self.assertEqual(self.run_main(json.dumps(needs(a="success", b="failure"))), 1)

    def test_missing_or_invalid_input_fails_closed(self):
        for payload in (None, "", "not json", "[]", "null", "5"):
            self.assertEqual(self.run_main(payload), 1, payload)


if __name__ == "__main__":
    unittest.main()
