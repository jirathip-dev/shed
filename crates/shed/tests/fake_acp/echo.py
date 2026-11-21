"""Any prompt with no file of its own: the reply "you said: <prompt>" ("[<model>] you said: …" after a switch
from "fast")."""


def run(agent, text):
    said = f"you said: {text}" if agent.MODEL == "fast" else f"[{agent.MODEL}] you said: {text}"
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    return "end_turn"
