"""The same usage_update as "usage", then the reply "counted" whose response's `_meta.quota.token_count` has
claude-acp 0.100.0's counters (100 input, 900 cache reads, 1,000 cache writes, 50 output)."""


def run(agent, text):
    agent.update({"sessionUpdate": "usage_update", "used": 53000, "size": 200000,
                  "cost": {"amount": 0.045, "currency": "USD"}})
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "counted"}})
    counts = {"totalTokens": 2050, "inputTokens": 100, "cachedInputTokens": 900,
              "cachedWriteTokens": 1000, "outputTokens": 50, "reasoningOutputTokens": 0}
    return {"stopReason": "end_turn", "_meta": {"quota": {"token_count": counts}}}
