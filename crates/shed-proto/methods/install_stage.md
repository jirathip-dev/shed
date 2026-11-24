wire: install.stage
params: Stage
result: Install

Downloads `<tool>-<arch>-<os>` to `<dest>.new.part`, checks its sha256 against `SHA256SUMS`, makes it 755, checks that its `--version` contains `build` and renames it to `<dest>.new`; the live binary is untouched.
A failed check removes the file and is `UNSTAGED`, naming it (`asset`, `download`, `checksum`, `version`); `build` already staged fetches nothing. Only shed's own binary or the client's tool, else `NOT_OWN_BINARY`.
