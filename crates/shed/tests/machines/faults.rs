use std::path::Path;
use std::process::{Output, Stdio};
use std::time::Duration;

use shed_server::clock;

use crate::common::{Server, temp_dir};
use crate::{SHED, fault, link, linked, ok, script, shed_on};

const CALL_BOUND: Duration = Duration::from_secs(10);

fn within(bound: Duration, root: &Path, args: &[&str]) -> (Output, Duration) {
    let started = clock::now();
    let mut child = shed_on(root, args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("shed runs");
    while child.try_wait().expect("try_wait").is_none() {
        if started.elapsed() > bound {
            child.kill().ok();
            child.wait().ok();
            panic!("shed {args:?} still blocked after {bound:?}");
        }
        clock::sleep(Duration::from_millis(20));
    }
    let took = started.elapsed();
    (child.wait_with_output().expect("shed ends"), took)
}

fn until(what: &str, bound: Duration, mut check: impl FnMut() -> bool) -> Duration {
    let started = clock::now();
    while !check() {
        assert!(started.elapsed() < bound, "no {what} within {bound:?}");
        clock::sleep(Duration::from_millis(50));
    }
    started.elapsed()
}

fn shed_at(root: &Path, target: &str, dir: &str) {
    let dir = root.join(dir);
    let exec = format!("exec '{SHED}' --dir '{}' \"$@\"\n", dir.display());
    script(&root.join(format!("{target}/shed")), &exec);
}

fn two(root: &Path) -> [Server; 2] {
    let (remote, local) = linked(root);
    shed_at(root, "b2.example", "remote");
    ok(root, &["machine", "add", "b2", "b2.example"]);
    until("b2's link", CALL_BOUND, || {
        link(root, "b2")["state"] == "up"
    });
    let cwd = root.join("remote");
    let cwd = cwd.to_str().expect("utf-8 dir");
    let run = ["--on", "b2", "run", "--name", "t1", "--cwd", cwd];
    ok(root, &[&run[..], &["--", "sleep", "100"]].concat());
    [remote, local]
}

fn up(root: &Path, machine: &str) -> bool {
    link(root, machine)["state"] == "up"
}

fn refused(out: &Output, code: &str) {
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "{out:?}");
    assert!(stderr.contains(code), "{stderr}");
}

#[test]
fn a_hung_link_never_blocks_a_call_to_another_machine_and_comes_back_when_it_clears() {
    let root = temp_dir("fault-hang");
    let _servers = two(&root);
    fault(&root, "user@box.example hang\n");
    let bound = CALL_BOUND + Duration::from_secs(3);
    let (out, took) = within(bound, &root, &["--on", "b2", "ls"]);
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(listed.contains("t1"), "{listed} after {took:?}");
    let (out, _) = within(Duration::from_secs(3), &root, &["ls"]);
    let listed = String::from_utf8_lossy(&out.stdout);
    assert!(
        listed.contains("b2/t1") && !listed.contains("box/t1"),
        "{listed}"
    );
    until("box marked not up", Duration::from_secs(2), || {
        !up(&root, "box")
    });
    assert!(up(&root, "b2"), "the other link stays up");
    fault(&root, "");
    until("box back up", Duration::from_secs(15), || up(&root, "box"));
}

#[test]
fn up_or_down_is_one_fact_a_call_that_meets_a_hung_link_fails_in_bound_and_the_link_reads_down() {
    let root = temp_dir("fault-one-fact");
    let _servers = two(&root);
    fault(&root, "user@box.example hang\n");
    let bound = CALL_BOUND + Duration::from_secs(3);
    let (out, _) = within(bound, &root, &["get", "box/t1"]);
    refused(&out, "NO_LINK: box");
    until("box marked not up", Duration::from_secs(2), || {
        !up(&root, "box")
    });
    let (out, _) = within(Duration::from_secs(2), &root, &["get", "box/t1"]);
    refused(&out, "NO_LINK: box");
    fault(&root, "");
}

