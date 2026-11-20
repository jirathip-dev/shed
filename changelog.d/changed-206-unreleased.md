- `shed wait --output` takes groups `( … )`, alternation `a|b`, counts `{n,m}`, `\b` and anchors anywhere,
  on a matcher whose time is bounded by pattern size times line length. `(`, `)`, `|`, and `^` or `$` in
  the middle of a pattern are now operators; a construct shed lacks (lookaround, backreferences) is an
  error that names it.
