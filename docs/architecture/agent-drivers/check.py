#!/usr/bin/env python3
"""#172's checks of `claude` driven in stream-json with documented flags only, no adapter and no control
protocol: do a lane's hooks run under the permission flags (hooks), does a process stopped mid-turn resume
to a usable session (stop), what does SIGINT do to a turn (interrupt), and does the prompt cache survive a
stop and a resume (cache).

Each check runs in a scratch folder of its own on the account `claude` is logged in to, with only HOME,
PATH, USER, TERM, LANG and TMPDIR from the environment (as scripts/measure-claude-direct.py), and stops the
agent the way shed's `kill_tree` does: SIGTERM to its process group, SIGHUP 100 ms later, SIGKILL at two seconds.

    check.py hooks | stop | interrupt | cache | all  [--model sonnet]
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import uuid

KEEP = ("HOME", "PATH", "USER", "TERM", "LANG", "TMPDIR")
BASE = ["claude", "-p", "--input-format", "stream-json", "--output-format", "stream-json",
        "--include-partial-messages", "--verbose", "--replay-user-messages", "--setting-sources", "project,local", "--strict-mcp-config"]
ALLOW = ["--permission-mode", "bypassPermissions", "--permission-prompts", "none"]
DENY = ["--permission-mode", "default", "--permission-prompts", "none"]
SLEEP = "python3 -c 'import time; time.sleep(120)'"
HOOK = """import json, os, sys
e = json.load(sys.stdin)
cmd = (e.get("tool_input") or {}).get("command", "")
with open(os.path.join(os.path.dirname(__file__), "hooks.log"), "a") as log:
    log.write(" ".join(filter(None, [e["hook_event_name"], e.get("tool_name"), cmd])) + "\\n")
if e["hook_event_name"] == "PreToolUse" and "forbidden" in cmd:
    print("the lane's hook refuses this command", file=sys.stderr)
    sys.exit(2)
