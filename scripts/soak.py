#!/usr/bin/env python3
"""The nightly soak on real machines (#308), timed against the targets in ci/soak.toml.

Usage: scripts/soak.py [--config FILE] [--out DIR] LABEL[=SSH-TARGET]...

A LABEL alone is this machine; LABEL=TARGET is a machine reached over ssh. The soak starts a shed of its own
(the `shed` on PATH) in a temp folder, adds each TARGET to it as a machine, and measures on each machine:

  link_up_s    from `machine add` to the link reading up
  start_ms     the slowest tenth of `run` answers, over the throwaway terminals
  read_ms      the slowest tenth from `run` to the terminal's output read back
  dead_link_s  from freezing every link's ssh at once (SIGSTOP, as a dead network) to `machine ls` reading
               the link not up, while a caller lists terminals
  stuck        terminals still running 10 s after `kill`

It writes DIR/soak.json (the numbers), DIR/soak.md (each number against its target) and DIR/misses.txt (one
`LABEL TARGET VALUE LIMIT` line a miss), and exits 1 on a miss. Every terminal, ssh and folder it made is
gone when it ends.
"""

import json
import math
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

WAIT = 60


def config(path):
    found, table = {}, ""
    with open(path) as f:
        for raw in f:
            line = raw.split("#", 1)[0].strip()
            if line.startswith("["):
                table = line.strip("[] ") + "."
            elif line:
                key, _, value = line.partition("=")
                found[table + key.strip()] = float(value)
    return found


def kill(pids):
    for pid in pids:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


class Soak:
    def __init__(self, root):
        self.dir = os.path.join(root, "local")
        self.serve = subprocess.Popen(
            ["shed", "serve", "--dir", self.dir], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        self.until(lambda: self.shed("ls").returncode == 0, 10)

    def shed(self, *args):
        try:
            return subprocess.run(["shed", "--dir", self.dir, *args], capture_output=True, text=True, timeout=WAIT)
        except subprocess.TimeoutExpired:
            return subprocess.CompletedProcess(args, 124, "", "timed out")

    def until(self, check, limit):
        started = time.monotonic()
        while time.monotonic() - started < limit:
            if check():
                return time.monotonic() - started
            time.sleep(0.2)
        return None

    def state(self, label):
        listed = self.shed("--json", "machine", "ls").stdout or "[]"
        return next((m.get("link", {}).get("state") for m in json.loads(listed) if m["name"] == label), None)

    def terminals(self, label, n, remote):
        on = ["--on", label] if remote else []
        name = lambda i: f"soak-{os.getpid()}-{i}"
        full = lambda i: f"{label}/{name(i)}" if remote else name(i)
        starts, reads, stuck = [], [], 0
        for i in range(n):
            started = time.monotonic()
            cmd = f"echo {name(i)}-ready; exec sleep 600"
            if self.shed(*on, "run", "--name", name(i), "--cwd", "/tmp", "--", "sh", "-c", cmd).returncode:
                continue
            starts.append(time.monotonic() - started)
            if self.until(lambda: f"{name(i)}-ready" in self.shed("read", full(i)).stdout, WAIT) is not None:
                reads.append(time.monotonic() - started)
        for i in range(n):
            self.shed("kill", full(i))
        for i in range(n):
            exited = lambda: json.loads(self.shed("--json", "get", full(i)).stdout or "{}").get("state") == "exited"
            if self.shed("get", full(i)).returncode == 0 and self.until(exited, 10) is None:
                stuck += 1
            self.shed("rm", full(i))
        tenth = lambda xs: round(1000 * sorted(xs)[math.ceil(0.9 * len(xs)) - 1]) if len(xs) == n else None
        return {"start_ms": tenth(starts), "read_ms": tenth(reads), "stuck": stuck}

    def ssh(self):
        listed = subprocess.run(["ps", "-Aww", "-o", "pid=,command="], capture_output=True, text=True).stdout
        for line in listed.splitlines():
            pid, _, command = line.strip().partition(" ")
            if command.startswith("ssh") and self.dir in command:
                yield int(pid)

    def dead_links(self, results):
        linked = [label for label, numbers in results.items() if numbers.get("link_up_s")]
        frozen = list(self.ssh())
        for pid in frozen:
            os.kill(pid, signal.SIGSTOP)
        started, left = time.monotonic(), set(linked)

        def noticed():
            self.shed("ls")
            for label in [label for label in left if self.state(label) != "up"]:
                results[label]["dead_link_s"] = round(time.monotonic() - started, 1)
                left.discard(label)
            return not left

        self.until(noticed, 120)
        kill(frozen)

    def machine(self, label, target, n):
        if target is None:
            return {"link_up_s": 0, "dead_link_s": 0, **self.terminals(label, n, False)}
        if self.shed("machine", "add", label, target).returncode:
            return {}
        up = self.until(lambda: self.state(label) == "up", WAIT)
        if up is None:
            return {"link_up_s": None}
        return {"link_up_s": max(round(up, 1), 0.1), **self.terminals(label, n, True)}

    def end(self):
        self.serve.terminate()
        self.serve.wait()
        kill(self.ssh())


def report(targets, results, out):
    misses, rows = [], ["| machine | " + " | ".join(targets) + " |", "|---" * (len(targets) + 1) + "|"]
    for label, numbers in results.items():
        cells = []
        for target, limit in targets.items():
            value = numbers.get(target)
            missed = value is None or value > limit
            if missed:
                misses.append(f"{label} {target} {value} {limit:g}")
            cells.append(f"{'**miss** ' if missed else ''}{value} (≤ {limit:g})")
        rows.append(f"| {label} | " + " | ".join(cells) + " |")
    os.makedirs(out, exist_ok=True)
    version = subprocess.run(["shed", "--version"], capture_output=True, text=True).stdout.strip()
    with open(os.path.join(out, "soak.json"), "w") as f:
        json.dump({"at": int(time.time()), "shed": version, "machines": results}, f, indent=2)
    with open(os.path.join(out, "soak.md"), "w") as f:
        f.write(f"## Soak: {version}\n\n" + "\n".join(rows) + "\n")
    with open(os.path.join(out, "misses.txt"), "w") as f:
        f.write("".join(m + "\n" for m in misses))
    return misses


def main(args):
    path, out = "ci/soak.toml", "soak"
    while args[:1] in (["--config"], ["--out"]):
        if args[0] == "--config":
            path = args[1]
        else:
            out = args[1]
        args = args[2:]
    found = config(path)
    targets = {k.split(".", 1)[1]: v for k, v in found.items() if k.startswith("targets.")}
    root = tempfile.mkdtemp(prefix="shed-soak-", dir="/tmp")
    soak, results = None, {}
    try:
        soak = Soak(root)
        for arg in args:
            label, _, target = arg.partition("=")
            results[label] = soak.machine(label, target or None, int(found["terminals"]))
        soak.dead_links(results)
    finally:
        if soak:
            soak.end()
        shutil.rmtree(root, ignore_errors=True)
    misses = report(targets, results, out)
    print(open(os.path.join(out, "soak.md")).read())
    print("".join(f"miss: {m}\n" for m in misses), end="")
    return 1 if misses else 0


sys.exit(main(sys.argv[1:]))
