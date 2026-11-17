- A provider record's `[cache]` gives the prompt cache's lifetime on a plan and on an API key and the variables
  that mean an API key; an ACP lane hibernates after 600 s or that lifetime, whichever is shorter (unless
  `$SHED_ACP_HIBERNATE_AFTER` is set), `get` shows `session.cache_likely` (`warm` or `cold`) while idle and
  `shed top` shows it as `CACHE-LIKELY`; `shed model` after a turn says the switch drops the prompt cache.
