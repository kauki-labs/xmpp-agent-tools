"""Classify one chat message: a prompt, a bridge command, a harness command, or a rejection.

Claude takes an unknown `/name` as plain text (docs/experiments.md E6), so the bridge validates names against the command
list from the init event and stops the rest here. `//name` and `!name` reach the bridge when a client such as Gajim eats a
leading slash. A file path such as `/etc/hosts` is a prompt, not a command.
"""
from __future__ import annotations

import difflib
import re
from dataclasses import dataclass

_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9:_.-]*")  # plugin skills carry a colon, e.g. code-review:code-review


@dataclass(frozen=True)
class Prompt:
    text: str


@dataclass(frozen=True)
class Local:
    name: str
    args: str


@dataclass(frozen=True)
class Harness:
    name: str
    args: str
    text: str  # what to send to the harness: "/name args"


@dataclass(frozen=True)
class Rejected:
    reason: str  # "empty", "unknown" or "interactive"
    detail: str = ""
    suggestions: tuple[str, ...] = ()


LOCAL_COMMANDS = frozenset({"new", "stop", "status", "verbose", "mode", "help"})


def _command_body(text: str) -> str | None:
    """The text after the command prefix, or None when the message does not start with one."""
    if text.startswith("//"):
        return text[2:]
    if text.startswith("/"):
        return text[1:]
    if text.startswith("!") and len(text) > 1 and text[1].isalnum():
        return text[1:]
    return None


def route(text, harness_commands=(), terminal_commands=()):
    text = text.strip()
    if not text:
        return Rejected("empty")
    body = _command_body(text)
    if body is None:
        return Prompt(text)
    token = body.split(None, 1)[0] if body.strip() else ""
    if token and not _NAME.fullmatch(token):
        return Prompt(text)  # a path or other text that merely starts with a slash
    if not token:
        return Rejected("unknown")
    args = body[len(token):].strip()
    lowered = token.lower()
    if lowered in LOCAL_COMMANDS:
        return Local(lowered, args)
    if token in terminal_commands:
        return Rejected("interactive", token)
    if token in harness_commands:
        return Harness(token, args, f"/{token} {args}" if args else f"/{token}")
    known = sorted(set(harness_commands) | LOCAL_COMMANDS)
    return Rejected("unknown", token, tuple(difflib.get_close_matches(token, known, n=3, cutoff=0.6)))
