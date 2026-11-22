wire: send
params: SendParams
on: name
input: yes

Paste text into the terminal, then press enter: bracketed when the program has turned bracketed paste on, unless `no_paste`; no enter with `no_enter`.
While a person has an unsent line (typed through `terminal.input`), the send waits until they press enter or clear it (`ctrl+u`, `ctrl+c`), or `DRAFT_HELD` once `draft_wait` seconds (default 60, 0 off) pass after their last key, writing nothing.
Sends go in one at a time, each whole.
ACP: the text goes to the bridge as it is.
