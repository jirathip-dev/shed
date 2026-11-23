""""proxy PATH": a call to $FAKE_BASE_URL + PATH with the run's $FAKE_KEY and a stale auth header of its own; the
reply is "proxied STATUS: BODY". "proxy limit" hits the usage limit instead."""
import os
import urllib.error
import urllib.request


def run(agent, text):
    arg = text.split(" ", 1)[1]
    if arg == "limit":
        said = "You've hit your limit · resets 3pm"
    else:
        heads = {"x-api-key": os.environ["FAKE_KEY"], "authorization": "Bearer stale-of-its-own"}
        call = urllib.request.Request(os.environ["FAKE_BASE_URL"] + arg, data=b'{"model":"opus"}', headers=heads)
        try:
            with urllib.request.urlopen(call, timeout=10) as answer:
                said = f"proxied {answer.status}: {answer.read().decode()}"
        except urllib.error.HTTPError as e:
            said = f"proxied {e.code}: {e.read().decode()}"
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    return "end_turn"
