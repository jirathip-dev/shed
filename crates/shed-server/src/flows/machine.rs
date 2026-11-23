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
    was: &BTreeMap<String, String>,
) -> Vec<Seen<Machine>> {
    let one = |m: &machines::Machine| {
        let entry = machines::entry(m);
        let link = match links.get(&m.name) {
            Some(l) if machines::key_changed(l) => Some(Fact::KeyMismatch),
            Some(l) if l.state == LinkState::Up => Some(Fact::LinkUp),
            Some(l) if l.state == LinkState::Down => Some(Fact::LinkDown),
            _ => None,
        };
        let mut facts = match was.get(&m.name).map(|w| w.split_once('|')) {
            None => vec![Fact::Listed],
            Some(Some((_, e))) if e != entry => vec![Fact::Change],
            Some(_) => vec![],
        };
        facts.extend(link);
        let view = format!("{link:?}|{entry}");
        Seen::new(m.name.clone(), view, facts, "listed", LISTED)
    };
    listed.iter().map(one).collect()
}
