//! The flow tables and their pure engine: each `flows/<flow>.md` becomes a module here with its `State`, `Fact`, `Action` and `row()`; a [`Tracker`] steps one subject through it, with time passed in as a number.

use std::collections::BTreeMap;
use std::fmt::Debug;

#[cfg(feature = "schema")]
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::ErrorCode;

include!(concat!(env!("OUT_DIR"), "/flows.rs"));

pub use link::State as Link;
pub use machine::State as Machine;
pub use terminal::State as Life;

/// The flows, as events and reports name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum FlowName {
    /// A terminal name on one shed.
    Terminal,
    /// The held link to one machine.
    Link,
    /// A machine in the main host's list.
    Machine,
    /// This shed's own drain.
    Drain,
    /// One account of one provider.
    Account,
    /// One proxy key.
    Key,
}

/// One flow's generated table.
pub trait Table: 'static {
    /// Its states, the wire values.
    type State: Copy + Eq + Debug + Serialize + DeserializeOwned + 'static;
    /// Its facts.
    type Fact: Copy + Eq + Debug + 'static;
    /// Its actions.
    type Action: Copy + Eq + Debug + 'static;
    /// Its name.
    const NAME: FlowName;
    /// The state a new subject has.
    const START: Self::State;
    /// The fact the clock feeds when a row's time ran out.
    const GAVE_UP: Self::Fact;
    /// Every row, in file order.
    const ROWS: &'static [Row<Self>];
    /// Every fact, in file order.
    const FACTS: &'static [Self::Fact];
    /// The one row for this pair.
    fn row(state: Self::State, fact: Self::Fact) -> &'static Row<Self>;
}

/// One row: in its states, on its facts, go to `next`, do `act`, and set the clock.
#[derive(Debug)]
pub struct Row<T: Table + ?Sized> {
    /// Its id, as `T9`.
    pub id: &'static str,
    /// The state after it; `None` stays.
    pub next: Option<T::State>,
    /// What it does.
    pub act: Do<T::Action>,
    /// Who owes the next fact and by when; `None` leaves the clock as it was.
    pub clock: Option<Clock>,
}

/// What a row does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do<A> {
    /// Nothing.
    Nothing,
    /// Run this action.
    Act(A),
    /// Answer the call with this error.
    Refuse(ErrorCode),
    /// It cannot happen: logged `unexpected` if it does.
    Never,
}

/// Who owes the next fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// This server: it retries its action twice before it gives up.
    Shed,
    /// The program in the terminal.
    Program,
    /// Whoever uses shed.
    Caller,
}

/// A row's clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clock {
    /// Who owes the next fact.
    pub owner: Owner,
    /// By when.
    pub within: Within,
    /// The `stuck` reason when the time runs out.
    pub or: &'static str,
}

/// When the next fact is owed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Within {
    /// Within this many seconds.
    Secs(u64),
    /// The action runs again every this many seconds, for as long as the state stays.
    Every(u64),
    /// Nothing is owed.
    Rest,
}

/// A subject's place in its flow, as every listing shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Flow<S> {
    /// Its state.
    pub state: S,
    /// When it entered `state`, unix seconds.
    pub since: u64,
    /// The row that put it there.
    pub row: String,
    /// Set while an escalation stands; cleared when the state moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stuck: Option<Stuck>,
}

/// An escalation that stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Stuck {
    /// The row's `or`.
    pub why: String,
    /// The row whose time ran out.
    pub row: String,
    /// When it gave up, unix seconds.
    pub since: u64,
}

/// Whether a flow's table only watches today's code or drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Facts step the table, no action runs, and today's code is compared.
    Shadow,
    /// The table's actions run.
    Drive,
}

/// How the table and today's code differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum Differs {
    /// They showed different states for more than 2 s.
    State,
    /// The table's row would have acted and today's code did not.
    Action,
}

/// One line of `flow-shadow.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Disagreement {
    /// When, unix seconds.
    pub at: u64,
    /// The build that wrote it.
    pub build: String,
    /// The flow.
    pub flow: FlowName,
    /// The subject.
    pub id: String,
    /// How they differ.
    pub kind: Differs,
    /// The table's row at that moment.
    pub row: String,
    /// The last fact the table took.
    pub fact: String,
    /// The state the table holds, or the action its row names.
    pub table: String,
    /// What today's code shows or did.
    pub code: String,
}

/// `flow.report` params.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct ReportParams {
    /// Disagreements from this time on, unix seconds; default the last 7 days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<u64>,
    /// Ask this machine, over its held link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

/// `flow.report` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Report {
    /// The build that answered.
    pub build: String,
    /// Disagreements from this time on, unix seconds.
    pub since: u64,
    /// One entry a flow.
    pub flows: Vec<FlowReport>,
}

