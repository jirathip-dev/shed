#!/usr/bin/env python3
"""Measure Codex sessions five ways (#128), as scripts/measure-claude-direct.py did for Claude (#133):

- acp (npx): the `codex-acp` adapter as shed runs it today (`npx -y`), one process tree per session; the
  adapter is Node and starts its own `codex app-server` for its one session;
- acp (node): the same adapter installed once and run by `node`, as #170 runs npx agents;
- app-server: one `codex app-server`, each session a thread on it;
- exec: `codex exec --json`, the JSON-lines mode a stream-driver provider would run, one process per turn;
- lane: real `shed` lanes on codex-acp (their own server in a scratch folder), hibernated.

For 1, 5 and 10 sessions, each given one tiny turn: the processes, their summed RSS and their macOS physical
footprint 2 s after the last turn (exec has none between turns; its row is the peak of one turn's process).
Times, from the first session: ready (process start to the session id), first update (prompt sent to the
first text, thought or tool item), and the wake: the sessions stopped and one started again on the same
session (ACP `session/load`, `thread/resume` on a fresh server, `codex exec resume`), from that start to the
next turn's first update. app-server's warm wake is `thread/unsubscribe` and `thread/resume` on the running
server. The lane mode starts 1, 5 and 10 lanes, gives each one turn, waits until each is hibernated, sums
their bridges' memory and times a send to the first update. Medians over --runs. Every turn uses the
account `codex login` signed in to.

    scripts/measure-codex.py [--runs 3] [--adapter @agentclientprotocol/codex-acp@2.12.0] [--shed shed]
"""

import argparse
import importlib.util
import json
import os
import platform
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location("direct", os.path.join(HERE, "measure-claude-direct.py"))
d = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(d)

ADAPTER = "@agentclientprotocol/codex-acp@2.12.0"
COUNTS = (1, 5, 10)
FIRST, AGAIN = d.FIRST, d.AGAIN


def footprint(pid):
    """macOS's physical footprint in KiB (compressed pages included, which RSS leaves out); None elsewhere."""
    if sys.platform != "darwin":
        return None
    out = subprocess.run(["footprint", "-p", str(pid)], capture_output=True, text=True).stdout
    for line in out.splitlines():
        if "Footprint:" in line:
            num, unit = line.split("Footprint:")[1].split()[:2]
            return round(float(num) * {"B": 1 / 1024, "KB": 1, "MB": 1024, "GB": 1024 * 1024}[unit])
    return None


def snap(*roots):
    """Every process under `roots`: (pid, RSS KiB, footprint KiB, command)."""
    return [(pid, rss, footprint(pid), cmd) for root in roots for pid, rss, cmd in d.tree(root)]


def kib(procs, col=1):
    return sum(p[col] or 0 for p in procs)


def acp_run(argv, cwd):
    out, sessions = {}, []
    for n in COUNTS:
        while len(sessions) < n:
            s = d.Acp(argv, cwd)
            ready, sid = s.start()
            sent = time.monotonic() - s.a.began
            first = s.turn(sid, FIRST) - sent
            sessions.append((s, sid))
            if len(sessions) == 1:
                out["ready"], out["first"] = ready, first
        time.sleep(2)
        out[n] = snap(*(s.a.proc.pid for s, _ in sessions))
    for s, _ in sessions:
        s.a.stop()
    s = d.Acp(argv, cwd)
    s.start(sessions[0][1])
    out["wake"] = s.turn(sessions[0][1], AGAIN)
    s.a.stop()
    return out


