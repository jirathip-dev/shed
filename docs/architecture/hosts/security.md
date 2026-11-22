# The security model

It extends #256's ([../accounts/security.md](../accounts/security.md)) and changes none of its rules. #256's two
promises still read the same: a credential never leaves the main host unless a run asks for it, and a lane never
loads the owner's own setup. Five properties follow, each with what holds it. "Test" means a test the slice
ships; "ban" and "layer" mean an `.drover/arch.toml` line from [adr.md](adr.md).

## 1. A secret is read by the program that uses it, and leaves the main host only inside a run that names it

| rule | held by |
|---|---|
| A secret is a `Secret`: no `Debug`, `Display`, `Serialize` or `Clone`. | the type; a compile-fail doc test |
| No shed method returns a secret. `secret.set` takes one; `secret.ls` gives names and dates. | the API: there is no get; test: every method's result after a set, searched for the value |
| cooee and drover read their own secrets in their own process. On macOS a Keychain item opens only to the signed program that saved it, with prompts off. | the Keychain's access list; test in each app: a second, differently signed binary gets `Refused` |
| A file wider than 0600 is not read. | `Error::Open`, drover's `SECRET_FILE_OPEN`; test; a doctor row with the `chmod` |
| A Keychain that does not answer in 5 seconds is an error. | test, moved from cooee (#713) |
| An unsigned program on macOS saves nothing. | `Error::Unsigned`; test, moved from cooee |
| A value goes to shed only in fields that are memory only: `run`'s `env` and MCP servers, `exec`'s `env` and `stdin`, `vm.exec`'s `stdin`, `send`'s text, `secret.set`'s `value`. None is in a log, an event, `get`, the state file or a hibernated session. | #191's tests for `run`'s `env`, run again for each field |
| A run on a machine carries its values in that one request, over the held ssh link, into that run's environment. No file is written on any host. | test: a fake machine keeps every byte it is sent; drover's `.env` file and `secrets-left` are deleted |
| shed's own secrets are used on the main host only: `vm.*` and `secret.*` are not routed to a machine. | the method files' `on:`; test |
| A provider's key reaches a verb's shell on stdin, never in an argument, and is cut out of the stderr shed returns. | test: the runner's recorded arguments; test with a verb that prints its key |
| `Secret::expose` is called in the store's two files and `vm.rs`. | ban `.expose()` |

## 2. Only a build the main host checked is installed

| rule | held by |
|---|---|
| The main host reads `SHA256SUMS` once; a machine gets the URL with that sum, so every host installs the bytes the main host named. | test: a machine that is served another file answers `UNSTAGED checksum` |
| `releases` is an `https://` URL. | `INVALID_PARAMS`; test |
| The staged binary's `--version` must name the build, and on macOS with an identifier it must carry that Developer ID signature. | tests, moved from shed's and drover's self-update |
| The live binary is never written over: staged beside it, renamed in; the old one stays as `.prev`. | test: a reader holding the old file open |
| `gate` is arguments for the staged binary, not a command line. | the type: `Vec<String>` run on `<dest>.new` |
| `dest` and `links` are under the machine's home, and never a shed binary: shed still updates only itself. | `INVALID_PARAMS`; test |
| `machine.add` with `install` copies shed's binary and nothing else: no account, secret, key or config. | test: a fake machine keeps every byte it is sent |

## 3. A VM is what its record says, reached by a pinned key, and holds nothing of ours

| rule | held by |
|---|---|
| A record is a built-in file or the owner's file in shed's config folder; no method writes one. | the API |
| Every value filled into a verb is checked against one character set first; `$C` and stdin are never filled in. | `INVALID_PARAMS`; test with quotes, spaces and `$(…)` in each field |
| A verb that needs the VM's id and gets nothing does not run. (boat reads an empty id as the VM it runs in.) | `NO_VM`; test, moved from drover |
| The host key is pinned at the first start. Another key later fails the start and stops the VM, until a caller says `repin`. | `VM_KEY_CHANGED`; test, moved from drover |
| A VM machine's ssh options are arguments of shed's own `ssh`; nothing writes `~/.ssh/config`. | test: the runner's recorded arguments |
| A `container` VM joins only the network the record makes with `--internal`; one of that name that is not host-only fails the create. | `VM_VERB`; test, moved from cooee-desk |
| After each start shed tries the caller's `probe` addresses from inside; one answer stops the VM. | `VM_REACHED`; test, moved from cooee-desk |
| No credential goes to a VM. Lanes on a VM machine sign in through #256's proxy with a per-run key; `vm.exec` sends a command and its stdin. | #256's tests; test: the bytes of a `vm.exec` |
| shed starts, stops and destroys a VM only when called; the one exception is stopping a VM whose start failed. | test: an idle server over a fake provider makes no call |

## 4. A sandbox is real or the run does not start, and a lane still never loads the owner's own setup

| rule | held by |
|---|---|
| `sandbox` with no `srt` on that machine is an error; shed never runs the command without it. | `SANDBOX_MISSING`; test |
| The rules are written by the shed on the machine that runs the command, in its own state folder (0600), with links resolved there. | test on a fake home with linked folders, moved from drover |
| A `write` path that holds shed's state or config folder, the rules, or the folder `srt` or shed is installed in is refused. | `INVALID_PARAMS`; test |
| shed always denies reads of its own state and config (accounts, token files, proxy keys, secrets, VM pins), its full socket and `~/.ssh`. A caller's `read` cannot open them. | test: drover's negative probes, run inside a sandbox shed made |
| The only shed socket inside is the inside socket: six methods, none of which starts a process, reads a file or names an account, a secret, a VM or an install. | ban `inside.sock`; test: every other method there is `METHOD_NOT_FOUND` |
| A run on a named account may write its account folder only; the owner's default config folder is not in its reads. | test; this adds to #256's property 3 |
| A wake from hibernation, an account switch and a server restart start the agent under the same rules. | test per path |
| #256's rules are unchanged: shed sets the account's variables and nothing else, `auto` never picks `default`, a named account's folder holds no setup, and drover's `ACP_UNGUARDED` and `SHED_OLD` checks run before `run`. | #256's tests |

## 5. A slot cannot be lost or held for ever

| rule | held by |
|---|---|
| A slot belongs to a connection and ends when it closes, however the holder died. | test: the holder is killed; the next ticket is admitted |
| A take answers by `wait_s` at the latest. | test on a fake clock |
| Limits are set on the full socket only. | the inside socket's method list |

## What this does not cover

- An unsandboxed lane on the main host. It runs as the owner's user, so 0600 files are open to it: secrets on
  Linux, #256's token files, shed's config and its VM records. Keychain values on macOS are not. A lane that
  must not reach them runs sandboxed, or on another machine.
- A VM provider's own login. The `boat` CLI keeps one; shed holds only the API key.
- What a cloud VM reaches on the network. Desks are isolated; cloud hosts are not.
- Spend. shed enforces no budget: a VM an app forgot keeps running.
- Another user on the main host with root.
