- Agents run only over ACP (protocol 6): screen-driven terminal agents are gone. A provider record has no
  `[terminal]`, `[[rules]]` or `[hooks]` (a user file that still has one is skipped and `shed doctor` names
  it), `send` to a plain terminal types the text as before, `shed setup` and the `NOT_DELIVERED` error are
  removed, and the dsh-TUI record is dropped. Plain shells, `screen.watch`, `send`, `keys`, `read`, `wait`,
  `state` and `run --idle-after` are unchanged.