class AppServer(d.Acp):
    def __init__(self, codex, cwd):
        super().__init__([codex, "app-server"], cwd)
        self.call("initialize", {"clientInfo": {"name": "measure-codex", "version": "0"}})
        self.a.send({"method": "initialized"})

    def thread(self, tid=None):
        method, params = ("thread/resume", {"threadId": tid}) if tid else ("thread/start", {})
        at, result, _ = self.call(method, {**params, "cwd": self.cwd})
        return at, result["thread"]["id"]

    def turn(self, tid, text):
        mark, sent = len(self.a.lines), time.monotonic() - self.a.began
        threading.Thread(target=self.call, args=("turn/start", {
            "threadId": tid, "input": [{"type": "text", "text": text}]}), daemon=True).start()
        mine = lambda m: m.get("params", {}).get("threadId") == tid
        _, first, _ = self.a.until(lambda m: mine(m) and (m.get("method") == "item/agentMessage/delta" or (
            m.get("method") == "item/started" and m["params"]["item"]["type"] != "userMessage")), mark)
        self.a.until(lambda m: mine(m) and m.get("method") == "turn/completed", mark)
        return sent, first


def server_run(codex, cwd):
    out, threads = {}, []
    s = AppServer(codex, cwd)
    for n in COUNTS:
        while len(threads) < n:
            ready, tid = s.thread()
            sent, first = s.turn(tid, FIRST)
            threads.append(tid)
            if len(threads) == 1:
                out["ready"], out["first"] = ready, first - sent
        time.sleep(2)
        out[n] = snap(s.a.proc.pid)
    s.call("thread/unsubscribe", {"threadId": threads[0]})
    began = time.monotonic() - s.a.began
    s.thread(threads[0])
    _, first = s.turn(threads[0], AGAIN)
    out["warm wake"] = first - began
    s.a.stop()
    s = AppServer(codex, cwd)
    s.thread(threads[0])
    out["wake"] = s.turn(threads[0], AGAIN)[1]
    s.a.stop()
    return out


def exec_turn(codex, cwd, text, session=None):
    argv = [codex, "exec"] + (["resume", session] if session else []) + ["--json", "--skip-git-repo-check", text]
    a = d.Agent(argv, cwd)
    a.proc.stdin.close()
    peak = []

    def sample():
        while a.proc.poll() is None:
            procs = snap(a.proc.pid)
            if kib(procs) > kib(peak):
                peak[:] = procs
            time.sleep(0.1)

    threading.Thread(target=sample, daemon=True).start()
    _, ready, started = a.until(lambda m: m.get("type") == "thread.started")
    _, first, _ = a.until(lambda m: m.get("type") in ("item.started", "item.completed"))
    a.until(lambda m: m.get("type") == "turn.completed")
    a.proc.wait()
    return ready, first, started["thread_id"], peak


def exec_run(codex, cwd):
    ready, first, session, peak = exec_turn(codex, cwd, FIRST)
    _, wake, _, _ = exec_turn(codex, cwd, AGAIN, session)
    return {"ready": ready, "first": first - ready, "wake": wake, 1: peak}


