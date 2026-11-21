""""flood N": N one-character chunks, then the reply. With `--canned` they are one pre-built write (no
per-update encoding) 0.2 s after the prompt (once shed has saved the turn's `working` state), so a bench
timing from the first chunk to the reply times shed's throughput rather than this agent."""
import json
import sys
import time


def run(agent, text):
    n = int(text.split()[1])
    chunk = {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}
    if agent.CANNED:
        line = json.dumps({"jsonrpc": "2.0", "method": "session/update",
                           "params": {"sessionId": agent.SESSION, "update": chunk}}) + "\n"
        time.sleep(0.2)
        sys.stdout.write(line * n)
    else:
        for _ in range(n):
            agent.update(chunk)
    return "end_turn"
