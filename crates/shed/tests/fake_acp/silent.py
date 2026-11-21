"""Nothing, then like "slow"."""


def run(agent, text):
    while agent.read().get("method") != "session/cancel":
        pass
    return "cancelled"
