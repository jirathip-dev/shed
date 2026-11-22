#!/usr/bin/env python3
"""A fake ACP v1 agent for shed's tests: a scripted JSON-RPC server on stdio.

It checks every client message against the fields the ACP schema requires and exits 3 (with the
reason on stderr) on the first that does not fit.

A prompt runs `fake_acp/<word>.py`, named by its first word (each file's docstring says what it does),
else `fake_acp/echo.py`. The file's `run(agent, text)` is handed this module and returns the stop reason,
or the whole result.

It offers the models "fast" and "smart" (a `model` config option). `--history FILE` keeps the session's
prompts and updates in FILE (JSON lines) and advertises loadSession, which replays them; without it
loadSession is off.

`--sessions` advertises sessionCapabilities list and close and loadSession: session/list returns
sess-1 and sess-2 (one per page) and session/load takes either. `--closed FILE` appends each
session/close's sessionId to FILE, advertised or not (so a test sees a close that should not come).
`--v2` answers initialize with protocolVersion 2. `--unique` names each new session `sess-<pid>` instead
of sess-1 (lanes sharing a folder then have their own transcripts), and session/load takes any id.

`--mcp FILE` is the JSON list of MCP servers session/new and session/load must get (default none);
each entry is checked against ACP's stdio, http and sse shapes. `--mcp-caps http,sse` advertises
those `mcpCapabilities`.

`--meta FILE` is the JSON object session/new and session/load must get as `_meta`; without it they
must carry no `_meta` and no key shed did not send before.
"""
import importlib.util
import json
import os
import sys

sys.dont_write_bytecode = True

UNIQUE = "--unique" in sys.argv
SESSION = f"sess-{os.getpid()}" if UNIQUE else "sess-1"
HISTORY = sys.argv[sys.argv.index("--history") + 1] if "--history" in sys.argv else None
MODEL = "fast"
PROMPTS = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fake_acp")


MCP = json.load(open(sys.argv[sys.argv.index("--mcp") + 1])) if "--mcp" in sys.argv else []
META = json.load(open(sys.argv[sys.argv.index("--meta") + 1])) if "--meta" in sys.argv else None
CAPS = sys.argv[sys.argv.index("--mcp-caps") + 1].split(",") if "--mcp-caps" in sys.argv else []
KEPT = []
CANNED = "--canned" in sys.argv
REPLAY = sys.argv[sys.argv.index("--replay") + 1] if "--replay" in sys.argv else None
SESSIONS = "--sessions" in sys.argv
CLOSED = sys.argv[sys.argv.index("--closed") + 1] if "--closed" in sys.argv else None
LISTED = [{"sessionId": "sess-1", "cwd": "/work", "title": "First", "updatedAt": "2026-11-01T10:00:00Z"},
          {"sessionId": "sess-2", "cwd": "/work", "title": "Second"}]


def options():
    return [{"id": "model", "name": "Model", "category": "model", "type": "select",
             "currentValue": MODEL, "options": [{"value": "fast", "name": "Fast"},
                                                {"value": "smart", "name": "Smart"}]}]


def fail(why):
    sys.stderr.write(f"fake_acp: {why}\n")
    sys.exit(3)


def check(cond, why):
    if not cond:
        fail(why)


def check_mcp(method, servers):
    """Exits unless `servers` fit ACP's MCP server shapes and are the ones --mcp names."""
    pairs = lambda l: isinstance(l, list) and all(  # noqa: E731
        isinstance(p, dict) and set(p) == {"name", "value"} for p in l)
    for s in servers:
        kind = s.get("type", "stdio")
        check(isinstance(s.get("name"), str), f"{method}: mcpServers name: {s}")
        if kind == "stdio":
            check(isinstance(s.get("command"), str) and isinstance(s.get("args"), list)
                  and pairs(s.get("env")), f"{method}: stdio server: {s}")
        else:
            check(kind in CAPS, f"{method}: {kind} server not advertised")
            check(isinstance(s.get("url"), str) and pairs(s.get("headers")),
                  f"{method}: {kind} server: {s}")
    check(servers == MCP, f"{method}: mcpServers {servers} != {MCP}")


def check_keys(method, params, keys):
    """Exits unless `params` has exactly `keys`, plus `_meta` equal to --meta's when it is given."""
    want = set(keys) | ({"_meta"} if META is not None else set())
    check(set(params) == want, f"{method}: keys {sorted(params)} != {sorted(want)}")
    check(params.get("_meta") == META, f"{method}: _meta {params.get('_meta')} != {META}")


def send(msg):
    msg["jsonrpc"] = "2.0"
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()


def update(u):
    KEPT.append(u)
    send({"method": "session/update", "params": {"sessionId": SESSION, "update": u}})


def keep():
    """Appends this turn's updates to the history file."""
    if HISTORY:
        with open(HISTORY, "a") as f:
            f.writelines(json.dumps(u) + "\n" for u in KEPT)
    KEPT.clear()


