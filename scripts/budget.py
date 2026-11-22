#!/usr/bin/env python3
"""The test time budget (`just budget`): each crate's tests within `[package.metadata.gate] test_secs`.

Builds every test binary of the workspace under ROOT (default .), then runs them crate by crate from the
crate's folder, as `cargo test` does, timing each crate. Fails when a test fails, a crate under
ROOT/crates states no budget, or a crate's tests take longer than it. The budget scales with the host's
load per CPU (1-minute average, the higher of before and after the crate), never below itself.
"""

import glob
import json
import os
import re
import subprocess
import sys
import time

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
    load, started = os.getloadavg()[0], time.monotonic()
    for exe in binaries.get(crate, []):
        if subprocess.run([exe], cwd=crate).returncode != 0:
            failed.append(f"{name}: {os.path.basename(exe)} failed")
    took = time.monotonic() - started
    scale = max(1.0, max(load, os.getloadavg()[0]) / (os.cpu_count() or 1))
    limit = float(found.group(1)) * scale
    print(f"budget: {name}: {took:.1f} s of {limit:.1f} s (test_secs {found.group(1)} at load x{scale:.1f})")
    if took > limit:
        failed.append(f"{name}: the tests took {took:.1f} s, over test_secs = {found.group(1)} at load x{scale:.1f}")

for line in failed:
    print(f"budget: {line}")
sys.exit(1 if failed else 0)
