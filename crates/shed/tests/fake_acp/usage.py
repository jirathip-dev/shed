"""A usage_update (53,000 of 200,000 tokens, 0.045 USD), then the reply "counted"."""


def run(agent, text):
    agent.update({"sessionUpdate": "usage_update", "used": 53000, "size": 200000,
                  "cost": {"amount": 0.045, "currency": "USD"}})
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "counted"}})
    return "end_turn"
