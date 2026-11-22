#!/usr/bin/env bash
# Target 1 of the script ADR (docs/architecture/script/adr.md, "Performance"): the blocked-lanes example on a
# real remote host, both ways. Makes 8 terminals on <machine> labelled fleet=perf, each reported blocked, then
# runs RUNS times (10 unless given) with no link held, so each `shed --on` call is its own `shed connect`
# over SSH: `ls` then a `read` per lane (the per-call path), and `script -f blocked.js` (one request).
# An `ssh` shim first on PATH counts the sessions. Prints the median and range of each, then removes the
# terminals. <machine> is a name from `shed machine ls`; SHED picks the local binary (default: shed on PATH).
set -euo pipefail
[ $# -ge 1 ] || { echo "usage: $0 <machine> [runs]" >&2; exit 2; }
root=$(cd "$(dirname "$0")/.." && pwd)
exec python3 -I - "$root" "$@" <<'PY'
import json, os, shutil, statistics, subprocess, sys, tempfile, time

root, machine = sys.argv[1], sys.argv[2]
runs = int(sys.argv[3]) if len(sys.argv) > 3 else 10
shed = shutil.which(os.environ.get("SHED", "shed"))
real_ssh = shutil.which("ssh")
listed = json.loads(subprocess.run([shed, "machine", "ls", "--json"], check=True, capture_output=True).stdout)
target = next((m["target"] for m in listed if m["name"] == machine), None)
if target is None:
    sys.exit(f"{machine}: not in `shed machine ls`")
work = tempfile.mkdtemp(prefix="shed-script-remote-")
os.makedirs(f"{work}/bin")
with open(f"{work}/config.toml", "w") as f:
    f.write(f'[machines]\n{machine} = "{target}"\n')
with open(f"{work}/bin/ssh", "w") as f:
    f.write(f'#!/bin/sh\necho x >> "{work}/sessions"\nexec "{real_ssh}" "$@"\n')
os.chmod(f"{work}/bin/ssh", 0o755)
env = dict(os.environ, PATH=f"{work}/bin:{os.environ['PATH']}")
names = [f"perf-{i}" for i in range(8)]


def on(*args):
    out = subprocess.run([shed, "--dir", work, "--on", machine, *args], env=env, capture_output=True)
    if out.returncode != 0:
        sys.exit(f"shed --on {machine} {' '.join(args)}: exit {out.returncode}: {out.stderr.decode().strip()}")
    return out.stdout


def sessions():
    try:
        with open(f"{work}/sessions") as f:
            return len(f.readlines())
    except FileNotFoundError:
        return 0


def timed(way):
    before, start = sessions(), time.perf_counter()
    way()
    return (time.perf_counter() - start) * 1000, sessions() - before


def per_call():
    on("ls", "--label", "fleet=perf", "--json")
    for name in names:
        on("read", name, "--lines", "15")


def script():
    if not os.path.exists(f"{work}/blocked.js"):
        example = open(f"{root}/skill/script/examples/blocked.js").read()
        with open(f"{work}/blocked.js", "w") as f:
            f.write(example.replace('fleet: "shed"', 'fleet: "perf"'))
    reply = json.loads(on("script", "-f", f"{work}/blocked.js", "--json"))
    if len(reply.get("result") or []) != len(names):
        sys.exit(f"script: {reply}")


def row(what, samples):
    ms = [s[0] for s in samples]
    count = sorted({s[1] for s in samples})
    print(f"{what:<44} median {statistics.median(ms):>9.1f} ms   range {min(ms):.1f} to {max(ms):.1f}   ssh sessions {count}")


try:
    for name in names:
        script_line = "printf 'Bash(cargo test --workspace --locked)\\nDo you want to proceed?\\n> 1. Yes   2. No\\n'; exec cat >/dev/null"
        on("run", "--name", name, "--cwd", "/", "--label", "fleet=perf", "--", "sh", "-c", script_line)
        on("state", name, "blocked", "--note", "Bash: cargo test")
    print(f"{machine}: {runs} runs each, 8 blocked lanes, no link held")
    row("one remote ls", [timed(lambda: on("ls", "--label", "fleet=perf", "--json")) for _ in range(runs)])
    row("per call: ls and 8 reads, 9 shed commands", [timed(per_call) for _ in range(runs)])
    row("script: script -f blocked.js, 1 command", [timed(script) for _ in range(runs)])
finally:
    for name in names:
        for verb in ("kill", "rm"):
            subprocess.run([shed, "--dir", work, "--on", machine, verb, name], env=env, capture_output=True)
    subprocess.run([real_ssh, "-o", f"ControlPath={work}/ssh-%C", "-O", "exit", target], capture_output=True)
    shutil.rmtree(work, ignore_errors=True)
PY
