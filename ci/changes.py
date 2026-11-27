"""Classify the changed paths of a pull request so that jobs that do not apply are skipped.

`code`: anything that is not documentation, a workflow file or a blank line. Fail-safe: an unknown file type, a script under docs/, or an
empty list all run the code jobs. `workflows`: anything under .github/.
Reads one path per line from stdin and writes `code=true|false` and `workflows=true|false` to $GITHUB_OUTPUT (or stdout).
"""
from __future__ import annotations

import sys

_DOC_SUFFIXES = (".md",)


def _is_doc(path):
    return path == "LICENSE" or path.endswith(_DOC_SUFFIXES)


def classify(paths):
    paths = [p.strip() for p in paths if p and p.strip()]
    if not paths:
        return {"code": True, "workflows": True}
    workflows = any(p.startswith(".github/") for p in paths)
    code = any(not p.startswith(".github/") and not _is_doc(p) for p in paths)
    return {"code": code, "workflows": workflows}


def main(stdin=None, output_path=None):
    stdin = sys.stdin if stdin is None else stdin
    result = classify(stdin.read().splitlines())
    lines = [f"{key}={'true' if value else 'false'}" for key, value in sorted(result.items())]
    if output_path:
        with open(output_path, "a", encoding="utf-8") as out:
            out.write("\n".join(lines) + "\n")
    else:
        print("\n".join(lines))
    return 0


if __name__ == "__main__":
    sys.exit(main(output_path=__import__("os").environ.get("GITHUB_OUTPUT")))
