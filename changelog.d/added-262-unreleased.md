- `shed account add|ls|login|status|rm` and the `account.*` methods: shed keeps one config folder per account
  on the main host (made empty, or an existing one adopted by link), an optional 0600 setup token beside it, and
  what the status command last said; the login runs on a redacted screen. `shed doctor` reports a signed-out
  account, a folder holding a setup, and a token file's age and mode.
