"""A plan, a mode change, a tool call and a permission request; then the tool completes and the reply is
"wrote notes.txt" (allowed), or it fails and the reply is "denied" (rejected)."""


def run(agent, text):
    agent.update({"sessionUpdate": "plan", "entries": [
        {"content": "Write notes", "priority": "high", "status": "in_progress"}]})
    agent.update({"sessionUpdate": "current_mode_update", "currentModeId": "code"})
    agent.update({"sessionUpdate": "tool_call", "toolCallId": "call-1", "title": "Write notes.txt",
                  "kind": "edit", "status": "pending"})
    agent.send({"id": 100, "method": "session/request_permission", "params": {
        "sessionId": agent.SESSION,
        "toolCall": {"toolCallId": "call-1"},
        "options": [
            {"optionId": "allow-once", "name": "Allow", "kind": "allow_once"},
            {"optionId": "reject-once", "name": "Reject", "kind": "reject_once"},
        ]}})
    outcome, _ = agent.wait_response(100)
    if outcome["outcome"] == "cancelled":
        return "cancelled"
    if outcome["optionId"] == "allow-once":
        agent.update({"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "completed"})
        agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "wrote "}})
        agent.update({"sessionUpdate": "agent_message_chunk",
                      "content": {"type": "text", "text": "notes.txt"}})
    else:
        agent.update({"sessionUpdate": "tool_call_update", "toolCallId": "call-1", "status": "failed"})
        agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "denied"}})
    return "end_turn"
