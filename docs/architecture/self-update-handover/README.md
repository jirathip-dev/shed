# Self-update and the serve handover (#161)

## Summary

Decision: serve hands over by replacing itself in place: same process id, so every terminal stays its child; the socket and the open terminals pass to the new build through the kernel, scrollback is replayed from the logs on disk, clients reconnect. The updater follows main's head only, asks serve "can this build take over?" before it swaps, and never restarts serve cold.
Slices: (1) a terminal can be detached and picked up again with its screen; (2) serve's handover; (3) the self-update command, its 5-minute job, pause and resume, doctor row, reconnecting commands; (4) docs.
Open questions: (1) a build that passes the check but crashes right after taking over ends that host's plain terminals (ACP sessions survive): accept (recommended), or add a keeper process, about 120 more lines? (2) Slices 1 and 2 need nothing from the release issue #160: build them first as their own issue (recommended), or all in #161?

The question, the options and the consequences are in [adr.md](adr.md). This file has how it reads, the
shapes, the boundaries, the statecharts and the slices.

## How it reads

The owner, once per host (the manual first install, `docs/releases.md`):

```console
$ shed install                      # writes dev.shed.serve and dev.shed.update, starts both
$ shed self-update --dry-run
current  0.1.0 (e55a367), serve e55a367
target   build-1a2b3c4 (main)
would    download shed-aarch64-apple-darwin, check SHA256SUMS and --version, swap, hand serve over
$ shed self-update --pause          # the job still fires, each pass says "paused since …" and exits 0
$ shed self-update --resume
```

One pass of the job, `crates/shed/src/update.rs`. Each step only does what is still missing, so a pass
that died half way is finished by the next one:

```rust
fn pass(paths: &Paths, exe: &Path, dry: bool) -> anyhow::Result<Pass> {
    if let Some(since) = paused(paths) {
        return Ok(Pass::Paused { since });
    }
    let target = Target::main_head()?;                    // git ls-remote <repo> refs/heads/main
    let mut serve = Client::connect(&paths.socket).ok();
    if target.is(BUILD) {
        return hand_over(serve, &target, exe);            // the binary is current; serve may still be owed
    }
    let Some(sums) = target.sums()? else {                // 404 on build-<sha7>/SHA256SUMS
        return Ok(Pass::Waiting { tag: target.tag() });
    };
    if dry {
        return Ok(Pass::Would { target });
    }
    let staged = Staged::fetch(&target, &sums, exe)?;     // <exe>.new: sha256, then `--version` names the sha
    if let Some(serve) = &mut serve {
        serve.call(Method::HandoverCheck, &check(&target, &staged))?;   // refused: nothing was swapped
    }
    staged.swap()?;                                       // <exe> -> <exe>.prev, <exe>.new -> <exe>, by rename
    hand_over(serve, &target, exe)
}
```

`Staged` removes `<exe>.new` when dropped unswapped, so every `?` above leaves the live binary and serve
as they were; the caller turns a failed fetch into `Pass::Rejected` and a refused check into
`Pass::Deferred`. `hand_over` calls `Method::Handover` when a serve answers on another build, then runs
`<exe> hello` until it exits 0 (binary and serve on one build), for at most 60 s.

Serve, `crates/shed-server/src/lib.rs`. The CLI says how this serve was started; serve needs that to
start itself again:

```rust
pub fn serve(paths: &Paths, sample_every: Duration, start: Start, again: &Restart) -> Result<(), Error>

loop {
    let leave = tokio::select! {
        () = accept(&listener, &registry, &links) => continue,
        () = &mut stop => Leave::Stop,
        Some(build) = asked.recv() => Leave::HandOver(build),
    };
    match leave {
        Leave::Stop => break,                              // as today: save, release lanes, end the rest
        Leave::HandOver(build) => {
            let why = handover::run(&registry, &links, &listener, again).await;
            writeln!(io::stderr(), "shed serve: no handover to {build}: {why}").ok();
        }                                                  // run returns only when it did not happen
    }
}

Method::HandoverCheck => json(&handover::check(registry, again, &p.build, &p.candidate).await?),
Method::Handover => {                                      // the same check, on our own binary
    let ok = handover::check(registry, again, &p.build, &again.exe).await?;
    asked.send(p.build).ok();                              // the caller has its answer before the drain
    json(&ok)
}
```

The handover itself, `crates/shed-server/src/handover.rs`:

