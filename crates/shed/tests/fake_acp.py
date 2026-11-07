#!/usr/bin/env python3
"""A fake ACP v1 agent for shed's tests: a scripted JSON-RPC server on stdio.

It checks every client message against the fields the ACP schema requires and exits 3 (with the
reason on stderr) on the first that does not fit. Prompts:

- "tool": a plan, a mode change, a tool call and a permission request; then the tool completes and the
  reply is "wrote notes.txt" (allowed), or it fails and the reply is "denied" (rejected).
- "slow": one chunk ("thinking"), then waits for session/cancel and stops with `cancelled`.
- "auth": the expired-login reply of claude-acp 0.100.0, then like "slow" (that adapter never ends it).
- "silent": nothing, then like "slow".
- "busy": a tool call with a progress update every 0.3 s for 2.4 s, then the reply "busy done".
- "flood N": N one-character chunks, then the reply. With `--canned` they are one pre-built write (no
  per-update encoding) 0.2 s after the prompt (once shed has saved the turn's `working` state), so a
  bench timing from the first chunk to the reply times shed's throughput rather than this agent.
- "unknown": an update of a kind no ACP schema has, then the reply "still here".
- "commands": the slash commands /plan, the same again, then /plan and /review (with an input hint).
- anything else: the reply "you said: <prompt>" ("[<model>] you said: …" after a switch from "fast").

It offers the models "fast" and "smart" (a `model` config option). `--history FILE` keeps the session's
prompts and updates in FILE (JSON lines) and advertises loadSession, which replays them; without it
loadSession is off.

`--mcp FILE` is the JSON list of MCP servers session/new and session/load must get (default none);
each entry is checked against ACP's stdio, http and sse shapes. `--mcp-caps http,sse` advertises
those `mcpCapabilities`.
"""
import json
import sys
import time

SESSION = "sess-1"
HISTORY = sys.argv[sys.argv.index("--history") + 1] if "--history" in sys.argv else None
MODEL = "fast"


MCP = json.load(open(sys.argv[sys.argv.index("--mcp") + 1])) if "--mcp" in sys.argv else []
CAPS = sys.argv[sys.argv.index("--mcp-caps") + 1].split(",") if "--mcp-caps" in sys.argv else []
KEPT = []
CANNED = "--canned" in sys.argv


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


def tool_turn():
    update({"sessionUpdate": "plan", "entries": [
        {"content": "Write notes", "priority": "high", "status": "in_progress"}]})
    update({"sessionUpdate": "current_mode_update", "currentModeId": "code"})
    update({"sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "Write notes.txt",
            "kind": "edit", "status": "pending"})
    send({"id": 100, "method": "session/request_permission", "params": {
        "sessionId": SESSION,
        "toolCall": {"toolCallId": "call-1"},
        "options": [
            {"optionId": "allow-once", "name": "Allow", "kind": "allow_once"},
            {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
        ]}})
    outcome, _ = wait_response(100)
    if outcome["outcome"] == "cancelled":
        return "cancelled"
    if outcome["optionId"] == "allow-once":
        update({"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed"})
        update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "wrote "}})
        update({"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": "notes.txt"}})
    else:
        update({"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "failed"})
        update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "denied"}})
    return "end_turn"


def main():
    global MODEL
    while True:
        msg = read()
        method, params, rid = msg.get("method"), msg.get("params", {}), msg.get("id")
        if method == "initialize":
            check(isinstance(rid, (int, str)), "initialize: id")
            check(params.get("protocolVersion") == 1, "initialize: protocolVersion 1")
            check(isinstance(params.get("clientCapabilities"), dict), "initialize: clientCapabilities")
            caps = {"loadSession": HISTORY is not None,
                    "mcpCapabilities": {k: k in CAPS for k in ("http", "sse")}}
            send({"id": rid, "result": {"protocolVersion": 1, "agentCapabilities": caps,
                                        "authMethods": []}})
        elif method == "session/new":
            check(str(params.get("cwd", "")).startswith("/"), "session/new: absolute cwd")
            check_mcp("session/new", params.get("mcpServers"))
            send({"id": rid, "result": {"sessionId": SESSION, "configOptions": options()}})
        elif method == "session/load":
            check(HISTORY is not None, "session/load without loadSession")
            check(params.get("sessionId") == SESSION, "session/load: sessionId")
            check(str(params.get("cwd", "")).startswith("/"), "session/load: absolute cwd")
            check_mcp("session/load", params.get("mcpServers"))
            with open(HISTORY) as f:
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
            if text == "tool":
                stop = tool_turn()
            elif text in ("slow", "auth", "silent"):
                said = {"slow": "thinking", "silent": None, "auth": "Failed to authenticate: "
                        "OAuth session expired and could not be refreshed"}[text]
                if said:
                    update({"sessionUpdate": "agent_message_chunk",
                            "content": {"type": "text", "text": said}})
                while read().get("method") != "session/cancel":
                    pass
                stop = "cancelled"
            elif text == "busy":
                update({"sessionUpdate": "tool_call", "toolCallId": "call-2", "title": "Build",
                        "status": "in_progress"})
                for _ in range(8):
                    time.sleep(0.3)
                    update({"sessionUpdate": "tool_call_update", "toolCallId": "call-2",
                            "status": "in_progress"})
                update({"sessionUpdate": "tool_call_update", "toolCallId": "call-2",
                        "status": "completed"})
                update({"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "busy done"}})
                stop = "end_turn"
            elif text.startswith("flood ") and CANNED:
                chunk = {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}
                line = json.dumps({"jsonrpc": "2.0", "method": "session/update",
                                   "params": {"sessionId": SESSION, "update": chunk}}) + "\n"
                time.sleep(0.2)
                sys.stdout.write(line * int(text.split()[1]))
                stop = "end_turn"
            elif text.startswith("flood "):
                for _ in range(int(text.split()[1])):
                    update({"sessionUpdate": "agent_message_chunk",
                            "content": {"type": "text", "text": "x"}})
                stop = "end_turn"
            elif text == "commands":
                plan = {"name": "plan", "description": "Make a plan"}
                review = {"name": "review", "description": "Review the diff",
                          "input": {"hint": "files"}}
                for cmds in ([plan], [plan], [plan, review]):
                    update({"sessionUpdate": "available_commands_update",
                            "availableCommands": cmds})
                update({"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "commands sent"}})
                stop = "end_turn"
            elif text == "unknown":
                update({"sessionUpdate": "brand_new_kind", "whatever": [1, 2]})
                update({"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "still here"}})
                stop = "end_turn"
            else:
                said = f"you said: {text}" if MODEL == "fast" else f"[{MODEL}] you said: {text}"
                update({"sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": said}})
                stop = "end_turn"
            keep()
            send({"id": rid, "result": {"stopReason": stop}})
        elif method == "session/cancel":
            pass
        else:
            fail(f"unexpected message: {msg}")


main()
