- `shed account use <provider> <name>` (`account.use`): the owner's switch. It makes a ready named account the
  one `auto` picks first and moves every live ACP or stream lane on `auto` or `default` to it, copying the
  session and loading it there; a rate-limited or mid-turn lane gets one prompt to go on. `--clear` drops the
  preference, and `account ls` shows it.
