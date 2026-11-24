use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

use shed_client::{Calls as _, Shed};
use shed_server::proto::{
    ErrorCode, ExecParams, FileParams, LinkState, PROTOCOL, Proc, StatsParams,
};

use crate::common::{self, Server, temp_dir};
use crate::{fake_hosts, ok};

#[test]
fn machine_add_links_file_mode_exec_env_procs_and_top_then_machine_rm() {
    let root = temp_dir("split");
    fake_hosts(&root);
    let _remote = Server::start(&root.join("remote"));
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap()
    );
    let _local = Server::with_env(&root.join("local"), "3600", &[("PATH", &path)]);
    let shed = Shed::at(shed_server::Paths::in_dir(&root.join("local")).socket);

    shed.machine_add("box", "user@box.example").expect("add");
    shed.machine_add("box", "user@box.example")
        .expect("the same pair twice is one machine");
    let taken = shed.machine_add("box", "other.example").unwrap_err();
    assert_eq!(taken.code(), Some(ErrorCode::InvalidParams), "{taken}");
    common::eventually("the link to box", || {
        shed.links().expect("links")["box"].state == LinkState::Up
    });
    let link = shed.links().expect("links").remove("box").expect("box");
    let uname = |flag| {
        let out = Command::new("uname").arg(flag).output().expect("uname");
        String::from_utf8(out.stdout)
            .expect("utf-8")
            .trim()
            .to_owned()
    };
    assert_eq!(link.down_since, None);
    assert_eq!(link.hello.map(|h| h.protocol), Some(PROTOCOL));
    assert_eq!(link.os, Some(uname("-s").to_lowercase()));
    assert_eq!(link.arch, Some(uname("-m")));
    assert!(link.home.is_some_and(|h| h.is_absolute()));

    for machine in [None, Some("box")] {
        exec_env_and_file_mode(&shed, &root, machine);
    }
    procs_and_top(&shed, &root);
    shed.machine_rm("box").expect("rm");
    assert!(!shed.links().expect("links").contains_key("box"));
    shed.machine_rm("box").expect("rm twice is a no-op");
}

fn exec_env_and_file_mode(shed: &Shed, root: &Path, machine: Option<&str>) {
    let machine = machine.map(str::to_owned);
    let ran = shed
        .exec(&ExecParams {
            cmd: ["sh", "-c", "printf %s \"$SPLIT_VAR\""]
                .map(str::to_owned)
                .into(),
            env: BTreeMap::from([("SPLIT_VAR".into(), "a b".into())]),
            machine: machine.clone(),
            ..ExecParams::default()
        })
        .expect("exec");
    assert_eq!(ran.stdout, b"a b", "{machine:?}");

    let file = root.join(format!("remote/secret-{}", machine.is_some()));
    let write = |mode| {
        shed.file_write(&FileParams {
            path: file.clone(),
            data: Some(b"s3cret".to_vec()),
            mode,
            machine: machine.clone(),
            ..FileParams::default()
        })
    };
    let mode = || {
        std::fs::metadata(&file)
            .expect("written")
            .permissions()
            .mode()
            & 0o7777
    };
    write(Some(0o600)).expect("write 0600");
    assert_eq!(
        (mode(), std::fs::read(&file).expect("read")),
        (0o600, b"s3cret".to_vec())
    );
    write(None).expect("write again");
    assert_eq!(mode(), 0o600, "the mode is kept without one");
    write(Some(0o640)).expect("write 0640");
    assert_eq!(mode(), 0o640);
    let refused = write(Some(0o17_777)).unwrap_err();
    assert_eq!(refused.code(), Some(ErrorCode::Io), "{refused}");
}

fn procs_and_top(shed: &Shed, root: &Path) {
    let cwd = root.join("remote");
    let cwd = cwd.to_str().expect("utf-8");
    let run = ["--on", "box", "run", "--name", "t", "--cwd", cwd, "--"];
    ok(root, &[&run[..], &["sh", "-c", "sleep 101; true"]].concat());
    let mut procs: Vec<Proc> = Vec::new();
    common::eventually("sh and its sleep", || {
        procs = shed.procs("box/t").expect("procs");
        procs.len() == 2
    });
    let [sh, sleep] = &procs[..] else {
        unreachable!()
    };
    assert_eq!((sh.depth, sh.own), (0, true), "{procs:?}");
    assert_eq!(
        (sleep.depth, sleep.own, sleep.ppid),
        (1, false, sh.pid),
        "{procs:?}"
    );
    assert!(sleep.command.contains("sleep 101"), "{procs:?}");

    let stats = shed
        .stats(&StatsParams {
            top: Some(100_000),
            machine: Some("box".into()),
            ..StatsParams::default()
        })
        .expect("stats on box");
    let s = &stats.now;
    assert!(
        s.mem_total_mb > 0 && s.mem_free_mb <= s.mem_total_mb,
        "{s:?}"
    );
    assert!(
        s.disk_total_mb >= s.disk_free_mb && s.disk_total_mb > 0,
        "{s:?}"
    );
    assert_eq!(s.mem_level.is_some(), cfg!(target_os = "macos"), "{s:?}");
    let held = stats.top_mem.iter().find(|p| p.pid == sleep.pid);
    assert_eq!(held.and_then(|p| p.terminal.as_deref()), Some("box/t"));
    assert!(held.is_some_and(|p| p.command.contains("sleep 101")));
    assert_eq!(stats.top_cpu.len(), stats.top_mem.len());
    let one = shed
        .stats(&StatsParams {
            top: Some(1),
            ..StatsParams::default()
        })
        .expect("stats here");
    assert_eq!((one.top_cpu.len(), one.top_mem.len()), (1, 1));
    let rss: Vec<u64> = stats.top_mem.iter().map(|p| p.rss_mb).collect();
    assert!(rss.windows(2).all(|w| w[0] >= w[1]), "most first: {rss:?}");

    ok(root, &["kill", "box/t"]);
    busy_cpu_ms(shed, root);
}

fn busy_cpu_ms(shed: &Shed, root: &Path) {
    let busy = "i=0; while [ $i -lt 1000000 ]; do i=$((i+1)); done";
    ok(root, &["run", "--name", "busy", "--", "sh", "-c", busy]);
    let cpu_ms = || {
        shed.procs("busy")
            .expect("procs")
            .first()
            .map_or(0, |p| p.cpu_ms)
    };
    let mut first = 0;
    common::eventually("busy uses CPU", || {
        first = cpu_ms();
        first > 0
    });
    common::eventually("its cpu_ms grows", || cpu_ms() > first);
    ok(root, &["kill", "busy"]);
}