def lane_run(shed, argv, cwd, tmp, run):
    sd = os.path.join(tmp, f"s{run}")
    os.makedirs(sd)
    cli = lambda *a: subprocess.run([shed, "--dir", sd, *a], env=d.env(), capture_output=True, text=True,
                                    check=True)
    server = subprocess.Popen([shed, "--dir", sd, "serve"], env={**d.env(), "SHED_ACP_HIBERNATE_AFTER": "5"},
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    while not os.path.exists(os.path.join(sd, "shed.sock")):
        time.sleep(0.05)
    events = d.Agent([shed, "--dir", sd, "events", "--json"], cwd)
    out, lanes = {}, []
    try:
        for n in COUNTS:
            while len(lanes) < n:
                name = f"l{len(lanes)}"
                began = time.monotonic()
                cli("run", "--acp", "codex-acp", "--policy", "allow", "--name", name, "--cwd", cwd, "--", *argv)
                cli("wait", name, "--output", "ACP session ready", "--timeout", "120")
                out.setdefault("ready", time.monotonic() - began)
                cli("send", name, FIRST)
                cli("wait", name, "--until", "done", "--timeout", "300")
                lanes.append(name)
            for name in lanes:
                cli("wait", name, "--until", "hibernated", "--timeout", "120")
            out[n] = snap(*(json.loads(cli("get", name, "--json").stdout)["pid"] for name in lanes))
        mark, sent = len(events.lines), time.monotonic()
        cli("send", lanes[0], AGAIN)
        _, at, _ = events.until(lambda m: m.get("event") in ("text", "tool_call", "plan"), mark)
        out["wake"] = at + events.began - sent
        cli("wait", lanes[0], "--until", "done", "--timeout", "300")
        return out
    finally:
        events.stop()
        for name in lanes:
            subprocess.run([shed, "--dir", sd, "kill", name], env=d.env(), capture_output=True)
        server.terminate()
        server.wait()


def report(mode, rs, tmp):
    print(f"\n{mode} (median of {len(rs)}):")
    for key in ("ready", "first", "wake", "warm wake"):
        if key in rs[0]:
            print(f"  {key}: {statistics.median(r[key] for r in rs) * 1000:.0f} ms "
                  f"(runs {[round(r[key] * 1000) for r in rs]})")
    for n in COUNTS:
        if n in rs[0]:
            print(f"  {n} sessions: {len(rs[-1][n])} processes, RSS {statistics.median(kib(r[n]) for r in rs):.0f} "
                  f"KiB (runs {[kib(r[n]) for r in rs]}), footprint {statistics.median(kib(r[n], 2) for r in rs):.0f}"
                  f" KiB (runs {[kib(r[n], 2) for r in rs]})")
    last = max(n for n in COUNTS if n in rs[-1])
    for _, rss, foot, cmd in rs[-1][last][:8]:
        print(f"    RSS {rss:>8} KiB, footprint {foot or 0:>8} KiB  {d.short(cmd, tmp)}")


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--adapter", default=ADAPTER, help="the npm package@version of the ACP adapter")
    p.add_argument("--codex", default="codex", help="the codex binary for app-server and exec")
    p.add_argument("--shed", default="shed", help="the shed binary for the lane mode (skipped if not found)")
    args = p.parse_args()
    if not shutil.which("npx") or not shutil.which(args.codex):
        sys.exit("needs npx and a signed-in codex on the PATH")
    acp = ["npx", "-y", args.adapter]
    with tempfile.TemporaryDirectory(prefix="cx-") as tmp:
        cwd = os.path.join(tmp, "work")
        os.mkdir(cwd)
        warm = d.Acp(acp, cwd)
        warm.call("initialize", {"protocolVersion": 1, "clientCapabilities": {}})
        warm.a.stop()
        subprocess.run(["npm", "install", "--silent", "--prefix", os.path.join(tmp, "npm"), args.adapter],
                       check=True, env=d.env())
        mods = os.path.join(tmp, "npm", "node_modules")
        with open(os.path.join(mods, args.adapter.rsplit("@", 1)[0], "package.json")) as f:
            meta = json.load(f)
        bin_ = meta["bin"] if isinstance(meta["bin"], str) else next(iter(meta["bin"].values()))
        node = ["node", os.path.join(mods, args.adapter.rsplit("@", 1)[0], bin_)]
        with open(os.path.join(mods, "@openai", "codex", "package.json")) as f:
            bundled = json.load(f)["version"]
        print(f"machine: {platform.platform()}, {os.cpu_count()} cpus; node {d.version(['node', '--version'])}")
        print(f"adapter: {args.adapter} (its codex {bundled}); codex: {d.version([args.codex, '--version'])}")
        modes = [("acp (npx)", lambda n: acp_run(acp, cwd)), ("acp (node)", lambda n: acp_run(node, cwd)),
                 ("app-server", lambda n: server_run(args.codex, cwd)),
                 ("exec", lambda n: exec_run(args.codex, cwd))]
        if shutil.which(args.shed):
            modes.append(("lane (hibernated)", lambda n: lane_run(args.shed, acp, cwd, tmp, n)))
        results = {m: [] for m, _ in modes}
        for n in range(args.runs):
            for mode, run in modes:
                results[mode].append(run(n))
        for mode, _ in modes:
            report(mode, results[mode], tmp)


if __name__ == "__main__":
    main()
