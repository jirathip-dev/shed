# Shapes (#309)

Sketches for the impl lanes: names may change in review, the fields and their meaning may not.

## The data file

`crates/shed-proto/flows/<flow>.md`, one per flow. [tables.md](tables.md) shows all six.

```
subject: a terminal name on one shed; each run of it has an id
states: free starting running asleep done exited gone
start: free
rest: free running asleep done gone

| fact | source | read as |
|---|---|---|
| exit | process | the reaper has its exit status |

| id | state | fact | next | do | owner | within | or |
|---|---|---|---|---|---|---|---|
| T9 | starting, running, asleep, done | exit, lost | exited | end-tree | shed | 10 | tree-left |
```

| key or column | values |
|---|---|
| `states` | names in `snake_case`; they are the wire values |
| `start` | the state a new subject has |
| `rest` | the states where nothing is owed |
| `source` | one of the closed list in `build/flows.rs` |
| `next` | a state, or `=` |
| `do` | `-`, an action name, `refuse <ERROR_CODE>`, `never` |
| `owner` | `shed`, `program`, `caller`, or `=` |
| `within` | seconds, `every <seconds>`, `rest`, or `=` |
| `or` | a `stuck` reason in `kebab-case`, `-`, or `=` |

## The engine: `crates/shed-proto/src/flow.rs`

```rust
/// One flow's generated table.
pub trait Table: 'static {
    type State: Copy + Eq + Serialize + DeserializeOwned + 'static;
    type Fact: Copy + Eq + 'static;
    type Action: Copy + Eq + 'static;
    const NAME: FlowName;
    const START: Self::State;
    const GAVE_UP: Self::Fact;
    const ROWS: &'static [Row<Self>];
    fn row(state: Self::State, fact: Self::Fact) -> &'static Row<Self>;   // generated: a match with no catch-all
}

pub struct Row<T: Table + ?Sized> {
    pub id: &'static str,                    // "T9"
    pub next: Option<T::State>,              // None: stays
    pub act: Do<T::Action>,
    pub clock: Option<Clock>,                // None: the clock is left as it was
}

pub enum Do<A> { Nothing, Act(A), Refuse(ErrorCode), Never }
pub enum Owner { Shed, Program, Caller }
pub struct Clock { pub owner: Owner, pub within: Within, pub or: &'static str }
pub enum Within { Secs(u32), Every(u32), Rest }

/// One subject's place in its flow. Its fields are private: only `on` and `tick` move it.
pub struct Tracker<T: Table> { /* state, since, row, armed: Option<(Clock, due, tries)>, stuck */ }

impl<T: Table> Tracker<T> {
    pub fn new(now: u64) -> Self;
    pub fn resume(kept: &Flow<T::State>) -> Self;        // from a saved record
    pub fn state(&self) -> T::State;
    pub fn kept(&self) -> Flow<T::State>;                // what a record saves and the wire shows
    pub fn on(&mut self, fact: T::Fact, now: u64) -> Step<T>;
    pub fn tick(&mut self, now: u64) -> Option<Step<T>>; // a clock ran out: a retry, or `stuck` and the `gave_up` row
    pub fn due(&self) -> Option<u64>;                    // when `tick` next has something to say
}

pub struct Step<T: Table> {
    pub row: &'static Row<T>,
    pub from: T::State,
    pub to: T::State,
    pub run: Option<T::Action>,              // for the runner to hand to the flow's `act`
    pub refuse: Option<ErrorCode>,           // for the handler to answer
    pub stuck: Option<&'static str>,         // set once, on the step that gave up
    pub unexpected: bool,                    // a `never` row fired: logged, counted in the report
}

pub enum FlowName { Terminal, Link, Machine, Drain, Account, Key }
```

Generated per flow, from its file: `shed_proto::flow::terminal::{State, Fact, Action, Terminal}` and so on.
`terminal::State` is exported as `Life`, `link::State` as `Link`, `machine::State` as `Machine`.

## The wire: one shape per flow

```rust
/// A subject's place in its flow, as every listing shows it.
pub struct Flow<S> {
    pub state: S,
    pub since: u64,                          // when it entered `state`, unix seconds
    pub row: String,                         // the row that put it there: "T16"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Stuck>,                // set while an escalation stands; cleared when the state moves
}
pub struct Stuck { pub why: String, pub row: String, pub since: u64 }   // why: the row's `or`
```

| flow | read with | field | new beside it |
|---|---|---|---|
| terminal | `ls`, `get`, and what `run`, `kill`, `rm` return | `TerminalInfo.flow: Flow<Life>` | `TerminalInfo.id: String`, the run's id |
| machine | `machine.ls` (new) | `MachineInfo.flow: Flow<Machine>` | `by`, `drain` |
| link | `machine.ls` | `MachineInfo.link.flow: Flow<Link>` | - |
| drain | `machine.stats` | `MachineStats.draining: Option<Draining>`, as #307 has it | - |
| account | `account.ls`, `account.status` | `Account.flow: Flow<account::State>` | - |
| key | `proxy.keys` | `ProxyKey.flow: Flow<key::State>` | `ProxyKey.run: String`, `why` |

