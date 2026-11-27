"""Merge gate: the only required check of the repository.

It runs after every other job (`needs: [...]`, `if: always()`). It is green when every job is `success` or `skipped` (a job that
did not apply to the change), and red for anything else: `failure`, `cancelled`, an unknown value, a malformed entry, or no jobs
at all. The needs context arrives as JSON in the environment variable NEEDS_JSON, never interpolated into a command.
"""
from __future__ import annotations

import json
import os
import sys

GREEN = frozenset({"success", "skipped"})


def _result(entry):
    return entry.get("result") if isinstance(entry, dict) else None


def evaluate(needs):
    if not isinstance(needs, dict) or not needs:
        return False, "merge gate: no jobs to check, failing closed"
    lines, ok = [], True
    for name in sorted(needs):
        result = _result(needs[name])
        green = isinstance(result, str) and result in GREEN
        ok = ok and green
        lines.append(f"{'ok  ' if green else 'FAIL'} {name}: {result!r}")
    header = "merge gate: all jobs success or skipped" if ok else "merge gate: at least one job is not success or skipped"
    return ok, "\n".join([header, *lines])


def main(argv=None, environ=None):
    environ = os.environ if environ is None else environ
    raw = environ.get("NEEDS_JSON", "")
    try:
        needs = json.loads(raw)
    except (TypeError, ValueError):
        print("merge gate: NEEDS_JSON is missing or not JSON, failing closed", file=sys.stderr)
        return 1
    ok, report = evaluate(needs)
    print(report)
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
