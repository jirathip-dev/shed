"""One chunk ("thinking"), then waits for session/cancel and stops with `cancelled`."""


def run(agent, text):
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "thinking"}})
    while agent.read().get("method") != "session/cancel":
        pass
    return "cancelled"
