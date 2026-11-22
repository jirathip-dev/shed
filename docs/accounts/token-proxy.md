# The token proxy: a run on another machine with a per-run key

A run on another machine never gets an account's token. `run` with `machine` and `account` (a name or `auto`)
makes a key for that one run, `shk-` and 48 hex digits, and sends the far shed the record's two `[proxy]`
variables instead: `base_url_var` points at the machine's relay, `key_var` holds the key. The account needs a
live setup token (`shed account login <provider> <name> --token`, or `account add --token-file`) and its record a
`[proxy]` table, else the run is `ACCOUNT_LOCAL_ONLY`; a machine whose forward is down, or whose shed is under
protocol 5, is `NO_PROXY`.

## The path of a call

- The agent calls `http://127.0.0.1:<port>` on its machine. That shed's relay (`proxy.relay`) keeps one loopback
  port for good (`relay-port` beside its state file) and passes the bytes to the newest forward's socket; it holds
  no key and no token.
- The main host asks the machine `proxy.relay` when its link comes up and holds `ssh -R <socket>:proxy.sock` beside
  the link, restarting it with a fresh socket whenever it ends. `machine.links` shows the relay port as `proxy`
  while that `ssh` runs. It ends with the link, and with the server.
- `proxy.sock` (0600, beside `shed.sock`) is the proxy. It finds the key by its sha256 in `proxy-keys.json`
  (0600; the key itself is kept nowhere), checks the path against the record's `paths` before any token file
  opens, drops the caller's `authorization`, `x-api-key` and hop-by-hop headers, adds the record's `auth` and
  `append` headers and calls `upstream` through `curl`, its config, the token in it, over stdin.

## A key's life

| event | the key |
|---|---|
| `run` on a machine with an account | made, `live`, for that terminal |
| the terminal exits, is killed or removed; a link comes up and the machine no longer runs it | `ended` |
| `proxy.revoke <id>` (`shed proxy revoke`) | `revoked` |
| the lane, on `auto`, hits a limit | the account is marked `limited`, the key signs with the next account's token, and the lane gets one prompt to go on |

There is no timer. `proxy.keys` (`shed proxy keys`) lists every key, newest first, never a key itself.

## A refused token

An upstream 401 marks the account's token `rejected` (`account.ls`'s `token.rejected`) once, however many lanes
use it; the proxy then answers 401 itself, naming the `--token` login, until a new token is saved.