#[test]
fn latency_slows_every_call_and_never_drops_the_link() {
    let root = temp_dir("fault-latency");
    let _servers = two(&root);
    fault(&root, "user@box.example latency 300\n");
    let (out, took) = within(Duration::from_secs(5), &root, &["--on", "box", "ls"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("t1"));
    assert!(
        took >= Duration::from_millis(300),
        "{took:?}: the latency never applied"
    );
    assert!(up(&root, "box"));
}

#[test]
fn packet_loss_delays_calls_and_every_one_answers() {
    let root = temp_dir("fault-loss");
    let _servers = two(&root);
    fault(&root, "user@box.example loss 50\n");
    for _ in 0..10 {
        let (out, _) = within(Duration::from_secs(5), &root, &["--on", "box", "ls"]);
        assert!(String::from_utf8_lossy(&out.stdout).contains("t1"));
    }
    assert!(up(&root, "box"));
}

#[test]
fn a_stall_shorter_than_the_bound_is_waited_out_on_the_same_link() {
    let root = temp_dir("fault-stall");
    let _servers = two(&root);
    let ssh = crate::box_ssh(&root);
    fault(&root, "user@box.example stall 3\n");
    let (out, took) = within(CALL_BOUND, &root, &["--on", "box", "ls"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("t1"));
    assert!(
        took >= Duration::from_secs(2),
        "{took:?}: the stall never applied"
    );
    assert!(up(&root, "box"));
    assert_eq!(crate::box_ssh(&root), ssh, "no reconnect for a stall");
}

#[test]
fn a_cut_link_fails_calls_at_once_and_reconnects_when_the_host_answers() {
    let root = temp_dir("fault-cut");
    let _servers = two(&root);
    fault(&root, "user@box.example cut\n");
    until("box marked not up", Duration::from_secs(3), || {
        !up(&root, "box")
    });
    let (out, _) = within(Duration::from_secs(2), &root, &["get", "box/t1"]);
    refused(&out, "NO_LINK: box");
    fault(&root, "");
    until("box back up", Duration::from_secs(12), || up(&root, "box"));
}

#[test]
fn a_host_gone_mid_run_is_noticed_in_bound_and_rejoins_when_back() {
    let root = temp_dir("fault-gone");
    let _servers = two(&root);
    fault(&root, "user@box.example gone 1\n");
    let bound = CALL_BOUND + Duration::from_secs(3);
    let (out, _) = within(bound, &root, &["ls"]);
    assert!(String::from_utf8_lossy(&out.stdout).contains("b2/t1"));
    until("box marked not up", Duration::from_secs(2), || {
        !up(&root, "box")
    });
    let (out, _) = within(Duration::from_secs(2), &root, &["get", "box/t1"]);
    refused(&out, "NO_LINK: box");
    fault(&root, "");
    until("box back up", Duration::from_secs(15), || up(&root, "box"));
}

#[test]
fn an_address_change_moves_the_link_to_the_new_address() {
    let root = temp_dir("fault-moved");
    let _servers = two(&root);
    let _moved = Server::start(&root.join("moved"));
    shed_at(&root, "new.example", "moved");
    let there = root.join("moved");
    let there = there.to_str().expect("utf-8 dir");
    let run = [
        "run",
        "--name",
        "moved-here",
        "--cwd",
        there,
        "--",
        "sleep",
        "100",
    ];
    let out = std::process::Command::new(SHED)
        .arg("--dir")
        .arg(there)
        .args(run)
        .output();
    assert!(out.expect("shed runs").status.success());
    fault(&root, "user@box.example moved new.example\n");
    let bound = CALL_BOUND + Duration::from_secs(3);
    let (out, _) = within(bound, &root, &["get", "box/t1"]);
    refused(&out, "NO_LINK: box");
    until(
        "the link at the new address",
        Duration::from_secs(15),
        || up(&root, "box") && ok(&root, &["--on", "box", "ls"]).contains("moved-here"),
    );
}

#[test]
fn a_changed_host_key_keeps_the_link_down_until_it_is_fixed() {
    let root = temp_dir("fault-rekey");
    let _servers = two(&root);
    fault(&root, "user@box.example cut\n");
    until("box marked not up", Duration::from_secs(3), || {
        !up(&root, "box")
    });
    fault(&root, "user@box.example rekey\n");
    let ssh = crate::box_ssh(&root);
    until("a refused reconnect", Duration::from_secs(8), || {
        crate::box_ssh(&root) > ssh
    });
    assert!(!up(&root, "box"), "{}", link(&root, "box"));
    let (out, _) = within(Duration::from_secs(2), &root, &["get", "box/t1"]);
    refused(&out, "NO_LINK: box");
    fault(&root, "");
    until("box back up", Duration::from_secs(12), || up(&root, "box"));
}

#[test]
#[ignore = "TODO(#309): a finished terminal still holds its name"]
fn a_finished_terminal_is_not_ambiguous_its_name_runs_again() {
    let root = temp_dir("symptom-finished");
    let _servers = two(&root);
    ok(&root, &["kill", "b2/t1"]);
    let cwd = root.join("remote");
    let cwd = cwd.to_str().expect("utf-8 dir");
    let run = [
        "--on", "b2", "run", "--name", "t1", "--cwd", cwd, "--", "true",
    ];
    let (out, _) = within(CALL_BOUND, &root, &run);
    assert!(out.status.success(), "{out:?}");
}

#[test]
#[ignore = "TODO(#309): a machine written to the config file is not linked until a machine call"]
fn the_machine_list_is_reconciled_with_the_config_file() {
    let root = temp_dir("symptom-machines");
    let _servers = two(&root);
    let config = shed_server::Paths::in_dir(&root.join("local")).config;
    let text = std::fs::read_to_string(&config).expect("the config");
    std::fs::write(
        &config,
        text.replace("b2 = \"b2.example\"\n", "b3 = \"b2.example\"\n"),
    )
    .expect("the config, by another writer");
    let (out, _) = within(CALL_BOUND, &root, &["get", "b3/t1"]);
    assert!(out.status.success(), "{out:?}");
    let (out, _) = within(CALL_BOUND, &root, &["get", "b2/t1"]);
    refused(&out, "NO_LINK: b2");
}
