use std::collections::BTreeMap;

use shed_proto::LinkState;
use shed_proto::flow::link::{Action, Fact, Link, State};

use super::{Seen, Shadow};

impl Shadow for Link {
    const GONE: Option<Fact> = None;
    const UNDONE: &'static [Action] = &[Action::Cut, Action::Fresh];
}

pub(crate) fn observe(
    report: &BTreeMap<String, shed_proto::Link>,
    was: &BTreeMap<String, String>,
) -> Vec<Seen<Link>> {
    let one = |(id, link): (&String, &shed_proto::Link)| {
        let dropped = was.get(id).is_some_and(|w| w == "reconnecting");
        let (fact, code, agrees): (_, _, &'static [State]) = match link.state {
            LinkState::Up => (Fact::Dialed, "up", &[State::Up, State::Slow]),
            LinkState::Reconnecting => (
                Fact::Closed,
                "reconnecting",
                &[State::Hung, State::Reconnecting],
            ),
            LinkState::Down if dropped => (Fact::Refused, "down", &[State::Down]),
            LinkState::Down => (Fact::DialFailed, "down", &[State::Down]),
        };
        let (id, seen, facts) = (id.clone(), code.to_owned(), vec![fact]);
        Seen {
            id,
            seen,
            facts,
            code,
            agrees,
        }
    };
    report.iter().map(one).collect()
}
