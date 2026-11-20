#!/usr/bin/env python3
"""The code-search ADR's benchmark: the prototype against rg, time and output size per query.

    rustc --edition 2024 -O -o "$TMPDIR/proto" docs/architecture/code-search/bench/prototype.rs
    git archive ce4424e | tar -x -C "$TMPDIR/shed-at-base"
    python3 -I docs/architecture/code-search/bench/bench.py "$TMPDIR/proto" "$TMPDIR/shed-at-base" [BIG_TREE]

Each command runs RUNS times, the two tools interleaved; a cell is the median wall time of the whole
process, then the median CPU time (user and system, every thread) in brackets, which a busy host moves
far less. Every grep row also checks that both tools matched the same set of path:line pairs.
"""

import json
import os
import resource
import statistics
import subprocess
import sys
import time

RUNS = 15

GREPS = [
    ("literal, rare", "Pattern::new", ["-F"]),
    ("literal, common", "spawn_blocking", ["-F"]),
    ("literal, whole word", "wait", ["-F", "-w", "-g", "*.rs", "-g", "!tests/"]),
    ("literal, any case", "handover_refused", ["-F", "-i"]),
    ("pattern with a literal in it", r"fn \w+_limit", []),
    ("alternation of literals", "TODO|FIXME|HACK", []),
    ("group and alternation", r"Method::(Exec|FileRead|FileWrite)\b", []),
    ("counted repeat, no literal", r"\d{2,}\.\d+\.\d+", ["-g", "*.toml"]),
    ("anchored, optional group", r"^\s*pub (async )?fn [a-z_]+\(&mut self", []),
    ("no literal: every line tried", r"[0-9a-f]{40}", []),
    ("no match", "zzzz_not_here", ["-F"]),
]


def cpu():
    used = resource.getrusage(resource.RUSAGE_CHILDREN)
    return (used.ru_utime + used.ru_stime) * 1000


def run(cmd, cwd):
    start, before = time.perf_counter(), cpu()
    done = subprocess.run(cmd, cwd=cwd, capture_output=True)
    return ((time.perf_counter() - start) * 1000, cpu() - before), done


def median(cmds, cwd):
    wall, used = [[] for _ in cmds], [[] for _ in cmds]
    for _ in range(RUNS):
        for i, cmd in enumerate(cmds):
            took = run(cmd, cwd)[0]
            wall[i].append(took[0])
            used[i].append(took[1])
    return [f"{statistics.median(w):.1f} ({statistics.median(u):.1f}) ms" for w, u in zip(wall, used)]


def rg_lines(out):
    hits = set()
    for line in out.decode("utf-8", "replace").splitlines():
        path, n, _ = line.split(":", 2)
        hits.add((path, int(n)))
    return hits


def proto_lines(out):
    hits = set()
    for f in json.loads(out)["files"]:
        for g in f["groups"]:
            hits.update((f["path"], line["n"]) for line in g["lines"])
    return hits


def size(n):
    return f"{n:,}"


def main():
    proto, root = sys.argv[1], sys.argv[2]
    rg = ["rg", "--no-require-git", "-n", "--no-heading", "--color", "never"]
    print(f"load average {os.getloadavg()[0]:.1f}, {RUNS} runs each\n")
    print("| query | flags | lines / files | rg | rg -j1 | prototype | rg bytes | text bytes | json bytes | same lines |")
    print("|---|---|---|---|---|---|---|---|---|---|")
    for label, query, flags in GREPS:
        theirs = rg + flags + ["-e", query]
        ours = [proto, "grep"] + flags + [query, "."]
        _, a = run(theirs, root)
        _, text = run(ours, root)
        _, js = run(ours + ["--json"], root)
        if text.returncode:
            print(f"| {label} | `{query}` | | | | | | {text.stderr.decode().strip()} | | |")
            continue
        want, got = rg_lines(a.stdout), proto_lines(js.stdout)
        files = len({p for p, _ in got})
        t = median([theirs, theirs + ["-j1"], ours], root)
        same = "yes" if want == got else f"NO ({len(want ^ got)} differ)"
        flag = " ".join(flags)
        print(
            f"| {label} | `{query}` {flag} | {len(got)} / {files} | {t[0]} | {t[1]} | {t[2]} "
            f"| {size(len(a.stdout))} | {size(len(text.stdout))} | {size(len(js.stdout))} | {same} |"
        )

    print("\n| other | rg or cat | prototype | their bytes | text bytes | json bytes |")
    print("|---|---|---|---|---|---|")
    over = [proto, "grep", "-F", "self", "."]
    _, refused = run(over, root)
    count = over + ["--count"]
    theirs = rg + ["-F", "-c", "-e", "self"]
    _, a = run(theirs, root)
    _, b = run(count, root)
    _, bj = run(count + ["--json"], root)
    t = median([theirs, count], root)
    print(f"| over the limit: `self` -F | | exit {refused.returncode}: `{refused.stderr.decode().strip()}` | | | |")
    print(f"| the same with `--count` | {t[0]} | {t[1]} | {size(len(a.stdout))} | {size(len(b.stdout))} | {size(len(bj.stdout))} |")
    for path in ["crates/shed-server/src/registry.rs", "docs/protocol.md"]:
        theirs, ours = ["cat", path], [proto, "outline", path]
        _, a = run(theirs, root)
        _, b = run(ours, root)
        _, bj = run(ours + ["--json"], root)
        t = median([theirs, ours], root)
        print(f"| outline `{path}` | {t[0]} | {t[1]} | {size(len(a.stdout))} | {size(len(b.stdout))} | {size(len(bj.stdout))} |")
    for query in ["handover", "line cap", "wait output pattern"]:
        theirs = ["rg", "--no-require-git", "-l", "-i", "-e", query.replace(" ", "|")]
        ours = [proto, "find", query, "."]
        _, a = run(theirs, root)
        _, b = run(ours, root)
        _, bj = run(ours + ["--json"], root)
        t = median([theirs, ours], root)
        print(f"| find `{query}` (rg -l -i) | {t[0]} | {t[1]} | {size(len(a.stdout))} | {size(len(b.stdout))} | {size(len(bj.stdout))} |")

    if len(sys.argv) > 3:
        big = sys.argv[3]
        print("\n| big tree | rg | rg -j1 | prototype, limits off | prototype, limits on |")
        print("|---|---|---|---|---|")
        for query, flags in [("spawn_blocking", ["-F"]), ("fn poll_", ["-F"]), ("[0-9a-f]{40}", [])]:
            theirs = ["rg", "--no-require-git", "-c"] + flags + ["-e", query]
            ours = [proto, "grep", "--count"] + flags + [query, "."]
            cells = []
            for cmd in [theirs, theirs + ["-j1"], ours + ["--no-limits"], ours]:
                (wall, used), done = run(cmd, big)
                out = done.stdout.decode("utf-8", "replace").splitlines()
                took = f"{wall / 1000:.1f} ({used / 1000:.1f}) s"
                if done.returncode:
                    cells.append(f"{took}, exit {done.returncode}: `{done.stderr.decode().strip()}`")
                else:
                    files = len(out) - (0 if cmd[0] == "rg" else 1)
                    cells.append(f"{took}, {files} files")
            print(f"| `{query}` {' '.join(flags)} | " + " | ".join(cells) + " |")


main()
