# The sandbox: `run`'s and `exec`'s `sandbox`

A terminal or a command can run inside Anthropic's sandbox-runtime (`srt`): Seatbelt on macOS, bubblewrap on
Linux, with srt's own allowlist proxy for the network (#259, slice H5). shed is the mechanism. The caller
decides whether a run is sandboxed and which folders and hosts it needs; shed writes the rules, resolves the
links, adds what the agent itself needs and its own denies, and wraps the command.

```rust
shed.run(&RunParams {
    acp: Some("claude-acp".into()),
    sandbox: Some(Sandbox {
        write: vec![worktree, git_dir, target, scratch, cache_dir, store_dir],
        read: toolchains,
        hosts: vec!["github.com".into(), "crates.io".into()],
        ..Sandbox::default()
    }),
    ..run
})?;
```

`shed run --sandbox '{"write":["/w"],"hosts":["github.com"]}' -- cmd` and `shed exec --sandbox …` take the same
object as JSON.

## The rules

shed on the machine that runs the command writes `sandbox/<terminal>.json` (0600, folder 0700) beside its state
file, so a `machine` run's rules and links are that machine's. `exec`'s rules are removed when it returns; a
terminal's go with `rm`.

| | entries |
|---|---|
| denyRead | the home folder, `~/.ssh`, shed's state folder (accounts, token files, secrets, transcripts, installed agents), shed's config folder (records, VM pins), the full socket, and the caller's `deny` |
| allowRead | the caller's `read`; the agent record's `[sandbox] read`; the installed ACP adapter; the folders `srt` and shed are in; the inside socket's folder; every write path |
| allowWrite | the caller's `write`; the record's `[sandbox] write`; the run's temp folder |
| network | the caller's `hosts` and the record's; none listed, no network |
| unix sockets | the inside socket and the caller's `sockets` (macOS by path; Linux cannot filter by path, so srt's socket filter is off there and the denied folders hide shed's own sockets) |
| mach | srt's defaults and the caller's `mach` (macOS) |

On macOS a read allow beats a deny, so the more specific `allowRead` entries open folders inside the denied home.
On Linux each denied folder is an empty tmpfs inside and each allow is bound back over it: a write outside the
allowed folders lands in that tmpfs and is gone when the command exits, so the real file is never written.

Every allowed path is resolved as well: its real path, the targets of the links directly in it, and each linked
folder on the way to it (Seatbelt follows no link for you).

A `write` that holds or lies inside shed's state or config folder, or holds the folder `srt` or shed is in, is
`INVALID_PARAMS`, so a sandboxed run can rewrite neither its own rules nor the program that applies them. No
`srt` on the server's PATH is `SANDBOX_MISSING`; shed never runs the command without it. `sandbox` with `keep`
or `warm` is `INVALID_PARAMS`: a terminal kept as a shell, or a spare made before the rules, would be outside them.

## The agent's own needs

An agent record's `[sandbox]` names what the agent needs, `~` for home and `{config}` for the run's config
folder: its account's folder on a run with `account`, else the record's `default_folder`.

```toml
# providers/claude.toml
[sandbox]
tmp_var = "CLAUDE_CODE_TMPDIR"       # srt takes the run's temp folder from it
read = ["~/.local/share/claude", "~/.local/bin/claude", "~/Library/Keychains"]
write = ["{config}", "~/.claude.json"]
hosts = ["*.anthropic.com", "claude.ai", "*.claude.ai"]
```

The temp folder is the caller's `tmp`, else `sandbox/<terminal>.tmp` beside the rules.

## ACP: the agent inside, the bridge outside

An ACP terminal's bridge stays outside: it holds the full socket, the transcript and the account switch. It
puts `srt --settings <rules>` before the agent each time it starts one, so a wake from hibernation and an account
switch run the agent under the same rules, and a server restart hands the bridge over with them.

## The inside socket

`inside/inside.sock` beside the state file (0600, its folder 0700) serves `hello`, `heartbeat`, `state` and
`emit`, and `slot.take` and `slot.free` once slots exist; any other method there is `METHOD_NOT_FOUND`. A
sandboxed command has `SHED_SOCKET` set to it, which the CLI honours, so a hook can report its state but cannot
start a terminal, read a file, or name an account, a secret or a VM.

## Proven by

`crates/shed/tests/sandbox.rs`, on macOS and Linux: drover's negative probes (`~/.ssh`, a file in shed's state
folder, `state.json`, another folder in home, a host off the list, a method off the inside list) all fail inside
a sandbox shed made, while the work folder and two cache folders take writes; the refused writes; the rules'
mode and removal; every method on the inside socket. Without `srt` on PATH the same tests prove
`SANDBOX_MISSING` and that nothing ran.
