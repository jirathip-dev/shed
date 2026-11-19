#!/usr/bin/env python3
"""A fake agent for shed's stream driver tests: it speaks the `plain` wire on stdio.

Each start appends its arguments (a JSON list) to `--log FILE`. `--store DIR` keeps one file per session,
written when the session takes its first prompt. `--session-id ID` starts a new session (exit 1 when DIR
has it already), `--resume ID` one DIR has (else exit 1 at once, nothing on stdout), `--model M` sets the
model the replies name. `--mcp-config FILE` is read at the start; its mode and text are kept.

Like the agents it stands for it prints nothing before the first prompt, except that a resumed start
first prints a turn end that is nobody's (an agent tidying up what its stopped process left).

Each prompt line `{"prompt": P}` is echoed (`"prompted"`), then:
- "tool": the tool call Build, in progress, then completed, and the reply "built".
- "slow": the tool call Sleep, in progress, the text "thinking", then nothing until it is stopped.
- "mcp": the reply "<mode> <text>" of the MCP file.
- "env": the reply "WRAPPED=<$WRAPPED>".
- anything else: the reply "[<model>] you said: <P>".
"""
import json
import os
import sys
import time


def arg(name):
    return sys.argv[sys.argv.index(name) + 1] if name in sys.argv else None


def say(value):
    sys.stdout.write(json.dumps(value) + "\n")
    sys.stdout.flush()


with open(arg("--log"), "a") as log:
    log.write(json.dumps(sys.argv[1:]) + "\n")
session = arg("--session-id") or arg("--resume")
saved = os.path.join(arg("--store"), session)
if arg("--resume") and not os.path.exists(saved):
    sys.stderr.write(f"No conversation found with session ID: {session}\n")
    sys.exit(1)
if arg("--session-id") and os.path.exists(saved):
    sys.stderr.write(f"Session ID {session} is already in use\n")
    sys.exit(1)
MCP = None
if arg("--mcp-config"):
    with open(arg("--mcp-config")) as f:
        MCP = f"{oct(os.stat(arg('--mcp-config')).st_mode & 0o777)} {f.read()}"
MODEL = arg("--model") or "default"
if arg("--resume"):
    say({"turn_end": {"stop": "tidied"}})

while True:
    line = sys.stdin.readline()
    if not line:
        break
    prompt = json.loads(line)["prompt"]
    open(saved, "a").close()
    say("prompted")
    if prompt == "tool":
        say({"tool": {"id": "t1", "title": "Build", "kind": "execute", "status": "in_progress"}})
        say({"tool": {"id": "t1", "status": "completed"}})
        reply = "built"
    elif prompt == "slow":
        say({"tool": {"id": "t2", "title": "Sleep", "kind": "execute", "status": "in_progress"}})
        say({"text": "thinking"})
        time.sleep(600)
    elif prompt == "mcp":
        reply = MCP
    elif prompt == "env":
        reply = f"WRAPPED={os.environ.get('WRAPPED')}"
    else:
        reply = f"[{MODEL}] you said: {prompt}"
    say({"text": reply})
    say({"turn_end": {"stop": "end_turn"}})
