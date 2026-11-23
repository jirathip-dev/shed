# ADR: one transition table drives every shed flow (#309)

## Summary

Decision: every shed flow (a terminal, a link, a machine, an account, a proxy key, and the drain #307 adds) gets its states, its facts and its rows from one data file each, and the build turns the six files into the only code that may change a state. A row says: in this state, on this fact, go there, do this, who owes the next move, by when, and which event goes out when two retries have failed. A missing row stops the build. What the states mean: a finished terminal is "exited" while shed ends what its program left behind and "gone" once nothing of it runs, and its name stays held until it is removed, expires, or a new run takes it over in one call; a machine is offline only when its link is down; a key ends only when the shed on its machine says its run is gone; an account is "switched" when it is at its limit and its runs were moved off it.
Slices: (1) the six tables, the engine and the gap check; (2) the runner and the fact readers in shadow, with the disagreement report; then one flow at a time, each after its own shadow week and each deleting the code it replaces: (3) terminal, (4) link, (5) machine, (6) account, (7) key; (8) the old wire fields go once drover and cooee read the new ones. #307's pinned host key and drain are built on the table from their first day.
Open questions: (1) when a program exits and leaves processes behind, shed ends them (recommended: a server restart already does, and a key cannot end while they run), or leaves them and the terminal stays "exited"? (2) a flow takes over on its report alone, through the normal review (recommended), or on your OK each time? (3) one cap raise of about 700 lines in its own cap issue, sized by `drover cap berth` when filed; about 150 of them come back by slice 8.

**Status:** proposed by the architect lane, 2026-11-23, read on shed staging `675ab71`. It follows the owner's
decisions of the same day (the same as drover#1649, sized to shed). Callers, statecharts, engine, gap test,
shadow week and slices: [README.md](README.md). Facts and tables: [tables.md](tables.md). Types:
[shapes.md](shapes.md).

## The question

shed's states live where each was first needed, and each reader combines them its own way:

- **A terminal** is `running` or `exited`, plus whatever its program last reported (`done`, `hibernated`,
  `idle`…), plus an exit status that is `null` for "lost". 38 lines compare `State::Running` or
  `State::Exited`. `machine.stats` counts by one combination, `ls` shows another, `run` refuses `NAME_TAKEN`
  for all of them without saying which.
- **Up or down** is answered three ways: the held link's state (`links.rs`), `machine stats --on`'s own fresh
  ssh dial with a 5 s limit and its own `seen/` folder (`machines::stats`), and whether a call happens to get
  through. The link's calls share one connection behind one lock and read with no deadline; a dial reads the
  hello with no deadline either (`Conn::over`). So one silent link holds every caller, and a dial that hangs
  after TCP connects holds the link's thread, and its state, for good.
- **A proxy key** ends when an `exited` or `killed` event arrives for its terminal's name, when the listing
  read at link-up shows no running terminal of that name, when `rm` passes through the main host, and when the
  `run` that it was made for returns any error (`proxy.rs`). A name is reused by the next lane, and an error
  includes "the link dropped after the run was sent".
- **The machine list** is a file two programs write (the CLI and the server) and the server reads only when
  someone calls `machine.links`, `machine.add` or `machine.rm`. A removal leaves no trace of who or when.
- **An account** has four states derived in three places (`Store::account`, `choice::usable`, `proxy.rs`'s
  `watch`), and the move to another account on a limit is a branch of the proxy's event loop.

The owner decided the cure: one table of (state, fact) → next, action, owner, timeout, escalation drives every
flow; facts are a closed list, each from one source; CI fails any gap; a shadow week before it takes over. This
ADR decides the table's form, where it lives, what every row is, and how it is rolled out.

## Options

**What the table is**

| | option | verdict |
|---|---|---|
| A | A statechart document beside the code, as drover's `STATES` was | not driving anything is what failed: the code and the chart drift, and nothing notices a pair with no actor |
| B | One exhaustive Rust `match` per flow | the compiler finds gaps, but it is code paths: no owner, time or escalation per row, nothing a report or a chart can read, and shadow cannot name the row that disagreed |
| C | Rows as Rust constants in one file | typed, but one file every flow's PR edits, and about 700 lines against the cap |
| D | **One data file per flow, turned into enums and rows by the build step** | chosen: the rows are data a person, a chart and the report all read; a flow's PR edits its own file; shed-proto already builds `Method` and `ErrorCode` from `methods/*.md` and `errors/*.md` this way |

**Where it lives**

| | option | verdict |
|---|---|---|
| E | A new crate | one more crate in the tagged client release, to hold 180 lines |
| F | shed-core | core may use no other crate, so it cannot name the wire states, and a trait in core could not be implemented for shed-proto's types from the server |
| G | **shed-proto** | chosen: a flow's states are wire values, so the enum on the wire and the enum in the rows are one generated type. The engine is pure, so "proto uses nothing" still holds |

**Who changes a state**

| | option | verdict |
|---|---|---|
| H | Each module steps its own tracker | the same spread as today, with a table attached |
| I | **One runner holds every tracker; readers only produce facts, actions only act** | chosen: one place to shadow, to log, to emit events and to test; privacy and one ban enforce it |

**Timeouts and escalation**

| | option | verdict |
|---|---|---|
| J | A watchdog beside the table (drover's `undriven.rs`) | it notices afterwards, and has no row to name |
| K | A fall-back state named inside each row | a second transition hidden in a column the gap check cannot see |
| L | **The clock is a source: after the retries it feeds the fact `gave_up`, and that pair has a row like any other** | chosen: what happens after an escalation is in the table, and a state with no `gave_up` row is a gap |

**What "finished" means for a terminal**

| | option | verdict |
|---|---|---|
| M | `gone` = lost in a restart, `exited` = seen to exit | two words for "it ended"; neither says whether anything still runs, which is what a key, a wip save and a VM delete need to know |
| N | `gone` = removed from the list | then `gone` and "name freed" are one thing, and the key rule "ends when gone" would mean "never while anyone forgot to `rm`" |
| O | **`exited` = the program ended, shed is ending what it left; `gone` = the process table shows nothing of it; the name is freed by `rm`, by expiry or by `run --replace`** | chosen: each word is one observable fact, and drover#1366's own wording ("a key ends only after its process is confirmed gone") becomes a row |

**What a key is bound to**

| | option | verdict |
|---|---|---|
| P | The terminal's name, as today | a replaced lane has the same name, so the old lane's end can end the new lane's key |
| Q | **The run's id** | chosen: a caller may set it, shed makes one otherwise; the same id again is the same run, which also makes `run` safe to retry |

**Rollout**

| | option | verdict |
|---|---|---|
| R | Convert flow by flow with no shadow | the owner decided a shadow week |
| S | **Shadow all five at once, take over one flow at a time** | chosen: one week of evidence for all, and a wrong row costs only its own flow a second week |

## Decision

1. **The table.** Six data files in `crates/shed-proto/flows/`. Each names its subject, its states, its start
   and resting states, its facts with one source each, and its rows `(state, fact) → next, do, owner, within,
   or`. [tables.md](tables.md) holds all six in full: 265 pairs, 106 rows.
2. **The facts.** A closed list per flow, 45 in all besides `gave_up`, from 12 sources. A call is a fact only
   after the edge accepted it. No answer is never a fact about a terminal or a key: it is `unknown`, and
   `unknown` ends nothing.
3. **The engine.** `shed_proto::flow`: generated `State`, `Fact`, `Action` and `row()` per flow, and a pure
   `Tracker` that takes a fact and the time and returns the step. It reads no clock and does no I/O.
4. **The runner.** `crates/shed-server/src/flows.rs` holds every tracker and is the only code that steps one.
   Each flow has one file with its readers and its actions.
5. **Owner, time, escalation.** A row that moves or acts names who owes the next fact and by when. shed
   retries its own action twice. Then one `stuck` event names the row and the reason, the subject's `flow`
   shows `stuck` until it moves, and the `gave_up` row says where it goes. shed has no owner thread: a caller
   routes the event.
6. **The gap test.** The build step refuses a pair with no row or two, a moving row with no owner, time or
   reason, and a non-resting state entered with no time. Three tests pin it.
7. **One wire shape.** Every listed subject carries `flow: { state, since, row, stuck? }`: a terminal in `ls`
   and `get`, a machine and its link in the new `machine.ls`, an account in `account.ls`, a key in
   `proxy.keys`. Every change is one `flow` event, every escalation one `stuck` event, on the `events` stream
   that exists.
8. **Shadow, then take-over.** As in [README.md](README.md): disagreements go to `flow-shadow.jsonl`, are read
   with `shed flow report`, and a flow takes over after seven clean days, in the PR that deletes its old code.
9. **`run` gets two fields.** `id`: the same id again answers the same run. `replace`: take the name from a
   holder that is `exited` or `gone`, in the same call. `NAME_TAKEN` names the holder's state and id.
10. **One writer for the machine list.** The server. `shed machine add|rm` call it; a hand edit is seen within a
    minute and logged `by: file`.

## The four root causes

Each symptom, with what happens to it and the rows that do it.

| root cause | symptom | verdict | rows |
|---|---|---|---|
| 1. A finished terminal is ambiguous | drover#1597: `done` counts as live | fixed on shed's side: `done` and `asleep` are states of their own, in `ls` and in `machine.stats`'s counts. What drover does with a `done` lane past its stall time is a row of drover#1649 | T16, T17 |
| | drover#1545: an extended round waits behind a same-name `done` lane | fixed: the refusal names the holder's state, so drover's row kills a `done` holder and replaces it in one retry-safe `run`. Its amendment's real loop (a start refused over a host wrongly offline, then a stale pending marker) is fixed by run ids and by root cause 2's rows | T11, T10, T28, T35; M14 |
| | drover#1598: `NAME_TAKEN` instead of the message | fixed: the refusal carries the state and the run id, and input to a `running`, `done` or `asleep` holder is a row. Choosing to send is drover#1571's | T11; T14, T19, T22 |
| 2. Up or down is not one fact | drover#1623: stuck offline while up | moot as filed: the stale fact was drover's own `cloud-stop` reading, which goes with drover#1567 and drover#1644. Its twin in shed is fixed: a dial is cut at 15 s, dialing never stops, and the first good dial is `up` at once | L2, L5, L7; L1, L4; M14 |
| | drover#1510: one slow handshake marks a host offline (closed) | fixed: late is `slow`, never down; a drop is `reconnecting`; `down` needs three failed dials in about a minute; a machine is `offline` only from `link_down` | L10, L11, L13, L7; M5 |
| | drover#1544: a stale `HOST_OFFLINE` (closed) | fixed: `offline` ends at the link's first `up`, with no cache between, and `machine stats --on`'s own dial, which could say offline while the link was up, is deleted | M14; slice 4 |
| | drover#1254: a hung connect blocks the pass (closed) | fixed: 30 s of silence is `hung`, and the cut returns every waiting call; a late call fails alone at its deadline and later calls get a fresh connection; a call to a link that is not up is answered at once | L12, L13, L11 |
| | shed#195: `state` hangs instead of giving up (closed) | fixed by its own 2 s deadline; the rule is now general: no call over a link without one | L11 |
| 3. A key's life follows a guess | drover#1253, drover#1366: a lane judged dead loses its key (closed) | moot once drover#1567 retires drover's proxy: drover ends no key. The same class in shed is fixed: a key in use ends only on the owning shed's answer that its run is gone | K6, K7 |
| | 2026-11-23: `401 lane key ended` under running lanes | fixed: a `run` whose answer was lost is looked for, not revoked; facts are matched by run id, so an old lane's end cannot hit the new lane's key; no answer ends nothing. Shadow's `action` lines will count how often today's code ends a key the table would keep | K3, K6, K7 |
| 4. The machine list is not reconciled | drover#1616: the list was emptied and nothing re-added it (closed in drover, which now re-adds each pass) | fixed at the source: a removal is one logged step with who asked; adding twice is once; the file and the links are compared each minute; a removed machine's keys do not end, so a re-add resumes its lanes; a lost `rm` no longer leaves `NAME_TAKEN` | M9, M1, M7; K7; T28, T35 |

## Consequences

- A state changes in one place. The 38 terminal, 10 link, 27 account and 8 key state comparisons in `src/`
  become reads of `flow.state`.
- **Behaviour that changes**, each at its flow's take-over:
  - A program that exits and leaves processes behind has them ended: SIGTERM at once, SIGKILL 2 s later
    (question 1). Child terminals with `outlive` are untouched.
  - `machine stats --on` goes over the held link. With the link not up it says so at once, with the link's
    state and since when, and does not dial.
  - A call to a machine whose link is `hung`, `reconnecting` or `down` fails at once with `NO_LINK`. Today it
    may wait.
  - `shed machine add|rm` need the server (the CLI starts it, as other commands do). A removed machine stays
    in `machine ls` for 24 h.
  - A key outlives its program by the seconds shed needs to end the tree. A key whose machine was removed
    stays `in_use`; it cannot be used without the link, and `proxy.revoke` ends it.
  - `NAME_TAKEN` gains `data` with the holder's `flow` and `id`.
- Every state change is an event. A flapping link is visible as `up`/`slow` events, about one a minute at
  worst (`steady` needs four beats).
- A `stuck` event can be missed by a caller that was not listening. The `stuck` field in the listing stays set
  until the subject moves, so a caller that polls sees it too.
- The tables are the documentation: `docs/flows.md` is generated from them, and this folder's
  [tables.md](tables.md) is superseded by the data files when slice 1 merges.
- shed-proto's build step grows a second job. It stays outside `src/`, so outside the cap; the engine is about
  180 lines inside it.
- Timings are constants in the rows (180 s to start, 10 s to end a tree, 15 s a dial, 30 s of silence, 60 s
  between dials when down). Changing one is a one-cell diff, reviewed like code. There is no setting.
- drover#1649's table reads shed's states as its lane and host facts. The two tables share no code: shed's
  ends at the event, drover's starts there.
- #307's ADR stands. Its slice 2 writes the drain flow's file and the three `draining` rows rather than a flag
  with its own checks; its slice 1's pinned key is what makes `refused` over a key observable.

## Hot files

Nothing here adds a shared list. The table is six files in a collected folder, the readers and actions six
files in another, the chart is generated. Two files many PRs edit lose their state code and get no new entries
from this work: `crates/shed-server/src/links.rs` (slice 4) and `crates/shed-server/src/registry/mod.rs`
(slice 3).

The impl lane adds these `.drover/arch.toml` lines.

Slice 1, in `generated = [...]` and below it:

```toml
    "docs/flows.md",

[[collect]]
dir = "crates/shed-proto/flows"
glob = "*.md"
```

Slice 2:

```toml
[[collect]]
dir = "crates/shed-server/src/flows"
glob = "*.rs"

[[ban]]
pattern = "Tracker"
paths = ["crates/*/src/**"]
except = ["crates/shed-proto/src/flow.rs", "crates/shed-server/src/flows.rs"]
reason = "only the runner steps a flow: give it a fact"
```

Slice 5 needs no line: `machines::add`, `remove` and `save` stop being public, so the CLI cannot write the
list and the compiler says so.

`docs/flows.md` is written by `scripts/generate.sh` (`just generate`), from a `SHED_BLESS=1` run of
shed-proto's flow test, as `docs/protocol.md` is.
