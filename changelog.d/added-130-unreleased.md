- Screen rules for PTY agents: a terminal labelled `agent=<agent>` checks its provider record's `[[rules]]`
  (a state, a priority, the last N screen lines and texts to match) on screen changes, at most every 250 ms;
  a rule above the agent's own report outranks it while its screen shows. Claude Code's folder-trust prompt
  reads `blocked` (`permission`). `get` says where a state came from: `reported.from` is `reported`,
  `rule:<id>` or `silence`. See docs/hooks.md.