def read():
    line = sys.stdin.readline()
    if not line:
        sys.exit(0)
    msg = json.loads(line)
    check(msg.get("jsonrpc") == "2.0", f"not JSON-RPC 2.0: {line}")
    if msg.get("method") == "session/cancel":
        check(msg.get("id") is None, "session/cancel must be a notification")
        check(msg["params"].get("sessionId") == SESSION, "session/cancel: sessionId")
    return msg


def wait_response(rid):
    """Reads until the response to our request `rid`; True if a cancel came first."""
    cancelled = False
    while True:
        msg = read()
        if msg.get("method") == "session/cancel":
            cancelled = True
            continue
        if msg.get("id") == rid and "method" not in msg:
            outcome = msg["result"]["outcome"]
            check(outcome["outcome"] in ("selected", "cancelled"), f"outcome: {outcome}")
            if outcome["outcome"] == "selected":
                check(isinstance(outcome.get("optionId"), str), "selected needs optionId")
            return outcome, cancelled


def prompt(text):
    """The `run` of the prompt file named by the text's first word, else echo.py's."""
    word = text.split(" ", 1)[0]
    path = os.path.join(PROMPTS, f"{word}.py")
    if not (word.isalpha() and os.path.isfile(path)):
        path = os.path.join(PROMPTS, "echo.py")
    spec = importlib.util.spec_from_file_location(f"fake_acp_{word}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.run


def main():
    global MODEL, SESSION
    while True:
        msg = read()
        method, params, rid = msg.get("method"), msg.get("params", {}), msg.get("id")
        if method == "initialize":
            check(isinstance(rid, (int, str)), "initialize: id")
            check(params.get("protocolVersion") == 1, "initialize: protocolVersion 1")
            check(isinstance(params.get("clientCapabilities"), dict), "initialize: clientCapabilities")
            caps = {"loadSession": HISTORY is not None or SESSIONS,
                    "mcpCapabilities": {k: k in CAPS for k in ("http", "sse")}}
            if SESSIONS:
                caps["sessionCapabilities"] = {"list": {}, "close": {}}
            version = 2 if "--v2" in sys.argv else 1
            send({"id": rid, "result": {"protocolVersion": version, "agentCapabilities": caps,
                                        "authMethods": []}})
        elif method == "session/list" and SESSIONS:
            page = 1 if params.get("cursor") == "p2" else 0
            result = {"sessions": [LISTED[page]]}
            if page == 0:
                result["nextCursor"] = "p2"
            send({"id": rid, "result": result})
        elif method == "session/close":
            if CLOSED:
                with open(CLOSED, "a") as f:
                    f.write(params.get("sessionId") + "\n")
            check(SESSIONS, "session/close without sessionCapabilities.close")
            send({"id": rid, "result": {}})
        elif method == "session/new":
            check(str(params.get("cwd", "")).startswith("/"), "session/new: absolute cwd")
            check_mcp("session/new", params.get("mcpServers"))
            check_keys("session/new", params, ["cwd", "mcpServers"])
            send({"id": rid, "result": {"sessionId": SESSION, "configOptions": options()}})
        elif method == "session/load":
            check(HISTORY is not None or SESSIONS, "session/load without loadSession")
            ids = [s["sessionId"] for s in LISTED] if SESSIONS else [SESSION]
            check(UNIQUE or params.get("sessionId") in ids, "session/load: sessionId")
            SESSION = params["sessionId"]
            check(str(params.get("cwd", "")).startswith("/"), "session/load: absolute cwd")
            check_mcp("session/load", params.get("mcpServers"))
            check_keys("session/load", params, ["sessionId", "cwd", "mcpServers"])
            with open(HISTORY or "/dev/null", "a+" if HISTORY else "r") as f:
                f.seek(0)
                for line in f:
                    send({"method": "session/update",
                          "params": {"sessionId": SESSION, "update": json.loads(line)}})
            send({"id": rid, "result": {"configOptions": options()}})
        elif method == "session/set_config_option":
            check(params.get("sessionId") == SESSION, "set_config_option: sessionId")
            if params.get("configId") == "model" and params.get("value") in ("fast", "smart"):
                MODEL = params["value"]
                send({"id": rid, "result": {"configOptions": options()}})
            else:
                send({"id": rid, "error": {"code": -32602, "message": f"no model {params}"}})
        elif method == "session/prompt":
            check(params.get("sessionId") == SESSION, "session/prompt: sessionId")
            blocks = params.get("prompt")
            check(isinstance(blocks, list) and blocks, "session/prompt: prompt blocks")
            check(all(b.get("type") == "text" and isinstance(b.get("text"), str) for b in blocks),
                  "session/prompt: text blocks")
            text = blocks[0]["text"]
            KEPT.append({"sessionUpdate": "user_message_chunk",
                         "content": {"type": "text", "text": text}})
            result = prompt(text)(sys.modules[__name__], text)
            keep()
            send({"id": rid, "result": result if isinstance(result, dict) else {"stopReason": result}})
        elif method == "session/cancel":
            pass
        else:
            fail(f"unexpected message: {msg}")


main()