```rust
/// `machine.ls` result: every machine in the list, and those removed in the last 24 h.
pub struct MachineInfo {
    pub name: String,
    pub target: String,                      // with #307's slice 1: port, pinned key, identity file
    pub flow: Flow<Machine>,
    pub by: Option<String>,                  // who last added or removed it: a caller's name, or "file"
    pub link: Option<LinkInfo>,              // none once removed or key_changed
    pub drain: Option<Draining>,             // the last `machine.stats` answer's, #307
}
/// Today's `Link` with `state`, `since` and `down_since` folded into `flow` and `last_up`.
pub struct LinkInfo {
    pub flow: Flow<Link>,
    pub last_up: Option<u64>,                // when it last left `up` or `slow`; none while up
    pub error: Option<String>,               // the last dial's error, one line
    pub hello: Option<Hello>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub home: Option<PathBuf>,
    pub proxy: Option<u16>,
}

// `run`
pub struct RunParams {
    // …as today, and:
    pub id: Option<String>,                  // `[A-Za-z0-9._-]{1,64}`; the same id again answers the same run. Default: shed makes one
    pub replace: bool,                       // take the name from a holder that is exited or gone
}

// `NAME_TAKEN`, `STILL_RUNNING`, `NO_LINK`: the refusal says what it found
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,                 // NAME_TAKEN: { "id": …, "flow": Flow<Life> }; NO_LINK: { "flow": Flow<Link> }
}

// events: two kinds for every flow, on the stream that exists
pub enum EventKind {
    // …as today, and:
    Flow(Moved),
    Stuck(GaveUp),
}
pub struct Moved { pub flow: FlowName, pub id: String, pub from: String, pub to: String, pub fact: String, pub row: String }
pub struct GaveUp { pub flow: FlowName, pub id: String, pub state: String, pub row: String, pub why: String, pub tries: u8 }
```

An event's `name` is the subject: a terminal's name (with `id` its run), a machine's name for the machine and
link flows, `<agent>/<account>` for an account, the key's id for a key. Events of flows other than terminal
carry no labels and go to streams that asked for none.

`machine.stats` counts terminals by flow state: `starting`, `running`, `asleep`, `done`, `exited`, `gone`.

Kept for one protocol window and derived from the flow, then dropped in slice 8: `TerminalInfo.state`
(`running` for starting to done, `exited` for exited and gone), `machine.links` and `Link`, the old
`AccountState` (`ready`, `limited`, `signed-out`; `drifted` stays readable as `Account.drift`) and `KeyState`.

## Records

| flow | its `Flow<S>` is saved in | by |
|---|---|---|
| terminal | the terminal's entry in `state.json`, with its `id` | the registry's saver, as today |
| link | memory: every link starts `down` at a server start | - |
| machine | `machines.json` beside the state file: state, since, by; the list file keeps name and target | the machine flow's actions |
| drain | `drain.json` beside the state file, as #307 has it | the drain flow's action |
| account | the account's reading in the store | shed-accounts' store |
| key | `proxy-keys.json`, with the run id | shed-accounts' keys |

A record's `Flow<S>` sits in a wrapper whose setter is private to the runner's file.

## The shadow log and the report

```rust
/// One line of `flow-shadow.jsonl`.
pub struct Disagreement {
    pub at: u64,
    pub build: String,
    pub flow: FlowName,
    pub id: String,                          // the subject
    pub kind: Differs,                       // state | action
    pub row: String,                         // the table's row at that moment
    pub fact: String,
    pub table: String,                       // the state the table holds, or the action its row names
    pub code: String,                        // what today's code shows or did
}

/// `flow.report` params and result; `shed flow report` prints it.
pub struct ReportParams { pub since: Option<u64>, pub machine: Option<String> }
pub struct Report { pub build: String, pub since: u64, pub flows: Vec<FlowReport> }
pub struct FlowReport {
    pub flow: FlowName,
    pub mode: Mode,                          // shadow | drive
    pub subjects: u64,
    pub fired: BTreeMap<String, u64>,        // row id → times
    pub never_seen: Vec<String>,             // facts with no observation
    pub unexpected: BTreeMap<String, u64>,   // `never` rows that fired
    pub stuck: BTreeMap<String, u64>,        // `or` reason → times
    pub disagreements: Vec<Class>,           // shadow only
}
pub struct Class { pub kind: Differs, pub row: String, pub table: String, pub code: String, pub count: u64, pub first: u64, pub last: u64, pub sample: String }
```

The counts are kept in `flow-counts.json` beside the state file and survive a restart and a self-update.
`flow.report` stays after the take-overs; only `disagreements` and `flow-shadow.jsonl` go with slice 8.
