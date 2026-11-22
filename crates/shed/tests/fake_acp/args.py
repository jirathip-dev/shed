"""The reply is this agent's arguments, joined by spaces."""
import sys


def run(agent, text):
    said = " ".join(sys.argv[1:])
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    return "end_turn"
