wire: agents
result: Vec<AgentInfo>

Each agent shed has a provider record for or has installed, by id, as `shed agents --json` prints it: the record (its limit and login-expired lines, hooks and program facts), its `source` (`built-in` or the user file in `<config>/providers/` that adds or overrides it) and the installed adapter.
A bad user file is skipped (`shed doctor` names it).
