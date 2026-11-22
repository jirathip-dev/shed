- `read` returns a `cursor`, and `read` with `since: <cursor>` returns only what is new: the lines that
  scrolled off since, then the screen only when it changed (`changed`). Lines scrollback dropped before
  they were read are counted in `lost`. ACP terminals take `since` for their events by `seq`.
