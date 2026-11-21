#!/usr/bin/env python3
"""A test must not assert that something took less than a wall-clock time (#245).

Such a ceiling holds on an idle laptop and goes red on Linux CI or a loaded host
(#164's first-token time, #216's `kill < 500 ms`). `wall-clock-bound.py [--base REV]` reads the lines
added under crates/*/tests since REV (ARCH_BASE, else origin/staging's merge base, else HEAD) and
prints `file:line` for each `assert!` that puts a measured time below a bound.
"""

import os
import re
import subprocess
import sys

TIME = re.compile(r"\.elapsed\(\)|\btook\b|\bwaited\b|_ms\b|\bms\b|_secs\b")
CEILING = re.compile(r"<=?\s*(std::time::)?(Duration::from_|[\d_.]+\b)")


def base(args):
    if len(args) > 1 and args[0] == "--base":
        return args[1]
    if os.environ.get("ARCH_BASE"):
        return os.environ["ARCH_BASE"]
    merge = subprocess.run(["git", "merge-base", "HEAD", "origin/staging"], capture_output=True, text=True)
    return merge.stdout.strip() or "HEAD"


def added(rev):
    diff = subprocess.run(
        ["git", "diff", "-U0", "--no-color", rev, "--", "crates/*/tests/*.rs"],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    path, line = None, 0
    for text in diff.splitlines():
        if text.startswith("+++ "):
            path = text[6:] if text.startswith("+++ b/") else None
        elif text.startswith("@@"):
            line = int(re.search(r"\+(\d+)", text).group(1))
        elif text.startswith("+") and path:
            yield path, line, text[1:]
            line += 1


def ceilings(lines):
    opened = False
    for path, line, text in lines:
        if (opened or "assert!(" in text) and TIME.search(text) and CEILING.search(text):
            yield path, line, text.strip()
        opened = text.rstrip().endswith("assert!(")


def main(args):
    rev = base(args)
    hits = list(ceilings(added(rev)))
    for path, line, text in hits:
        print(f"{path}:{line}: {text}")
    if hits:
        print(
            "wall-clock-bound: a test asserts a time ceiling, which goes red on a loaded host or Linux CI;"
            " assert the outcome or the order instead, drive a paused tokio clock (shed_core::clock::tokio_now),"
            " or report the time from an #[ignore]d report test"
        )
        return 1
    print(f"wall-clock-bound: no new time ceiling in a test since {rev}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
