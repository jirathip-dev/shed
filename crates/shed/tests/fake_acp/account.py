""""account": the reply "on NAME, session file: yes|no", NAME the base name of $FAKE_HOME, and whether the
session's file is in it. "account limit" first keeps that file in $FAKE_HOME, as Claude Code keeps its
sessions, then hits the usage limit."""
import os


def run(agent, text):
    home = os.environ["FAKE_HOME"]
    file = os.path.join(home, "projects", "p", f"{agent.SESSION}.jsonl")
    if text == "account limit":
        os.makedirs(os.path.dirname(file), exist_ok=True)
        open(file, "w").write("{}\n")
    said = f"on {os.path.basename(home)}, session file: {'yes' if os.path.isfile(file) else 'no'}"
    if text == "account limit":
        said += "\nYou've hit your limit · resets 3pm"
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": said}})
    return "end_turn"
