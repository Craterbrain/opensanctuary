#!/usr/bin/env python3
"""Refresh the `_Last edited: ..._` line of Markdown files.

Default (used by .githooks/pre-commit): stamp now, since the commit being made
is the edit. `--from-git` instead stamps each file's last commit date, except
files with real uncommitted changes (ignoring this line), which get now.
A date-only line stays date-only; a line with a time keeps a time.

    tools/update_last_edited.py [--from-git] FILE...
"""
import re
import subprocess
import sys
from datetime import datetime
from pathlib import Path

LINE = re.compile(r"^_Last edited: (\d{4}-\d{2}-\d{2})( \d{2}:\d{2})?_$")


def git(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(["git", *args], capture_output=True, text=True)


def git_stamp(path: Path) -> datetime:
    if git("ls-files", "--error-unmatch", str(path)).returncode == 0:
        if git("diff", "HEAD", "--quiet", "-I^_Last edited", "--", str(path)).returncode == 0:
            out = git("log", "-1", "--format=%cI", "--", str(path)).stdout.strip()
            if out:
                return datetime.fromisoformat(out).replace(tzinfo=None)
    return datetime.now()


def refresh(path: Path, from_git: bool) -> bool:
    lines = path.read_text(encoding="utf-8").split("\n")
    for i, line in enumerate(lines):
        m = LINE.match(line)
        if not m:
            continue
        when = git_stamp(path) if from_git else datetime.now()
        new = f"_Last edited: {when.strftime('%Y-%m-%d %H:%M' if m.group(2) else '%Y-%m-%d')}_"
        if new == line:
            return False
        lines[i] = new
        path.write_text("\n".join(lines), encoding="utf-8")
        return True
    return False


if __name__ == "__main__":
    args = sys.argv[1:]
    from_git = "--from-git" in args
    for arg in (a for a in args if a != "--from-git"):
        if refresh(Path(arg), from_git):
            print(f"updated {arg}")
