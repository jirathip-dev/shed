wire: read
params: ReadParams
result: ReadResult
on: name

The visible screen, or with `history` the scrollback and screen, as plain text; `lines` keeps the last N lines.
Every read returns a `cursor`; with `since: <cursor>` only what is new: the lines that scrolled off since (`lost: N` when scrollback dropped N of them first), then the screen only when it changed (`changed`), so nothing new is an empty `text`.
ACP: the transcript as text, or with `events` its events (each with its `seq`), paged by `before` and `n`, or with `since` only those after its `seq`.
