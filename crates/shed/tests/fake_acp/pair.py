"""Two tool calls with `locations` and `rawInput`, then a permission request for each, both in flight (ids
101 and 102, call-a's `rawInput` over 4 KiB); each call completes if allowed, else fails, and the reply names
each call's chosen option."""


def run(agent, text):
    calls = {"call-a": ("Edit a.rs", "x" * 5000), "call-b": ("Run ls", "ls -la")}
    for rid, (call, (title, raw)) in enumerate(calls.items(), 101):
        tool = {"toolCallId": call, "title": title, "kind": "execute",
                "locations": [{"path": f"/work/{call}.rs", "line": 3}], "rawInput": {"command": raw}}
        agent.update({"sessionUpdate": "tool_call", "status": "pending", **tool})
        agent.send({"id": rid, "method": "session/request_permission", "params": {
            "sessionId": agent.SESSION, "toolCall": {**tool, "content": [{"type": "content",
                "content": {"type": "text", "text": "diff"}}]},
            "options": [{"optionId": "allow-once", "name": "Allow", "kind": "allow_once"},
                        {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"}]}})
    chosen = {}
    while len(chosen) < 2:
        msg = agent.read()
        if msg.get("id") in (101, 102) and "method" not in msg:
            chosen[msg["id"]] = msg["result"]["outcome"].get("optionId", "cancelled")
    for rid, call in enumerate(calls, 101):
        status = "completed" if chosen[rid] == "allow-once" else "failed"
        agent.update({"sessionUpdate": "tool_call_update", "toolCallId": call, "status": status})
    said = f"call-a {chosen[101]}, call-b {chosen[102]}"
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    return "end_turn"
