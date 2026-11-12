# 0002: PTY and VT parser crates

**Status:** accepted (M1a, #3)

## Context

A terminal needs a PTY whose child leads a new session with the PTY as its controlling terminal, so
`ctrl+c` reaches it and its whole process group can be signalled. Doing that by hand needs `pre_exec`
(`setsid`, `TIOCSCTTY`), which is `unsafe`. The README allows one reviewed `unsafe` PTY module, or a
vetted crate. The screen model needs a VT parser.

## Decision

- **PTY: [`pty-process`](https://crates.io/crates/pty-process) 0.5 (MIT), blocking API, no `async`
  feature.** Its only dependency is `rustix`. It opens the PTY, makes the child a session leader with the
  PTY as its controlling terminal, and resizes; its one `unsafe` (`pre_exec` calling `setsid` and an
  `ioctl`) is small and reviewed upstream. shed keeps `#![forbid(unsafe_code)]` in every crate, with no
  exception.
- **Signals and `/proc` helpers: `rustix`** (already pulled in by `pty-process`, same version), for
  `kill` of a process or group, page size and clock ticks, all safe.
- **VT parser: [`vte`](https://crates.io/crates/vte) 0.15 (MIT OR Apache-2.0)**, alacritty's parser,
  without its `ansi` feature. shed's own screen model sits on its `Perform` trait and keeps text only.
- **Process table:** `/proc` on Linux, `ps` on macOS, behind one `ProcessTable` trait. No `sysinfo` crate.

`portable-pty` was the other candidate; it pulls in more crates (`nix`, `filedescriptor`, `serial2`,
`shared_library`, …) for platforms shed does not target.

## Consequences

- No `unsafe` in shed; three new direct dependencies (`pty-process`, `rustix`, `vte`), small trees.
- Wide and combining characters are measured with `unicode-width` (#85).
- `ps` is one process spawn per sample on macOS; fine for sampling, revisit if it shows up in profiles.
