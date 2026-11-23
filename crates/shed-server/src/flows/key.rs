use std::collections::BTreeMap;

use shed_proto::flow::key::{Action, Fact, Key, State};
use shed_proto::{KeyState, ProxyKey, TerminalInfo};

use super::{Seen, Shadow};

impl Shadow for Key {
    const GONE: Option<Fact> = None;
    const UNDONE: &'static [Action] = &[Action::Find];
}

pub(crate) fn observe(
    keys: &[ProxyKey],
    there: &BTreeMap<String, Option<Vec<TerminalInfo>>>,
) -> Vec<Seen<Key>> {
    let one = |k: &ProxyKey| {
        let machine = k.terminal.split_once('/').map(|(m, _)| m);
        let listed = machine.and_then(|m| there.get(m)?.as_ref());
        let fact = match listed.map(|ts| ts.iter().find(|t| t.name == k.terminal)) {
            None => Fact::Unknown,
            Some(Some(t)) if t.state == shed_proto::State::Running => Fact::Alive,
            Some(_) => Fact::Gone,
        };
        let (code, agrees): (_, &'static [State]) = match k.state {
            KeyState::Live => ("live", &[State::Issued, State::InUse]),
            KeyState::Ended | KeyState::Revoked => ("ended", &[State::Ended]),
        };
        let mut facts = vec![fact];
        facts.extend((k.state == KeyState::Revoked).then_some(Fact::Revoke));
        let (id, seen) = (k.id.clone(), format!("{code}:{fact:?}"));
        Seen {
            id,
            seen,
            facts,
            code,
            agrees,
        }
    };
    keys.iter().map(one).collect()
}
