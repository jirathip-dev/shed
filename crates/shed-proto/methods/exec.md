wire: exec
params: ExecParams
result: ExecResult
on: machine

Run a program without a terminal in a process group of its own, `stdin` fed to it and `env` over the server's variables (kept in memory only), and answer with its exit code (128 + the signal when one ended it) and the exact bytes it wrote.
The group is ended when the program returns or `timeout` seconds pass (`IO`); output over `REPLY_LIMIT` bytes is `IO`, never cut.
`machine` runs it there, over the held link.
