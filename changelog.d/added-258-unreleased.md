- Terminal (non-ACP) agents as provider records: a record's `[terminal]` table gives the start command, the
  process names, the pinned version, its first-run wizard (texts and the keys that dismiss it) and how a
  prompt lands (`ready`, `echo`, `accepted`). `shed run` labelled `agent=<agent>` presses the wizard's keys
  when it shows; `send` waits for a ready state, types the text, retypes it until its start shows, presses
  enter and waits for an accepted state, else fails `NOT_DELIVERED`. dsh-TUI is the first:
  `providers/dsh.toml`.
