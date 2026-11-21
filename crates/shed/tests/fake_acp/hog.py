""""hog [MB]": starts a child holding MB (default 200) of memory for 60 s, then the reply "started it"."""
import subprocess
import sys


def run(agent, text):
    mb = int(text.split()[1]) if " " in text else 200
    hold = f"import time; b = b'x' * ({mb} << 20); time.sleep(60)"
    subprocess.Popen([sys.executable, "-c", hold, "https://user:secret@example.invalid/"],
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL)
    agent.update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "started it"}})
    return "end_turn"
