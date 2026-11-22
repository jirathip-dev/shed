# Usage and limit marks

shed reads an account's usage with the record's `[usage]` probe (its command in the account's folder, read by
the windows' markers) whenever it runs the status command: `account.status`, after a login, and on a beat.
A reading stands for `usage.fresh` seconds. Every minute shed reads again each signed-in account that has a
run and a reading older than that; `account.ls` reads a stale account again in the background, so the next
call shows it. `checked_at` is when it last read one.

An account becomes `limited` when a lane on it reports `rate-limited`. It stays limited until
`limited_until`: the reset the lane's limit line carries (read by `when`), else the reset of the fullest
window in a usage reading taken at once, else now plus the record's `usage.hold`. A later reading whose
fullest window is under 100 % ends it early. A run on a limited account is refused `ACCOUNT_LIMITED`, and
`auto` skips it ([choice.md](choice.md)).
