use serde_json::Value;

use crate::common::{Server, eventually, temp_dir};
use crate::{alive, ok, pids, run_sh, run_shell};

#[test]
fn kill_ends_only_its_own_tree_and_the_other_terminals_survive() {
    let dir = temp_dir("rel-kill");
    let _server = Server::start(&dir);
    for name in ["a", "b", "c"] {
        run_sh(&dir, name, "sleep 100 & echo child=$!; wait");
    }
    let [a, b, c] = ["a", "b", "c"].map(|n| pids(&dir, n));
    ok(&dir, &["kill", "b"]);
    eventually("b's tree gone", || !alive(&b.0) && !alive(&b.1));
    for (name, (pid, child)) in [("a", &a), ("c", &c)] {
        assert!(alive(pid) && alive(child), "{name} survives");
        let info: Value = serde_json::from_str(&ok(&dir, &["--json", "get", name])).expect("json");
        assert_eq!(info["state"], "running", "{name}");
    }
}

#[test]
fn kill_ends_an_interactive_shell_without_waiting_out_the_grace() {
    let dir = temp_dir("rel-shell");
    let _server = Server::start(&dir);
    run_shell(&dir, "sh");
    assert_eq!(ok(&dir, &["kill", "sh"]), "sh exited 129\n");
}

#[test]
fn kill_lets_a_sigterm_handler_run_before_the_hangup() {
    let dir = temp_dir("rel-tidy");
    let _server = Server::start(&dir);
    run_sh(
        &dir,
        "tidy",
        "trap 'exit 7' TERM; echo ready; while :; do sleep 1; done",
    );
    eventually("ready", || ok(&dir, &["read", "tidy"]).contains("ready"));
    assert_eq!(ok(&dir, &["kill", "tidy"]), "tidy exited 7\n");
}
