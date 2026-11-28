#!/usr/bin/env python3
"""Removed lines lower the size baseline, so headroom is not kept for later (#311).

`shrink.py` rewrites .drover/arch-baseline.txt with `.gatekeeper/run baseline core/arch` and keeps the rewrite
when it only lowers or drops entries: exit 1, commit it. A rewrite that grows or adds an entry is put back
(core/arch judges growth). `shrink.py DIR` judges DIR/old.txt against DIR/new.txt the same way.
"""

import pathlib
import subprocess
import sys

BASELINE = pathlib.Path(".drover/arch-baseline.txt")


def entries(text):
    return {k: int(n) for k, n in (line.rsplit(" ", 1) for line in text.splitlines() if line.strip())}


def lowered(old, new):
    o, n = entries(old), entries(new)
    return o != n and all(k in o and v <= o[k] for k, v in n.items())


def baseline():
    old = BASELINE.read_text() if BASELINE.exists() else ""
    subprocess.run([".gatekeeper/run", "baseline", "core/arch"], check=True, stdout=subprocess.DEVNULL)
    new = BASELINE.read_text() if BASELINE.exists() else ""
    if lowered(old, new):
        print(f"lowered {BASELINE}: commit it")
        print("\n".join(sorted(set(old.splitlines()) - set(new.splitlines()))))
        return 1
    BASELINE.write_text(old)
    return 0


def main(args):
    if args:
        d = pathlib.Path(args[0])
        return 1 if lowered((d / "old.txt").read_text(), (d / "new.txt").read_text()) else 0
    return baseline()


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
