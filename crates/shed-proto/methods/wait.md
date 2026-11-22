wire: wait
params: WaitParams
result: WaitResult
on: name

Answer once the reported state is one of `until` (`idle` too, from the `idle_after` fallback; `blocked` only of `kind`, if given) or a visible screen line matches the pattern `output` (`matched`), the program exits (`exited`) or `timeout` seconds pass (`timeout`).
The server blocks; nothing polls.
With `children`, `name` is a parent: `all` matches once no child is working (`terminal` is the parent), `any` on the next child that reports `done`, `blocked` or `error`, or exits (`terminal` is that child; `exited` when no child runs).
