"""An update of a kind no ACP schema has, then the reply "still here"."""


def run(agent, text):
    agent.update({"sessionUpdate": "brand_new_kind", "whatever": [1, 2]})
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "still here"}})
    return "end_turn"
