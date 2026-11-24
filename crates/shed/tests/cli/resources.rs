use std::io::{BufRead as _, BufReader};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

use crate::common::{Server, temp_dir};
use crate::{SHED, shed_in, stdout};
use shed_server::clock;

#[test]
fn a_child_allocating_50_mb_fires_a_resources_event_and_tops_the_list() {
    let dir = temp_dir("resources");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::sampling(&dir, "0.2");
    let mut events = Command::new(SHED)
        .args(["--dir", d, "events", "--label", "test=mem"])
        .stdout(Stdio::piped())
        .spawn()
        .expect("shed events");
    let mut lines = BufReader::new(events.stdout.take().expect("stdout")).lines();
    let mut next = || -> Value {
        let line = lines.next().expect("an event").expect("a line");
        serde_json::from_str(&line).expect("event is JSON")
    };
    stdout(&shed_in(
        &dir,
        &["run", "--name", "small", "--cwd", d, "--", "sleep", "100"],
    ));
    let script = r#"sh -c 'x=$(head -c 52428800 /dev/zero | tr "\0" x); sleep 100'"#;
    let run = ["run", "--name", "mem", "--label", "test=mem", "--cwd", d];
    stdout(&shed_in(
        &dir,
        &[&run[..], &["--", "sh", "-c", script]].concat(),
    ));
    assert_eq!(next()["event"], "started");
    let deadline = clock::now() + clock::PATIENCE;
    loop {
        assert!(clock::now() < deadline, "no 50 MB resources event");
        let event = next();
        assert_eq!(
            (&event["name"], &event["event"]),
            (&json!("mem"), &json!("resources"))
        );
        if event["rss_mb"].as_u64().expect("rss_mb") >= 50 {
            break;
        }
    }
    events.kill().ok();
    events.wait().ok();

    let get: Value = serde_json::from_str(&stdout(&shed_in(&dir, &["get", "mem"]))).expect("json");
    assert!(get["resources"]["rss_mb"].as_u64() >= Some(50), "{get}");
    let top = stdout(&shed_in(&dir, &["top"]));
    let names: Vec<&str> = top
        .lines()
        .skip(1)
        .filter(|l| !l.contains(" awake, "))
        .filter_map(|l| l.split(' ').next())
        .collect();
    assert_eq!(names, ["mem", "small"], "{top}");
    let totals = top.lines().last().unwrap_or_default();
    assert!(
        totals.starts_with("2 awake, 0 hibernated; agent 0M, work "),
        "{top}"
    );
}

#[test]
fn timing_report() {
    const CALLS: usize = 30;
    let dir = temp_dir("timing");
    let d = dir.to_str().expect("utf-8 dir");
    let _server = Server::start(&dir);
    stdout(&shed_in(
        &dir,
        &[
            "run",
            "--name",
            "t",
            "--cwd",
            d,
            "--",
            "sh",
            "-c",
            "exec cat >/dev/null",
        ],
    ));
    let calls = [
        &["ls"][..],
        &["get", "t"],
        &["read", "t"],
        &["send", "t", "x"],
        &["keys", "t", "enter"],
        &["exec", "--", "true"],
    ];
    for args in calls {
        let mut ms: Vec<f64> = (0..CALLS)
            .map(|_| {
                let start = clock::now();
                stdout(&shed_in(&dir, args));
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        let median = ms[CALLS / 2];
        let note = if median < 20.0 { "ok" } else { "SLOW" };
        eprintln!(
            "timing: shed {:<6} median {median:5.1} ms, p90 {:5.1} ms, max {:5.1} ms over {CALLS} calls (target < 20 ms, soft): {note}",
            args.join(" "),
            ms[CALLS * 9 / 10],
            ms[CALLS - 1],
        );
    }
    for (program, signal) in [(&["sleep", "600"][..], 143), (&["sh"], 129)] {
        let mut ms = [vec![], vec![], vec![]];
        for i in 0..8 {
            let name = format!("life{i}");
            let mut run = vec!["run", "--name", &name, "--cwd", d, "--"];
            run.extend(program);
            let steps = [run, vec!["kill", &name], vec!["rm", &name]];
            for (step, args) in ms.iter_mut().zip(&steps) {
                let start = clock::now();
                let said = stdout(&shed_in(&dir, args));
                step.push(start.elapsed().as_secs_f64() * 1000.0);
                if args[0] == "kill" {
                    assert_eq!(said, format!("{name} exited {signal}\n"), "{program:?}");
                }
            }
        }
        let [run, kill, rm] = ms.map(|mut m| {
            m.sort_by(f64::total_cmp);
            m[4]
        });
        let program = program.join(" ");
        eprintln!(
            "timing: lifecycle {program:<9} run {run:5.1} ms, kill {kill:5.1} ms, rm {rm:5.1} ms (median of 8, kill < 500 ms, soft)"
        );
    }
}
