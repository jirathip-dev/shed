- `shed run --acp <id> --account auto` runs on the provider's signed-in named account with the most headroom
  (never `default`), and when the lane hits a usage limit shed marks that account limited, copies the session's
  files to the next account, starts the agent there on the same session and sends one prompt to go on (an
  `account` event with `was` and `why: limit`). `account.ls` lists each provider's accounts in the order `auto`
  picks, with `runs` and `limited_until`, and reads a stale usage again in the background; accounts with a run
  are read every `usage.fresh` seconds. A run on a limited account is refused `ACCOUNT_LIMITED`; `auto` with
  every account signed out is `ACCOUNT_SIGNED_OUT`.