```rust
pub(crate) async fn run(…) -> Refusal {
    drain(registry, DRAIN).await?;                         // short requests end; streams and waits are cut
    links.close().await;                                   // ssh children ended and reaped
    let frozen = lock(registry).freeze()?;                 // terminals detached, state file written
    let (ours, theirs) = UnixStream::pair()?;
    queue(&ours, listener, &frozen.masters)?;              // non-blocking, 64 fds a message
    let failed = runner::replace(&again.exe, &again.inherit, theirs.into());
    lock(registry).inherit(take(&ours)?);                  // exec failed: pick our own terminals up again
    Refusal::Exec(failed)
}
```

(`?` on a `Refusal` is shorthand: every early exit re-adopts what was frozen and returns to serving.)

The new image starts as `shed serve [--dir D] --inherit` and reads the same queue from its stdin:

```rust
let (listener, masters) = match start {
    Start::Inherit => handover::take(io::stdin())?.split(),  // the listener, then one master per terminal
    Start::Cold => (bind(&paths.socket)?, Vec::new()),       // as today: connect check, unlink, bind
};
let registry = Registry::load(&paths.state, &paths.socket, masters)?;
```

## What crosses, and how

| what | how |
|---|---|
| the listening socket | an fd, first in the queue. It is never closed or unlinked; connects wait in its backlog |
| each PTY master | an fd in the queue (`SCM_RIGHTS` on a socket pair that becomes the new image's stdin) |
| the child processes | nothing moves: the pid is the same, so the new image waits on each child and keeps its real exit status |
| scrollback | the history log on disk, its tail replayed (as a restart does today) |
| visible screen, pens, cursor, alt-screen and bracketed-paste modes | a repaint string per terminal in the state file's `handover` section, fed after the replay |
| name, labels, cwd, command, reported state, records, session, recipe | the state file, as today |
| notes waiting for a busy parent | the state file (`notes`, new: they are lost on any restart today) |
| event numbering | `seq` in the `handover` section; a reader with an older `since` gets the existing `reset` |
| an ACP session | it lives in the bridge process, which is untouched; the bridge reconnects, as it does today |
| client connections, attach and event streams, `wait` calls | not carried: closed during the drain; clients reconnect |
| sampler and memory-guard counters, drafts, start timings | rebuilt; the first sample only seeds them |
| machine links | closed before the exec, opened again after |

The fd queue was measured on a macOS host (Darwin 27.2): 1,024 copies of a PTY master queued in 16 messages of 64 fds, then
received after `exec` in the same pid. One fd per message stops at 64 messages on the default buffers, so
the sender batches and never blocks; a queue that will not take them all is a refusal before the exec.

## Data shapes

Protocol (`shed-proto`), two methods and one error code:

```rust
/// `handover.check`: would this serve hand over to the binary at `candidate`? Changes nothing.
pub struct HandoverCheckParams { pub build: String, pub candidate: PathBuf }
/// `handover`: hand over to the binary this serve was started from, which must be `build`.
pub struct HandoverParams { pub build: String }
/// The answer to both; `handover` sends it, then drains and execs.
pub struct Handover { pub from: String, pub to: String, pub terminals: usize }
ErrorCode::HandoverRefused        // the message is the reason, one line
```

Serve (`shed-server::handover`):

```rust
pub const FORMAT: u32 = 1;                      // of the fd queue and the `handover` section
pub enum Start { Cold, Inherit }
pub struct Restart {                            // built once by the CLI, before any swap
    pub exe: PathBuf,                           // current_exe at start (Linux appends " (deleted)" later)
    pub inherit: Vec<OsString>,                 // serve [--dir D] --sample S --inherit
    pub preflight: Vec<OsString>,               // serve [--dir D] --preflight
}
/// One JSON line on the stdout of `shed serve --preflight`, after it parsed the state file read-only.
pub struct Preflight { pub build: String, pub protocol: u32, pub oldest: u32, pub inherits: Vec<u32> }
pub(crate) struct Inherited { pub listener: OwnedFd, pub masters: Vec<OwnedFd> }
pub(crate) enum Refusal {
    Busy,                                       // a handover is under way, or a request outlived DRAIN (10 s)
    Preflight(String),                          // the binary did not start, or cannot read the state file
    Build { asked: String, found: String },
    Format { ours: u32, inherits: Vec<u32> },
    Bridge { terminal: String, speaks: u32, oldest: u32 },   // a live ACP bridge the new build would refuse
    Queue(io::Error),
    Exec(io::Error),
}
```

State file (`registry.rs`): the manifest is the state file, plus one section that exists only between
the freeze and the new image's first save.

```rust
struct StateFile { version: u32, terminals: Vec<Saved>, handover: Option<Handed> }
struct Handed {
    format: u32,
    from: String,                               // the build that froze
    seq: u64,                                   // the last event number given out
    screens: Vec<(String, String)>,             // (terminal, repaint), in the order of the queued masters
}
struct Saved { /* as today */ notes: Vec<String>, speaks: Option<u32> }   // speaks: its bridge's protocol
```

Core (`shed-core`):

```rust
impl Terminal {
    /// Stops the reader once the PTY is quiet, flushes the log, stops the reaper; the program runs on.
    pub fn detach(&self) -> Result<Detached, Error>;
    /// Picks up a detached terminal of this process: log tail, then `repaint`; waits on `pid` as its child.
    pub fn inherit(master: OwnedFd, pid: i32, history: &History, repaint: &str) -> Result<Self, Error>;
}
pub struct Detached { pub master: OwnedFd, pub repaint: String }
impl Screen {
    /// VT text that gives a blank screen of this size the same rows, pens, cursor and modes.
    pub fn repaint(&self) -> String;
}
/// runner: replaces this process with `program`; returns only the error when it could not.
pub fn replace(program: &Path, args: &[OsString], stdin: OwnedFd) -> io::Error;
```

The updater (`crates/shed/src/update.rs`):

```rust
struct Target { sha: String }                   // 40 hex, read from refs/heads/main and nowhere else
impl Target { fn tag(&self) -> String }         // "build-<sha7>": the only tag the updater can name
enum Pass {
    Paused { since: u64 },
    UpToDate { build: String },
    Waiting { tag: String },                    // main's head has no release yet
    Would { target: Target },                   // --dry-run
    Rejected { tag: String, why: String },      // checksum, --version, download: nothing swapped
    Deferred { tag: String, why: String },      // serve refused the check: nothing swapped
    Updated { from: String, to: String, serve: Served },
}
enum Served { HandedOver, NotRunning, Pending(String) }    // Pending: swapped, serve busy; next pass
```

Files:

```
<exe>  <exe>.new  <exe>.prev                    live, staged, the build before (kept for a manual rollback)
<state dir>/update/paused                       its mtime is "since"
<state dir>/update/status.json                  the last pass: { at, current, target, outcome, why }
<state dir>/update/lock                         flock: one pass at a time
dev.shed.update (farrier's fixtures/units/)    StartInterval 300, `<exe> self-update`
dev.shed.update.service, .timer                 oneshot; OnActiveSec=0, OnUnitActiveSec=300 (since #238)
```

What this needs from the two blocking issues:

| from | what | used by |
|---|---|---|
| #160 | tag `build-<sha7>` on main only; assets `shed-<target triple>` and `SHA256SUMS`; `shed --version` ends in `(<sha7>)` | `Target::tag`, `Staged::fetch` |
| #159 | `hello` carries `protocol` and the oldest protocol served | `Preflight`, `Refusal::Bridge`, the old updater and bridges talking to the new serve |

## Module boundaries

The layer rule stays (core and proto use nothing, server uses both, cli uses server). New code goes in two
new files; the hot files get registration lines only.

| crate | file | holds | may call |
|---|---|---|---|
| shed-core | `runner.rs` | `replace`, the only `exec` | std only |
| | `terminal.rs`, `screen.rs`, `history.rs` | `detach`, `inherit`, `repaint`, the replay without the restart suffix | each other, as today |
| shed-proto | `lib.rs` | the two methods, three types, one code | nothing |
| shed-server | `handover.rs` (new) | `check`, `run`, `drain`, `queue`, `take`, `Restart`, `Start`, `Preflight`, `Refusal`; `hand_over` and `take_over` move here from `acp.rs` | `registry`, `links`, `runner` |
| | `registry.rs` | `freeze`, `inherit`, `load(…, inherited)`, the `handover` section, `notes`, `speaks` | `handover` types only |
| | `lib.rs` | `serve`'s new arguments, the loop above, two dispatch arms | `handover` |
| | `acp.rs` | calls `handover::{hand_over, take_over}` for the bridge's lend, nothing else changes | `handover` |
| | `config.rs` | `update_source()`: the repo and release URLs, from the environment for tests | nothing |
| shed | `update.rs` (new) | `self_update`, `pass`, `Target`, `Staged`, `Pass`, status and pause files | `shed_server::{Client, runner, config, clock}`, `out` |
| | `service.rs` | two jobs instead of one (since #238 farrier-service `Job` values; golden copies in farrier's `fixtures/units/`); doctor's `update` row | `update` (status only) |
| | `main.rs` | `Command::SelfUpdate`, hidden `serve --inherit` and `--preflight`, one reconnect helper for `wait`, `events`, `attach`, `watch` | `update`, `service` |

Nothing outside `handover.rs` builds a `Restart` argv or touches the fd queue. Nothing outside
`update.rs` names a release tag or writes beside the binary. `update.rs` never signals, stops or starts
serve: its only levers are the two methods.

## Statecharts

Serve. Everything left of the exec can be undone, and is on any refusal:

```
              handover(build)            preflight ok            drained
  Serving ──────────────────▶ Checking ───────────▶ Draining ───────────▶ Frozen ──── exec ───▶ Inheriting ──▶ Serving
     ▲                           │ refused              │ Busy                │ Queue, Exec        (new image,     (new build)
     │                           ▼                      ▼                     ▼                     same pid)
     └───────────────────────────┴──────────────────────┴──── re-adopt in place
```

- Checking: no handover under way; `<exe> serve --preflight` exits 0; its `build` is the one asked for;
  its `inherits` has our `FORMAT`; its `oldest` is at most every live ACP terminal's `speaks`.
- Draining: accept stops (the listener stays open). Streams and `wait` calls are closed. Every other
  request in flight gets 10 s to answer, else `Busy` and serving resumes.
- Frozen: each running terminal is detached and its repaint taken; the state file is written with the
  `handover` section and synced; the fds are queued.
- Inheriting: fds taken from stdin; state file loaded; each named terminal picked up by its saved pid,
  its rules, idle time, reported state and records set again; the lock file locked again; `seq`
  continued; accept starts; the first save drops the section. A master that is missing marks that
  terminal exited. An empty queue means a cold start.

A terminal:

```
  Live ── detach ──▶ Detached ── inherit (new image, or the old one after a refusal) ──▶ Live
   reader, reaper     no reader; output waits in the PTY;
                      an exit waits as a zombie, status kept
```

One pass of the updater:

```
            paused                              no release
  start ───────────▶ Paused        Resolve ───────────────▶ Waiting
    │                                 │ head is our build ─────────────────────────────┐
    └────────────▶ Resolve            ▼                                                 ▼
                                   Fetch ──▶ Verify ──▶ Check ──▶ Swap ──▶ Hand over ──▶ Updated / UpToDate
                                     │ download  │ sha256,    │ serve       (no undo: a verified binary stays;
                                     ▼           ▼ --version  ▼ refused      a busy serve is `Pending`, next pass)
                                   Rejected   Rejected     Deferred
```

Left of Swap nothing on the host has changed. The job prints a line only when the outcome differs from
`status.json`'s.

## Slices

| # | what | files | src lines |
|---|---|---|---|
| 1 | detach, inherit, repaint, replace | `shed-core/src/{terminal,screen,history,runner}.rs` | about +130 |
| 2 | handover: methods, check, drain, freeze, queue, take, inherit; `notes`, `speaks`, `seq` | `shed-proto/src/lib.rs`, `shed-server/src/{handover,registry,lib,acp,events}.rs`, the two generated protocol docs | about +210 |
| 3 | `self-update`, the job, pause and resume, doctor row, reconnecting commands, job texts (since #238 rendered by farrier-service) | `shed/src/{update,service,main,attach,watch}.rs`, `shed-server/src/config.rs` | about +190 |
| 4 | docs | `docs/self-update.md` (new), a pointer in `docs/service.md`, `changelog.d/added-161.md`; the version-mismatch hints stop saying `cargo install` | 0 |

Tests drive the real binary:

- Slices 1 and 2: start serve, run a shell terminal and a fake ACP session, attach a client, call
  `handover` with serve's own build. Assert the same serve pid, both terminals running with their pids,
  `read` showing the text from before, input reaching the shell, the client attached again, an events
  reader with an old `since` getting `reset`, and the shell's later `exit 3` recorded as 3. A second
  handover right after must pass too. Unit: `feed(repaint(s))` on a blank screen equals `s`.
- Slice 3: a release folder on disk (`file://`) and a local git repo as main. Wrong checksum, an asset
  whose `--version` names another sha, no release, and only a `build-staging-<sha7>` release each leave
  the binary's and serve's build unchanged. The updating pass needs a second build with another stamp:
  `build.rs` takes `SHED_BUILD` from the environment when set, and `just test` builds it once.

The cap has 490 lines of headroom today (11,155 of 11,645) and this is about +530: the impl lane should
expect the standard `drover cap` ask. Moving the job texts out of `service.rs` gives about 25 back and is counted (since #238 farrier renders them).