"""


class Agent:
    """`claude` on piped stdio in a process group of its own, its stdout lines read on a thread."""

    def __init__(self, cwd, flags):
        self.argv = BASE + flags
        env = {k: os.environ[k] for k in KEEP if k in os.environ}
        self.proc = subprocess.Popen(self.argv, cwd=cwd, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, text=True, start_new_session=True)
        self.lines, self.seen = [], 0
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.proc.stdout:
            try:
                self.lines.append(json.loads(line))
            except ValueError:
                pass

    def say(self, text):
        line = {"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}}
        self.proc.stdin.write(json.dumps(line) + "\n")
        self.proc.stdin.flush()

    def wait(self, want, timeout=180):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            while self.seen < len(self.lines):
                self.seen += 1
                if want(self.lines[self.seen - 1]):
                    return self.lines[self.seen - 1]
            if self.proc.poll() is not None and self.seen >= len(self.lines):
                return None
            time.sleep(0.02)
        return None

    def turn(self, text):
        self.say(text)
        return self.wait(lambda m: m.get("type") == "result")

    def stop(self, sig=signal.SIGTERM):
        for sig, wait in ((sig, 0.1), (signal.SIGHUP, 1.9), (signal.SIGKILL, None)):
            try:
                os.killpg(self.proc.pid, sig)
                return self.proc.wait(wait)
            except subprocess.TimeoutExpired:
                continue
            except ProcessLookupError:
                return self.proc.wait()

    def close(self):
        self.proc.stdin.close()
        code = self.proc.wait(30)
        return code, self.proc.stderr.read().strip()


def asleep():
    return subprocess.run(["pgrep", "-f", "time.sleep.120"], capture_output=True, text=True).stdout.split()


def scratch(name):
    return os.path.realpath(tempfile.mkdtemp(prefix=f"shed172-{name}-"))


def brief(result):
    if result is None:
        return "no result line"
    keys = ("subtype", "is_error", "stop_reason", "num_turns")
    said = " ".join(f"{k}={result.get(k)}" for k in keys)
    return f"{said} result={result.get('result', '')!r:.120}"


def show(agent):
    print("$ " + " ".join(agent.argv))


def hooks(model):
    for policy, flags, ask in (
        ("allow", ALLOW, "Run exactly these two Bash commands, each in its own call, and never retry or work "
                         "around a refused one: `touch allowed.txt`, then `touch forbidden.txt`. Then reply with "
                         "the one word finished."),
        ("deny", DENY, "Run the Bash command `touch denied.txt`. Never retry or work around a refusal. Then "
                       "reply with the one word finished."),
    ):
        cwd = scratch(f"hooks-{policy}")
        os.mkdir(os.path.join(cwd, ".claude"))
        with open(os.path.join(cwd, "hook.py"), "w") as f:
            f.write(HOOK)
        run = {"hooks": [{"type": "command", "command": f"python3 '{cwd}/hook.py'"}]}
        events = {e: [dict(run, matcher="Bash")] for e in ("PreToolUse", "PostToolUse")}
        events.update({e: [run] for e in ("UserPromptSubmit", "Stop")})
        with open(os.path.join(cwd, ".claude", "settings.local.json"), "w") as f:
            json.dump({"hooks": events}, f)
        agent = Agent(cwd, flags + ["--model", model])
        print(f"\n## hooks, policy {policy}")
        show(agent)
        result = agent.turn(ask)
        init = next(m for m in agent.lines if m.get("subtype") == "init")
        print(f"init: permissionMode={init['permissionMode']} apiKeySource={init['apiKeySource']} "
              f"claude_code_version={init['claude_code_version']}")
        print("result:", brief(result))
        print("permission_denials:", [d.get("tool_name") for d in (result or {}).get("permission_denials", [])])
        print("hooks.log:")
        print("  " + open(os.path.join(cwd, "hooks.log")).read().strip().replace("\n", "\n  "))
        print("files:", sorted(f for f in os.listdir(cwd) if f.endswith(".txt")))
        print("exit on stdin close:", agent.close()[0])


def stop(model):
    def tool_started(m):
        blocks = m.get("message", {}).get("content", []) if m.get("type") == "assistant" else []
        return any(b.get("type") == "tool_use" for b in blocks)

    def text_delta(m):
        return m.get("type") == "stream_event" and m["event"].get("delta", {}).get("type") == "text_delta"

    cases = (
        ("mid tool call, SIGTERM", f"Run the Bash command `{SLEEP}` in the foreground, then reply finished.", tool_started,
         signal.SIGTERM, "Which Bash command were you running when you were stopped? Reply with the command only."),
        ("mid reply, SIGKILL", "Count from 1 to 400, one number per line, nothing else.", text_delta,
         signal.SIGKILL, "What did I ask you to do in my first message? One short sentence."),
    )
    for name, ask, started, sig, after in cases:
        cwd, session = scratch("stop"), str(uuid.uuid4())
        agent = Agent(cwd, ALLOW + ["--model", model, "--session-id", session])
        print(f"\n## stop, {name}")
        show(agent)
        agent.say(ask)
        hit = agent.wait(started)
        time.sleep(3)
        print("the command's processes before the stop:", len(asleep()))
        code = agent.stop(sig)
        ended = any(m.get("type") == "result" for m in agent.lines)
        print(f"stopped {'after' if hit else 'WITHOUT'} the turn's first {name.split(',')[0][4:]}: exit {code}, "
              f"turn ended first: {ended}")
        left = asleep()
        print("the command's processes left:", len(left))
        agent = Agent(cwd, ALLOW + ["--model", model, "--resume", session])
        show(agent)
        result = agent.turn(after)
        init = next((m for m in agent.lines if m.get("subtype") == "init"), {})
        print(f"init: session_id is the same: {init.get('session_id') == session}")
        print("result:", brief(result))
        print("second turn after the resume:", brief(agent.turn("Reply with the one word again.")))
        print("exit on stdin close:", agent.close()[0])

    cwd, session = scratch("stop"), str(uuid.uuid4())
    agent = Agent(cwd, ALLOW + ["--model", model, "--session-id", session])
    print("\n## stop, before any prompt")
    show(agent)
    time.sleep(3)
    print("lines before a prompt:", len(agent.lines), "exit", agent.stop())
    for how in ("--resume", "--session-id"):
        agent = Agent(cwd, ALLOW + ["--model", model, how, session])
        show(agent)
        result = agent.turn("Reply with just OK.")
        code, err = agent.close()
        print(f"result: {brief(result)}; exit {code}; stderr: {err[:160]!r}")


def interrupt(model):
    cwd, session = scratch("interrupt"), str(uuid.uuid4())
    agent = Agent(cwd, ALLOW + ["--model", model, "--session-id", session])
    print("\n## interrupt: SIGINT mid tool call")
    show(agent)
    agent.say(f"Run the Bash command `{SLEEP}` in the foreground, then reply finished.")
    agent.wait(lambda m: m.get("type") == "assistant" and any(
        b.get("type") == "tool_use" for b in m["message"].get("content", [])))
    time.sleep(3)
    print("the command's processes before the signal:", len(asleep()))
    os.kill(agent.proc.pid, signal.SIGINT)
    result = agent.wait(lambda m: m.get("type") == "result", 15)
    print("result after SIGINT:", brief(result))
    time.sleep(1)
    print("process alive after SIGINT:", agent.proc.poll() is None, "exit", agent.proc.poll())
    left = asleep()
    print("the command's processes left:", len(left))
    if agent.proc.poll() is None:
        print("next turn on the same process:", brief(agent.turn("Reply with the one word again.")))
        print("exit on stdin close:", agent.close()[0])
    else:
        agent = Agent(cwd, ALLOW + ["--model", model, "--resume", session])
        show(agent)
        print("next turn after --resume:", brief(agent.turn("Reply with the one word again.")))
        agent.close()


def cache(model):
    cwd, session = scratch("cache"), str(uuid.uuid4())
    rows = []

    def turn(agent, label, text):
        usage = (agent.turn(text) or {}).get("usage", {})
        counts = [usage.get(k, 0) for k in ("input_tokens", "cache_read_input_tokens",
                                             "cache_creation_input_tokens", "output_tokens")]
        share = 100 * counts[1] // max(1, sum(counts[:3]))
        rows.append(f"| {label} | {counts[0]:,} | {counts[1]:,} | {counts[2]:,} | {counts[3]:,} | {share}% |")

    agent = Agent(cwd, ALLOW + ["--model", model, "--session-id", session])
    print("\n## cache")
    show(agent)
    turn(agent, "1", "Remember the word lantern. Reply with just OK.")
    turn(agent, "2", "Reply with just OK again.")
    print("stopped: exit", agent.stop())
    agent = Agent(cwd, ALLOW + ["--model", model, "--resume", session])
    show(agent)
    turn(agent, "3, the first after the stop and `--resume`", "Which word did I ask you to remember? One word.")
    turn(agent, "4", "Reply with just OK.")
    print("stopped: exit", agent.stop())
    other = "haiku" if model != "haiku" else "sonnet"
    agent = Agent(cwd, ALLOW + ["--model", other, "--resume", session])
    show(agent)
    turn(agent, f"5, the first after a restart with `--model {other}`", "Which word was it? One word.")
    init = next(m for m in agent.lines if m.get("subtype") == "init")
    print("init.model after the restart:", init["model"])
    agent.close()
    print("| turn | input | cache read | cache write | output | cache share |\n| --- | --- | --- | --- | --- | --- |")
    print("\n".join(rows))


def main():
    checks = {"hooks": hooks, "stop": stop, "interrupt": interrupt, "cache": cache}
    args = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    args.add_argument("check", choices=[*checks, "all"])
    args.add_argument("--model", default="sonnet")
    args = args.parse_args()
    version = subprocess.run(["claude", "--version"], capture_output=True, text=True).stdout.strip()
    print(f"claude {version}, {time.strftime('%Y-%m-%d')}, model {args.model}")
    for name, check in checks.items():
        if args.check in (name, "all"):
            check(args.model)


if __name__ == "__main__":
    sys.exit(main())
