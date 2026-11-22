# Session options

`--meta <json>` (the `run` method's `meta`) is one JSON object that shed sends unchanged as `_meta` on
`session/new`, on `session/load` and on each wake from hibernation; without it the requests carry no `_meta`.
shed does not read it: which keys exist is the agent's business; see the agent's own docs. It reaches the
bridge in its environment only, never the agent's command line, environment or a label. drover's lanes
(herdsman#923) and cooee's personas are its callers. A resume needs `--meta` again.

The agent's slash commands (`available_commands_update`) are the session's `commands`
(`[name, description, input hint]`); each change emits a `session` event. Run one by sending `/name …`.
