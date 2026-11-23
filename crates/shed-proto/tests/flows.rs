//! The gap check the build runs on each `flows/*.md` (the shipped tables pass, a gap or an untimed busy state fails the build, `docs/flows.md` is their chart: `just generate`), and the engine stepping the generated tables with time as a number.
#![cfg(test)]

#[path = "../../../build/flows.rs"]
mod flows;

use std::path::Path;

use shed_proto::ErrorCode;
use shed_proto::flow::link::{Action as LinkAction, Fact as LinkFact, Link as LinkTable};
use shed_proto::flow::terminal::{Action, Fact, Terminal};
use shed_proto::flow::{Flow, Life, Link, Machine, Tracker};

fn dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn terminal_without(line: &str, with: &str) -> Vec<String> {
    let text = std::fs::read_to_string(dir().join("flows/terminal.md")).unwrap();
    assert!(
        text.contains(line),
        "the fixture's row is not in terminal.md"
    );
    flows::parse("terminal", &text.replace(line, with)).1
}

#[test]
fn the_six_shipped_tables_build_with_no_fault() {
    let loaded = flows::load(&dir().join("flows")).unwrap();
    let names: Vec<&str> = loaded.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        ["account", "drain", "key", "link", "machine", "terminal"]
    );
    let pairs: usize = loaded.iter().map(|f| f.states.len() * f.facts.len()).sum();
    let rows: usize = loaded.iter().map(|f| f.rows.len()).sum();
    assert_eq!((pairs, rows), (265, 106));
}

#[test]
fn a_table_with_one_row_removed_fails_naming_the_gap() {
    let faults = terminal_without(
        "| T26 | exited | empty | gone | - | caller | rest | - |\n",
        "",
    );
    assert!(
        faults.contains(&"flows/terminal.md: gap: (exited, empty) has no row".to_owned()),
        "{faults:?}"
    );
}

#[test]
fn a_row_entering_a_busy_state_with_no_time_fails() {
    let faults = terminal_without(
        "| T1 | free | run, replace | starting | start | program | 180 | start-slow |",
        "| T1 | free | run, replace | starting | start | program | rest | - |",
    );
    assert_eq!(
        faults,
        ["flows/terminal.md: T1: starting is no resting state, so the row needs a time"]
    );
}

#[test]
fn flows_md_matches_the_tables() {
    let want = flows::chart(&flows::load(&dir().join("flows")).unwrap());
    let path = dir().join("../../docs/flows.md");
    if std::env::var_os("SHED_BLESS").is_some() {
        std::fs::write(&path, &want).unwrap();
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(have == want, "docs/flows.md is stale; run `just generate`");
}

fn running() -> Tracker<Terminal> {
    let mut t = Tracker::<Terminal>::new(0);
    assert_eq!(t.on(Fact::Run, 0).run, Some(Action::Start));
    assert_eq!(t.on(Fact::Ready, 1).row.id, "T4");
    t
}

#[test]
fn a_start_the_program_never_finishes_is_stuck_at_once_and_held() {
    let mut t = Tracker::<Terminal>::new(0);
    let step = t.on(Fact::Run, 0);
    assert_eq!((step.row.id, step.to), ("T1", Life::Starting));
    assert!(t.tick(179).is_none());
    let gave_up = t.tick(180).unwrap();
    assert_eq!((gave_up.stuck, gave_up.row.id), (Some("start-slow"), "T8"));
    assert_eq!(t.state(), Life::Starting);
    assert_eq!(
        t.kept().stuck.map(|s| (s.why, s.row)),
        Some(("start-slow".into(), "T1".into()))
    );
    assert_eq!(t.due(), None);
    assert!(t.on(Fact::Ready, 200).stuck.is_none());
    assert_eq!((t.state(), t.kept().stuck), (Life::Running, None));
}

#[test]
fn shed_retries_its_own_action_twice_then_the_gave_up_row_holds_it() {
    let mut t = running();
    let step = t.on(Fact::Exit, 100);
    assert_eq!(
        (step.to, step.run, t.due()),
        (Life::Exited, Some(Action::EndTree), Some(110))
    );
    for at in [110, 120] {
        let retry = t.tick(at).unwrap();
        assert_eq!(
            (retry.row.id, retry.run, retry.stuck),
            ("T9", Some(Action::EndTree), None)
        );
    }
    let gave_up = t.tick(130).unwrap();
    assert_eq!(
        (gave_up.row.id, gave_up.stuck, gave_up.run),
        ("T33", Some("tree-left"), None)
    );
    assert_eq!(t.state(), Life::Exited);
    assert_eq!(t.on(Fact::Empty, 131).to, Life::Gone);
    assert_eq!(
        t.kept(),
        Flow {
            state: Life::Gone,
            since: 131,
            row: "T26".into(),
            stuck: None
        }
    );
}

#[test]
fn an_every_row_runs_again_with_no_count_until_the_state_moves() {
    let mut t = Tracker::<LinkTable>::new(0);
    assert_eq!(t.on(LinkFact::DialFailed, 5).run, Some(LinkAction::Dial));
    for at in [65, 125, 185, 245] {
        let again = t.tick(at).unwrap();
        assert_eq!(
            (again.run, again.stuck, again.to),
            (Some(LinkAction::Dial), None, Link::Down)
        );
    }
    assert_eq!(t.on(LinkFact::Dialed, 250).run, Some(LinkAction::Learn));
    assert_eq!((t.state(), t.due()), (Link::Up, None));
}

#[test]
fn a_refused_or_impossible_fact_leaves_the_state() {
    let mut t = Tracker::<Terminal>::new(0);
    assert_eq!(t.on(Fact::Input, 1).refuse, Some(ErrorCode::NoSuchTerminal));
    let never = t.on(Fact::Exit, 2);
    assert!(never.unexpected);
    assert_eq!((never.to, t.kept().since), (Life::Free, 0));
    let mut live = running();
    assert_eq!(live.on(Fact::Run, 5).refuse, Some(ErrorCode::NameTaken));
    assert_eq!(live.state(), Life::Running);
}

#[test]
fn a_resumed_subject_keeps_its_state_and_its_clock() {
    let mut t = Tracker::<Terminal>::new(0);
    t.on(Fact::Run, 100);
    let kept = t.kept();
    assert_eq!(serde_json::to_value(&kept).unwrap()["state"], "starting");
    let back: Flow<Life> = serde_json::from_value(serde_json::to_value(&kept).unwrap()).unwrap();
    let resumed = Tracker::<Terminal>::resume(&back);
    assert_eq!(
        (resumed.state(), resumed.due()),
        (Life::Starting, Some(280))
    );
    assert_eq!(
        serde_json::to_value(Machine::KeyChanged).unwrap(),
        "key_changed"
    );
}
