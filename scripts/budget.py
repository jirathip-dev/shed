#!/usr/bin/env python3
"""The test time budget (`just budget`): each crate's tests within `[package.metadata.gate] test_secs`.

Builds every test binary of the workspace under ROOT (default .), then runs them crate by crate from the
crate's folder, as `cargo test` does. Fails when a test fails (naming each failing test), a crate under
ROOT/crates states no budget, or a crate's tests use more CPU seconds (user and system, of the test
binaries and the processes they wait for) than it. CPU time, not wall time: a busy host slows a test but does not make it use more
CPU, so load cannot fail the budget (#310).
"""

import glob
import json
import os
import re
import resource
import subprocess
import sys
import time


def cpu():
    used = resource.getrusage(resource.RUSAGE_CHILDREN)
    return used.ru_utime + used.ru_stime


FAILED = re.compile(r"^test (\S+) \.\.\. FAILED$", re.M)
BUDGET = re.compile(r"^\[package\.metadata\.gate\]\n(?:[^\[\n].*\n|\n)*?test_secs *= *([0-9.]+)", re.M)

root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else ".")
locked = ["--locked"] if os.path.exists(os.path.join(root, "Cargo.lock")) else []
build = ["cargo", "test", "--workspace", "--no-run", "--message-format=json-render-diagnostics", *locked]
built = subprocess.run(build, cwd=root, stdout=subprocess.PIPE, text=True)
if built.returncode != 0:
    sys.exit("budget: the tests did not build")
binaries = {}
for line in built.stdout.splitlines():
    msg = json.loads(line)
    if msg.get("reason") == "compiler-artifact" and msg["profile"]["test"] and msg.get("executable"):
        binaries.setdefault(os.path.dirname(msg["manifest_path"]), []).append(msg["executable"])

failed = []
for manifest in sorted(glob.glob(os.path.join(root, "crates/*/Cargo.toml"))):
    crate, name = os.path.dirname(manifest), os.path.relpath(manifest, root)
    found = BUDGET.search(open(manifest).read())
    if not found:
        failed.append(f"{name}: no [package.metadata.gate] test_secs")
        continue
    started, cpu_started = time.monotonic(), cpu()
    for exe in binaries.get(crate, []):
        ran = subprocess.run([exe], cwd=crate, stdout=subprocess.PIPE, text=True)
        print(ran.stdout, end="", flush=True)
        if ran.returncode != 0:
            tests = ", ".join(FAILED.findall(ran.stdout)) or "no test result: it crashed"
            failed.append(f"{name}: {os.path.basename(exe)} failed: {tests}")
    used, took, limit = cpu() - cpu_started, time.monotonic() - started, float(found.group(1))
    print(f"budget: {name}: {used:.1f} CPU s of {limit:g} ({took:.1f} s wall, reported only)")
    if used > limit:
        failed.append(f"{name}: the tests used {used:.1f} CPU s, over test_secs = {found.group(1)}")

for line in failed:
    print(f"budget: {line}")
sys.exit(1 if failed else 0)
