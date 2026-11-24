- `run` and `exec` take a `sandbox` (`--sandbox <json>`): the command, or an ACP terminal's agent each time its
  bridge starts it, runs inside `srt` with rules shed writes on that machine (the caller's folders and hosts,
  the agent record's new `[sandbox]`, and shed's own denies of home, `~/.ssh`, its state, config and socket). No
  `srt` there is the new `SANDBOX_MISSING`. A sandboxed command reaches only the new inside socket
  (`SHED_SOCKET`), which serves `hello`, `heartbeat`, `state` and `emit` (docs/hosts/sandbox.md).
