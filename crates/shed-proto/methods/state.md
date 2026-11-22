wire: state
params: StateParams
result: TerminalInfo
on: name

Report the terminal's state (`working`, `blocked`, `done` or `error`) with an optional note; a program can also print `ESC ] 7501 ; state=<s>[;note=<n>] BEL`, which never reaches the screen.
The terminal also speaks the Program Status Protocol (OSC 7501, revision 0.4): `state=<s>[:<key>=<value>]…` with `idle` and `clear`, `id`, `kind`, `progress`, `app` and base64 `title` and `msg`, and answers the probe `ESC ] 7501 ; ?`.
A report with an `id` is a record of its own (`records`); on exit `working` and `blocked` ones are dropped.
