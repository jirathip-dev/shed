wire: install.swap
params: Swap
result: Install

Keeps the live binary as `<dest>.prev` (a hard link) and renames `<dest>.new` over it, once `<dest>.new --version` answers with `build`; else `UNSTAGED` and nothing moves.
Live already `build`: nothing. Only shed's own binary or the client's tool, else `NOT_OWN_BINARY`.
