#!/usr/bin/env python3
"""Measure one awake Claude session four ways (#133, #180): through claude-agent-acp started by npx, through
the same adapter installed once and run by node, through the `claude` binary's own stream-json mode, and as a
real `shed` lane on the stream driver (its own server in a scratch folder, `driver = "stream"` in its
`providers/claude.toml`, policy `allow`; the tree is the bridge and `claude`).

For each: time from process start to ready (ACP: the session/new reply; stream-json: the init message) and
to the turn's first update, the process tree's RSS once the turn is done (every process, with its command),
and the wake: the agent stopped, started again on the same session (ACP session/load, `claude --resume`),
and the time from that start to the next turn's first update. Medians over --runs. Each run is two tiny
turns on the account `claude` is logged in to. The agents run with only HOME, PATH, USER, TERM, LANG and
TMPDIR from the environment (as the plan smoke in docs/acp.md), so no API key or parent session leaks in.

    scripts/measure-claude-direct.py [--runs 3] [--adapter @agentclientprotocol/claude-agent-acp@0.107.0]
                                     [--shed shed]
"""

import argparse
import glob
import json
import os
import platform
import shutil
import signal
import statistics
import subprocess
import sys
import tempfile
import threading
import time

ADAPTER = "@agentclientprotocol/claude-agent-acp@0.107.0"
FIRST = "Remember the word lantern. Reply with just OK."
AGAIN = "Which word did I ask you to remember? One word."
UPDATES = {"agent_message_chunk", "agent_thought_chunk", "tool_call", "plan"}
KEEP = ("HOME", "PATH", "USER", "TERM", "LANG", "TMPDIR")


def env():
    return {k: os.environ[k] for k in KEEP if k in os.environ}


def tree(root):
    out = subprocess.run(["ps", "-A", "-o", "pid=,ppid=,rss=,command="], capture_output=True, text=True,
                         check=True).stdout
    procs = {}
    for line in out.splitlines():
        pid, ppid, rss, cmd = line.split(None, 3)
        procs[int(pid)] = (int(ppid), int(rss), cmd)
    found, todo = [], [root]
    while todo:
        pid = todo.pop()
        if pid in procs:
            found.append((pid, procs[pid][1], procs[pid][2]))
            todo += [p for p, (pp, _, _) in procs.items() if pp == pid]
    return found


