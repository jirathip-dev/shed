# Warm sessions

`shed run --acp … --warm` (the `run` method's `warm`) asks for a ready spare session. The recipe is the run's
agent, command, folder, policy, MCP servers, meta and size; the name, labels and parent are not part of it.
If a spare of that recipe is ready (its session answered `session/new`), the run takes it: the terminal is
renamed to the run's name with its labels and parent, its output log moves with it (the transcript is the
session's, so it stays), its
`session.ready_ms` is 0 and the bridge reports under the new name from then on. Otherwise the run starts cold.
Either way the server then starts the next spare, `.warm-N` in `ls`, unless the recipe has one already or
`$SHED_WARM_MAX` (default 2) spares are running. A spare nobody takes for `$SHED_WARM_IDLE` seconds (default
600) is killed and removed, as are spares left from before a server restart. `--warm` takes no `--resume`.
A taken session hibernates and wakes like any other, under the same name and session id.
