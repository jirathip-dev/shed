`shed events [--label k=v] [--since N]`

Stream starts, state changes, exits, kills and resource moves as JSON lines; `--since` replays those still kept (the last 4,096 besides agent text chunks); a client too far behind is dropped with a `lagged` line.
