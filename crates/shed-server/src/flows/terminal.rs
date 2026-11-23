use std::collections::BTreeMap;

use shed_core::proctree::ProcessInfo;
use shed_proto::flow::terminal::{Action, Fact, State as Life, Terminal};
use shed_proto::{State, TerminalInfo};

use super::{Seen, Shadow};

impl Shadow for Terminal {
    const GONE: Option<Fact> = Some(Fact::Rm);
    const UNDONE: &'static [Action] = &[Action::EndTree];
}

fn live(reported: &str) -> (Fact, &'static str, &'static [Life]) {
    match reported {
        "done" => (Fact::Done, "running+done", &[Life::Done]),
        "hibernated" => (Fact::Slept, "running+hibernated", &[Life::Asleep]),
        "" => (Fact::Ready, "running", &[Life::Starting, Life::Running]),
        _ => (Fact::Working, "running", &[Life::Starting, Life::Running]),
    }
}

pub(crate) fn observe(
    infos: &[TerminalInfo],
    leaders: &BTreeMap<String, i32>,
    procs: Option<&[ProcessInfo]>,
    was: &BTreeMap<String, String>,
) -> Vec<Seen<Terminal>> {
    let one = |t: &TerminalInfo| {
        let was = was.get(&t.name);
        let mut facts = Vec::new();
        if was.is_none() {
            facts.push(Fact::Run);
        }
        let (seen, code, agrees) = match t.state {
            State::Running => {
                let reported = t.reported.as_ref().map_or("", |r| r.state.as_str());
                let (fact, code, agrees) = live(reported);
                facts.push(fact);
                (format!("running:{reported}"), code, agrees)
            }
            State::Exited => {
                if was.is_none_or(|w| !w.starts_with("exited")) {
                    facts.push(if t.exit.is_some() {
                        Fact::Exit
                    } else {
                        Fact::Lost
                    });
                }
                let leader = leaders.get(&t.name);
                let runs = |p: &[ProcessInfo], l| p.iter().any(|q| q.pgid == l && !q.zombie);
                let empty = procs.is_some_and(|p| leader.is_none_or(|l| !runs(p, *l)));
                facts.extend(empty.then_some(Fact::Empty));
                let agrees: &[Life] = &[Life::Exited, Life::Gone];
                (format!("exited:{empty}"), "exited", agrees)
            }
        };
        let id = t.name.clone();
        Seen {
            id,
            seen,
            facts,
            code,
            agrees,
        }
    };
    infos.iter().map(one).collect()
}
