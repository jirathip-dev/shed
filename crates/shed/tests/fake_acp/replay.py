"""With `--replay FILE`, FILE's recorded session updates (JSON lines) in order; a line
`{"request_permission": {...}}` is sent as that request and waits for its answer."""
import json


def run(agent, text):
    for n, line in enumerate(open(agent.REPLAY) if agent.REPLAY else []):
        u = json.loads(line)
        if "request_permission" in u:
            agent.send({"id": 200 + n, "method": "session/request_permission",
                        "params": {"sessionId": agent.SESSION, **u["request_permission"]}})
            agent.wait_response(200 + n)
        else:
            agent.update(u)
    return "end_turn"
