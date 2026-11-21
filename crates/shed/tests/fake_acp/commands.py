"""The slash commands /plan, the same again, then /plan and /review (with an input hint)."""


def run(agent, text):
    plan = {"name": "plan", "description": "Make a plan"}
    review = {"name": "review", "description": "Review the diff", "input": {"hint": "files"}}
    for cmds in ([plan], [plan], [plan, review]):
        agent.update({"sessionUpdate": "available_commands_update", "availableCommands": cmds})
    agent.update({"sessionUpdate": "agent_message_chunk",
                  "content": {"type": "text", "text": "commands sent"}})
    return "end_turn"