/// One flow in a [`Report`]: its counts since they were first kept, and its disagreements since `since`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct FlowReport {
    /// The flow.
    pub flow: FlowName,
    /// Shadow or drive.
    pub mode: Mode,
    /// The subjects it tracks now.
    pub subjects: u64,
    /// Row id to times it fired.
    pub fired: BTreeMap<String, u64>,
    /// Facts it never took, besides `gave_up`.
    pub never_seen: Vec<String>,
    /// `never` rows that fired, to times.
    pub unexpected: BTreeMap<String, u64>,
    /// `stuck` reasons, to times.
    pub stuck: BTreeMap<String, u64>,
    /// The shadow lines, by class.
    #[serde(default)]
    pub disagreements: Vec<Class>,
}

/// Shadow lines that differ the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(JsonSchema))]
pub struct Class {
    /// How they differ.
    pub kind: Differs,
    /// The table's row.
    pub row: String,
    /// The table's state or action.
    pub table: String,
    /// What today's code showed or did.
    pub code: String,
    /// How many lines.
    pub count: u64,
    /// The first line's time.
    pub first: u64,
    /// The last line's time.
    pub last: u64,
    /// One subject it was seen on.
    pub sample: String,
}

/// What one fact or one clock did.
#[derive(Debug)]
pub struct Step<T: Table> {
    /// The row that fired.
    pub row: &'static Row<T>,
    /// The state before.
    pub from: T::State,
    /// The state after.
    pub to: T::State,
    /// The action for the runner to run.
    pub run: Option<T::Action>,
    /// The error for the handler to answer.
    pub refuse: Option<ErrorCode>,
    /// The `or` reason, on the step that gave up.
    pub stuck: Option<&'static str>,
    /// A `never` row fired.
    pub unexpected: bool,
}

struct Armed<T: Table> {
    row: &'static Row<T>,
    clock: Clock,
    due: u64,
    tries: u8,
}

/// One subject's place in its flow. Its fields are private: only `on` and `tick` move it.
pub struct Tracker<T: Table> {
    flow: Flow<T::State>,
    armed: Option<Armed<T>>,
}

impl<T: Table> Tracker<T> {
    /// A new subject, in the flow's start state.
    #[must_use]
    pub fn new(now: u64) -> Self {
        let flow = Flow {
            state: T::START,
            since: now,
            row: String::new(),
            stuck: None,
        };
        Self { flow, armed: None }
    }

    /// A subject from its saved record, its clock armed again from the row that put it there.
    #[must_use]
    pub fn resume(kept: &Flow<T::State>) -> Self {
        let mut tracker = Self {
            flow: kept.clone(),
            armed: None,
        };
        if let Some(row) = T::ROWS.iter().find(|r| r.id == kept.row) {
            tracker.arm(row, kept.since);
        }
        tracker
    }

    /// Its state.
    #[must_use]
    pub fn state(&self) -> T::State {
        self.flow.state
    }

    /// What a record saves and the wire shows.
    #[must_use]
    pub fn kept(&self) -> Flow<T::State> {
        self.flow.clone()
    }

    /// When `tick` next has something to say.
    #[must_use]
    pub fn due(&self) -> Option<u64> {
        self.armed.as_ref().map(|a| a.due)
    }

    /// Take one fact at `now`.
    pub fn on(&mut self, fact: T::Fact, now: u64) -> Step<T> {
        let row = T::row(self.flow.state, fact);
        let from = self.flow.state;
        let to = row.next.unwrap_or(from);
        if to != from {
            self.flow = Flow {
                state: to,
                since: now,
                row: row.id.to_owned(),
                stuck: None,
            };
        }
        self.arm(row, now);
        step(row, from, to)
    }

    /// The clock at `now`: a retry, an `every` run, or `stuck` and the `gave_up` row.
    pub fn tick(&mut self, now: u64) -> Option<Step<T>> {
        let armed = self.armed.as_mut().filter(|a| a.due <= now)?;
        let (row, state) = (armed.row, self.flow.state);
        let retry = armed.clock.owner == Owner::Shed && matches!(row.act, Do::Act(_));
        match armed.clock.within {
            Within::Every(secs) => armed.due = now + secs,
            Within::Secs(secs) if retry && armed.tries < 2 => {
                armed.tries += 1;
                armed.due = now + secs;
            }
            _ => {
                let why = armed.clock.or;
                self.armed = None;
                let stuck = Stuck {
                    why: why.to_owned(),
                    row: row.id.to_owned(),
                    since: now,
                };
                self.flow.stuck = Some(stuck);
                let mut gave_up = self.on(T::GAVE_UP, now);
                gave_up.stuck = Some(why);
                return Some(gave_up);
            }
        }
        Some(step(row, state, state))
    }

    fn arm(&mut self, row: &'static Row<T>, now: u64) {
        let Some(clock) = row.clock else { return };
        self.armed = match clock.within {
            Within::Secs(secs) | Within::Every(secs) => Some(Armed {
                row,
                clock,
                due: now + secs,
                tries: 0,
            }),
            Within::Rest => None,
        };
    }
}

fn step<T: Table>(row: &'static Row<T>, from: T::State, to: T::State) -> Step<T> {
    let act = row.act;
    Step {
        row,
        from,
        to,
        run: if let Do::Act(a) = act { Some(a) } else { None },
        refuse: if let Do::Refuse(e) = act {
            Some(e)
        } else {
            None
        },
        stuck: None,
        unexpected: act == Do::Never,
    }
}
