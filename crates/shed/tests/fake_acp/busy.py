"""A tool call with a progress update every 0.3 s for 2.4 s, then the reply "busy done"."""
import time


def run(agent, text):
    agent.update({"sessionUpdate": "tool_call", "toolCallId": "call-2", "title": "Build",
                  "status": "in_progress"})
    for _ in range(8):
        time.sleep(0.3)
        agent.update({"sessionUpdate": "tool_call_update", "toolCallId": "call-2",
                      "status": "in_progress"})
    agent.update({"sessionUpdate": "tool_call_update", "toolCallId": "call-2", "status": "completed"})
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "busy done"}})
    return "end_turn"
