use std::collections::BTreeMap;

use shed_proto::LinkState;
use shed_proto::flow::machine::{Action, Fact, Machine, State};

use super::{Seen, Shadow};
use crate::machines;

impl Shadow for Machine {
    const GONE: Option<Fact> = Some(Fact::Unlisted);
    const UNDONE: &'static [Action] = &[];
}

const LISTED: &[State] = &[
    State::Added,
    State::Up,
    State::Offline,
    State::Draining,
    State::KeyChanged,
];

pub(crate) fn observe(
    listed: &[machines::Machine],
    links: &BTreeMap<String, shed_proto::Link>,
    draining: &BTreeMap<String, bool>,
    was: &BTreeMap<String, String>,
) -> Vec<Seen<Machine>> {
    let one = |m: &machines::Machine| {
        let link = links.get(&m.name).map(|l| l.state);
        let mut facts: Vec<_> = (!was.contains_key(&m.name))
            .then_some(Fact::Listed)
            .into_iter()
            .collect();
        facts.extend(match link {
            Some(LinkState::Up) => Some(Fact::LinkUp),
            Some(LinkState::Down) => Some(Fact::LinkDown),
            _ => None,
        });
        let drain = draining.get(&m.name).copied();
        facts.extend(drain.map(|d| if d { Fact::Draining } else { Fact::Serving }));
        let view = format!("{link:?}:{drain:?}");
        Seen::new(m.name.clone(), view, facts, "listed", LISTED)
    };
    listed.iter().map(one).collect()
}
