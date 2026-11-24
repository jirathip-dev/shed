wire: install.back
params: Dest
result: Install

Renames `<dest>.prev` over the live binary, once `<dest>.prev --version` answers; else `UNSTAGED` and nothing moves.
Only shed's own binary or the client's tool, else `NOT_OWN_BINARY`.
