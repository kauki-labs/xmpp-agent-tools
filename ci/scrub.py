"""Scrub check for a public repository: no internal names, addresses, home paths or personal names in tracked text files.

Usage: python3 ci/scrub.py [repo-root]   (exit 1 and a list when something is found). Use placeholders instead: chat.example.org,
192.0.2.1 (a documentation address), /home/user. The rules are regular expressions built so that this file does not match them.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys

_OCTET = r"(?:25[0-5]|2[0-4]\d|1?\d?\d)"
_RULES = [
    ("internal-domain", re.compile(r"kauki\.xyz", re.IGNORECASE)),
    ("private-address", re.compile(
        r"(?<![\d.])(?:"
        rf"10\.{_OCTET}\.{_OCTET}\.{_OCTET}"
        rf"|192\.168\.{_OCTET}\.{_OCTET}"
        rf"|172\.(?:1[6-9]|2\d|3[01])\.{_OCTET}\.{_OCTET}"
        rf"|100\.(?:6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.{_OCTET}\.{_OCTET}"
        rf"|169\.254\.{_OCTET}\.{_OCTET}"
        r")(?![\d.]*\d)")),
    ("home-path", re.compile(r"/(?:Users|home)/(?!runner\b|user\b|example\b)[A-Za-z0-9_.-]+")),
    # the words are joined from pieces so that this file does not match its own rule
    ("personal", re.compile("|".join(["tee" + "bor", "cso" + "ka", "hop" + "rnet", r"\.ts" + r"\.net"]), re.IGNORECASE)),
]
_SKIP_NAMES = frozenset({"LICENSE"})


def find_problems(text, path=""):
    found = []
    for number, line in enumerate(text.splitlines(), 1):
        for rule, pattern in _RULES:
            if pattern.search(line):
                found.append((number, rule, line.strip()[:160]))
    return found


def _tracked(root):
    out = subprocess.run(["git", "-C", root, "ls-files", "-z"], capture_output=True, check=True).stdout
    return [p for p in out.decode().split("\0") if p]


def scan(root, paths=None):
    problems = []
    for rel in paths if paths is not None else _tracked(root):
        if os.path.basename(rel) in _SKIP_NAMES:
            continue
        try:
            with open(os.path.join(root, rel), "rb") as f:
                data = f.read()
        except OSError:
            continue
        if b"\0" in data:
            continue  # binary
        for number, rule, snippet in find_problems(data.decode("utf-8", errors="replace"), rel):
            problems.append((rel, number, rule, snippet))
    return problems


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    root = argv[0] if argv else os.getcwd()
    problems = scan(root)
    for rel, number, rule, snippet in problems:
        print(f"{rel}:{number}: {rule}: {snippet}")
    if problems:
        print(f"\nscrub: {len(problems)} problem(s). Use placeholders (chat.example.org, 192.0.2.1, /home/user).", file=sys.stderr)
        return 1
    print("scrub: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main())