class Agent:
    """A child speaking newline-delimited JSON on stdio, read on a thread with arrival times."""

    def __init__(self, argv, cwd):
        self.began = time.monotonic()
        self.proc = subprocess.Popen(argv, cwd=cwd, env=env(), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.DEVNULL, text=True, start_new_session=True)
        self.lines, self.cv = [], threading.Condition()
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.proc.stdout:
            try:
                msg = json.loads(line)
            except ValueError:
                continue
            with self.cv:
                self.lines.append((time.monotonic(), msg))
                self.cv.notify_all()
        with self.cv:
            self.lines.append((time.monotonic(), None))
            self.cv.notify_all()

    def send(self, msg):
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def until(self, test, after=0, timeout=300):
        """The first message from index `after` that passes `test`: (index, seconds since start, message)."""
        end = time.monotonic() + timeout
        i = after
        with self.cv:
            while True:
                while i < len(self.lines):
                    at, msg = self.lines[i]
                    if msg is None:
                        raise RuntimeError(f"the agent exited ({self.proc.poll()})")
                    if test(msg):
                        return i, at - self.began, msg
                    i += 1
                if not self.cv.wait(end - time.monotonic()):
                    raise TimeoutError("no answer from the agent")

    def stop(self):
        try:
            os.killpg(self.proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.proc.wait()


class Acp:
    def __init__(self, argv, cwd):
        self.a, self.cwd, self.id = Agent(argv, cwd), cwd, 0

    def call(self, method, params):
        self.id += 1
        mark = len(self.a.lines)
        self.a.send({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params})
        want = self.id
        while True:
            i, at, msg = self.a.until(lambda m: m.get("id") is not None, mark)
            mark = i + 1
            if "method" in msg:
                self.a.send({"jsonrpc": "2.0", "id": msg["id"], "result": {"outcome": {"outcome": "cancelled"}}})
            elif msg["id"] == want:
                if "error" in msg:
                    raise RuntimeError(f"{method}: {msg['error']}")
                return at, msg["result"], mark

    def start(self, session=None):
        self.call("initialize", {"protocolVersion": 1, "clientCapabilities": {
            "fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False}})
        params = {"cwd": self.cwd, "mcpServers": []}
        if session:
            at, _, _ = self.call("session/load", {**params, "sessionId": session})
            return at, session
        at, result, _ = self.call("session/new", params)
        return at, result["sessionId"]

    def turn(self, session, text):
        mark, want = len(self.a.lines), self.id + 1
        threading.Thread(target=self.call, args=("session/prompt", {
            "sessionId": session, "prompt": [{"type": "text", "text": text}]}), daemon=True).start()
        _, first, _ = self.a.until(lambda m: m.get("method") == "session/update"
                                   and m["params"]["update"].get("sessionUpdate") in UPDATES, mark)
        self.a.until(lambda m: m.get("id") == want and "result" in m, mark)
        return first


def acp_run(argv, cwd):
    s = Acp(argv, cwd)
    ready, session = s.start()
    first = s.turn(session, FIRST)
    time.sleep(2)
    procs = tree(s.a.proc.pid)
    s.a.stop()
    s = Acp(argv, cwd)
    s.start(session)
    wake = s.turn(session, AGAIN)
    s.a.stop()
    return ready, first, procs, wake


def direct(claude, cwd, session=None):
    argv = [claude, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--include-partial-messages"] + (["--resume", session] if session else [])
    return Agent(argv, cwd)


def direct_turn(a, text):
    mark = len(a.lines)
    a.send({"type": "user", "message": {"role": "user", "content": text}})
    _, ready, init = a.until(lambda m: m.get("type") == "system" and m.get("subtype") == "init", mark)
    _, first, _ = a.until(lambda m: m.get("type") == "assistant" or (
        m.get("type") == "stream_event" and m["event"].get("type") == "content_block_delta"), mark)
    a.until(lambda m: m.get("type") == "result", mark)
    return ready, first, init["session_id"]


def direct_run(claude, cwd):
    a = direct(claude, cwd)
    ready, first, session = direct_turn(a, FIRST)
    time.sleep(2)
    procs = tree(a.proc.pid)
    a.stop()
    a = direct(claude, cwd, session)
    _, wake, _ = direct_turn(a, AGAIN)
    a.stop()
    return ready, first, procs, wake


def lane_run(shed, cwd, tmp, n):
    sd = os.path.join(tmp, f"s{n}")
    os.makedirs(os.path.join(sd, "providers"))
    with open(os.path.join(sd, "providers", "claude.toml"), "w") as f:
        f.write('driver = "stream"\n')
    cli = lambda *a: subprocess.run([shed, "--dir", sd, *a], env=env(), capture_output=True, text=True, check=True)
    server = subprocess.Popen([shed, "--dir", sd, "serve"], env={**env(), "SHED_ACP_HIBERNATE_AFTER": "5"},
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    while not os.path.exists(os.path.join(sd, "shed.sock")):
        time.sleep(0.05)
    events = Agent([shed, "--dir", sd, "events", "--json"], cwd)
    try:
        began = time.monotonic()
        cli("run", "--acp", "claude-acp", "--policy", "allow", "--name", "lane", "--cwd", cwd)
        cli("wait", "lane", "--output", "ACP session ready", "--timeout", "60")
        ready = time.monotonic() - began

        def turn(text, until):
            mark, sent = len(events.lines), time.monotonic()
            cli("send", "lane", text)
            _, at, _ = events.until(lambda m: m.get("event") in ("text", "tool_call", "plan"), mark)
            cli("wait", "lane", "--until", *until, "--timeout", "300")
            return at + events.began - sent

        first = turn(FIRST, ["done"])
        time.sleep(2)
        procs = tree(json.loads(cli("get", "lane", "--json").stdout)["pid"])
        cli("wait", "lane", "--until", "hibernated", "--timeout", "120")
        wake = turn(AGAIN, ["done"])
        return ready, first, procs, wake
    finally:
        events.stop()
        subprocess.run([shed, "--dir", sd, "kill", "lane"], env=env(), capture_output=True)
        server.terminate()
        server.wait()


def short(cmd, tmp):
    for path, name in ((os.path.realpath(tmp), "$TMP"), (tmp, "$TMP"), (os.path.expanduser("~"), "~")):
        cmd = cmd.replace(path, name)
    return cmd[:110]


def version(argv):
    try:
        return subprocess.run(argv, capture_output=True, text=True, env=env(), timeout=60).stdout.strip()
    except (OSError, subprocess.SubprocessError):
        return "?"


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--runs", type=int, default=3)
    p.add_argument("--adapter", default=ADAPTER, help="the npm package@version of the ACP adapter")
    p.add_argument("--claude", help="the claude binary for stream-json (default: the one the adapter's SDK runs)")
    p.add_argument("--shed", default="shed", help="the shed binary for the lane mode")
    args = p.parse_args()
    if not shutil.which("npx") or not shutil.which("npm"):
        sys.exit("needs node, npm and npx on the PATH")
    name = args.adapter.rsplit("@", 1)[0]
    with tempfile.TemporaryDirectory(prefix="claude-direct-") as tmp:
        cwd = os.path.join(tmp, "work")
        os.mkdir(cwd)
        subprocess.run(["npm", "install", "--silent", "--prefix", os.path.join(tmp, "npm"), args.adapter],
                       check=True, env=env())
        pkg = os.path.join(tmp, "npm", "node_modules", name)
        with open(os.path.join(pkg, "package.json")) as f:
            meta = json.load(f)
        bin_ = meta["bin"] if isinstance(meta["bin"], str) else next(iter(meta["bin"].values()))
        warm = Acp(["npx", "-y", args.adapter], cwd)
        warm.call("initialize", {"protocolVersion": 1, "clientCapabilities": {}})
        warm.a.stop()
        sdk = os.path.join(tmp, "npm", "node_modules", "@anthropic-ai", "claude-agent-sdk", "package.json")
        with open(sdk) as f:
            sdk = json.load(f)["version"]
        claude = args.claude or glob.glob(os.path.join(tmp, "npm", "node_modules", "@anthropic-ai",
                                                       "claude-agent-sdk-*", "claude"))[0]
        print(f"machine: {platform.platform()}, {os.cpu_count()} cpus; node {version(['node', '--version'])}")
        print(f"adapter: {args.adapter} (Claude Agent SDK {sdk}); claude: {version([claude, '--version'])}")
        modes = [("npx", lambda: acp_run(["npx", "-y", args.adapter], cwd)),
                 ("node", lambda: acp_run(["node", os.path.join(pkg, bin_)], cwd)),
                 ("stream-json", lambda: direct_run(claude, cwd)),
                 ("shed lane", lambda: lane_run(args.shed, cwd, tmp, n))]
        results = {m: [] for m, _ in modes}
        for n in range(args.runs):
            for mode, run in modes:
                results[mode].append(run())
        for mode, _ in modes:
            rs = results[mode]
            med = lambda i: statistics.median(r[i] for r in rs)
            rss = [sum(k for _, k, _ in r[2]) for r in rs]
            print(f"\n{mode}: ready {med(0) * 1000:.0f} ms, first update {med(1) * 1000:.0f} ms, "
                  f"wake to first update {med(3) * 1000:.0f} ms, awake {statistics.median(rss):.0f} KiB "
                  f"(median of {len(rs)}; RSS per run {rss})")
            for pid, kib, cmd in rs[-1][2]:
                print(f"  {kib:>8} KiB  {short(cmd, tmp)}")


if __name__ == "__main__":
    main()
