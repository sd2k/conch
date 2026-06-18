#!/usr/bin/env python3
"""Analyze shell commands in Claude Code transcripts to prioritize conch support.

Scans Claude Code transcript JSONL files (default: ~/.claude/projects/**/*.jsonl),
extracts every Bash tool invocation, splits each into individual command heads
— handling pipes, `&&`/`||`/`;`, `$(...)`/backtick substitution, env-var
assignments, and wrapper commands (sudo, env, xargs, …) — and reports the most
frequently used commands.

The point: conch is a shell whose external commands are WASI components (gh,
sqlite3, curl, …). Knowing which commands actually show up in real sessions
tells us what to build/support next. POSIX builtins are flagged separately since
conch implements those itself (via brush).

Usage:
    scripts/analyze-claude-commands.py [TOP_N] [TRANSCRIPT_DIR]

    TOP_N           how many to show (default 10)
    TRANSCRIPT_DIR  root to scan (default ~/.claude/projects)
"""

from __future__ import annotations

import json
import os
import re
import sys
from collections import Counter
from pathlib import Path

# Prefix words that wrap the real command; skip them and look at the next token.
WRAPPERS = {
    "sudo", "env", "time", "nice", "nohup", "command", "builtin", "exec",
    "xargs", "timeout", "stdbuf", "setsid", "ionice", "then", "do", "else",
    "elif", "watch", "doas",
}

# Shell keywords / punctuation that aren't commands to prioritize.
SKIP = {
    "", "(", ")", "{", "}", "[", "]", "[[", "]]", "!", "fi", "done", "esac",
    "in", "if", "while", "for", "case", "select", "until", "function",
}

# Common POSIX shell builtins — conch implements these itself, so flag them.
BUILTINS = {
    "cd", "echo", "export", "set", "unset", "read", "source", ".", "eval",
    "exit", "return", "pwd", "test", "true", "false", "alias", "unalias",
    "local", "shift", "trap", "wait", "kill", "jobs", "fg", "bg", "umask",
    "type", "hash", "printf", ":", "let", "declare", "typeset", "getopts",
    "history", "dirs", "pushd", "popd", "command",
}

# Split on shell operators and command-substitution boundaries. Splitting on
# single `|`/`&` also covers `||`/`&&` (we only take the first token after).
OPERATORS = re.compile(r"\$\(|\)|`|[|;&\n]")
ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
REDIRECT = re.compile(r"^[0-9]*[<>]")
HAS_LETTER = re.compile(r"[A-Za-z]")
# A plausible command/program name (after basename): letters/digits/_ . + -
# Rejects fragments from multi-line quoted args we split through without a full
# quote-aware parser (e.g. "Status:", prose, "key=val:").
VALID_NAME = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_.+-]*$")
# A heredoc start: `<<EOF`, `<< "EOF"`, `<<-EOF` (not the `<<<` here-string).
HEREDOC = re.compile(r"<<-?\s*['\"]?([A-Za-z_][A-Za-z0-9_]*)['\"]?")
# fd-duplicating / merging redirections contain `&` (e.g. `2>&1`, `>&2`, `&>f`);
# strip them before operator-splitting so the `&` doesn't spawn a bogus segment.
FD_REDIRECT = re.compile(r"[0-9]*>&[0-9-]+|&>>?")


def command_head(segment: str) -> str | None:
    """Return the command program invoked by one pipeline segment, or None."""
    tokens = segment.strip().split()
    for tok in tokens:
        if REDIRECT.match(tok):       # 2>&1, >file, <in
            continue
        if ASSIGNMENT.match(tok):     # FOO=bar prefix
            continue
        if tok in WRAPPERS:           # sudo, env, xargs, then, do, …
            continue
        if tok in SKIP:               # control keyword / punctuation
            return None
        head = os.path.basename(tok.strip("\"'"))
        if not head or head in SKIP or head.startswith("-"):
            return None
        # Real command names contain a letter and only name-safe characters —
        # rejects pure numbers, redirection leftovers, and quoted-arg fragments.
        if not HAS_LETTER.search(head) or not VALID_NAME.match(head):
            return None
        return head
    return None


def strip_heredocs(cmd: str) -> str:
    """Drop heredoc bodies (their content lines are data, not commands)."""
    lines = cmd.split("\n")
    out: list[str] = []
    i = 0
    while i < len(lines):
        line = lines[i]
        out.append(line)
        m = HEREDOC.search(line)
        i += 1
        if m:
            delim = m.group(1)
            while i < len(lines) and lines[i].strip() != delim:
                i += 1
            i += 1  # also skip the closing delimiter line
    return "\n".join(out)


def commands_in(cmd: str):
    """Yield each command head in a (possibly compound) shell command string."""
    cmd = cmd.replace("\\\n", " ")     # join line continuations
    cmd = strip_heredocs(cmd)          # drop heredoc bodies (data, not commands)
    cmd = FD_REDIRECT.sub(" ", cmd)    # drop 2>&1, >&2, &>file before splitting
    for segment in OPERATORS.split(cmd):
        head = command_head(segment)
        if head:
            yield head


def bash_commands(obj: dict):
    """Yield Bash tool_use command strings from one transcript JSON object."""
    msg = obj.get("message")
    if not isinstance(msg, dict):
        return
    content = msg.get("content")
    if not isinstance(content, list):
        return
    for block in content:
        if (
            isinstance(block, dict)
            and block.get("type") == "tool_use"
            and block.get("name") == "Bash"
        ):
            command = (block.get("input") or {}).get("command")
            if isinstance(command, str):
                yield command


def main() -> int:
    top_n = int(sys.argv[1]) if len(sys.argv) > 1 else 10
    root = Path(sys.argv[2]) if len(sys.argv) > 2 else Path.home() / ".claude" / "projects"

    files = sorted(root.glob("**/*.jsonl"))
    if not files:
        print(f"No transcripts found under {root}", file=sys.stderr)
        return 1

    counts: Counter[str] = Counter()
    invocations = 0
    for f in files:
        try:
            with f.open(encoding="utf-8", errors="replace") as fh:
                for line in fh:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        obj = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    for cmd in bash_commands(obj):
                        invocations += 1
                        for head in commands_in(cmd):
                            counts[head] += 1
        except OSError:
            continue

    external = Counter({c: n for c, n in counts.items() if c not in BUILTINS})

    def table(title: str, items) -> None:
        print(f"\n{title}")
        print(f"{'#':>3}  {'count':>7}  command")
        print("  " + "-" * 32)
        for i, (cmd, n) in enumerate(items, 1):
            tag = "  [builtin]" if cmd in BUILTINS else ""
            print(f"{i:>3}  {n:>7}  {cmd}{tag}")

    print(f"Scanned {len(files)} transcript(s) under {root}")
    print(f"Bash invocations: {invocations:,}   distinct commands: {len(counts):,}   "
          f"total command uses: {sum(counts.values()):,}")
    table(f"Top {top_n} commands (all)", counts.most_common(top_n))
    table(f"Top {top_n} external commands (conch component candidates)",
          external.most_common(top_n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
