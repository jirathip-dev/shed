""""env NAME": the reply "NAME has N chars" (the length of its value, never the value), or "no NAME"."""
import os


def run(agent, text):
    name = text[4:]
    said = f"{name} has {len(os.environ[name])} chars" if name in os.environ else f"no {name}"
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    return "end_turn"
