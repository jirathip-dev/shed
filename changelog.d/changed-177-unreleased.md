- shed owns ACP sessions: each has a record and a transcript in `sessions/` beside the state file.
  `run --resume` replays shed's own transcript (the agent's replay is dropped), `agents sessions` lists shed's
  sessions without starting the agent, `rm` deletes a session's files, and sessions go
  `$SHED_SESSIONS_KEEP` days (default 30) after their last turn. `session.driver` is new (`acp`).
