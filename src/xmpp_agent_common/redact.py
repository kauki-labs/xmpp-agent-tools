"""Redact secrets and describe a tool call for chat.

Tool arguments can hold tokens, and an approval request lands in the room archive (365 days). `describe_tool` redacts first and
cuts second, so a token can never survive as a prefix. File content and edit text are never shown, only their size. The patterns
cover common token shapes and key=value pairs; an unknown secret format will pass, which is why arguments are also cut short.
"""
from __future__ import annotations

import json
import re

_KEYWORDS = r"(?:password|passwd|pwd|secret|token|api[_-]?key|credential)"

_PATTERNS = [
    (re.compile(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----", re.DOTALL), "[redacted private key]"),
    (re.compile(r"(?<=://)[^/\s:@]+:[^/\s@]+@"), "[redacted]@"),
    (re.compile(r"(authorization\s*:\s*)(?:(?:bearer|basic|token)\s+)?[^\s\"']+", re.IGNORECASE), r"\1[redacted]"),
    (re.compile(r"\bbearer\s+[A-Za-z0-9._~+/=-]{10,}", re.IGNORECASE), "Bearer [redacted]"),
    (re.compile(r"\bAKIA[0-9A-Z]{16}\b"), "[redacted aws key]"),
    (re.compile(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b|\bgithub_pat_[A-Za-z0-9_]{20,}\b"), "[redacted token]"),
    (re.compile(r"\bxox[abprs]-[A-Za-z0-9-]{10,}"), "[redacted token]"),
    (re.compile(r"\bsk-[A-Za-z0-9_-]{16,}"), "[redacted token]"),
    (re.compile(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}"), "[redacted token]"),
    # --password 'x' (a space separates a flag from its value)
    (re.compile(r"(--?[A-Za-z0-9_-]*" + _KEYWORDS + r"[A-Za-z0-9_-]*)(\s+)(\"[^\"]*\"|'[^']*'|\S+)", re.IGNORECASE), r"\1\2[redacted]"),
    # NAME=value, name: value, "name": "value"
    (re.compile(r"\b([A-Za-z0-9_.-]*" + _KEYWORDS + r"[A-Za-z0-9_.-]*)([\"']?\s*[=:]\s*)(\"[^\"]*\"|'[^']*'|\S+)", re.IGNORECASE), r"\1\2[redacted]"),
]


def redact(text):
    for pattern, replacement in _PATTERNS:
        text = pattern.sub(replacement, text)
    return text


def _cut(text, max_chars):
    if len(text) <= max_chars:
        return text
    marker = f" … [+{len(text)} more chars]"  # the digit count is an upper bound for the exact one below
    keep = max(0, max_chars - len(marker))
    return text[:keep] + f" … [+{len(text) - keep} more chars]"


def _value(v):
    return v if isinstance(v, str) else json.dumps(v, ensure_ascii=False, default=str)


def describe_tool(tool_name, tool_input, max_chars=400):
    i = tool_input if isinstance(tool_input, dict) else {}
    g = lambda k: _value(i.get(k, ""))
    if tool_name == "Bash":
        out = f"Bash: {g('command')}" + (f" ({g('description')})" if i.get("description") else "")
    elif tool_name == "Write":
        out = f"Write {g('file_path')} ({len(g('content'))} chars)"
    elif tool_name == "Edit":
        out = f"Edit {g('file_path')} (replace {len(g('old_string'))} chars with {len(g('new_string'))} chars)"
    elif tool_name == "MultiEdit":
        edits = i.get("edits")
        out = f"MultiEdit {g('file_path')} ({len(edits) if isinstance(edits, list) else 0} edits)"
    elif tool_name == "NotebookEdit":
        out = f"NotebookEdit {g('notebook_path')}"
    elif tool_name == "Read":
        out = f"Read {g('file_path')}"
    elif tool_name == "Glob":
        out = f"Glob {g('pattern')}" + (f" in {g('path')}" if i.get("path") else "")
    elif tool_name == "Grep":
        out = f"Grep {g('pattern')}" + (f" in {g('path')}" if i.get("path") else "")
    elif tool_name == "WebFetch":
        out = f"WebFetch {g('url')}"
    elif tool_name == "WebSearch":
        out = f"WebSearch {g('query')}"
    else:
        pairs = ", ".join(f"{k}={_cut(redact(_value(v)), 80)}" for k, v in i.items())
        out = f"{tool_name} {pairs}".rstrip()
    return _cut(redact(out), max_chars)
