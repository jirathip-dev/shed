"""The expired-login reply of claude-acp 0.100.0, then like "slow" (that adapter never ends it)."""


def run(agent, text):
    said = "Failed to authenticate: OAuth session expired and could not be refreshed"
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    while agent.read().get("method") != "session/cancel":
        pass
    return "cancelled"
