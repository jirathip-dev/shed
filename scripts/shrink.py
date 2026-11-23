#!/usr/bin/env python3
"""Removed lines lower the size baseline and the cap, so headroom is not kept for later (#311).

`shrink.py [--base REV]` rewrites .drover/arch-baseline.txt with `drover arch-check --write-baseline` and keeps
the rewrite when it only lowers or drops entries: exit 1, commit it. A rewrite that grows or adds an entry is
put back (arch-check judges growth). With --base it compares the counted lines of ci/cap.toml at REV and now,
and when they fell it says the cap to lower to: lanes may not edit ci/cap.toml, so the orchestrator files it.
`shrink.py DIR` judges DIR/old.txt against DIR/new.txt the same way.
"""

import os
import pathlib
import re
import subprocess
import sys
import tempfile

BASELINE = pathlib.Path(".drover/arch-baseline.txt")
DROVER = os.environ.get("DROVER", "drover")
COUNT = re.compile(r"src lines \(formatted\): (\d+) / cap (\d+)")


def entries(text):
    return {k: int(n) for k, n in (line.rsplit(" ", 1) for line in text.splitlines() if line.strip())}


def lowered(old, new):
    o, n = entries(old), entries(new)
    return o != n and all(k in o and v <= o[k] for k, v in n.items())


def baseline():
    old = BASELINE.read_text() if BASELINE.exists() else ""
    subprocess.run([DROVER, "arch-check", "--write-baseline"], check=True, stdout=subprocess.DEVNULL)
    new = BASELINE.read_text()
    if lowered(old, new):
        print(f"lowered {BASELINE}: commit it")
        print("\n".join(sorted(set(old.splitlines()) - set(new.splitlines()))))
        return 1
    BASELINE.write_text(old)
    return 0


def count(root):
    out = subprocess.run(
        [DROVER, "cap-check", "--config", "ci/cap.toml"], cwd=root, capture_output=True, text=True
    ).stdout
    found = COUNT.search(out)
    if not found:
        sys.exit(f"shrink: no count in drover cap-check's output at {root}:\n{out}")
    return int(found[1]), int(found[2])


def cap(rev):
    with tempfile.TemporaryDirectory(prefix="shed-shrink-") as tmp:
        tree = subprocess.run(["git", "archive", rev], check=True, capture_output=True).stdout
        subprocess.run(["tar", "-x", "-C", tmp], input=tree, check=True)
        base_lines, base_cap = count(tmp)
    lines, now = count(".")
    removed = base_lines - lines
    if removed > 0 and now > base_cap - removed:
        print(
            f"the counted lines fell by {removed} since {rev} ({base_lines} -> {lines}): "
            f"lower the cap to {base_cap - removed} (now {now}); the orchestrator files it"
        )


def main(args):
    if args and args[0] != "--base":
        d = pathlib.Path(args[0])
        return 1 if lowered((d / "old.txt").read_text(), (d / "new.txt").read_text()) else 0
    status = baseline()
    if len(args) == 2:
        cap(args[1])
    return status


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
