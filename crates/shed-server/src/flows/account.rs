use shed_proto::AccountState;
use shed_proto::flow::account::{Account, Action, Fact, State};

use super::{Seen, Shadow};

impl Shadow for Account {
    const GONE: Option<Fact> = None;
    const UNDONE: &'static [Action] = &[];
}

pub(crate) fn observe(accounts: &[(u32, shed_proto::Account)]) -> Vec<Seen<Account>> {
    let one = |(runs, a): &(u32, shed_proto::Account)| {
        let (facts, code, agrees): (_, _, &'static [State]) = match a.state {
            AccountState::Ready => (vec![Fact::Reset, Fact::In], "ready", &[State::SignedIn]),
            AccountState::Limited if *runs == 0 => (
                vec![Fact::Limited, Fact::Moved],
                "limited",
                &[State::AtLimit, State::Switched],
            ),
            AccountState::Limited => (
                vec![Fact::Limited],
                "limited+runs",
                &[State::AtLimit, State::Switched],
            ),
            AccountState::SignedOut | AccountState::Drifted => (
                vec![Fact::Reset, Fact::Out],
                "signed-out",
                &[State::SignedOut],
            ),
        };
        let (id, seen) = (format!("{}/{}", a.provider, a.name), code.to_owned());
        Seen {
            id,
            seen,
            facts,
            code,
            agrees,
        }
    };
    accounts.iter().map(one).collect()
}
