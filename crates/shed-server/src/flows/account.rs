use shed_proto::AccountState;
use shed_proto::flow::account::{Account, Action, Fact, State};

use super::{Seen, Shadow};

impl Shadow for Account {
    const GONE: Option<Fact> = None;
    const UNDONE: &'static [Action] = &[];
}

pub(crate) fn observe(accounts: &[(u32, shed_proto::Account)]) -> Vec<Seen<Account>> {
    let one = |(runs, a): &(u32, shed_proto::Account)| {
        let limited: &[State] = &[State::AtLimit, State::Switched];
        let (facts, code, agrees) = match a.state {
            AccountState::Ready => (vec![Fact::Reset, Fact::In], "ready", &[State::SignedIn][..]),
            AccountState::Limited if *runs == 0 => {
                (vec![Fact::Limited, Fact::Moved], "limited", limited)
            }
            AccountState::Limited => (vec![Fact::Limited], "limited+runs", limited),
            AccountState::SignedOut | AccountState::Drifted => (
                vec![Fact::Reset, Fact::Out],
                "signed-out",
                &[State::SignedOut][..],
            ),
        };
        Seen::new(
            format!("{}/{}", a.provider, a.name),
            code.to_owned(),
            facts,
            code,
            agrees,
        )
    };
    accounts.iter().map(one).collect()
}
